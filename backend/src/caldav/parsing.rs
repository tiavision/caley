use anyhow::bail;
use chrono::{FixedOffset, NaiveDate, NaiveDateTime, TimeDelta, TimeZone as _, Utc};
use quick_xml::Reader;
use quick_xml::events::Event as XmlEvent;
use rrule::RRuleSet;

use crate::xml_util::resolve_ref;

use super::types::{Alarm, Attendee, Calendar, Event, PartStat, RecurrenceInfo, SyncResult};

struct DateProperty {
    value: String,
    tzid: Option<String>,
    resolved: ResolvedTz,
}

struct RawIcalEvent {
    uid: String,
    summary: String,
    description: String,
    location: String,
    dtstart: NaiveDateTime,
    dtstart_is_date: bool,
    dtstart_tz: ResolvedTz,
    duration: TimeDelta,
    rrule: Option<String>,
    rdates: Vec<DateProperty>,
    exdates: Vec<DateProperty>,
    recurrence_id: Option<NaiveDateTime>,
    recurrence_id_tz: ResolvedTz,
    alarms: Vec<Alarm>,
    attendees: Vec<Attendee>,
}

/// A resolved iCalendar TZID, produced by `resolve_tzid` and reused by every
/// downstream formatting/matching/expansion step for a given date value.
#[derive(Clone, Debug)]
enum ResolvedTz {
    /// Resolved to a real IANA zone, either matched directly against the raw
    /// TZID or via the Windows/CLDR timezone name table.
    Named(chrono_tz::Tz),
    /// Resolved via an embedded VTIMEZONE block to a fixed UTC offset, with
    /// no IANA equivalent found. `canonical` is the raw TZID string, kept
    /// only as a display label.
    Fixed {
        offset: FixedOffset,
        canonical: String,
    },
    /// The raw value carried a `Z` suffix (RFC 5545 UTC form).
    Utc,
    /// No TZID and no `Z` suffix (RFC 5545 floating time), or the TZID could
    /// not be resolved by any fallback in `resolve_tzid`. The wall-clock time
    /// is preserved as-is and rendered in the viewer's local timezone.
    Floating,
}

impl ResolvedTz {
    /// The string to store as `Event.tz` for display purposes. Empty means
    /// floating time.
    fn display_tz(&self) -> String {
        match self {
            ResolvedTz::Named(tz) => tz.name().to_string(),
            ResolvedTz::Fixed { canonical, .. } => canonical.clone(),
            ResolvedTz::Utc => "UTC".to_string(),
            ResolvedTz::Floating => String::new(),
        }
    }

    /// Whether the value denotes a fixed instant (anything but floating time).
    fn is_anchored(&self) -> bool {
        !matches!(self, ResolvedTz::Floating)
    }
}

/// Resolve a raw ICS TZID string to a usable timezone, trying in order:
/// 1. Direct IANA parse (the common case: Google/Apple/Thunderbird-style
///    producers already use IANA names).
/// 2. A Windows/CLDR timezone name table lookup (handles Outlook/Exchange
///    TZIDs like "Eastern Standard Time").
/// 3. The event's own embedded VTIMEZONE block, if any, evaluated for `at`.
/// 4. Floating — the wall-clock time is preserved as-is.
///
/// `value_is_utc` reflects whether the raw date value carried a `Z` suffix;
/// it decides between UTC and floating when no TZID resolves.
fn resolve_tzid(
    tzid: Option<&str>,
    timezones: &[ical::parser::ical::component::IcalTimeZone],
    at: NaiveDateTime,
    value_is_utc: bool,
) -> ResolvedTz {
    let Some(tzid) = tzid else {
        return if value_is_utc {
            ResolvedTz::Utc
        } else {
            ResolvedTz::Floating
        };
    };

    if let Ok(tz) = tzid.parse::<chrono_tz::Tz>() {
        return ResolvedTz::Named(tz);
    }

    if let Some(iana) = super::windows_tz::windows_to_iana(tzid) {
        if let Ok(tz) = iana.parse::<chrono_tz::Tz>() {
            return ResolvedTz::Named(tz);
        }
    }

    if let Some(vtz) = timezones
        .iter()
        .find(|vtz| vtimezone_tzid(vtz).as_deref() == Some(tzid))
    {
        if let Some(offset) = resolve_vtimezone_offset(vtz, at) {
            return ResolvedTz::Fixed {
                offset,
                canonical: tzid.to_string(),
            };
        }
    }

    eprintln!("caldav: unable to resolve TZID {tzid:?}, treating time as floating");
    if value_is_utc {
        ResolvedTz::Utc
    } else {
        ResolvedTz::Floating
    }
}

fn vtimezone_tzid(vtz: &ical::parser::ical::component::IcalTimeZone) -> Option<String> {
    vtz.properties
        .iter()
        .find(|p| p.name == "TZID")
        .and_then(|p| p.value.clone())
}

/// Compute the UTC offset in effect at `at` (a naive local datetime in the
/// VTIMEZONE's own wall clock) from its STANDARD/DAYLIGHT transitions.
/// Each transition has a DTSTART anchor and either recurs yearly via an
/// RRULE (e.g. `FREQ=YEARLY;BYDAY=1SU;BYMONTH=11`) or occurs once. This
/// picks whichever transition (across all STANDARD/DAYLIGHT rules) most
/// recently took effect on or before `at`.
fn resolve_vtimezone_offset(
    vtz: &ical::parser::ical::component::IcalTimeZone,
    at: NaiveDateTime,
) -> Option<FixedOffset> {
    let mut best: Option<(NaiveDateTime, FixedOffset)> = None;
    let mut earliest: Option<(NaiveDateTime, FixedOffset)> = None;

    for transition in &vtz.transitions {
        let dtstart_raw = transition
            .properties
            .iter()
            .find(|p| p.name == "DTSTART")
            .and_then(|p| p.value.as_deref())?;
        let transition_dtstart = parse_dtstart(dtstart_raw)?;
        let offset_to_raw = transition
            .properties
            .iter()
            .find(|p| p.name == "TZOFFSETTO")
            .and_then(|p| p.value.as_deref())?;
        let offset_to = parse_ical_utc_offset(offset_to_raw)?;
        let rrule_val = transition
            .properties
            .iter()
            .find(|p| p.name == "RRULE")
            .and_then(|p| p.value.clone());

        let occurrences: Vec<NaiveDateTime> = match rrule_val {
            Some(rrule_str) => {
                let rrule_input = format!(
                    "DTSTART:{}Z\nRRULE:{}\n",
                    transition_dtstart.format("%Y%m%dT%H%M%S"),
                    rrule_str
                );
                match rrule_input.parse::<RRuleSet>() {
                    Ok(set) => {
                        let window_start =
                            rrule::Tz::UTC.from_utc_datetime(&(at - TimeDelta::days(370)));
                        let window_end =
                            rrule::Tz::UTC.from_utc_datetime(&(at + TimeDelta::days(370)));
                        set.after(window_start)
                            .before(window_end)
                            .all(64)
                            .dates
                            .into_iter()
                            .map(|dt| dt.naive_utc())
                            .collect()
                    }
                    Err(_) => vec![transition_dtstart],
                }
            }
            None => vec![transition_dtstart],
        };

        for occ in occurrences {
            if earliest.is_none_or(|(e, _)| occ < e) {
                earliest = Some((occ, offset_to));
            }
            if occ <= at && best.is_none_or(|(b, _)| occ > b) {
                best = Some((occ, offset_to));
            }
        }
    }

    best.or(earliest).map(|(_, offset)| offset)
}

/// Parse an RFC 5545 §3.3.14 UTC-offset value (`[+-]HHMM[SS]`) as used in
/// VTIMEZONE's `TZOFFSETFROM`/`TZOFFSETTO` properties.
fn parse_ical_utc_offset(s: &str) -> Option<FixedOffset> {
    let s = s.trim();
    let (sign, digits) = match s.as_bytes().first() {
        Some(b'+') => (1, &s[1..]),
        Some(b'-') => (-1, &s[1..]),
        _ => (1, s),
    };
    if digits.len() < 4 {
        return None;
    }
    let hours: i32 = digits.get(0..2)?.parse().ok()?;
    let mins: i32 = digits.get(2..4)?.parse().ok()?;
    let secs: i32 = digits.get(4..6).and_then(|s| s.parse().ok()).unwrap_or(0);
    FixedOffset::east_opt(sign * (hours * 3600 + mins * 60 + secs))
}

/// Convert a naive local datetime under a fixed offset into naive UTC.
fn fixed_offset_to_utc_naive(dt: NaiveDateTime, offset: FixedOffset) -> NaiveDateTime {
    match offset.from_local_datetime(&dt) {
        chrono::LocalResult::Single(local_dt) => local_dt.naive_utc(),
        chrono::LocalResult::Ambiguous(local_dt, _) => local_dt.naive_utc(),
        chrono::LocalResult::None => dt,
    }
}

pub(crate) fn resolve_url(current_url: &str, href: &str) -> String {
    if href.starts_with("http://") || href.starts_with("https://") {
        href.to_string()
    } else if href.starts_with('/') {
        let scheme_end = current_url.find("://").map(|i| i + 3).unwrap_or(0);
        let after_scheme = &current_url[scheme_end..];
        let host = after_scheme.split('/').next().unwrap_or("");
        let scheme = &current_url[..scheme_end.saturating_sub(3)];
        format!("{scheme}://{host}{href}")
    } else {
        let base_trimmed = current_url.trim_end_matches('/');
        let href_trimmed = href.trim_start_matches('/');
        format!("{base_trimmed}/{href_trimmed}")
    }
}

pub(crate) fn extract_hrefs(xml: &str, element_local_name: &str) -> anyhow::Result<Vec<String>> {
    let mut reader = Reader::from_str(xml);

    let mut in_target = false;
    let mut in_href = false;
    let mut hrefs = Vec::new();
    let mut current_href = String::new();
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf)? {
            XmlEvent::Start(e) => {
                let local = e.local_name();
                let local = local.into_inner();
                if local == element_local_name {
                    in_target = true;
                } else if in_target && local == "href" {
                    in_href = true;
                    current_href.clear();
                }
            }
            XmlEvent::Text(e) => {
                if in_href {
                    current_href.push_str(&e.xml10_content());
                }
            }
            XmlEvent::GeneralRef(r) => {
                if in_href {
                    current_href.push_str(&resolve_ref(&r));
                }
            }
            XmlEvent::End(e) => {
                let local = e.local_name();
                let local = local.into_inner();
                if local == "href" {
                    if in_href && !current_href.is_empty() {
                        hrefs.push(current_href.trim().to_string());
                    }
                    in_href = false;
                    current_href = String::new();
                } else if local == element_local_name {
                    in_target = false;
                }
            }
            XmlEvent::Eof => break,
            _ => {}
        }
    }

    if hrefs.is_empty() {
        bail!("element '{element_local_name}' not found in response")
    }
    Ok(hrefs)
}

pub(crate) fn extract_text_prop(xml: &str, prop_local_name: &str) -> Option<String> {
    let mut reader = Reader::from_str(xml);
    let mut in_prop = false;
    let mut text = String::new();
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf).ok()? {
            XmlEvent::Start(e) => {
                let local = e.local_name().into_inner().to_string();
                if local == prop_local_name {
                    in_prop = true;
                    text.clear();
                }
            }
            XmlEvent::Text(e) => {
                if in_prop {
                    text.push_str(&e.xml10_content());
                }
            }
            XmlEvent::GeneralRef(r) => {
                if in_prop {
                    text.push_str(&resolve_ref(&r));
                }
            }
            XmlEvent::End(e) => {
                let local = e.local_name().into_inner().to_string();
                if local == prop_local_name && in_prop {
                    let trimmed = text.trim().to_string();
                    if !trimmed.is_empty() {
                        return Some(trimmed);
                    }
                    in_prop = false;
                }
            }
            XmlEvent::Eof => break,
            _ => {}
        }
    }
    None
}

pub(super) fn parse_all_calendars(xml: &str, base_url: &str) -> anyhow::Result<Vec<Calendar>> {
    let mut reader = Reader::from_str(xml);

    let base_url_resolved = resolve_url(base_url, "");
    let mut calendars = Vec::new();
    let mut current_href: Option<String> = None;
    let mut current_displayname: Option<String> = None;
    let mut current_color: Option<String> = None;
    let mut in_href = false;
    let mut in_displayname = false;
    let mut in_color = false;
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf)? {
            XmlEvent::Start(e) => {
                let local = e.local_name();
                let local = local.into_inner();
                match local {
                    "href" => {
                        in_href = true;
                        current_href = Some(String::new());
                    }
                    "displayname" => {
                        in_displayname = true;
                        current_displayname = Some(String::new());
                    }
                    "calendar-color" => {
                        in_color = true;
                        current_color = Some(String::new());
                    }
                    _ => {}
                }
            }
            XmlEvent::Text(e) => {
                let text = e.xml10_content();
                if in_href {
                    current_href.get_or_insert_default().push_str(&text);
                } else if in_displayname {
                    current_displayname.get_or_insert_default().push_str(&text);
                } else if in_color {
                    current_color.get_or_insert_default().push_str(&text);
                }
            }
            XmlEvent::GeneralRef(r) => {
                let text = resolve_ref(&r);
                if in_href {
                    current_href.get_or_insert_default().push_str(&text);
                } else if in_displayname {
                    current_displayname.get_or_insert_default().push_str(&text);
                } else if in_color {
                    current_color.get_or_insert_default().push_str(&text);
                }
            }
            XmlEvent::End(e) => {
                let local = e.local_name();
                let local = local.into_inner();
                match local {
                    "href" => in_href = false,
                    "displayname" => in_displayname = false,
                    "calendar-color" => in_color = false,
                    "response" => {
                        if let Some(href) = current_href
                            .as_deref()
                            .map(str::trim)
                            .filter(|h| !h.is_empty())
                        {
                            let resolved = resolve_url(base_url, href);
                            if resolved.trim_end_matches('/')
                                != base_url_resolved.trim_end_matches('/')
                            {
                                let name = current_displayname
                                    .as_deref()
                                    .map(str::trim)
                                    .filter(|n| !n.is_empty())
                                    .map(str::to_string)
                                    .unwrap_or_else(|| {
                                        resolved
                                            .trim_end_matches('/')
                                            .rsplit('/')
                                            .next()
                                            .unwrap_or("unknown")
                                            .to_string()
                                    });
                                calendars.push(Calendar {
                                    name,
                                    href: resolved,
                                    color: current_color
                                        .as_deref()
                                        .map(str::trim)
                                        .filter(|c| !c.is_empty())
                                        .map(str::to_string),
                                });
                            }
                        }
                        current_href = None;
                        current_displayname = None;
                        current_color = None;
                    }
                    _ => {}
                }
            }
            XmlEvent::Eof => break,
            _ => {}
        }
    }

    Ok(calendars)
}

pub(crate) fn unescape_ical_text(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\\' && i + 1 < chars.len() {
            match chars[i + 1] {
                'n' | 'N' => {
                    result.push('\n');
                    i += 2;
                }
                ',' => {
                    result.push(',');
                    i += 2;
                }
                ';' => {
                    result.push(';');
                    i += 2;
                }
                '\\' => {
                    result.push('\\');
                    i += 2;
                }
                _ => {
                    result.push(chars[i]);
                    i += 1;
                }
            }
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }
    result
}

fn parse_raw_events(ical_data: &str) -> Vec<RawIcalEvent> {
    let calendar = match ical::IcalParser::new(ical_data.as_bytes()).next() {
        Some(Ok(c)) => c,
        _ => return vec![],
    };

    let timezones = calendar.timezones;
    let mut results = Vec::new();

    for event in calendar.events {
        let mut uid: Option<String> = None;
        let mut summary: Option<String> = None;
        let mut description: Option<String> = None;
        let mut location: Option<String> = None;
        let mut dtstart_value: Option<String> = None;
        let mut dtstart_tzid: Option<String> = None;
        let mut dtstart_is_date = false;
        let mut dtend_value: Option<String> = None;
        let mut dtend_tzid: Option<String> = None;
        let mut duration_str: Option<String> = None;
        let mut rrule: Option<String> = None;
        let mut rdates: Vec<DateProperty> = Vec::new();
        let mut exdates: Vec<DateProperty> = Vec::new();
        let mut recurrence_id_value: Option<String> = None;
        let mut recurrence_id_tzid: Option<String> = None;
        let mut attendees: Vec<Attendee> = Vec::new();

        for property in event.properties {
            match property.name.as_str() {
                "UID" => uid = property.value,
                "SUMMARY" => summary = property.value.map(|v| unescape_ical_text(&v)),
                "DESCRIPTION" => description = property.value.map(|v| unescape_ical_text(&v)),
                "LOCATION" => location = property.value.map(|v| unescape_ical_text(&v)),
                "DTSTART" => {
                    dtstart_value = property.value.clone();
                    let mut found_tzid = None;
                    let mut found_value_date = false;
                    if let Some(params) = &property.params {
                        for (key, vals) in params {
                            if key == "TZID" {
                                if let Some(first_val) = vals.first() {
                                    found_tzid = Some(first_val.clone());
                                }
                            }
                            if key == "VALUE" {
                                if let Some(first_val) = vals.first() {
                                    if first_val == "DATE" {
                                        found_value_date = true;
                                    }
                                }
                            }
                        }
                    }
                    dtstart_tzid = found_tzid;
                    if !found_value_date {
                        if let Some(ref val) = property.value {
                            if !val.contains('T') {
                                dtstart_is_date = true;
                            }
                        }
                    } else {
                        dtstart_is_date = true;
                    }
                }
                "DTEND" => {
                    dtend_value = property.value;
                    if let Some(params) = &property.params {
                        for (key, vals) in params {
                            if key == "TZID" {
                                if let Some(first_val) = vals.first() {
                                    dtend_tzid = Some(first_val.clone());
                                }
                            }
                        }
                    }
                }
                "DURATION" => duration_str = property.value,
                "RRULE" => rrule = property.value,
                "RDATE" => {
                    if let Some(val) = property.value {
                        let mut rdate_tzid = None;
                        if let Some(params) = &property.params {
                            for (key, vals) in params {
                                if key == "TZID" {
                                    if let Some(first_val) = vals.first() {
                                        rdate_tzid = Some(first_val.clone());
                                    }
                                }
                            }
                        }
                        for v in val.split(',') {
                            let v = v.trim();
                            if !v.is_empty() {
                                rdates.push(DateProperty {
                                    value: v.to_string(),
                                    tzid: rdate_tzid.clone(),
                                    resolved: ResolvedTz::Floating,
                                });
                            }
                        }
                    }
                }
                "EXDATE" => {
                    if let Some(val) = property.value {
                        let mut exdate_tzid = None;
                        if let Some(params) = &property.params {
                            for (key, vals) in params {
                                if key == "TZID" {
                                    if let Some(first_val) = vals.first() {
                                        exdate_tzid = Some(first_val.clone());
                                    }
                                }
                            }
                        }
                        for v in val.split(',') {
                            let v = v.trim();
                            if !v.is_empty() {
                                exdates.push(DateProperty {
                                    value: v.to_string(),
                                    tzid: exdate_tzid.clone(),
                                    resolved: ResolvedTz::Floating,
                                });
                            }
                        }
                    }
                }
                "RECURRENCE-ID" => {
                    recurrence_id_value = property.value;
                    if let Some(params) = property.params {
                        for (key, vals) in params {
                            if key == "TZID" {
                                if let Some(first_val) = vals.first() {
                                    recurrence_id_tzid = Some(first_val.clone());
                                }
                            }
                        }
                    }
                }
                "ATTENDEE" => {
                    let raw = property.value.as_deref().unwrap_or("");
                    let email = raw.strip_prefix("mailto:").unwrap_or(raw).to_string();
                    if !email.is_empty() {
                        let mut cn = String::new();
                        let mut partstat = PartStat::default();
                        if let Some(params) = &property.params {
                            for (key, vals) in params {
                                match key.as_str() {
                                    "CN" => {
                                        cn = vals
                                            .first()
                                            .map(|s| unescape_ical_text(s))
                                            .unwrap_or_default()
                                    }
                                    "PARTSTAT" => {
                                        partstat = PartStat::from_ical(
                                            vals.first().map(|s| s.as_str()).unwrap_or(""),
                                        )
                                    }
                                    _ => {}
                                }
                            }
                        }
                        attendees.push(Attendee {
                            cn,
                            email,
                            partstat,
                        });
                    }
                }
                _ => {}
            }
        }

        let uid = uid.unwrap_or_default();
        let summary = summary.unwrap_or_default();
        let description = description.unwrap_or_default();
        let location = location.unwrap_or_default();
        let raw_dtstart = match dtstart_value.as_deref() {
            Some(s) => s,
            None => continue,
        };
        let dtstart = match parse_dtstart(raw_dtstart) {
            Some(dt) => dt,
            None => continue,
        };
        let dtstart_tz = resolve_tzid(
            dtstart_tzid.as_deref(),
            &timezones,
            dtstart,
            raw_dtstart.ends_with('Z'),
        );

        let duration = if let Some(ref dtend_raw) = dtend_value {
            match parse_dtstart(dtend_raw) {
                Some(dtend) => {
                    let dtend_tz = resolve_tzid(
                        dtend_tzid.as_deref(),
                        &timezones,
                        dtend,
                        dtend_raw.ends_with('Z'),
                    );
                    if dtstart_tz.is_anchored() && dtend_tz.is_anchored() {
                        // Instant delta: honors a DTEND whose TZID (or UTC
                        // form) differs from DTSTART's.
                        TimeDelta::seconds(
                            naive_to_utc_timestamp(dtend, &dtend_tz)
                                - naive_to_utc_timestamp(dtstart, &dtstart_tz),
                        )
                    } else {
                        // Floating on either side: nominal wall-clock delta.
                        dtend - dtstart
                    }
                }
                None => continue,
            }
        } else if let Some(ref dur) = duration_str {
            parse_ical_duration(dur).unwrap_or_else(|| {
                if dtstart_is_date {
                    TimeDelta::days(1)
                } else {
                    TimeDelta::zero()
                }
            })
        } else if dtstart_is_date {
            TimeDelta::days(1)
        } else {
            TimeDelta::zero()
        };

        let recurrence_id = recurrence_id_value.as_ref().and_then(|v| parse_dtstart(v));
        let recurrence_id_tz = match recurrence_id {
            Some(rid) => resolve_tzid(
                recurrence_id_tzid.as_deref(),
                &timezones,
                rid,
                recurrence_id_value
                    .as_deref()
                    .is_some_and(|v| v.ends_with('Z')),
            ),
            None => ResolvedTz::Floating,
        };

        for rdate in rdates.iter_mut() {
            let at = parse_dtstart(&rdate.value).unwrap_or(dtstart);
            rdate.resolved = resolve_tzid(
                rdate.tzid.as_deref(),
                &timezones,
                at,
                rdate.value.ends_with('Z'),
            );
        }
        for exdate in exdates.iter_mut() {
            let at = parse_dtstart(&exdate.value).unwrap_or(dtstart);
            exdate.resolved = resolve_tzid(
                exdate.tzid.as_deref(),
                &timezones,
                at,
                exdate.value.ends_with('Z'),
            );
        }

        let mut alarms: Vec<Alarm> = Vec::new();
        for alarm in &event.alarms {
            let Some(trigger) = alarm.properties.iter().find(|p| p.name == "TRIGGER") else {
                continue;
            };
            let Some(ref value) = trigger.value else {
                continue;
            };

            let mut is_absolute = false;
            let mut is_end_relative = false;
            if let Some(params) = &trigger.params {
                for (key, vals) in params {
                    if key == "VALUE" && vals.iter().any(|v| v == "DATE-TIME") {
                        is_absolute = true;
                    }
                    if key == "RELATED" && vals.iter().any(|v| v == "END") {
                        is_end_relative = true;
                    }
                }
            }

            if is_absolute {
                alarms.push(Alarm {
                    trigger_type: "absolute".to_string(),
                    minutes: None,
                    datetime: Some(value.clone()),
                });
            } else if let Some(mins) = parse_relative_trigger(value) {
                alarms.push(Alarm {
                    trigger_type: if is_end_relative {
                        "end".to_string()
                    } else {
                        "start".to_string()
                    },
                    minutes: Some(mins),
                    datetime: None,
                });
            }
        }

        results.push(RawIcalEvent {
            uid,
            summary,
            description,
            location,
            dtstart,
            dtstart_is_date,
            dtstart_tz,
            duration,
            rrule,
            rdates,
            exdates,
            recurrence_id,
            recurrence_id_tz,
            alarms,
            attendees,
        });
    }

    results
}

fn parse_relative_trigger(s: &str) -> Option<i64> {
    let negative = s.starts_with('-');
    let s = s.trim_start_matches(['+', '-']);
    let s = s.strip_prefix('P')?;
    let minutes = if let Some(rest) = s.strip_prefix('T') {
        if let Some(h) = rest.strip_suffix('H') {
            h.parse::<i64>().ok()? * 60
        } else if let Some(m) = rest.strip_suffix('M') {
            m.parse::<i64>().ok()?
        } else if let Some(sec) = rest.strip_suffix('S') {
            sec.parse::<i64>().ok()? / 60
        } else {
            return None;
        }
    } else if let Some(d) = s.strip_suffix('D') {
        d.parse::<i64>().ok()? * 1440
    } else {
        return None;
    };
    Some(if negative { -minutes } else { minutes })
}

fn parse_ical_duration(s: &str) -> Option<TimeDelta> {
    let s = s.trim();
    let negative = s.starts_with('-');
    let s = s.trim_start_matches('+').trim_start_matches('-');

    if !s.starts_with('P') {
        return None;
    }
    let s = &s[1..];

    let mut total_secs: i64 = 0;

    if let Some(week_pos) = s.find('W') {
        let weeks: i64 = s[..week_pos].parse().ok()?;
        total_secs += weeks * 7 * 86400;
        return Some(if negative {
            TimeDelta::seconds(-total_secs)
        } else {
            TimeDelta::seconds(total_secs)
        });
    }

    let (date_part, time_part) = if let Some(t_pos) = s.find('T') {
        (&s[..t_pos], &s[t_pos + 1..])
    } else {
        (s, "")
    };

    if let Some(day_pos) = date_part.find('D') {
        let days: i64 = date_part[..day_pos].parse().ok()?;
        total_secs += days * 86400;
    }

    let mut remaining = time_part;
    if let Some(hour_pos) = remaining.find('H') {
        let hours: i64 = remaining[..hour_pos].parse().ok()?;
        total_secs += hours * 3600;
        remaining = &remaining[hour_pos + 1..];
    }
    if let Some(min_pos) = remaining.find('M') {
        let mins: i64 = remaining[..min_pos].parse().ok()?;
        total_secs += mins * 60;
        remaining = &remaining[min_pos + 1..];
    }
    if let Some(sec_pos) = remaining.find('S') {
        let secs: i64 = remaining[..sec_pos].parse().ok()?;
        total_secs += secs;
    }

    Some(if negative {
        TimeDelta::seconds(-total_secs)
    } else {
        TimeDelta::seconds(total_secs)
    })
}

pub(super) fn parse_and_expand_events(
    raw_ical: &str,
    start: NaiveDate,
    end: NaiveDate,
) -> Vec<Event> {
    expand_events(parse_raw_events(raw_ical), start, end)
}

fn expand_events(raw_events: Vec<RawIcalEvent>, start: NaiveDate, end: NaiveDate) -> Vec<Event> {
    use std::collections::HashMap;

    let mut groups: HashMap<String, Vec<RawIcalEvent>> = HashMap::new();
    for event in raw_events {
        groups.entry(event.uid.clone()).or_default().push(event);
    }

    let mut events = Vec::new();

    for (_, group) in groups {
        let master = group.iter().find(|e| e.recurrence_id.is_none());
        let overrides: Vec<&RawIcalEvent> =
            group.iter().filter(|e| e.recurrence_id.is_some()).collect();

        let Some(master) = master else { continue };

        if master.rrule.is_some() || !master.rdates.is_empty() {
            let recurrence = master.rrule.as_ref().map(|r| parse_rrule_info(r));
            let expanded = expand_recurring(master, start, end);

            let master_dtstart_str = if master.dtstart_is_date {
                format_datetime_with_tz(master.dtstart, &ResolvedTz::Floating)
                    .split('T')
                    .next()
                    .unwrap_or("")
                    .to_string()
            } else {
                format_datetime_with_tz(master.dtstart, &master.dtstart_tz)
            };

            for dt in expanded {
                let matched_override = overrides.iter().find(|o| {
                    o.recurrence_id.as_ref().map_or(false, |rid| {
                        let rid_ts = naive_to_utc_timestamp(*rid, &o.recurrence_id_tz);
                        let exp_ts = naive_to_utc_timestamp(dt, &master.dtstart_tz);
                        rid_ts == exp_ts
                    })
                });

                if let Some(override_event) = matched_override {
                    events.push(create_event(
                        &override_event.uid,
                        &override_event.summary,
                        &override_event.description,
                        &override_event.location,
                        override_event.dtstart,
                        override_event.dtstart_is_date,
                        Some(override_event.dtstart + override_event.duration),
                        override_event.dtstart_is_date,
                        &override_event.dtstart_tz,
                        true,
                        recurrence.clone(),
                        master_dtstart_str.clone(),
                        override_event.alarms.clone(),
                        override_event.attendees.clone(),
                    ));
                } else {
                    events.push(create_event(
                        &master.uid,
                        &master.summary,
                        &master.description,
                        &master.location,
                        dt,
                        master.dtstart_is_date,
                        Some(dt + master.duration),
                        master.dtstart_is_date,
                        &master.dtstart_tz,
                        true,
                        recurrence.clone(),
                        master_dtstart_str.clone(),
                        master.alarms.clone(),
                        master.attendees.clone(),
                    ));
                }
            }
        } else {
            events.push(create_event(
                &master.uid,
                &master.summary,
                &master.description,
                &master.location,
                master.dtstart,
                master.dtstart_is_date,
                Some(master.dtstart + master.duration),
                master.dtstart_is_date,
                &master.dtstart_tz,
                false,
                None,
                String::new(),
                master.alarms.clone(),
                master.attendees.clone(),
            ));
        }
    }

    events
}

/// Append an `RDATE`/`EXDATE` line to a synthetic RRULE-crate input string,
/// using the property's resolved timezone (mirrors the DTSTART handling in
/// `expand_recurring`).
fn push_date_property_line(out: &mut String, prop: &str, date_property: &DateProperty) {
    match &date_property.resolved {
        ResolvedTz::Named(tz) => {
            out.push_str(&format!(
                "{prop};TZID={}:{}\n",
                tz.name(),
                date_property.value
            ));
        }
        ResolvedTz::Fixed { offset, .. } => {
            let line = parse_dtstart(&date_property.value)
                .map(|dt| fixed_offset_to_utc_naive(dt, *offset))
                .map(|utc_dt| format!("{prop}:{}Z\n", utc_dt.format("%Y%m%dT%H%M%S")))
                .unwrap_or_else(|| format!("{prop};VALUE=DATE-TIME:{}\n", date_property.value));
            out.push_str(&line);
        }
        // UTC and floating both use the synthetic block's internal UTC label
        // (matching the DTSTART line in `expand_recurring`); expansion results
        // are read back as naive wall clock, so the label cancels out.
        ResolvedTz::Utc | ResolvedTz::Floating => {
            let v = &date_property.value;
            if v.contains('T') {
                out.push_str(&format!("{prop}:{}Z\n", v.trim_end_matches('Z')));
            } else {
                out.push_str(&format!("{prop};VALUE=DATE-TIME:{v}\n"));
            }
        }
    }
}

fn expand_recurring(master: &RawIcalEvent, start: NaiveDate, end: NaiveDate) -> Vec<NaiveDateTime> {
    let mut rrule_str = String::new();

    if master.dtstart_is_date {
        rrule_str.push_str(&format!(
            "DTSTART;VALUE=DATE:{}\n",
            master.dtstart.format("%Y%m%d")
        ));
    } else {
        match &master.dtstart_tz {
            ResolvedTz::Named(tz) => {
                rrule_str.push_str(&format!(
                    "DTSTART;TZID={}:{}\n",
                    tz.name(),
                    master.dtstart.format("%Y%m%dT%H%M%S")
                ));
            }
            ResolvedTz::Fixed { offset, .. } => {
                let utc_dt = fixed_offset_to_utc_naive(master.dtstart, *offset);
                rrule_str.push_str(&format!("DTSTART:{}Z\n", utc_dt.format("%Y%m%dT%H%M%S")));
            }
            ResolvedTz::Utc | ResolvedTz::Floating => {
                rrule_str.push_str(&format!(
                    "DTSTART:{}Z\n",
                    master.dtstart.format("%Y%m%dT%H%M%S")
                ));
            }
        }
    }

    if let Some(ref rrule) = master.rrule {
        rrule_str.push_str(&format!("RRULE:{rrule}\n"));
    }

    for rdate in &master.rdates {
        push_date_property_line(&mut rrule_str, "RDATE", rdate);
    }

    for exdate in &master.exdates {
        push_date_property_line(&mut rrule_str, "EXDATE", exdate);
    }

    let rrule_set: RRuleSet = match rrule_str.parse() {
        Ok(set) => set,
        Err(e) => {
            eprintln!("Failed to parse RRuleSet: {e} - string was: {rrule_str:?}");
            return vec![master.dtstart];
        }
    };

    let start_utc = rrule::Tz::UTC
        .from_utc_datetime(&(start - TimeDelta::days(1)).and_hms_opt(0, 0, 0).unwrap());
    let end_utc = rrule::Tz::UTC
        .from_utc_datetime(&(end + TimeDelta::days(1)).and_hms_opt(23, 59, 59).unwrap());

    let rrule_set = rrule_set.after(start_utc).before(end_utc);
    let result = rrule_set.all(65535);

    result
        .dates
        .into_iter()
        .filter(|dt| {
            let date = dt.naive_local().date();
            date >= start && date <= end
        })
        .map(|dt| dt.naive_local())
        .collect()
}

fn format_datetime_with_tz(dt: NaiveDateTime, tz: &ResolvedTz) -> String {
    match tz {
        ResolvedTz::Named(tz) => match tz.from_local_datetime(&dt) {
            chrono::LocalResult::Single(local_dt) => {
                local_dt.format("%Y-%m-%dT%H:%M:%S%:z").to_string()
            }
            chrono::LocalResult::Ambiguous(local_dt, _) => {
                local_dt.format("%Y-%m-%dT%H:%M:%S%:z").to_string()
            }
            chrono::LocalResult::None => {
                format!("{}Z", dt.format("%Y-%m-%dT%H:%M:%S"))
            }
        },
        ResolvedTz::Fixed { offset, .. } => match offset.from_local_datetime(&dt) {
            chrono::LocalResult::Single(local_dt) => {
                local_dt.format("%Y-%m-%dT%H:%M:%S%:z").to_string()
            }
            chrono::LocalResult::Ambiguous(local_dt, _) => {
                local_dt.format("%Y-%m-%dT%H:%M:%S%:z").to_string()
            }
            chrono::LocalResult::None => {
                format!("{}Z", dt.format("%Y-%m-%dT%H:%M:%S"))
            }
        },
        ResolvedTz::Utc => format!("{}Z", dt.format("%Y-%m-%dT%H:%M:%S")),
        // Floating time keeps its naive form — no offset suffix — so the
        // frontend renders the wall clock in the viewer's local timezone.
        ResolvedTz::Floating => dt.format("%Y-%m-%dT%H:%M:%S").to_string(),
    }
}

#[allow(clippy::too_many_arguments)]
fn create_event(
    uid: &str,
    summary: &str,
    description: &str,
    location: &str,
    dtstart: NaiveDateTime,
    dtstart_is_date: bool,
    dtend: Option<NaiveDateTime>,
    dtend_is_date: bool,
    tz: &ResolvedTz,
    is_recurring: bool,
    recurrence: Option<RecurrenceInfo>,
    master_dtstart: String,
    alarms: Vec<Alarm>,
    attendees: Vec<Attendee>,
) -> Event {
    let dtstart_str = if dtstart_is_date {
        dtstart.format("%Y-%m-%d").to_string()
    } else {
        format_datetime_with_tz(dtstart, tz)
    };

    let date = dtstart.date();
    let start_time = if dtstart_is_date {
        None
    } else {
        Some(dtstart.format("%H:%M").to_string())
    };

    let (dtend_str, end_time) = match dtend {
        Some(dt) => {
            let s = if dtend_is_date {
                dt.format("%Y-%m-%d").to_string()
            } else {
                format_datetime_with_tz(dt, tz)
            };
            let et = if dtend_is_date {
                None
            } else {
                Some(dt.format("%H:%M").to_string())
            };
            (Some(s), et)
        }
        None => (None, None),
    };

    let anchored_ts = |dt: NaiveDateTime, is_date: bool| {
        (!is_date && tz.is_anchored()).then(|| naive_to_utc_timestamp(dt, tz))
    };

    Event {
        uid: uid.to_string(),
        summary: summary.to_string(),
        description: description.to_string(),
        location: location.to_string(),
        dtstart: dtstart_str,
        dtend: dtend_str,
        date,
        start_time,
        end_time,
        start_ts: anchored_ts(dtstart, dtstart_is_date),
        end_ts: dtend.and_then(|dt| anchored_ts(dt, dtend_is_date)),
        is_all_day: dtstart_is_date,
        tz: tz.display_tz(),
        is_recurring,
        recurrence,
        master_dtstart,
        alarms,
        attendees,
    }
}

fn parse_rrule_info(rrule_str: &str) -> RecurrenceInfo {
    let mut info = RecurrenceInfo {
        freq: String::new(),
        interval: 1,
        until: None,
        count: None,
        by_day: Vec::new(),
        by_month_day: Vec::new(),
        by_month: Vec::new(),
        by_set_pos: Vec::new(),
    };

    for part in rrule_str.split(';') {
        let part = part.trim();
        let Some((key, val)) = part.split_once('=') else {
            continue;
        };
        match key {
            "FREQ" => info.freq = val.to_string(),
            "INTERVAL" => info.interval = val.parse().unwrap_or(1),
            "UNTIL" => {
                let s = val.trim_end_matches('Z');
                let formatted = if let Ok(dt) = NaiveDateTime::parse_from_str(s, "%Y%m%dT%H%M%S") {
                    dt.format("%Y-%m-%d %H:%M:%S").to_string()
                } else if let Ok(d) = NaiveDate::parse_from_str(s, "%Y%m%d") {
                    d.format("%Y-%m-%d").to_string()
                } else {
                    val.to_string()
                };
                info.until = Some(formatted);
            }
            "COUNT" => info.count = val.parse().ok(),
            "BYDAY" => {
                info.by_day = val.split(',').map(|s| s.trim().to_string()).collect();
            }
            "BYMONTHDAY" => {
                info.by_month_day = val
                    .split(',')
                    .filter_map(|s| s.trim().parse().ok())
                    .collect();
            }
            "BYMONTH" => {
                info.by_month = val
                    .split(',')
                    .filter_map(|s| s.trim().parse().ok())
                    .collect();
            }
            "BYSETPOS" => {
                info.by_set_pos = val
                    .split(',')
                    .filter_map(|s| s.trim().parse().ok())
                    .collect();
            }
            _ => {}
        }
    }

    info
}

fn naive_to_utc_timestamp(dt: NaiveDateTime, tz: &ResolvedTz) -> i64 {
    match tz {
        ResolvedTz::Named(tz) => match tz.from_local_datetime(&dt) {
            chrono::LocalResult::Single(local_dt) => local_dt.timestamp(),
            chrono::LocalResult::Ambiguous(local_dt, _) => local_dt.timestamp(),
            chrono::LocalResult::None => dt.and_utc().timestamp(),
        },
        ResolvedTz::Fixed { offset, .. } => match offset.from_local_datetime(&dt) {
            chrono::LocalResult::Single(local_dt) => local_dt.timestamp(),
            chrono::LocalResult::Ambiguous(local_dt, _) => local_dt.timestamp(),
            chrono::LocalResult::None => dt.and_utc().timestamp(),
        },
        // Floating has no true instant; labeling it UTC keeps RECURRENCE-ID
        // matching self-consistent.
        ResolvedTz::Utc | ResolvedTz::Floating => dt.and_utc().timestamp(),
    }
}

fn parse_dtstart(raw: &str) -> Option<NaiveDateTime> {
    NaiveDateTime::parse_from_str(raw, "%Y%m%dT%H%M%S")
        .ok()
        .or_else(|| NaiveDateTime::parse_from_str(raw, "%Y%m%dT%H%M%SZ").ok())
        .or_else(|| {
            NaiveDate::parse_from_str(raw, "%Y%m%d")
                .ok()
                .map(|d| d.and_hms_opt(0, 0, 0).unwrap())
        })
}

pub fn preview_events_from_raw_ical(raw_ical: &str) -> Vec<Event> {
    parse_raw_events(raw_ical)
        .into_iter()
        .filter(|raw| raw.recurrence_id.is_none())
        .map(|raw| {
            let recurrence = raw.rrule.as_ref().map(|r| parse_rrule_info(r));
            create_event(
                &raw.uid,
                &raw.summary,
                &raw.description,
                &raw.location,
                raw.dtstart,
                raw.dtstart_is_date,
                Some(raw.dtstart + raw.duration),
                raw.dtstart_is_date,
                &raw.dtstart_tz,
                raw.rrule.is_some() || !raw.rdates.is_empty(),
                recurrence,
                String::new(),
                raw.alarms,
                raw.attendees,
            )
        })
        .collect()
}

pub(super) fn extract_events_with_hrefs(
    xml: &str,
    base_url: &str,
) -> anyhow::Result<Vec<(String, String, String)>> {
    let mut reader = Reader::from_str(xml);

    let mut results = Vec::new();
    let mut in_response = false;
    let mut in_href = false;
    let mut in_etag = false;
    let mut in_calendar_data = false;
    let mut current_href = String::new();
    let mut current_etag = String::new();
    let mut current_ical = String::new();
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf)? {
            XmlEvent::Start(e) => {
                let lname = e.local_name();
                let local = lname.into_inner();
                match local {
                    "response" => {
                        in_response = true;
                        current_href.clear();
                        current_etag.clear();
                        current_ical.clear();
                    }
                    "href" if in_response => in_href = true,
                    "getetag" => in_etag = true,
                    "calendar-data" => {
                        in_calendar_data = true;
                        current_ical.clear();
                    }
                    _ => {}
                }
            }
            XmlEvent::Text(e) => {
                let text = e.xml10_content();
                if in_calendar_data {
                    current_ical.push_str(&text);
                } else if in_href {
                    current_href.push_str(&text);
                } else if in_etag {
                    current_etag.push_str(&text);
                }
            }
            XmlEvent::GeneralRef(r) => {
                let text = resolve_ref(&r);
                if in_calendar_data {
                    current_ical.push_str(&text);
                } else if in_href {
                    current_href.push_str(&text);
                } else if in_etag {
                    current_etag.push_str(&text);
                }
            }
            XmlEvent::End(e) => {
                let lname = e.local_name();
                let local = lname.into_inner();
                match local {
                    "href" => in_href = false,
                    "getetag" => in_etag = false,
                    "calendar-data" => in_calendar_data = false,
                    "response" => {
                        if !current_href.is_empty() && !current_ical.is_empty() {
                            results.push((
                                resolve_url(base_url, current_href.trim()),
                                current_etag.trim().trim_matches('"').to_string(),
                                current_ical.trim().to_string(),
                            ));
                        }
                        in_response = false;
                    }
                    _ => {}
                }
            }
            XmlEvent::Eof => break,
            _ => {}
        }
    }
    Ok(results)
}

pub(super) fn extract_sync_result(xml: &str, base_url: &str) -> anyhow::Result<SyncResult> {
    let mut reader = Reader::from_str(xml);

    let mut new_token = String::new();
    let mut changed = Vec::new();
    let mut deleted = Vec::new();

    let mut depth: usize = 0;
    let mut in_response = false;
    let mut in_sync_token_top = false;
    let mut in_href = false;
    let mut in_etag = false;
    let mut in_calendar_data = false;
    let mut in_status = false;

    let mut current_href = String::new();
    let mut current_etag = String::new();
    let mut current_ical = String::new();
    let mut current_status = String::new();
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf)? {
            XmlEvent::Start(e) => {
                depth += 1;
                let lname = e.local_name();
                let local = lname.into_inner();
                match local {
                    "sync-token" if depth == 2 => in_sync_token_top = true,
                    "response" => {
                        in_response = true;
                        current_href.clear();
                        current_etag.clear();
                        current_ical.clear();
                        current_status.clear();
                    }
                    "href" if in_response => in_href = true,
                    "getetag" => in_etag = true,
                    "calendar-data" => {
                        in_calendar_data = true;
                        current_ical.clear();
                    }
                    "status" => in_status = true,
                    _ => {}
                }
            }
            XmlEvent::Text(e) => {
                let text = e.xml10_content();
                if in_sync_token_top {
                    new_token.push_str(&text);
                } else if in_calendar_data {
                    current_ical.push_str(&text);
                } else if in_href {
                    current_href.push_str(&text);
                } else if in_etag {
                    current_etag.push_str(&text);
                } else if in_status {
                    current_status.push_str(&text);
                }
            }
            XmlEvent::GeneralRef(r) => {
                let text = resolve_ref(&r);
                if in_sync_token_top {
                    new_token.push_str(&text);
                } else if in_calendar_data {
                    current_ical.push_str(&text);
                } else if in_href {
                    current_href.push_str(&text);
                } else if in_etag {
                    current_etag.push_str(&text);
                } else if in_status {
                    current_status.push_str(&text);
                }
            }
            XmlEvent::End(e) => {
                let lname = e.local_name();
                let local = lname.into_inner();
                match local {
                    "sync-token" => in_sync_token_top = false,
                    "href" => in_href = false,
                    "getetag" => in_etag = false,
                    "calendar-data" => in_calendar_data = false,
                    "status" => in_status = false,
                    "response" => {
                        let href = resolve_url(base_url, current_href.trim());
                        if current_status.contains("404") {
                            deleted.push(href);
                        } else if !current_ical.is_empty() {
                            changed.push((
                                href,
                                current_etag.trim().trim_matches('"').to_string(),
                                current_ical.trim().to_string(),
                            ));
                        }
                        in_response = false;
                    }
                    _ => {}
                }
                depth = depth.saturating_sub(1);
            }
            XmlEvent::Eof => break,
            _ => {}
        }
    }
    Ok(SyncResult {
        new_token: new_token.trim().to_string(),
        changed,
        deleted,
    })
}

pub(crate) fn extract_ctag_and_token(xml: &str) -> (Option<String>, Option<String>) {
    let mut reader = Reader::from_str(xml);

    let mut ctag: Option<String> = None;
    let mut sync_token: Option<String> = None;
    let mut in_ctag = false;
    let mut in_token = false;
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(XmlEvent::Start(e)) => {
                let lname = e.local_name();
                let local = lname.into_inner();
                match local {
                    "getctag" => in_ctag = true,
                    "sync-token" => in_token = true,
                    _ => {}
                }
            }
            Ok(XmlEvent::Text(e)) => {
                let text = e.xml10_content();
                let text = text.trim().to_string();
                if !text.is_empty() {
                    if in_ctag {
                        ctag = Some(text);
                    } else if in_token {
                        sync_token = Some(text);
                    }
                }
            }
            Ok(XmlEvent::End(e)) => {
                let lname = e.local_name();
                let local = lname.into_inner();
                match local {
                    "getctag" => in_ctag = false,
                    "sync-token" => in_token = false,
                    _ => {}
                }
            }
            Ok(XmlEvent::Eof) | Err(_) => break,
            _ => {}
        }
    }
    (ctag, sync_token)
}

/// Parse the expiry from a WebDAV push SUBSCRIBE response.
pub(super) fn extract_push_expiry(xml: &str) -> Option<i64> {
    let mut reader = Reader::from_str(xml);

    let mut in_expires = false;
    let mut expires_text = String::new();
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(XmlEvent::Start(e)) => {
                let lname = e.local_name();
                let local = lname.into_inner();
                if local == "expires" {
                    in_expires = true;
                    expires_text.clear();
                }
            }
            Ok(XmlEvent::Text(e)) => {
                if in_expires {
                    expires_text.push_str(&e.xml10_content());
                }
            }
            Ok(XmlEvent::GeneralRef(r)) => {
                if in_expires {
                    expires_text.push_str(&resolve_ref(&r));
                }
            }
            Ok(XmlEvent::End(e)) => {
                let lname = e.local_name();
                let local = lname.into_inner();
                if local == "expires" {
                    in_expires = false;
                }
            }
            Ok(XmlEvent::Eof) | Err(_) => break,
            _ => {}
        }
    }

    let text = expires_text.trim();
    if text.is_empty() {
        return None;
    }

    let now = Utc::now().timestamp();

    if text.starts_with('P') {
        let seconds = parse_iso_duration_secs(text)?;
        return Some(now + seconds);
    }

    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(text) {
        return Some(dt.timestamp());
    }

    if let Ok(dt) = chrono::DateTime::parse_from_str(text, "%a, %d %b %Y %H:%M:%S GMT") {
        return Some(dt.timestamp());
    }

    None
}

fn parse_iso_duration_secs(s: &str) -> Option<i64> {
    let s = s.strip_prefix('P')?;
    let (date_part, time_part) = if let Some(t) = s.split_once('T') {
        (t.0, t.1)
    } else {
        (s, "")
    };

    let mut total: i64 = 0;

    let mut rest = date_part;
    if let Some(pos) = rest.find('D') {
        let days: i64 = rest[..pos].parse().ok()?;
        total += days * 86400;
        rest = &rest[pos + 1..];
    }
    let _ = rest;

    let mut rest = time_part;
    if let Some(pos) = rest.find('H') {
        let hours: i64 = rest[..pos].parse().ok()?;
        total += hours * 3600;
        rest = &rest[pos + 1..];
    }
    if let Some(pos) = rest.find('M') {
        let mins: i64 = rest[..pos].parse().ok()?;
        total += mins * 60;
        rest = &rest[pos + 1..];
    }
    if let Some(pos) = rest.find('S') {
        let secs: i64 = rest[..pos].parse().ok()?;
        total += secs;
    }

    if total > 0 { Some(total) } else { None }
}

#[cfg(test)]
mod tz_resolution_tests {
    use super::*;

    fn preview_one(ical: &str) -> Event {
        preview_events_from_raw_ical(ical)
            .into_iter()
            .next()
            .expect("expected at least one previewed event")
    }

    #[test]
    fn direct_iana_tzid_is_unaffected() {
        let ical = "BEGIN:VCALENDAR\r\n\
VERSION:2.0\r\n\
PRODID:-//Test//EN\r\n\
BEGIN:VEVENT\r\n\
UID:direct-iana\r\n\
DTSTART;TZID=America/New_York:20260115T090000\r\n\
DTEND;TZID=America/New_York:20260115T100000\r\n\
SUMMARY:Winter Meeting\r\n\
END:VEVENT\r\n\
END:VCALENDAR\r\n";

        let event = preview_one(ical);
        assert_eq!(event.tz, "America/New_York");
        assert!(event.dtstart.starts_with("2026-01-15T09:00:00-05:00"));
    }

    #[test]
    fn windows_tzid_without_vtimezone_resolves_via_table() {
        let ical = "BEGIN:VCALENDAR\r\n\
VERSION:2.0\r\n\
PRODID:-//Test//EN\r\n\
BEGIN:VEVENT\r\n\
UID:windows-name\r\n\
DTSTART;TZID=Eastern Standard Time:20260715T090000\r\n\
DTEND;TZID=Eastern Standard Time:20260715T100000\r\n\
SUMMARY:Summer Meeting\r\n\
END:VEVENT\r\n\
END:VCALENDAR\r\n";

        let event = preview_one(ical);
        assert_eq!(event.tz, "America/New_York");
        // July is EDT (UTC-4), proving DST is correctly derived once
        // resolved to the real IANA zone, not just a raw offset guess.
        assert!(event.dtstart.starts_with("2026-07-15T09:00:00-04:00"));
    }

    #[test]
    fn windows_table_is_tried_before_embedded_vtimezone() {
        // Deliberately wrong VTIMEZONE offset (+09:00) attached to a
        // TZID that the Windows-name table already resolves correctly
        // (-05:00 in January). The table lookup must win.
        let ical = "BEGIN:VCALENDAR\r\n\
VERSION:2.0\r\n\
PRODID:-//Test//EN\r\n\
BEGIN:VTIMEZONE\r\n\
TZID:Eastern Standard Time\r\n\
BEGIN:STANDARD\r\n\
DTSTART:19700101T000000\r\n\
TZOFFSETFROM:+0900\r\n\
TZOFFSETTO:+0900\r\n\
TZNAME:BOGUS\r\n\
END:STANDARD\r\n\
END:VTIMEZONE\r\n\
BEGIN:VEVENT\r\n\
UID:ordering\r\n\
DTSTART;TZID=Eastern Standard Time:20260115T090000\r\n\
SUMMARY:Ordering Test\r\n\
END:VEVENT\r\n\
END:VCALENDAR\r\n";

        let event = preview_one(ical);
        assert_eq!(event.tz, "America/New_York");
        assert!(event.dtstart.starts_with("2026-01-15T09:00:00-05:00"));
    }

    #[test]
    fn custom_tzid_falls_back_to_embedded_vtimezone_fixed_offset() {
        let ical = "BEGIN:VCALENDAR\r\n\
VERSION:2.0\r\n\
PRODID:-//Test//EN\r\n\
BEGIN:VTIMEZONE\r\n\
TZID:Custom Zone\r\n\
BEGIN:STANDARD\r\n\
DTSTART:19700101T000000\r\n\
TZOFFSETFROM:-0300\r\n\
TZOFFSETTO:-0300\r\n\
TZNAME:CUSTOM\r\n\
END:STANDARD\r\n\
END:VTIMEZONE\r\n\
BEGIN:VEVENT\r\n\
UID:custom-zone\r\n\
DTSTART;TZID=Custom Zone:20260115T120000\r\n\
SUMMARY:Custom Zone Event\r\n\
END:VEVENT\r\n\
END:VCALENDAR\r\n";

        let event = preview_one(ical);
        assert_eq!(event.tz, "Custom Zone");
        assert!(event.dtstart.starts_with("2026-01-15T12:00:00-03:00"));
    }

    #[test]
    fn embedded_vtimezone_dst_transitions_are_resolved_per_date() {
        let ical = "BEGIN:VCALENDAR\r\n\
VERSION:2.0\r\n\
PRODID:-//Test//EN\r\n\
BEGIN:VTIMEZONE\r\n\
TZID:My/Custom\r\n\
BEGIN:STANDARD\r\n\
DTSTART:20071104T020000\r\n\
TZOFFSETFROM:-0400\r\n\
TZOFFSETTO:-0500\r\n\
TZNAME:CST\r\n\
RRULE:FREQ=YEARLY;BYMONTH=11;BYDAY=1SU\r\n\
END:STANDARD\r\n\
BEGIN:DAYLIGHT\r\n\
DTSTART:20070311T020000\r\n\
TZOFFSETFROM:-0500\r\n\
TZOFFSETTO:-0400\r\n\
TZNAME:CDT\r\n\
RRULE:FREQ=YEARLY;BYMONTH=3;BYDAY=2SU\r\n\
END:DAYLIGHT\r\n\
END:VTIMEZONE\r\n\
BEGIN:VEVENT\r\n\
UID:summer\r\n\
DTSTART;TZID=My/Custom:20260715T090000\r\n\
SUMMARY:Summer\r\n\
END:VEVENT\r\n\
BEGIN:VEVENT\r\n\
UID:winter\r\n\
DTSTART;TZID=My/Custom:20260115T090000\r\n\
SUMMARY:Winter\r\n\
END:VEVENT\r\n\
END:VCALENDAR\r\n";

        let events = preview_events_from_raw_ical(ical);
        assert_eq!(events.len(), 2);
        let summer = events.iter().find(|e| e.uid == "summer").unwrap();
        let winter = events.iter().find(|e| e.uid == "winter").unwrap();
        assert_eq!(summer.tz, "My/Custom");
        assert!(summer.dtstart.starts_with("2026-07-15T09:00:00-04:00"));
        assert!(winter.dtstart.starts_with("2026-01-15T09:00:00-05:00"));
    }

    #[test]
    fn recurring_event_with_windows_tzid_does_not_collapse() {
        let ical = "BEGIN:VCALENDAR\r\n\
VERSION:2.0\r\n\
PRODID:-//Test//EN\r\n\
BEGIN:VEVENT\r\n\
UID:weekly\r\n\
DTSTART;TZID=Romance Standard Time:20260302T140000\r\n\
DTEND;TZID=Romance Standard Time:20260302T150000\r\n\
RRULE:FREQ=WEEKLY;COUNT=5\r\n\
SUMMARY:Weekly Meeting\r\n\
END:VEVENT\r\n\
END:VCALENDAR\r\n";

        let start = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
        let end = NaiveDate::from_ymd_opt(2026, 4, 30).unwrap();
        let events = parse_and_expand_events(ical, start, end);

        assert_eq!(events.len(), 5);
        assert_eq!(events[0].tz, "Europe/Paris");
    }

    #[test]
    fn floating_time_stays_floating() {
        let ical = "BEGIN:VCALENDAR\r\n\
VERSION:2.0\r\n\
PRODID:-//Test//EN\r\n\
BEGIN:VEVENT\r\n\
UID:floating\r\n\
DTSTART:20260115T090000\r\n\
DTEND:20260115T100000\r\n\
SUMMARY:Floating Event\r\n\
END:VEVENT\r\n\
END:VCALENDAR\r\n";

        let event = preview_one(ical);
        assert_eq!(event.tz, "");
        assert_eq!(event.dtstart, "2026-01-15T09:00:00");
        assert_eq!(event.dtend.as_deref(), Some("2026-01-15T10:00:00"));
    }

    #[test]
    fn utc_z_value_reported_as_utc() {
        let ical = "BEGIN:VCALENDAR\r\n\
VERSION:2.0\r\n\
PRODID:-//Test//EN\r\n\
BEGIN:VEVENT\r\n\
UID:utc-z\r\n\
DTSTART:20260115T090000Z\r\n\
DTEND:20260115T100000Z\r\n\
SUMMARY:UTC Event\r\n\
END:VEVENT\r\n\
END:VCALENDAR\r\n";

        let event = preview_one(ical);
        assert_eq!(event.tz, "UTC");
        assert_eq!(event.dtstart, "2026-01-15T09:00:00Z");
        assert_eq!(event.dtend.as_deref(), Some("2026-01-15T10:00:00Z"));
    }

    #[test]
    fn unresolvable_tzid_falls_back_to_floating() {
        let ical = "BEGIN:VCALENDAR\r\n\
VERSION:2.0\r\n\
PRODID:-//Test//EN\r\n\
BEGIN:VEVENT\r\n\
UID:nonsense-zone\r\n\
DTSTART;TZID=Nonsense/Zone:20260115T090000\r\n\
SUMMARY:Unknown Zone Event\r\n\
END:VEVENT\r\n\
END:VCALENDAR\r\n";

        let event = preview_one(ical);
        assert_eq!(event.tz, "");
        // Wall clock is preserved instead of being shifted via a UTC relabel.
        assert_eq!(event.dtstart, "2026-01-15T09:00:00");
    }

    #[test]
    fn dtend_with_different_tzid_uses_instant_duration() {
        // Berlin 09:00 to New York 06:00 is a 3-hour event; the displayed
        // dtend stays in DTSTART's zone.
        let ical = "BEGIN:VCALENDAR\r\n\
VERSION:2.0\r\n\
PRODID:-//Test//EN\r\n\
BEGIN:VEVENT\r\n\
UID:cross-tzid\r\n\
DTSTART;TZID=Europe/Berlin:20260115T090000\r\n\
DTEND;TZID=America/New_York:20260115T060000\r\n\
SUMMARY:Cross Zone Event\r\n\
END:VEVENT\r\n\
END:VCALENDAR\r\n";

        let event = preview_one(ical);
        assert!(event.dtstart.starts_with("2026-01-15T09:00:00+01:00"));
        assert_eq!(event.dtend.as_deref(), Some("2026-01-15T12:00:00+01:00"));
    }

    #[test]
    fn dtend_utc_with_zoned_dtstart() {
        // New York 09:00 (14:00Z) to 15:00Z is a one-hour event.
        let ical = "BEGIN:VCALENDAR\r\n\
VERSION:2.0\r\n\
PRODID:-//Test//EN\r\n\
BEGIN:VEVENT\r\n\
UID:zoned-start-utc-end\r\n\
DTSTART;TZID=America/New_York:20260115T090000\r\n\
DTEND:20260115T150000Z\r\n\
SUMMARY:Mixed Form Event\r\n\
END:VEVENT\r\n\
END:VCALENDAR\r\n";

        let event = preview_one(ical);
        assert!(event.dtstart.starts_with("2026-01-15T09:00:00-05:00"));
        assert_eq!(event.dtend.as_deref(), Some("2026-01-15T10:00:00-05:00"));
    }
}

#[cfg(test)]
mod xml_entity_tests {
    use super::*;

    // quick-xml 0.42 reports entity references as separate events; make sure the
    // DAV parsers still reassemble text values that contain `&amp;` etc.

    #[test]
    fn extract_hrefs_reassembles_entities_in_href() {
        let xml = r#"<multistatus xmlns="DAV:">
            <response>
                <href>/calendars/user/a&amp;b/</href>
            </response>
        </multistatus>"#;
        let hrefs = extract_hrefs(xml, "response").unwrap();
        assert_eq!(hrefs, vec!["/calendars/user/a&b/"]);
    }

    #[test]
    fn extract_text_prop_reassembles_entities() {
        let xml = r#"<multistatus xmlns="DAV:">
            <prop><displayname>Rock &amp; Roll &#128512;</displayname></prop>
        </multistatus>"#;
        assert_eq!(
            extract_text_prop(xml, "displayname").as_deref(),
            Some("Rock & Roll 😀")
        );
    }

    #[test]
    fn extract_events_with_hrefs_parses_calendar_data_report() {
        let xml = "<multistatus xmlns=\"DAV:\" xmlns:c=\"urn:ietf:params:xml:ns:caldav\">\
            <response>\
                <href>/cal/work/evt1.ics</href>\
                <propstat><prop>\
                    <getetag>\"abc&amp;123\"</getetag>\
                    <c:calendar-data>\n\
BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:evt1\r\nEND:VEVENT\r\nEND:VCALENDAR\n\
                    </c:calendar-data>\
                </prop></propstat>\
            </response>\
        </multistatus>";
        let out = extract_events_with_hrefs(xml, "https://dav.example.com/").unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].0, "https://dav.example.com/cal/work/evt1.ics");
        assert_eq!(out[0].1, "abc&123");
        assert!(out[0].2.starts_with("BEGIN:VCALENDAR"));
        assert!(out[0].2.ends_with("END:VCALENDAR"));
    }

    #[test]
    fn parse_all_calendars_keeps_entity_in_displayname() {
        let xml = r#"<multistatus xmlns="DAV:">
            <response>
                <href>/cal/work/</href>
                <propstat><prop>
                    <displayname>Me &amp; Work</displayname>
                </prop></propstat>
            </response>
        </multistatus>"#;
        let cals = parse_all_calendars(xml, "https://dav.example.com/cal/").unwrap();
        assert_eq!(cals.len(), 1);
        assert_eq!(cals[0].name, "Me & Work");
    }
}
