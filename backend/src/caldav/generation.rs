use chrono::{NaiveDate, NaiveDateTime, TimeDelta, Utc};
use uuid::Uuid;

use super::types::{Alarm, Attendee, RecurrenceInfo};

pub fn generate_new_uid() -> String {
    Uuid::new_v4().to_string()
}

/// At most this many events in one imported file (a larger one is refused).
pub const MAX_IMPORT_EVENTS: usize = 1000;
/// A VTIMEZONE larger than this is not carried into the events (real ones are a few KB).
const MAX_VTIMEZONE_BYTES: usize = 16 * 1024;
/// At most this many VTIMEZONEs per event.
const MAX_EVENT_TIMEZONES: usize = 4;
/// At most this many calendar-level property lines, each at most this long, are carried into each event.
const MAX_CALENDAR_LINES: usize = 8;
const MAX_CALENDAR_LINE_BYTES: usize = 256;

/// A VCALENDAR file split into its events, each wrapped on demand (`events`), so a file is held once.
pub struct SplitCalendar {
    /// Calendar-level property lines (not VERSION, PRODID or METHOD: a stored object has no METHOD).
    header: String,
    /// (TZID, the VTIMEZONE's lines).
    timezones: Vec<(String, String)>,
    /// The VEVENT blocks, CRLF-joined.
    blocks: Vec<String>,
    /// More than MAX_IMPORT_EVENTS events: only the first are in `blocks`.
    pub too_many: bool,
}

impl SplitCalendar {
    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    /// Each event's (uid, VCALENDAR): the event, the calendar-level lines and the VTIMEZONEs its TZIDs
    /// name. An event without UID gets a new one.
    pub fn events(&self) -> impl Iterator<Item = (String, String)> + '_ {
        self.blocks.iter().map(|block| {
            let unfolded = unfold(block);
            let uid = unfolded
                .iter()
                .find_map(|l| l.strip_prefix("UID:"))
                .map(|u| u.trim().to_string())
                .filter(|u| !u.is_empty())
                .unwrap_or_else(generate_new_uid);
            let mut tzids: Vec<&str> = Vec::new();
            for line in &unfolded {
                for tzid in line_tzids(line) {
                    if !tzids.contains(&tzid) && tzids.len() < MAX_EVENT_TIMEZONES {
                        tzids.push(tzid);
                    }
                }
            }
            let mut wrapped =
                String::from("BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Caley//Caley//EN\r\n");
            wrapped.push_str(&self.header);
            for (_, vtimezone) in self
                .timezones
                .iter()
                .filter(|(id, _)| tzids.contains(&id.as_str()))
            {
                wrapped.push_str(vtimezone);
            }
            wrapped.push_str(block);
            wrapped.push_str("\r\nEND:VCALENDAR\r\n");
            (uid, wrapped)
        })
    }
}

/// The lines of a block with folded continuation lines joined.
fn unfold(block: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in block.lines() {
        match (line.strip_prefix([' ', '\t']), out.last_mut()) {
            (Some(rest), Some(last)) => last.push_str(rest),
            _ => out.push(line.to_owned()),
        }
    }
    out
}

/// The TZID parameters of a property line (`DTSTART;TZID=Europe/Berlin:…`, quoted or not).
fn line_tzids(line: &str) -> impl Iterator<Item = &str> {
    // the parameters end at the first `:` outside quotes
    let mut quoted = false;
    let end = line
        .char_indices()
        .find(|&(_, c)| {
            if c == '"' {
                quoted = !quoted;
            }
            c == ':' && !quoted
        })
        .map_or(line.len(), |(i, _)| i);
    line[..end].split(';').skip(1).filter_map(|param| {
        let (k, v) = param.split_once('=')?;
        k.eq_ignore_ascii_case("TZID").then(|| v.trim_matches('"'))
    })
}

/// Split a multi-VEVENT VCALENDAR blob into its events (`SplitCalendar::events`), at most
/// MAX_IMPORT_EVENTS. Each keeps only what it needs: the VTIMEZONEs its TZIDs name, and a few
/// calendar-level lines; other components (VTODO, …) and METHOD are left out.
pub fn split_vcalendar_into_events(ical_content: &str) -> SplitCalendar {
    let normalised = ical_content.replace("\r\n", "\n");
    let mut split = SplitCalendar {
        header: String::new(),
        timezones: Vec::new(),
        blocks: Vec::new(),
        too_many: false,
    };
    let mut header_lines = 0;
    // the top-level component being read (other than VCALENDAR), and how deep inside it
    let mut component: Option<(String, Vec<&str>)> = None;
    let mut depth = 0usize;

    for line in normalised.lines() {
        let line = line.trim_end_matches('\r');
        if let Some((name, lines)) = component.as_mut() {
            lines.push(line);
            if line.starts_with("BEGIN:") {
                depth += 1;
            } else if line.starts_with("END:") {
                depth -= 1;
                if depth == 0 {
                    let (name, lines) = (name.clone(), std::mem::take(lines));
                    component = None;
                    match name.as_str() {
                        "VEVENT" if split.blocks.len() < MAX_IMPORT_EVENTS => {
                            split.blocks.push(lines.join("\r\n"));
                        }
                        "VEVENT" => split.too_many = true,
                        "VTIMEZONE" => {
                            let text = lines.join("\r\n") + "\r\n";
                            let tzid = unfold(&text)
                                .iter()
                                .find_map(|l| l.strip_prefix("TZID:").map(|t| t.trim().to_owned()));
                            if let Some(tzid) = tzid {
                                if text.len() <= MAX_VTIMEZONE_BYTES
                                    && !split.timezones.iter().any(|(id, _)| *id == tzid)
                                {
                                    split.timezones.push((tzid, text));
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        } else if let Some(name) = line.strip_prefix("BEGIN:") {
            if name != "VCALENDAR" {
                component = Some((name.to_owned(), vec![line]));
                depth = 1;
            }
        } else if line.starts_with("END:")
            || line.starts_with("VERSION:")
            || line.starts_with("PRODID:")
            || line.starts_with("METHOD:")
            || line.starts_with([' ', '\t'])
            || line.is_empty()
        {
            // the wrapper's own, not for a stored object, or a folded line (of a dropped one)
        } else if header_lines < MAX_CALENDAR_LINES && line.len() <= MAX_CALENDAR_LINE_BYTES {
            split.header.push_str(line);
            split.header.push_str("\r\n");
            header_lines += 1;
        }
    }
    split
}

fn escape_ical_text(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace(';', "\\;")
        .replace(',', "\\,")
}

/// Generate a VCALENDAR with a single VEVENT.
/// `recurrence_id` is set for "edit this instance" overrides.
/// `recurrence` carries the RRULE information (None for non-recurring or instance overrides).
#[allow(clippy::too_many_arguments)]
pub fn generate_ical_event(
    uid: &str,
    summary: &str,
    description: &str,
    location: &str,
    date: &str,
    start_time: Option<&str>,
    end_time: Option<&str>,
    tz: &str,
    is_all_day: bool,
    recurrence_id: Option<(&str, &str)>,
    recurrence: Option<&RecurrenceInfo>,
    alarms: &[Alarm],
    attendees: &[Attendee],
) -> String {
    let dtstart = if is_all_day {
        date.replace('-', "")
    } else {
        let (d, t) = parse_datetime(date, start_time.unwrap_or("00:00"));
        if tz.is_empty() || tz == "UTC" {
            format!("{}T{}00Z", d, t)
        } else {
            format!("{}T{}00", d, t)
        }
    };

    let dtend = if is_all_day {
        // exclusive end: next day
        let d = NaiveDate::parse_from_str(date, "%Y-%m-%d")
            .unwrap_or_else(|_| chrono::Local::now().date_naive());
        let next = d + TimeDelta::days(1);
        next.format("%Y%m%d").to_string()
    } else {
        let (d, t) = parse_datetime(date, end_time.unwrap_or(start_time.unwrap_or("01:00")));
        if tz.is_empty() || tz == "UTC" {
            format!("{}T{}00Z", d, t)
        } else {
            format!("{}T{}00", d, t)
        }
    };

    let mut ical = String::from(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Caley//Caley//EN\r\nBEGIN:VEVENT\r\n",
    );

    ical.push_str(&format!("UID:{}\r\n", uid));
    ical.push_str(&format!(
        "DTSTAMP:{}\r\n",
        Utc::now().format("%Y%m%dT%H%M%SZ")
    ));
    ical.push_str("SEQUENCE:0\r\n");

    if is_all_day {
        ical.push_str(&format!("DTSTART;VALUE=DATE:{}\r\n", dtstart));
        ical.push_str(&format!("DTEND;VALUE=DATE:{}\r\n", dtend));
    } else if tz.is_empty() || tz == "UTC" {
        ical.push_str(&format!("DTSTART:{}\r\n", dtstart));
        ical.push_str(&format!("DTEND:{}\r\n", dtend));
    } else {
        ical.push_str(&format!("DTSTART;TZID={}:{}\r\n", tz, dtstart));
        ical.push_str(&format!("DTEND;TZID={}:{}\r\n", tz, dtend));
    }

    if let Some(rec) = recurrence {
        if !rec.freq.is_empty() {
            ical.push_str(&format!(
                "RRULE:{}\r\n",
                recurrence_info_to_rrule(rec, tz, is_all_day)
            ));
        }
    }

    if let Some((rid_date, rid_tz)) = recurrence_id {
        let is_date = rid_date.len() <= 10;
        // Keep only the local part: a trailing `Z`/`±HH:MM` offset must not
        // leak into the compacted RECURRENCE-ID value.
        let rid_local = rid_date.get(..19).unwrap_or(rid_date);
        let rid_ical = if is_date {
            rid_date.replace('-', "")
        } else if rid_tz.is_empty() || rid_tz == "UTC" {
            format!("{}Z", rid_local.replace('-', "").replace(':', ""))
        } else {
            rid_local.replace('-', "").replace(':', "")
        };
        if is_date && is_all_day {
            ical.push_str(&format!("RECURRENCE-ID;VALUE=DATE:{}\r\n", rid_ical));
        } else if is_date {
            // Timed override of an all-day master: use midnight to keep same type as DTSTART
            if rid_tz.is_empty() || rid_tz == "UTC" {
                ical.push_str(&format!("RECURRENCE-ID:{}T000000Z\r\n", rid_ical));
            } else {
                ical.push_str(&format!(
                    "RECURRENCE-ID;TZID={}:{}T000000\r\n",
                    rid_tz, rid_ical
                ));
            }
        } else if rid_tz.is_empty() || rid_tz == "UTC" {
            ical.push_str(&format!("RECURRENCE-ID:{}\r\n", rid_ical));
        } else {
            ical.push_str(&format!("RECURRENCE-ID;TZID={}:{}\r\n", rid_tz, rid_ical));
        }
    }

    ical.push_str(&format!("SUMMARY:{}\r\n", escape_ical_text(summary)));
    if !description.is_empty() {
        ical.push_str(&format!(
            "DESCRIPTION:{}\r\n",
            escape_ical_text(description)
        ));
    }
    if !location.is_empty() {
        ical.push_str(&format!("LOCATION:{}\r\n", escape_ical_text(location)));
    }

    for attendee in attendees {
        let cn_param = if !attendee.cn.is_empty() {
            format!(";CN={}", attendee.cn)
        } else {
            String::new()
        };
        ical.push_str(&format!(
            "ATTENDEE{};PARTSTAT={}:mailto:{}\r\n",
            cn_param,
            attendee.partstat.to_ical(),
            attendee.email
        ));
    }

    for alarm in alarms {
        ical.push_str("BEGIN:VALARM\r\nACTION:DISPLAY\r\nDESCRIPTION:Reminder\r\n");
        match alarm.trigger_type.as_str() {
            "absolute" => {
                if let Some(ref dt) = alarm.datetime {
                    ical.push_str(&format!("TRIGGER;VALUE=DATE-TIME:{}\r\n", dt));
                }
            }
            "end" => {
                if let Some(mins) = alarm.minutes {
                    let sign = if mins < 0 { "-" } else { "" };
                    ical.push_str(&format!(
                        "TRIGGER;RELATED=END:{}PT{}M\r\n",
                        sign,
                        mins.unsigned_abs()
                    ));
                }
            }
            _ => {
                if let Some(mins) = alarm.minutes {
                    let sign = if mins < 0 { "-" } else { "" };
                    ical.push_str(&format!("TRIGGER:{}PT{}M\r\n", sign, mins.unsigned_abs()));
                }
            }
        }
        ical.push_str("END:VALARM\r\n");
    }

    ical.push_str("END:VEVENT\r\nEND:VCALENDAR\r\n");
    ical
}

/// Convert a RecurrenceInfo back to an iCal RRULE string (inverse of parse_rrule_info).
pub fn recurrence_info_to_rrule(info: &RecurrenceInfo, tz: &str, is_all_day: bool) -> String {
    let mut parts = vec![format!("FREQ={}", info.freq)];
    if info.interval > 1 {
        parts.push(format!("INTERVAL={}", info.interval));
    }
    if !info.by_day.is_empty() {
        parts.push(format!("BYDAY={}", info.by_day.join(",")));
    }
    if !info.by_month_day.is_empty() {
        parts.push(format!(
            "BYMONTHDAY={}",
            info.by_month_day
                .iter()
                .map(|d| d.to_string())
                .collect::<Vec<_>>()
                .join(",")
        ));
    }
    if !info.by_month.is_empty() {
        parts.push(format!(
            "BYMONTH={}",
            info.by_month
                .iter()
                .map(|d| d.to_string())
                .collect::<Vec<_>>()
                .join(",")
        ));
    }
    if !info.by_set_pos.is_empty() {
        parts.push(format!(
            "BYSETPOS={}",
            info.by_set_pos
                .iter()
                .map(|d| d.to_string())
                .collect::<Vec<_>>()
                .join(",")
        ));
    }
    if let Some(count) = info.count {
        parts.push(format!("COUNT={}", count));
    }
    if let Some(ref until) = info.until {
        parts.push(format!("UNTIL={}", format_until(until, tz, is_all_day)));
    }
    parts.join(";")
}

fn format_until(until: &str, _tz: &str, is_all_day: bool) -> String {
    if is_all_day {
        if let Ok(d) = NaiveDate::parse_from_str(until, "%Y-%m-%d") {
            d.format("%Y%m%d").to_string()
        } else {
            until.to_string()
        }
    } else if let Ok(dt) = NaiveDateTime::parse_from_str(until, "%Y-%m-%d %H:%M:%S") {
        format!("{}T{}Z", dt.format("%Y%m%d"), dt.format("%H%M%S"))
    } else if let Ok(d) = NaiveDate::parse_from_str(until, "%Y-%m-%d") {
        format!("{}T235959Z", d.format("%Y%m%d"))
    } else {
        until.to_string()
    }
}

fn parse_datetime(date: &str, time: &str) -> (String, String) {
    let d = date.replace('-', "");
    if time.is_empty() {
        (d, "0000".to_string())
    } else {
        let t = time.replace(':', "");
        (d, t)
    }
}

/// Inject or replace a RECURRENCE-ID override VEVENT into an existing iCal string.
pub fn add_recurrence_override(
    ical_data: &str,
    override_vevent: &str,
    recurrence_id_value: &str,
) -> String {
    let ical = ical_data.replace("\r\n", "\n");
    let override_vevent = override_vevent.replace("\r\n", "\n");

    // Match overrides by the 8-digit YYYYMMDD date only, so VALUE=DATE and
    // datetime formats (e.g. T000000Z) both hit for the same calendar date.
    let rid_date: String = recurrence_id_value
        .replace('-', "")
        .chars()
        .take(8)
        .collect();

    let mut result = String::new();
    let mut found = false;
    let mut search_from = 0;

    while let Some(start) = ical[search_from..].find("BEGIN:VEVENT") {
        let abs_start = search_from + start;
        result.push_str(&ical[search_from..abs_start]);

        if let Some(end) = ical[abs_start..].find("\nEND:VEVENT") {
            let abs_end = abs_start + end + 1 + 10;
            let block = &ical[abs_start..abs_end];

            let needs_replace = block.lines().any(|l| {
                let t = l.trim();
                if !t.starts_with("RECURRENCE-ID:") && !t.starts_with("RECURRENCE-ID;") {
                    return false;
                }
                t.rsplit(':').next().map_or(false, |val| {
                    val.chars().take(8).collect::<String>() == rid_date
                })
            });

            if needs_replace && !found {
                result.push_str(&override_vevent);
                if !override_vevent.ends_with('\n') {
                    result.push('\n');
                }
                found = true;
            } else {
                result.push_str(block);
                if !block.ends_with('\n') {
                    result.push('\n');
                }
            }
            search_from = abs_end;
        } else {
            result.push_str(&ical[search_from..]);
            search_from = ical.len();
            break;
        }
    }
    result.push_str(&ical[search_from..]);

    if !found {
        if let Some(pos) = result.rfind("END:VCALENDAR") {
            let before = result[..pos].to_string();
            let after = result[pos..].to_string();
            let sep = if override_vevent.ends_with('\n') {
                ""
            } else {
                "\n"
            };
            result = format!("{}{}{}{}", before, override_vevent, sep, after);
        }
    }

    result.replace('\n', "\r\n")
}

/// Add EXDATE for one occurrence to the master VEVENT and drop any existing
/// override VEVENT for the same date.
pub fn add_exdate(ical_data: &str, occurrence_dtstart: &str, tz: &str) -> String {
    let ical = ical_data.replace("\r\n", "\n");
    let is_all_day = !occurrence_dtstart.contains('T');

    let exdate_line = if is_all_day {
        let date_compact = occurrence_dtstart.replace('-', "");
        format!("EXDATE;VALUE=DATE:{}", date_compact)
    } else {
        let parts: Vec<&str> = occurrence_dtstart.splitn(2, 'T').collect();
        let date_compact = parts[0].replace('-', "");
        let time_str = parts.get(1).copied().unwrap_or("000000Z");
        // "09:00:00-05:00" → "090000": drop separators and stop before any
        // trailing offset digits.
        let time_compact: String = time_str
            .chars()
            .filter(|c| c.is_ascii_digit())
            .take(6)
            .collect();
        if occurrence_dtstart.ends_with('Z') || tz.is_empty() || tz == "UTC" {
            format!("EXDATE:{}T{}Z", date_compact, time_compact)
        } else {
            format!("EXDATE;TZID={}:{}T{}", tz, date_compact, time_compact)
        }
    };

    let rid_date: String = occurrence_dtstart
        .replace('-', "")
        .chars()
        .take(8)
        .collect();

    let mut result = String::new();
    let mut search_from = 0;
    let mut master_modified = false;

    while let Some(rel_start) = ical[search_from..].find("BEGIN:VEVENT") {
        let abs_start = search_from + rel_start;
        result.push_str(&ical[search_from..abs_start]);

        if let Some(rel_end) = ical[abs_start..].find("\nEND:VEVENT") {
            let abs_end = abs_start + rel_end + 1 + 10; // \n + "END:VEVENT"
            let block = &ical[abs_start..abs_end];

            let has_recurrence_id = block.lines().any(|l| {
                let t = l.trim();
                t.starts_with("RECURRENCE-ID:") || t.starts_with("RECURRENCE-ID;")
            });

            if has_recurrence_id {
                let is_target = block.lines().any(|l| {
                    let t = l.trim();
                    if !t.starts_with("RECURRENCE-ID:") && !t.starts_with("RECURRENCE-ID;") {
                        return false;
                    }
                    t.rsplit(':').next().map_or(false, |val| {
                        val.chars().take(8).collect::<String>() == rid_date
                    })
                });
                if !is_target {
                    result.push_str(block);
                }
            } else if !master_modified {
                let before_end = &ical[abs_start..abs_start + rel_end];
                result.push_str(before_end);
                result.push('\n');
                result.push_str(&exdate_line);
                result.push('\n');
                result.push_str("END:VEVENT");
                master_modified = true;
            } else {
                result.push_str(block);
            }
            search_from = abs_end;
        } else {
            result.push_str(&ical[search_from..]);
            search_from = ical.len();
            break;
        }
    }
    result.push_str(&ical[search_from..]);
    result.replace('\n', "\r\n")
}

// ── Non-destructive event update ─────────────────────────────────────────────

/// Undo RFC 5545 line folding: join continuation lines (CRLF + whitespace).
fn unfold_lines(ical: &str) -> String {
    ical.replace("\r\n", "\n")
        .replace("\n ", "")
        .replace("\n\t", "")
}

/// Re-fold lines to ≤75 bytes each, using CRLF + SP continuation per RFC 5545.
fn fold_lines(ical: &str) -> String {
    let mut out = String::with_capacity(ical.len() + ical.len() / 32);
    for line in ical.split('\n') {
        if line.is_empty() {
            out.push_str("\r\n");
            continue;
        }
        let bytes = line.as_bytes();
        if bytes.len() <= 75 {
            out.push_str(line);
            out.push_str("\r\n");
            continue;
        }
        let mut pos = 75;
        while !line.is_char_boundary(pos) {
            pos -= 1;
        }
        out.push_str(&line[..pos]);
        out.push_str("\r\n");
        let mut rest = &line[pos..];
        while !rest.is_empty() {
            out.push(' ');
            let chunk = 74.min(rest.len());
            let mut end = chunk;
            while !rest.is_char_boundary(end) {
                end -= 1;
            }
            out.push_str(&rest[..end]);
            out.push_str("\r\n");
            rest = &rest[end..];
        }
    }
    out
}

/// Extract the property name from an unfolded iCal line (part before first `:` or `;`).
fn prop_name(line: &str) -> &str {
    let c = line.find(':').unwrap_or(line.len());
    let s = line.find(';').unwrap_or(line.len());
    &line[..c.min(s)]
}

/// Properties we replace when doing a non-destructive update.
const REPLACE_PROPS: &[&str] = &[
    "DTSTAMP",
    "LAST-MODIFIED",
    "SEQUENCE",
    "SUMMARY",
    "DESCRIPTION",
    "LOCATION",
    "DTSTART",
    "DTEND",
    "DURATION",
    "RRULE",
    "RECURRENCE-ID",
];

/// Extract the email from an unfolded ATTENDEE line (the part after the last `:mailto:`).
fn attendee_email(line: &str) -> Option<&str> {
    line.rfind(":mailto:").map(|pos| &line[pos + 8..])
}

/// Return a copy of an ATTENDEE line with PARTSTAT set to `new_partstat`.
fn update_attendee_partstat(line: &str, new_partstat: &str) -> String {
    let Some(value_start) = line.rfind(":mailto:") else {
        return line.to_string();
    };
    let params = &line[..value_start];
    let value = &line[value_start..];
    let new_params = if let Some(ps) = params.find("PARTSTAT=") {
        let after = &params[ps + 9..];
        let end = after.find(';').unwrap_or(after.len());
        format!(
            "{}PARTSTAT={}{}",
            &params[..ps],
            new_partstat,
            &after[end..]
        )
    } else {
        format!("{};PARTSTAT={}", params, new_partstat)
    };
    format!("{}{}", new_params, value)
}

#[allow(clippy::too_many_arguments)]
fn rebuild_vevent(
    block: &str,
    summary: &str,
    description: &str,
    location: &str,
    date: &str,
    start_time: Option<&str>,
    end_time: Option<&str>,
    tz: &str,
    is_all_day: bool,
    new_sequence: u32,
    target_recurrence_id: Option<(&str, &str)>,
    recurrence: Option<&RecurrenceInfo>,
    alarms: &[Alarm],
    attendees: &[Attendee],
) -> String {
    let dtstart = if is_all_day {
        date.replace('-', "")
    } else {
        let (d, t) = parse_datetime(date, start_time.unwrap_or("00:00"));
        if tz.is_empty() || tz == "UTC" {
            format!("{}T{}00Z", d, t)
        } else {
            format!("{}T{}00", d, t)
        }
    };

    let dtend = if is_all_day {
        let d = NaiveDate::parse_from_str(date, "%Y-%m-%d")
            .unwrap_or_else(|_| chrono::Local::now().date_naive());
        (d + TimeDelta::days(1)).format("%Y%m%d").to_string()
    } else {
        let (d, t) = parse_datetime(date, end_time.unwrap_or(start_time.unwrap_or("01:00")));
        if tz.is_empty() || tz == "UTC" {
            format!("{}T{}00Z", d, t)
        } else {
            format!("{}T{}00", d, t)
        }
    };

    let mut kept = String::new();
    let mut original_attendee_lines: Vec<String> = Vec::new();
    let mut preserved_valarns: Vec<String> = Vec::new();
    let mut in_valarm = false;
    let mut current_valarm: Vec<&str> = Vec::new();
    for line in block.lines() {
        if line == "BEGIN:VALARM" {
            in_valarm = true;
            current_valarm.clear();
            continue;
        }
        if line == "END:VALARM" {
            in_valarm = false;
            // Preserve VALARMs whose ACTION is not DISPLAY — those are managed
            // by other clients (EMAIL, AUDIO) and should survive our edits.
            let is_display = current_valarm
                .iter()
                .any(|l| l.trim().eq_ignore_ascii_case("ACTION:DISPLAY"));
            if !is_display {
                let mut blk = String::from("BEGIN:VALARM\n");
                for vl in &current_valarm {
                    blk.push_str(vl);
                    blk.push('\n');
                }
                blk.push_str("END:VALARM\n");
                preserved_valarns.push(blk);
            }
            continue;
        }
        if in_valarm {
            current_valarm.push(line);
            continue;
        }
        if line == "BEGIN:VEVENT" || line == "END:VEVENT" {
            continue;
        }
        let pname = prop_name(line).to_ascii_uppercase();
        if pname == "ATTENDEE" {
            original_attendee_lines.push(line.to_string());
            continue;
        }
        if REPLACE_PROPS.contains(&pname.as_str()) {
            continue;
        }
        kept.push_str(line);
        kept.push('\n');
    }

    let now = Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
    let mut out = String::from("BEGIN:VEVENT\n");
    out.push_str(&kept);
    out.push_str(&format!("DTSTAMP:{}\n", now));
    out.push_str(&format!("LAST-MODIFIED:{}\n", now));
    out.push_str(&format!("SEQUENCE:{}\n", new_sequence));

    if is_all_day {
        out.push_str(&format!("DTSTART;VALUE=DATE:{}\n", dtstart));
        out.push_str(&format!("DTEND;VALUE=DATE:{}\n", dtend));
    } else if tz.is_empty() || tz == "UTC" {
        out.push_str(&format!("DTSTART:{}\n", dtstart));
        out.push_str(&format!("DTEND:{}\n", dtend));
    } else {
        out.push_str(&format!("DTSTART;TZID={}:{}\n", tz, dtstart));
        out.push_str(&format!("DTEND;TZID={}:{}\n", tz, dtend));
    }

    if let Some(rec) = recurrence {
        if !rec.freq.is_empty() {
            out.push_str(&format!(
                "RRULE:{}\n",
                recurrence_info_to_rrule(rec, tz, is_all_day)
            ));
        }
    }

    if let Some((rid_date, rid_tz)) = target_recurrence_id {
        let is_date = rid_date.len() <= 10;
        // Keep only the local part: a trailing `Z`/`±HH:MM` offset must not
        // leak into the compacted RECURRENCE-ID value.
        let rid_local = rid_date.get(..19).unwrap_or(rid_date);
        let rid_ical = if is_date {
            rid_date.replace('-', "")
        } else if rid_tz.is_empty() || rid_tz == "UTC" {
            format!("{}Z", rid_local.replace('-', "").replace(':', ""))
        } else {
            rid_local.replace('-', "").replace(':', "")
        };
        if is_date && is_all_day {
            out.push_str(&format!("RECURRENCE-ID;VALUE=DATE:{}\n", rid_ical));
        } else if is_date {
            if rid_tz.is_empty() || rid_tz == "UTC" {
                out.push_str(&format!("RECURRENCE-ID:{}T000000Z\n", rid_ical));
            } else {
                out.push_str(&format!(
                    "RECURRENCE-ID;TZID={}:{}T000000\n",
                    rid_tz, rid_ical
                ));
            }
        } else if rid_tz.is_empty() || rid_tz == "UTC" {
            out.push_str(&format!("RECURRENCE-ID:{}\n", rid_ical));
        } else {
            out.push_str(&format!("RECURRENCE-ID;TZID={}:{}\n", rid_tz, rid_ical));
        }
    }

    out.push_str(&format!("SUMMARY:{}\n", escape_ical_text(summary)));
    if !description.is_empty() {
        out.push_str(&format!("DESCRIPTION:{}\n", escape_ical_text(description)));
    }
    if !location.is_empty() {
        out.push_str(&format!("LOCATION:{}\n", escape_ical_text(location)));
    }

    // Attendee smart merge: for known emails, preserve the original line and
    // update only PARTSTAT; for new emails, generate a minimal line.
    for att in attendees {
        let email_lower = att.email.trim().to_lowercase();
        let orig = original_attendee_lines.iter().find(|l| {
            attendee_email(l)
                .map(|e| e.trim().to_lowercase())
                .as_deref()
                == Some(email_lower.as_str())
        });
        if let Some(orig_line) = orig {
            out.push_str(&update_attendee_partstat(
                orig_line,
                &att.partstat.to_ical(),
            ));
        } else {
            let cn = if !att.cn.is_empty() {
                format!(";CN={}", att.cn)
            } else {
                String::new()
            };
            out.push_str(&format!(
                "ATTENDEE{};PARTSTAT={}:mailto:{}",
                cn,
                att.partstat.to_ical(),
                att.email
            ));
        }
        out.push('\n');
    }

    // Emit preserved non-DISPLAY VALARMs (EMAIL, AUDIO) before form-managed ones.
    for valarm in &preserved_valarns {
        out.push_str(valarm);
    }
    for alarm in alarms {
        out.push_str("BEGIN:VALARM\nACTION:DISPLAY\nDESCRIPTION:Reminder\n");
        match alarm.trigger_type.as_str() {
            "absolute" => {
                if let Some(ref dt) = alarm.datetime {
                    out.push_str(&format!("TRIGGER;VALUE=DATE-TIME:{}\n", dt));
                }
            }
            "end" => {
                if let Some(mins) = alarm.minutes {
                    let sign = if mins < 0 { "-" } else { "" };
                    out.push_str(&format!(
                        "TRIGGER;RELATED=END:{}PT{}M\n",
                        sign,
                        mins.unsigned_abs()
                    ));
                }
            }
            _ => {
                if let Some(mins) = alarm.minutes {
                    let sign = if mins < 0 { "-" } else { "" };
                    out.push_str(&format!("TRIGGER:{}PT{}M\n", sign, mins.unsigned_abs()));
                }
            }
        }
        out.push_str("END:VALARM\n");
    }

    out.push_str("END:VEVENT");
    out
}

/// Update an existing event in a VCALENDAR non-destructively.
///
/// Returns None if the target VEVENT is not found (caller should fall back to
/// generate_ical_event).
#[allow(clippy::too_many_arguments)]
pub fn update_vcalendar_event(
    original_ical: &str,
    summary: &str,
    description: &str,
    location: &str,
    date: &str,
    start_time: Option<&str>,
    end_time: Option<&str>,
    tz: &str,
    is_all_day: bool,
    target_recurrence_id: Option<(&str, &str)>,
    recurrence: Option<&RecurrenceInfo>,
    alarms: &[Alarm],
    attendees: &[Attendee],
) -> Option<String> {
    let text = unfold_lines(original_ical);

    let rid_date_prefix: Option<String> =
        target_recurrence_id.map(|(rv, _)| rv.replace('-', "").chars().take(8).collect());

    let mut result = String::new();
    let mut search_from = 0;
    let mut found_target = false;

    while let Some(rel_start) = text[search_from..].find("BEGIN:VEVENT") {
        let abs_start = search_from + rel_start;
        result.push_str(&text[search_from..abs_start]);

        let Some(rel_end) = text[abs_start..].find("\nEND:VEVENT") else {
            result.push_str(&text[search_from..]);
            search_from = text.len();
            break;
        };
        let abs_end = abs_start + rel_end + 1 + 10; // '\n' + "END:VEVENT"
        let block = &text[abs_start..abs_end];

        let is_target = if !found_target {
            match &rid_date_prefix {
                None => !block
                    .lines()
                    .any(|l| l.starts_with("RECURRENCE-ID:") || l.starts_with("RECURRENCE-ID;")),
                Some(rid_prefix) => block.lines().any(|l| {
                    if !l.starts_with("RECURRENCE-ID:") && !l.starts_with("RECURRENCE-ID;") {
                        return false;
                    }
                    l.rsplit(':').next().map_or(false, |val| {
                        val.chars().take(8).collect::<String>() == *rid_prefix
                    })
                }),
            }
        } else {
            false
        };

        if is_target {
            let new_sequence = block
                .lines()
                .find(|l| l.starts_with("SEQUENCE:"))
                .and_then(|l| l.trim_start_matches("SEQUENCE:").parse::<u32>().ok())
                .unwrap_or(0)
                .saturating_add(1);

            let rebuilt = rebuild_vevent(
                block,
                summary,
                description,
                location,
                date,
                start_time,
                end_time,
                tz,
                is_all_day,
                new_sequence,
                target_recurrence_id,
                recurrence,
                alarms,
                attendees,
            );
            result.push_str(&rebuilt);
            result.push('\n');
            found_target = true;
        } else {
            result.push_str(block);
            result.push('\n');
        }
        search_from = abs_end;
    }
    result.push_str(&text[search_from..]);

    if !found_target {
        return None;
    }

    Some(fold_lines(&result))
}

#[cfg(test)]
mod split_tests {
    use super::*;

    const BERLIN: &str = "BEGIN:VTIMEZONE\nTZID:Custom Berlin\nBEGIN:STANDARD\nDTSTART:16011028T030000\nRRULE:FREQ=YEARLY;BYDAY=-1SU;BYMONTH=10\nTZOFFSETFROM:+0200\nTZOFFSETTO:+0100\nEND:STANDARD\nBEGIN:DAYLIGHT\nDTSTART:16010325T020000\nRRULE:FREQ=YEARLY;BYDAY=-1SU;BYMONTH=3\nTZOFFSETFROM:+0100\nTZOFFSETTO:+0200\nEND:DAYLIGHT\nEND:VTIMEZONE\n";

    /// Several events sharing a VTIMEZONE: each gets it (and only the zones it names), no METHOD, no
    /// other component.
    #[test]
    fn events_carry_the_timezones_they_name() {
        let ics = format!(
            "BEGIN:VCALENDAR\nVERSION:2.0\nPRODID:x\nMETHOD:PUBLISH\nX-WR-CALNAME:Team\n{BERLIN}\
             BEGIN:VTIMEZONE\nTZID:Other\nBEGIN:STANDARD\nDTSTART:19700101T000000\nTZOFFSETFROM:+0300\nTZOFFSETTO:+0300\nEND:STANDARD\nEND:VTIMEZONE\n\
             BEGIN:VTODO\nUID:todo\nEND:VTODO\n\
             BEGIN:VEVENT\nUID:a\nDTSTART;TZID=\"Custom Berlin\":20260702T080000\nEND:VEVENT\n\
             BEGIN:VEVENT\nUID:b\nDTSTART;TZID=Custom Berlin:20261124T080000\nDTEND;TZID=Custom Ber\n lin:20261124T090000\nEND:VEVENT\n\
             BEGIN:VEVENT\nUID:c\nDTSTART:20261124T080000Z\nEND:VEVENT\nEND:VCALENDAR\n"
        );
        let split = split_vcalendar_into_events(&ics);
        assert!(!split.too_many);
        let events: Vec<(String, String)> = split.events().collect();
        assert_eq!(
            events.iter().map(|(u, _)| u.as_str()).collect::<Vec<_>>(),
            ["a", "b", "c"]
        );
        for (uid, wrapped) in &events {
            assert!(!wrapped.contains("METHOD"), "{uid}");
            assert!(!wrapped.contains("VTODO"), "{uid}");
            assert!(!wrapped.contains("TZID:Other"), "{uid}");
            assert!(wrapped.contains("X-WR-CALNAME:Team"), "{uid}");
            assert_eq!(wrapped.contains("TZID:Custom Berlin"), uid != "c", "{uid}");
        }
        // the timezone is used: summer and winter offsets
        let a = crate::caldav::preview_events_from_raw_ical(&events[0].1);
        assert_eq!(a[0].dtstart, "2026-07-02T08:00:00+02:00");
        let b = crate::caldav::preview_events_from_raw_ical(&events[1].1);
        assert_eq!(b[0].dtstart, "2026-11-24T08:00:00+01:00");
        assert_eq!(b[0].dtend.as_deref(), Some("2026-11-24T09:00:00+01:00"));
    }

    /// A large calendar-level part is not copied into every event, and the events are bounded.
    #[test]
    fn a_file_is_held_once() {
        let mut ics = format!("BEGIN:VCALENDAR\nX-PAD:{}\n", "x".repeat(100_000));
        ics.push_str(&format!(
            "BEGIN:VTIMEZONE\nTZID:Big\nX-PAD:{}\nEND:VTIMEZONE\n",
            "y".repeat(50_000)
        ));
        for i in 0..MAX_IMPORT_EVENTS {
            ics.push_str(&format!(
                "BEGIN:VEVENT\nUID:e{i}\nDTSTART;TZID=Big:20261124T080000\nEND:VEVENT\n"
            ));
        }
        ics.push_str("END:VCALENDAR\n");
        let split = split_vcalendar_into_events(&ics);
        assert!(!split.too_many);
        let largest = split.events().map(|(_, w)| w.len()).max().unwrap();
        assert!(largest < 200, "{largest}");
        assert_eq!(split.events().count(), MAX_IMPORT_EVENTS);

        let more = ics.replace(
            "END:VCALENDAR\n",
            "BEGIN:VEVENT\nUID:one-more\nEND:VEVENT\nEND:VCALENDAR\n",
        );
        assert!(split_vcalendar_into_events(&more).too_many);
    }
}
