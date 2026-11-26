use anyhow::bail;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};

use chrono::{
    Datelike as _, Days, FixedOffset, NaiveDate, NaiveDateTime, TimeDelta, TimeZone as _, Utc,
};
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
    /// DTSTART's TZID as written (an occurrence of a VTIMEZONE-only zone is resolved again on its date).
    dtstart_tzid: Option<String>,
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
    /// Resolved via an embedded VTIMEZONE block, with no IANA equivalent found: the UTC offset in
    /// effect on this value's date (a recurring event's occurrences each resolve their own, see
    /// `expand_events`). `canonical` is the raw TZID string, kept only as a display label.
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

/// Resolves the TZIDs of one calendar object, with the transitions of its VTIMEZONEs computed once per
/// year (an Outlook VTIMEZONE's rules start in 1601, and an event may name it in thousands of EXDATEs).
struct TzResolver {
    timezones: Vec<ical::parser::ical::component::IcalTimeZone>,
    /// A VTIMEZONE's transitions (sorted) around one year, keyed by its index and the year; `None`
    /// for one that cannot be used.
    years: RefCell<HashMap<(usize, i32), Option<Vec<(NaiveDateTime, FixedOffset)>>>>,
    /// Transition years computed by the rrule crate (rules not of the usual shape), bounded per object.
    walks: Cell<usize>,
    warned: RefCell<HashSet<String>>,
}

/// How many (VTIMEZONE, year) pairs of an unusual shape one object may have computed by the crate.
const MAX_ZONE_WALKS: usize = 4;

impl TzResolver {
    fn new(timezones: Vec<ical::parser::ical::component::IcalTimeZone>) -> Self {
        Self {
            timezones,
            years: RefCell::default(),
            walks: Cell::new(0),
            warned: RefCell::default(),
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
    fn resolve(&self, tzid: Option<&str>, at: NaiveDateTime, value_is_utc: bool) -> ResolvedTz {
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

        if let Some(i) = self
            .timezones
            .iter()
            .position(|vtz| vtimezone_tzid(vtz).as_deref() == Some(tzid))
        {
            if let Some(offset) = self.vtimezone_offset(i, at) {
                return ResolvedTz::Fixed {
                    offset,
                    canonical: tzid.to_string(),
                };
            }
        }

        if self.warned.borrow_mut().insert(tzid.to_string()) {
            eprintln!("caldav: unable to resolve TZID {tzid:?}, treating time as floating");
        }
        if value_is_utc {
            ResolvedTz::Utc
        } else {
            ResolvedTz::Floating
        }
    }

    /// The UTC offset in effect at `at` (a naive local datetime in the VTIMEZONE's own wall clock):
    /// whichever transition (across all STANDARD/DAYLIGHT rules) most recently took effect on or
    /// before `at`, else the earliest one known.
    fn vtimezone_offset(&self, index: usize, at: NaiveDateTime) -> Option<FixedOffset> {
        let key = (index, at.year());
        if !self.years.borrow().contains_key(&key) {
            let transitions = self.transitions_around(&self.timezones[index], at.year());
            self.years.borrow_mut().insert(key, transitions);
        }
        let years = self.years.borrow();
        let transitions = years.get(&key)?.as_ref()?;
        transitions
            .iter()
            .rev()
            .find(|(t, _)| *t <= at)
            .or(transitions.first())
            .map(|(_, offset)| *offset)
    }

    /// The transitions of `vtz` from the start of `year - 1` to the end of `year + 1` (a transition
    /// without RRULE wherever it is), sorted; `None` when the zone cannot be used.
    fn transitions_around(
        &self,
        vtz: &ical::parser::ical::component::IcalTimeZone,
        year: i32,
    ) -> Option<Vec<(NaiveDateTime, FixedOffset)>> {
        let from = NaiveDate::from_ymd_opt(year.checked_sub(1)?, 1, 1)?.and_hms_opt(0, 0, 0)?;
        let to = NaiveDate::from_ymd_opt(year.checked_add(1)?, 12, 31)?.and_hms_opt(23, 59, 59)?;
        let mut out = Vec::new();
        for transition in &vtz.transitions {
            let prop = |name: &str| {
                transition
                    .properties
                    .iter()
                    .find(|p| p.name == name)
                    .and_then(|p| p.value.as_deref())
            };
            let dtstart = parse_dtstart(prop("DTSTART")?)?;
            let offset_to = parse_ical_utc_offset(prop("TZOFFSETTO")?)?;
            let Some(rule) = prop("RRULE") else {
                out.push((dtstart, offset_to));
                continue;
            };
            // A rule of another frequency (an occurrence a second, …) makes the zone unusable.
            if !rule.is_ascii() || rule_freq(rule).as_deref() != Some("YEARLY") {
                return None;
            }
            let dates = match yearly_transitions(dtstart, rule, from, to) {
                Some(dates) => dates,
                None => {
                    if self.walks.get() >= MAX_ZONE_WALKS {
                        return None;
                    }
                    self.walks.set(self.walks.get() + 1);
                    let start = fast_forward(dtstart, rule, from);
                    let text =
                        format!("DTSTART:{}Z\nRRULE:{rule}\n", start.format("%Y%m%dT%H%M%S"));
                    let utc = |dt: NaiveDateTime| rrule::Tz::UTC.from_utc_datetime(&dt);
                    match occurrences_between(&text, utc(from), utc(to), |_| true) {
                        Some(dates) => dates.into_iter().map(|d| d.naive_utc()).collect(),
                        // the crate refuses it (or panics on it): the transition happens once
                        None => vec![dtstart],
                    }
                }
            };
            out.extend(dates.into_iter().map(|d| (d, offset_to)));
        }
        out.sort_by_key(|(t, _)| *t);
        Some(out)
    }
}

fn vtimezone_tzid(vtz: &ical::parser::ical::component::IcalTimeZone) -> Option<String> {
    vtz.properties
        .iter()
        .find(|p| p.name == "TZID")
        .and_then(|p| p.value.clone())
}

/// A rule's FREQ, upper case.
fn rule_freq(rule: &str) -> Option<String> {
    rule.split(';').find_map(|part| {
        let (k, v) = part.split_once('=')?;
        k.trim()
            .eq_ignore_ascii_case("FREQ")
            .then(|| v.trim().to_ascii_uppercase())
    })
}

/// A recurrence rule Caley expands: ASCII (the rrule crate panics slicing inside a multibyte
/// character) and neither SECONDLY nor MINUTELY (no real event recurs that often, and expanding one
/// is unbounded work); an event with any other rule shows once, at its DTSTART.
fn usable_rule(rule: &str) -> bool {
    rule.is_ascii() && !matches!(rule_freq(rule).as_deref(), Some("SECONDLY" | "MINUTELY"))
}

/// The occurrences from `from` to `to` of a VTIMEZONE transition's yearly rule of the usual shape
/// (`FREQ=YEARLY`, an optional `BYMONTH` and `BYDAY=<n><weekday>`, an optional `UNTIL`), computed
/// directly; `None` for any other rule.
pub(super) fn yearly_transitions(
    dtstart: NaiveDateTime,
    rule: &str,
    from: NaiveDateTime,
    to: NaiveDateTime,
) -> Option<Vec<NaiveDateTime>> {
    let mut month = dtstart.month();
    let mut by_day: Option<(i32, chrono::Weekday)> = None;
    let mut until: Option<NaiveDateTime> = None;
    for part in rule.split(';').map(str::trim).filter(|p| !p.is_empty()) {
        let (k, v) = part.split_once('=')?;
        match k.to_ascii_uppercase().as_str() {
            "FREQ" => {}
            "INTERVAL" if v.trim() == "1" => {}
            "BYMONTH" => month = v.trim().parse().ok().filter(|m| (1..=12).contains(m))?,
            "BYDAY" => {
                let v = v.trim();
                let (n, wd) = v.split_at(v.len().checked_sub(2)?);
                let n: i32 = n.trim_start_matches('+').parse().ok()?;
                if n == 0 || !(-5..=5).contains(&n) {
                    return None;
                }
                by_day = Some((n, weekday_from_ical(wd)?));
            }
            "UNTIL" => until = Some(parse_dtstart(v.trim())?),
            _ => return None,
        }
    }
    let mut out = Vec::new();
    for year in from.year()..=to.year() {
        let day = match by_day {
            Some((n, wd)) => nth_weekday(year, month, n, wd),
            None => NaiveDate::from_ymd_opt(year, month, dtstart.day()),
        };
        let Some(at) = day.map(|d| d.and_time(dtstart.time())) else {
            continue;
        };
        if at >= dtstart && at >= from && at <= to && until.is_none_or(|u| at <= u) {
            out.push(at);
        }
    }
    Some(out)
}

fn weekday_from_ical(s: &str) -> Option<chrono::Weekday> {
    use chrono::Weekday::*;
    Some(match s.to_ascii_uppercase().as_str() {
        "MO" => Mon,
        "TU" => Tue,
        "WE" => Wed,
        "TH" => Thu,
        "FR" => Fri,
        "SA" => Sat,
        "SU" => Sun,
        _ => return None,
    })
}

/// The `n`th `weekday` of a month (`-1` the last), `None` when the month has no such day.
fn nth_weekday(year: i32, month: u32, n: i32, weekday: chrono::Weekday) -> Option<NaiveDate> {
    if n > 0 {
        NaiveDate::from_weekday_of_month_opt(year, month, weekday, u8::try_from(n).ok()?)
    } else {
        let first_next = if month == 12 {
            NaiveDate::from_ymd_opt(year.checked_add(1)?, 1, 1)?
        } else {
            NaiveDate::from_ymd_opt(year, month + 1, 1)?
        };
        let last = first_next.pred_opt()?;
        let back = (7 + last.weekday().num_days_from_monday() - weekday.num_days_from_monday()) % 7;
        let day = last.checked_sub_days(Days::new(u64::from(back)))?;
        let day = day.checked_sub_days(Days::new(7 * u64::try_from(-n - 1).ok()?))?;
        (day.month() == month).then_some(day)
    }
}

/// At most this many occurrences of one event in one requested window.
const MAX_OCCURRENCES: usize = 1000;
/// At most this many occurrences are walked before the window (after `fast_forward`); a rule that
/// needs more (a COUNT or UNTIL rule from far back, or BY parts that rarely match) shows nothing there.
const WALK_BUDGET: usize = 20_000;
/// At most this many RDATE and EXDATE values each per event; later ones are ignored.
const MAX_DATES: usize = 5000;

/// The occurrences of the recurrence set `text` (the rrule crate's input) from `from` to `to` that
/// `keep` keeps, at most MAX_OCCURRENCES; `None` for a set the crate refuses or panics on.
fn occurrences_between(
    text: &str,
    from: chrono::DateTime<rrule::Tz>,
    to: chrono::DateTime<rrule::Tz>,
    keep: impl Fn(&chrono::DateTime<rrule::Tz>) -> bool,
) -> Option<Vec<chrono::DateTime<rrule::Tz>>> {
    if !text.is_ascii() {
        return None;
    }
    let walk = || {
        let set = match text.parse::<RRuleSet>() {
            Ok(set) => set.limit(),
            Err(e) => {
                eprintln!("Failed to parse RRuleSet: {e} - string was: {text:?}");
                return None;
            }
        };
        let mut out = Vec::new();
        for (step, dt) in (&set).into_iter().enumerate() {
            if dt > to {
                break;
            }
            if dt >= from {
                if keep(&dt) {
                    out.push(dt);
                    if out.len() >= MAX_OCCURRENCES {
                        break;
                    }
                }
            } else if step >= WALK_BUDGET {
                eprintln!(
                    "caldav: recurrence walked {WALK_BUDGET} occurrences before {from}, stopped"
                );
                break;
            }
        }
        Some(out)
    };
    // nothing the walk touches is used after a panic
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(walk))
        .ok()
        .flatten()
}

/// `dtstart` moved forward by whole periods of `rule` to at most `target`, where that leaves the
/// occurrences from `target` on as they are: no COUNT (it counts from the first), no UNTIL before
/// `target`, and a period of whole days (DAILY, WEEKLY, HOURLY in whole days) or of months with the
/// same day (MONTHLY, YEARLY), so the crate need not walk from a DTSTART centuries back.
pub(super) fn fast_forward(
    dtstart: NaiveDateTime,
    rule: &str,
    target: NaiveDateTime,
) -> NaiveDateTime {
    if dtstart >= target {
        return dtstart;
    }
    let mut freq = None;
    let mut interval: u32 = 1;
    for part in rule.split(';') {
        let Some((k, v)) = part.split_once('=') else {
            continue;
        };
        let v = v.trim();
        match k.trim().to_ascii_uppercase().as_str() {
            "FREQ" => freq = Some(v.to_ascii_uppercase()),
            "INTERVAL" => match v.parse() {
                Ok(n) if n > 0 => interval = n,
                _ => return dtstart,
            },
            "COUNT" => return dtstart,
            "UNTIL" => match parse_dtstart(v) {
                Some(u) if target.checked_add_days(Days::new(2)).is_some_and(|t| u > t) => {}
                _ => return dtstart,
            },
            _ => {}
        }
    }
    let days = |step: u64| {
        let k = u64::try_from((target - dtstart).num_days()).unwrap_or(0) / step;
        dtstart
            .checked_add_days(Days::new(k * step))
            .unwrap_or(dtstart)
    };
    let months = |step: u32| {
        let diff =
            (target.year() - dtstart.year()) * 12 + target.month() as i32 - dtstart.month() as i32;
        let mut k = u32::try_from(diff).unwrap_or(0) / step;
        // the same day of the month, and not past `target` (a month without that day is skipped)
        for _ in 0..24 {
            if k == 0 {
                break;
            }
            if let Some(dt) = dtstart.checked_add_months(chrono::Months::new(k * step)) {
                if dt.day() == dtstart.day() && dt <= target {
                    return dt;
                }
            }
            k -= 1;
        }
        dtstart
    };
    let interval64 = u64::from(interval);
    match freq.as_deref() {
        Some("DAILY") => days(interval64),
        Some("WEEKLY") => days(7 * interval64),
        // whole days of `interval` hours: lcm(interval, 24) hours
        Some("HOURLY") => days(interval64 / gcd(interval64, 24)),
        Some("MONTHLY") => months(interval),
        Some("YEARLY") => interval.checked_mul(12).map_or(dtstart, months),
        _ => dtstart,
    }
}

fn gcd(a: u64, b: u64) -> u64 {
    if b == 0 { a } else { gcd(b, a % b) }
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

/// The object's VEVENTs, and the resolver of its TZIDs (its VTIMEZONEs), which expanding them uses.
fn parse_raw_events(ical_data: &str) -> (Vec<RawIcalEvent>, TzResolver) {
    let calendar = match ical::IcalParser::new(ical_data.as_bytes()).next() {
        Some(Ok(c)) => c,
        _ => return (vec![], TzResolver::new(vec![])),
    };

    let tz = TzResolver::new(calendar.timezones);
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
                "RRULE" => rrule = property.value.filter(|r| usable_rule(r)),
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
                            if !v.is_empty() && v.is_ascii() && rdates.len() < MAX_DATES {
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
                            if !v.is_empty() && v.is_ascii() && exdates.len() < MAX_DATES {
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
        let dtstart_tz = tz.resolve(dtstart_tzid.as_deref(), dtstart, raw_dtstart.ends_with('Z'));

        let duration = if let Some(ref dtend_raw) = dtend_value {
            match parse_dtstart(dtend_raw) {
                Some(dtend) => {
                    let dtend_tz =
                        tz.resolve(dtend_tzid.as_deref(), dtend, dtend_raw.ends_with('Z'));
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
        // An end past the dates chrono knows (`DURATION:P100000000D`) is no end: the default length.
        let duration = if dtstart.checked_add_signed(duration).is_some() {
            duration
        } else if dtstart_is_date {
            TimeDelta::days(1)
        } else {
            TimeDelta::zero()
        };

        let recurrence_id = recurrence_id_value.as_ref().and_then(|v| parse_dtstart(v));
        let recurrence_id_tz = match recurrence_id {
            Some(rid) => tz.resolve(
                recurrence_id_tzid.as_deref(),
                rid,
                recurrence_id_value
                    .as_deref()
                    .is_some_and(|v| v.ends_with('Z')),
            ),
            None => ResolvedTz::Floating,
        };

        for rdate in rdates.iter_mut() {
            let at = parse_dtstart(&rdate.value).unwrap_or(dtstart);
            rdate.resolved = tz.resolve(rdate.tzid.as_deref(), at, rdate.value.ends_with('Z'));
        }
        for exdate in exdates.iter_mut() {
            let at = parse_dtstart(&exdate.value).unwrap_or(dtstart);
            exdate.resolved = tz.resolve(exdate.tzid.as_deref(), at, exdate.value.ends_with('Z'));
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
            dtstart_tzid,
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

    (results, tz)
}

fn parse_relative_trigger(s: &str) -> Option<i64> {
    let negative = s.starts_with('-');
    let s = s.trim_start_matches(['+', '-']);
    let s = s.strip_prefix('P')?;
    let minutes = if let Some(rest) = s.strip_prefix('T') {
        if let Some(h) = rest.strip_suffix('H') {
            h.parse::<i64>().ok()?.checked_mul(60)?
        } else if let Some(m) = rest.strip_suffix('M') {
            m.parse::<i64>().ok()?
        } else if let Some(sec) = rest.strip_suffix('S') {
            sec.parse::<i64>().ok()? / 60
        } else {
            return None;
        }
    } else if let Some(d) = s.strip_suffix('D') {
        d.parse::<i64>().ok()?.checked_mul(1440)?
    } else {
        return None;
    };
    // an alarm more than ~100 years from its event is no alarm (and its arithmetic cannot overflow)
    (minutes.unsigned_abs() <= 100 * 366 * 1440).then_some(if negative {
        -minutes
    } else {
        minutes
    })
}

/// An RFC 5545 duration; `None` for a malformed one or one beyond what `TimeDelta` holds.
fn parse_ical_duration(s: &str) -> Option<TimeDelta> {
    let s = s.trim();
    let negative = s.starts_with('-');
    let s = s.trim_start_matches('+').trim_start_matches('-');
    let s = s.strip_prefix('P')?;

    let total = |n: &str, unit: i64| n.parse::<i64>().ok()?.checked_mul(unit);
    let mut total_secs: i64 = 0;

    if let Some(week_pos) = s.find('W') {
        total_secs = total(&s[..week_pos], 7 * 86400)?;
    } else {
        let (date_part, time_part) = match s.find('T') {
            Some(t_pos) => (&s[..t_pos], &s[t_pos + 1..]),
            None => (s, ""),
        };

        if let Some(day_pos) = date_part.find('D') {
            total_secs = total_secs.checked_add(total(&date_part[..day_pos], 86400)?)?;
        }

        let mut remaining = time_part;
        for (unit, secs) in [('H', 3600), ('M', 60), ('S', 1)] {
            if let Some(pos) = remaining.find(unit) {
                total_secs = total_secs.checked_add(total(&remaining[..pos], secs)?)?;
                remaining = &remaining[pos + 1..];
            }
        }
    }

    TimeDelta::try_seconds(if negative { -total_secs } else { total_secs })
}

/// When an occurrence starting at `start` ends; one that would end past the dates chrono knows
/// ends where it starts.
fn end_of(start: NaiveDateTime, duration: TimeDelta) -> NaiveDateTime {
    start.checked_add_signed(duration).unwrap_or(start)
}

pub(super) fn parse_and_expand_events(
    raw_ical: &str,
    start: NaiveDate,
    end: NaiveDate,
) -> Vec<Event> {
    let (raw_events, tz) = parse_raw_events(raw_ical);
    expand_events(raw_events, &tz, start, end)
}

/// The zone of an occurrence of `master` at `dt` (its wall clock): a VTIMEZONE-only zone's offset on
/// that date (the master's when the zone gives none there), any other zone as the master's.
fn occurrence_tz(master: &RawIcalEvent, tz: &TzResolver, dt: NaiveDateTime) -> ResolvedTz {
    match &master.dtstart_tz {
        ResolvedTz::Fixed { .. } => match tz.resolve(master.dtstart_tzid.as_deref(), dt, false) {
            fixed @ ResolvedTz::Fixed { .. } => fixed,
            _ => master.dtstart_tz.clone(),
        },
        other => other.clone(),
    }
}

fn expand_events(
    raw_events: Vec<RawIcalEvent>,
    tz: &TzResolver,
    start: NaiveDate,
    end: NaiveDate,
) -> Vec<Event> {
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
            let expanded = expand_recurring(master, tz, start, end);

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
                let dt_tz = occurrence_tz(master, tz, dt);
                let matched_override = overrides.iter().find(|o| {
                    o.recurrence_id.as_ref().map_or(false, |rid| {
                        let rid_ts = naive_to_utc_timestamp(*rid, &o.recurrence_id_tz);
                        let exp_ts = naive_to_utc_timestamp(dt, &dt_tz);
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
                        Some(end_of(override_event.dtstart, override_event.duration)),
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
                        Some(end_of(dt, master.duration)),
                        master.dtstart_is_date,
                        &dt_tz,
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
                Some(end_of(master.dtstart, master.duration)),
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
/// `expand_recurring`). `wall` is the master's VTIMEZONE-only zone (its TZID) when the set is
/// expanded in that zone's wall clock: a value is then written as that wall clock.
fn push_date_property_line(
    out: &mut String,
    prop: &str,
    date_property: &DateProperty,
    wall: Option<(&str, &TzResolver)>,
) {
    if let Some((tzid, tz)) = wall {
        let v = &date_property.value;
        let same_zone = date_property.tzid.as_deref() == Some(tzid);
        let line = match parse_dtstart(v) {
            Some(dt) if v.contains('T') => {
                let local = if same_zone || !date_property.resolved.is_anchored() {
                    dt
                } else {
                    // an instant: the zone's wall clock then (the offset looked up at the UTC
                    // time, off only within a few hours of a transition)
                    let utc = chrono::DateTime::from_timestamp(
                        naive_to_utc_timestamp(dt, &date_property.resolved),
                        0,
                    )
                    .map_or(dt, |d| d.naive_utc());
                    match tz.resolve(Some(tzid), utc, false) {
                        ResolvedTz::Fixed { offset, .. } => utc
                            .checked_add_signed(TimeDelta::seconds(i64::from(
                                offset.local_minus_utc(),
                            )))
                            .unwrap_or(utc),
                        _ => utc,
                    }
                };
                format!("{prop}:{}Z\n", local.format("%Y%m%dT%H%M%S"))
            }
            _ => format!("{prop};VALUE=DATE-TIME:{v}\n"),
        };
        out.push_str(&line);
        return;
    }
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

/// The occurrences of `master` from `start` to `end` (at most MAX_OCCURRENCES). DTSTART is first moved
/// forward to the window (`fast_forward`) and the walk to it is bounded (WALK_BUDGET); a set the
/// crate refuses or panics on is the event once, at its DTSTART.
///
/// A VTIMEZONE-only zone (`ResolvedTz::Fixed`) is expanded in its wall clock, as floating time is: the
/// rule, BYDAY and the window see the zone's local dates, and each occurrence gets the offset of its
/// own date afterwards (`occurrence_tz`).
fn expand_recurring(
    master: &RawIcalEvent,
    tz: &TzResolver,
    start: NaiveDate,
    end: NaiveDate,
) -> Vec<NaiveDateTime> {
    let (Some(from), Some(to)) = (
        start
            .checked_sub_days(Days::new(1))
            .and_then(|d| d.and_hms_opt(0, 0, 0)),
        end.checked_add_days(Days::new(1))
            .and_then(|d| d.and_hms_opt(23, 59, 59)),
    ) else {
        return vec![];
    };
    // a day's margin more for the offset between the event's zone and the window's UTC
    let dtstart = match (&master.rrule, from.checked_sub_days(Days::new(1))) {
        (Some(rule), Some(target)) => fast_forward(master.dtstart, rule, target),
        _ => master.dtstart,
    };

    let mut rrule_str = String::new();

    if master.dtstart_is_date {
        rrule_str.push_str(&format!(
            "DTSTART;VALUE=DATE:{}\n",
            dtstart.format("%Y%m%d")
        ));
    } else {
        match &master.dtstart_tz {
            ResolvedTz::Named(tz) => {
                rrule_str.push_str(&format!(
                    "DTSTART;TZID={}:{}\n",
                    tz.name(),
                    dtstart.format("%Y%m%dT%H%M%S")
                ));
            }
            // the wall clock, labelled UTC as floating time is (read back as naive wall clock)
            ResolvedTz::Fixed { .. } | ResolvedTz::Utc | ResolvedTz::Floating => {
                rrule_str.push_str(&format!("DTSTART:{}Z\n", dtstart.format("%Y%m%dT%H%M%S")));
            }
        }
    }

    if let Some(ref rrule) = master.rrule {
        rrule_str.push_str(&format!("RRULE:{rrule}\n"));
    }

    let wall = match (&master.dtstart_tz, master.dtstart_tzid.as_deref()) {
        (ResolvedTz::Fixed { .. }, Some(tzid)) if !master.dtstart_is_date => Some((tzid, tz)),
        _ => None,
    };
    for rdate in &master.rdates {
        push_date_property_line(&mut rrule_str, "RDATE", rdate, wall);
    }

    for exdate in &master.exdates {
        push_date_property_line(&mut rrule_str, "EXDATE", exdate, wall);
    }

    let utc = |dt: NaiveDateTime| rrule::Tz::UTC.from_utc_datetime(&dt);
    let in_window = |dt: &chrono::DateTime<rrule::Tz>| {
        let date = dt.naive_local().date();
        date >= start && date <= end
    };
    match occurrences_between(&rrule_str, utc(from), utc(to), in_window) {
        Some(dates) => dates.into_iter().map(|dt| dt.naive_local()).collect(),
        None => vec![master.dtstart],
    }
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
        .0
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
                Some(end_of(raw.dtstart, raw.duration)),
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
        return now.checked_add(seconds);
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
        total = total.checked_add(days.checked_mul(86400)?)?;
        rest = &rest[pos + 1..];
    }
    let _ = rest;

    let mut rest = time_part;
    if let Some(pos) = rest.find('H') {
        let hours: i64 = rest[..pos].parse().ok()?;
        total = total.checked_add(hours.checked_mul(3600)?)?;
        rest = &rest[pos + 1..];
    }
    if let Some(pos) = rest.find('M') {
        let mins: i64 = rest[..pos].parse().ok()?;
        total = total.checked_add(mins.checked_mul(60)?)?;
        rest = &rest[pos + 1..];
    }
    if let Some(pos) = rest.find('S') {
        let secs: i64 = rest[..pos].parse().ok()?;
        total = total.checked_add(secs)?;
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

    /// A zone known only by its VTIMEZONE (+01:00 in winter, +02:00 in summer, EU rules).
    const CUSTOM_ZONE: &str = "BEGIN:VTIMEZONE\r\n\
TZID:Custom/Zone\r\n\
BEGIN:STANDARD\r\n\
DTSTART:19701025T030000\r\n\
RRULE:FREQ=YEARLY;BYMONTH=10;BYDAY=-1SU\r\n\
TZOFFSETFROM:+0200\r\n\
TZOFFSETTO:+0100\r\n\
END:STANDARD\r\n\
BEGIN:DAYLIGHT\r\n\
DTSTART:19700329T020000\r\n\
RRULE:FREQ=YEARLY;BYMONTH=3;BYDAY=-1SU\r\n\
TZOFFSETFROM:+0100\r\n\
TZOFFSETTO:+0200\r\n\
END:DAYLIGHT\r\n\
END:VTIMEZONE\r\n";

    fn starts(events: &[Event]) -> Vec<(String, String)> {
        events
            .iter()
            .map(|e| (e.summary.clone(), e.dtstart.clone()))
            .collect()
    }

    fn ts(s: &str) -> i64 {
        chrono::DateTime::parse_from_rfc3339(s).unwrap().timestamp()
    }

    /// A rule in a VTIMEZONE-only zone recurs in that zone's wall clock, each occurrence with the
    /// offset in effect on its date; overrides, EXDATE and RDATE match in that zone.
    #[test]
    fn recurring_event_in_embedded_vtimezone_keeps_its_wall_clock() {
        let ical = format!(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Test//EN\r\n{CUSTOM_ZONE}\
BEGIN:VEVENT\r\n\
UID:weekly\r\n\
SUMMARY:Weekly\r\n\
DTSTART;TZID=Custom/Zone:20260105T090000\r\n\
DTEND;TZID=Custom/Zone:20260105T100000\r\n\
RRULE:FREQ=WEEKLY\r\n\
EXDATE;TZID=Custom/Zone:20260126T090000\r\n\
EXDATE:20260608T070000Z\r\n\
RDATE;TZID=Custom/Zone:20260707T090000\r\n\
END:VEVENT\r\n\
BEGIN:VEVENT\r\n\
UID:weekly\r\n\
SUMMARY:Moved\r\n\
RECURRENCE-ID;TZID=Custom/Zone:20260112T090000\r\n\
DTSTART;TZID=Custom/Zone:20260112T110000\r\n\
DTEND;TZID=Custom/Zone:20260112T120000\r\n\
END:VEVENT\r\n\
END:VCALENDAR\r\n"
        );
        let start = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let end = NaiveDate::from_ymd_opt(2026, 7, 31).unwrap();
        let events = parse_and_expand_events(&ical, start, end);
        let on = |date: &str| -> Vec<&Event> {
            events
                .iter()
                .filter(|e| e.date.to_string() == date)
                .collect()
        };

        let winter = on("2026-01-19");
        assert_eq!(winter.len(), 1, "{:?}", starts(&events));
        assert_eq!(winter[0].dtstart, "2026-01-19T09:00:00+01:00");
        assert_eq!(
            winter[0].dtend.as_deref(),
            Some("2026-01-19T10:00:00+01:00")
        );
        assert_eq!(winter[0].start_ts, Some(ts("2026-01-19T08:00:00Z")));
        assert_eq!(winter[0].tz, "Custom/Zone");

        let summer = on("2026-07-13");
        assert_eq!(summer.len(), 1);
        assert_eq!(summer[0].dtstart, "2026-07-13T09:00:00+02:00");
        assert_eq!(summer[0].start_ts, Some(ts("2026-07-13T07:00:00Z")));

        // the override replaces its occurrence
        let moved = on("2026-01-12");
        assert_eq!(moved.len(), 1);
        assert_eq!(moved[0].summary, "Moved");
        assert_eq!(moved[0].dtstart, "2026-01-12T11:00:00+01:00");

        // EXDATE removes one, RDATE adds one (a Tuesday in summer time)
        assert!(on("2026-01-26").is_empty());
        // an EXDATE in UTC: 07:00Z is 09:00 summer time
        assert!(on("2026-06-08").is_empty());
        let extra = on("2026-07-07");
        assert_eq!(extra.len(), 1);
        assert_eq!(extra[0].dtstart, "2026-07-07T09:00:00+02:00");

        // every other occurrence is a Monday at 09:00 local
        for e in events.iter().filter(|e| e.date.to_string() != "2026-07-07") {
            assert_eq!(e.date.weekday(), chrono::Weekday::Mon, "{}", e.dtstart);
            assert_eq!(
                e.start_time.as_deref(),
                Some(if e.summary == "Moved" {
                    "11:00"
                } else {
                    "09:00"
                })
            );
        }
        // 30 Mondays from Jan 5 to Jul 27, less the EXDATEs, plus the RDATE
        assert_eq!(events.len(), 29);
    }

    /// BYDAY and the window are the zone's local dates: 00:30 on a Monday stays on Monday.
    #[test]
    fn byday_in_embedded_vtimezone_is_the_local_weekday() {
        let ical = format!(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Test//EN\r\n{CUSTOM_ZONE}\
BEGIN:VEVENT\r\n\
UID:late\r\n\
SUMMARY:Late\r\n\
DTSTART;TZID=Custom/Zone:20260105T003000\r\n\
RRULE:FREQ=WEEKLY;BYDAY=MO\r\n\
END:VEVENT\r\n\
END:VCALENDAR\r\n"
        );
        let start = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let end = NaiveDate::from_ymd_opt(2026, 1, 31).unwrap();
        let events = parse_and_expand_events(&ical, start, end);
        let mut dates: Vec<String> = events.iter().map(|e| e.date.to_string()).collect();
        dates.sort();
        assert_eq!(
            dates,
            ["2026-01-05", "2026-01-12", "2026-01-19", "2026-01-26"],
            "{:?}",
            starts(&events)
        );
        for e in &events {
            assert_eq!(e.start_time.as_deref(), Some("00:30"));
            assert!(e.dtstart.ends_with("T00:30:00+01:00"), "{}", e.dtstart);
        }
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

/// A set's occurrences from `from` to `to` (naive UTC), as the expansion walks them.
#[cfg(test)]
pub(super) fn occurrences_for_tests(
    text: &str,
    from: NaiveDateTime,
    to: NaiveDateTime,
) -> Option<Vec<NaiveDateTime>> {
    let utc = |dt: NaiveDateTime| rrule::Tz::UTC.from_utc_datetime(&dt);
    occurrences_between(text, utc(from), utc(to), |_| true)
        .map(|v| v.into_iter().map(|d| d.naive_utc()).collect())
}
