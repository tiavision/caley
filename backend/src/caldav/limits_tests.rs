//! Hostile calendar data (an invite from anyone): no panic, bounded work, and ordinary events exact.

use std::time::{Duration, Instant};

use chrono::NaiveDate;

use super::parsing::{fast_forward, occurrences_for_tests, yearly_transitions};
use super::{CalDavClient, preview_events_from_raw_ical};

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

fn dt(y: i32, m: u32, day: u32) -> chrono::NaiveDateTime {
    d(y, m, day).and_hms_opt(0, 0, 0).unwrap()
}

fn cal(body: &str) -> String {
    format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:x\r\nBEGIN:VEVENT\r\nUID:e1\r\nSUMMARY:S\r\n{body}END:VEVENT\r\nEND:VCALENDAR\r\n"
    )
}

fn expand(raw: &str) -> Vec<super::Event> {
    CalDavClient::events_from_raw_ical(raw, d(2026, 11, 24), d(2026, 11, 26))
}

/// Runs `f`, failing when it takes longer than `limit` (a debug build is slower: the bound is loose).
fn timed<T>(what: &str, limit: Duration, f: impl FnOnce() -> T) -> T {
    let t = Instant::now();
    let out = f();
    let took = t.elapsed();
    println!("{what}: {took:?}");
    let limit = if cfg!(debug_assertions) {
        limit * 10
    } else {
        limit
    };
    assert!(took < limit, "{what} took {took:?}");
    out
}

const BUDGET: Duration = Duration::from_millis(100);

#[test]
fn durations_out_of_range_are_the_default_length() {
    for dur in [
        "P100000000D",
        "P99999999999W",
        "PT9999999999999999S",
        "P9223372036854775807D",
    ] {
        let raw = cal(&format!("DTSTART:20261125T080000Z\r\nDURATION:{dur}\r\n"));
        let events = expand(&raw);
        assert_eq!(events.len(), 1, "{dur}");
        assert_eq!(events[0].dtend.as_deref(), Some("2026-11-25T08:00:00Z"));
        assert_eq!(preview_events_from_raw_ical(&raw).len(), 1);
    }
    let raw = cal("DTSTART:20200101T080000Z\r\nDURATION:P100000000D\r\nRRULE:FREQ=DAILY\r\n");
    assert_eq!(expand(&raw).len(), 3);
    // an ordinary one is kept
    let raw = cal("DTSTART:20261125T080000Z\r\nDURATION:PT1H30M\r\n");
    assert_eq!(
        expand(&raw)[0].dtend.as_deref(),
        Some("2026-11-25T09:30:00Z")
    );
}

#[test]
fn alarms_out_of_range_are_dropped() {
    for trig in [
        "PT9223372036854775807M",
        "PT9223372036854775807H",
        "P9223372036854775807D",
        "-PT153722867280912931M",
    ] {
        let raw = cal(&format!(
            "DTSTART:20261125T080000Z\r\nDTEND:20261125T090000Z\r\nBEGIN:VALARM\r\nACTION:DISPLAY\r\nTRIGGER:{trig}\r\nEND:VALARM\r\n"
        ));
        assert!(expand(&raw)[0].alarms.is_empty(), "{trig}");
    }
}

#[test]
fn non_ascii_rules_are_no_rule() {
    let raw = cal("DTSTART:20261125T080000Z\r\nRRULE:FREQ=WEEKLY;BYDAY=\u{20ac}\r\n");
    let events = expand(&raw);
    assert_eq!(events.len(), 1);
    assert!(!events[0].is_recurring);
    assert_eq!(preview_events_from_raw_ical(&raw).len(), 1);
    // in a VTIMEZONE: the zone is unusable, the time floating
    let raw = "BEGIN:VCALENDAR\r\nBEGIN:VTIMEZONE\r\nTZID:X\r\nBEGIN:STANDARD\r\nDTSTART:20250101T000000\r\nRRULE:FREQ=YEARLY;BYDAY=\u{20ac}\r\nTZOFFSETFROM:+0200\r\nTZOFFSETTO:+0100\r\nEND:STANDARD\r\nEND:VTIMEZONE\r\nBEGIN:VEVENT\r\nUID:e1\r\nDTSTART;TZID=X:20261125T080000\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
    let events = preview_events_from_raw_ical(raw);
    assert_eq!(events[0].dtstart, "2026-11-25T08:00:00");
    // RDATE / EXDATE values
    let raw = cal(
        "DTSTART:20261125T080000Z\r\nRRULE:FREQ=DAILY\r\nEXDATE:2026112\u{e9}T080000Z\r\nRDATE:\u{1F600}\r\n",
    );
    assert_eq!(expand(&raw).len(), 2);
}

#[test]
fn non_ascii_sets_are_refused() {
    // before the crate sees them (it panics slicing inside the character)
    let (from, to) = (dt(2026, 11, 1), dt(2026, 12, 1));
    let rule = "DTSTART:20261125T080000Z\nRRULE:FREQ=WEEKLY;BYDAY=\u{20ac}\n";
    assert!(occurrences_for_tests(rule, from, to).is_none());
}

#[test]
fn secondly_and_minutely_rules_are_ignored() {
    for (start, rule) in [
        ("20261101T000000Z", "FREQ=SECONDLY"),
        ("20260101T000000Z", "FREQ=SECONDLY"),
        ("20200101T000000Z", "FREQ=MINUTELY"),
        ("20200101T000000Z", "freq=minutely;interval=1"),
    ] {
        let desc = "x".repeat(10_000);
        let raw = cal(&format!(
            "DTSTART:{start}\r\nDESCRIPTION:{desc}\r\nRRULE:{rule}\r\n"
        ));
        let events = timed(rule, BUDGET, || expand(&raw));
        assert_eq!(events.len(), 1, "{rule}: the event once, at its DTSTART");
        assert!(!events[0].is_recurring);
    }
    // the same in a VTIMEZONE: the zone is unusable
    let raw = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VTIMEZONE\r\nTZID:Custom\r\nBEGIN:STANDARD\r\nDTSTART:20250101T000000\r\nRRULE:FREQ=SECONDLY\r\nTZOFFSETFROM:+0200\r\nTZOFFSETTO:+0100\r\nEND:STANDARD\r\nEND:VTIMEZONE\r\nBEGIN:VEVENT\r\nUID:e1\r\nSUMMARY:S\r\nDTSTART;TZID=Custom:20261125T080000\r\nDTEND;TZID=Custom:20261125T090000\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
    let events = timed("VTIMEZONE SECONDLY", BUDGET, || expand(raw));
    assert_eq!(events[0].dtstart, "2026-11-25T08:00:00");
    timed("VTIMEZONE SECONDLY preview", BUDGET, || {
        preview_events_from_raw_ical(raw)
    });
}

#[test]
fn rules_from_far_back_are_fast_and_exact() {
    let cases: &[(&str, &str, usize)] = &[
        ("00010101T090000Z", "FREQ=DAILY", 3),
        ("16010101T090000Z", "FREQ=DAILY;INTERVAL=2", 1),
        ("00010101T090000Z", "FREQ=HOURLY", 72),
        ("16010101T003000Z", "FREQ=HOURLY;INTERVAL=5", 15),
        ("16010102T090000Z", "FREQ=WEEKLY;BYDAY=TU,WE", 2),
        ("16010131T090000Z", "FREQ=MONTHLY", 0),
        ("16011124T090000Z", "FREQ=MONTHLY;BYDAY=-1WE", 1),
        ("00011125T090000Z", "FREQ=YEARLY", 1),
    ];
    for (start, rule, n) in cases {
        let raw = cal(&format!("DTSTART:{start}\r\nRRULE:{rule}\r\n"));
        let events = timed(&format!("{rule} from {start}"), BUDGET, || expand(&raw));
        assert_eq!(events.len(), *n, "{rule} from {start}");
    }
    // the 31st of every month: November has none, but December does
    let raw = cal("DTSTART:16010131T090000Z\r\nRRULE:FREQ=MONTHLY\r\n");
    let events = CalDavClient::events_from_raw_ical(&raw, d(2026, 12, 30), d(2027, 1, 1));
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].dtstart, "2026-12-31T09:00:00Z");
}

/// Moving DTSTART forward gives the occurrences walking from the real one gives.
#[test]
fn fast_forward_keeps_the_occurrences() {
    let rules = [
        "FREQ=DAILY",
        "FREQ=DAILY;INTERVAL=3;BYMONTH=1,2,11",
        "FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,TH",
        "FREQ=WEEKLY;INTERVAL=3;WKST=SU",
        "FREQ=HOURLY;INTERVAL=5",
        "FREQ=HOURLY;INTERVAL=7;BYHOUR=1,8,15",
        "FREQ=MONTHLY",
        "FREQ=MONTHLY;INTERVAL=5",
        "FREQ=MONTHLY;BYDAY=-1FR",
        "FREQ=YEARLY;INTERVAL=3",
        "FREQ=YEARLY;BYMONTH=11;BYDAY=4TH",
        "FREQ=DAILY;UNTIL=20261128T000000Z",
    ];
    let target = d(2026, 11, 20).and_hms_opt(0, 0, 0).unwrap();
    // (an hourly walk from the real DTSTART must fit in the walk's budget: from 2025)
    let starts = [
        "20000131T093000",
        "19960229T170000",
        "20011118T000000",
        "20250131T093000",
        "20250228T170000",
    ];
    for start in starts {
        let dtstart = chrono::NaiveDateTime::parse_from_str(start, "%Y%m%dT%H%M%S").unwrap();
        for rule in rules {
            if rule.contains("HOURLY") && !start.starts_with("2025") {
                continue;
            }
            let moved = fast_forward(dtstart, rule, target);
            assert!(moved <= target);
            let walk = |start: chrono::NaiveDateTime| {
                let text = format!("DTSTART:{}Z\nRRULE:{rule}\n", start.format("%Y%m%dT%H%M%S"));
                occurrences_for_tests(&text, target, dt(2027, 3, 1)).unwrap()
            };
            assert_eq!(walk(moved), walk(dtstart), "{rule} from {start}");
        }
    }
    // COUNT counts from the first: not moved
    let dtstart = d(2000, 1, 1).and_hms_opt(0, 0, 0).unwrap();
    assert_eq!(
        fast_forward(dtstart, "FREQ=DAILY;COUNT=9000", target),
        dtstart
    );
}

#[test]
fn rare_rules_are_bounded() {
    for rule in [
        "FREQ=DAILY;BYMONTH=2;BYMONTHDAY=30",
        "FREQ=HOURLY;BYMONTH=2;BYMONTHDAY=30",
        "FREQ=YEARLY;BYMONTH=2;BYMONTHDAY=30",
        "FREQ=DAILY;COUNT=4000000000",
        "FREQ=DAILY;UNTIL=99991231T000000Z",
        "FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR,SA,SU;BYSETPOS=-1",
    ] {
        let raw = cal(&format!("DTSTART:00010101T000000Z\r\nRRULE:{rule}\r\n"));
        timed(rule, BUDGET, || expand(&raw));
    }
}

#[test]
fn occurrences_per_window_are_capped() {
    let minutes: Vec<String> = (0..60).map(|m| m.to_string()).collect();
    let raw = cal(&format!(
        "DTSTART:20261101T000000Z\r\nDESCRIPTION:{}\r\nRRULE:FREQ=HOURLY;BYMINUTE={}\r\n",
        "x".repeat(10_000),
        minutes.join(",")
    ));
    let events = timed("60 an hour", BUDGET, || expand(&raw));
    assert_eq!(events.len(), 1000);
}

/// An Outlook VTIMEZONE (yearly rules from 1601) named by thousands of EXDATEs: each date's offset
/// without walking the rules again, and the result exact.
#[test]
fn outlook_vtimezone_with_many_exdates() {
    for n in [100usize, 1000, 4000] {
        let ex: Vec<String> = (0..n)
            .map(|i| format!("2026{:02}{:02}T080000", 1 + (i % 12), 1 + (i % 28)))
            .collect();
        let raw = format!(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VTIMEZONE\r\nTZID:Custom\r\nBEGIN:STANDARD\r\nDTSTART:16011028T030000\r\nRRULE:FREQ=YEARLY;BYDAY=-1SU;BYMONTH=10\r\nTZOFFSETFROM:+0200\r\nTZOFFSETTO:+0100\r\nEND:STANDARD\r\nBEGIN:DAYLIGHT\r\nDTSTART:16010325T020000\r\nRRULE:FREQ=YEARLY;BYDAY=-1SU;BYMONTH=3\r\nTZOFFSETFROM:+0100\r\nTZOFFSETTO:+0200\r\nEND:DAYLIGHT\r\nEND:VTIMEZONE\r\nBEGIN:VEVENT\r\nUID:e1\r\nSUMMARY:S\r\nDTSTART;TZID=Custom:20260101T080000\r\nRRULE:FREQ=DAILY\r\nEXDATE;TZID=Custom:{}\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",
            ex.join(",")
        );
        let events = timed(&format!("{n} EXDATEs"), BUDGET, || {
            CalDavClient::events_from_raw_ical(&raw, d(2026, 11, 1), d(2026, 11, 30))
        });
        // November's excluded days: 3, 7, 11, 15, 19, 23, 27
        let mut days: Vec<u32> = events
            .iter()
            .map(|e| chrono::Datelike::day(&e.date))
            .collect();
        days.sort();
        let expected: Vec<u32> = (1..=30).filter(|d| (d + 1) % 4 != 0).collect();
        assert_eq!(days, expected, "{n}");
    }
    // the zone's offsets per date: standard and summer time
    let raw = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VTIMEZONE\r\nTZID:Custom\r\nBEGIN:STANDARD\r\nDTSTART:16011028T030000\r\nRRULE:FREQ=YEARLY;BYDAY=-1SU;BYMONTH=10\r\nTZOFFSETFROM:+0200\r\nTZOFFSETTO:+0100\r\nEND:STANDARD\r\nBEGIN:DAYLIGHT\r\nDTSTART:16010325T020000\r\nRRULE:FREQ=YEARLY;BYDAY=-1SU;BYMONTH=3\r\nTZOFFSETFROM:+0100\r\nTZOFFSETTO:+0200\r\nEND:DAYLIGHT\r\nEND:VTIMEZONE\r\nBEGIN:VEVENT\r\nUID:w\r\nDTSTART;TZID=Custom:20261124T080000\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nUID:s\r\nDTSTART;TZID=Custom:20260702T080000\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nUID:o\r\nDTSTART;TZID=Custom:20261025T040000\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
    let events = preview_events_from_raw_ical(raw);
    let start = |uid: &str| {
        events
            .iter()
            .find(|e| e.uid == uid)
            .unwrap()
            .dtstart
            .clone()
    };
    assert_eq!(start("w"), "2026-11-24T08:00:00+01:00");
    assert_eq!(start("s"), "2026-07-02T08:00:00+02:00");
    // the last Sunday of October 2026 is the 25th, from 03:00
    assert_eq!(start("o"), "2026-10-25T04:00:00+01:00");
}

/// The transitions computed directly are the ones the crate computes.
#[test]
fn yearly_transitions_match_the_crate() {
    let from = d(1990, 1, 1).and_hms_opt(0, 0, 0).unwrap();
    let to = d(2040, 12, 31).and_hms_opt(23, 59, 59).unwrap();
    for (start, rule) in [
        ("16011028T030000", "FREQ=YEARLY;BYDAY=-1SU;BYMONTH=10"),
        ("19700329T020000", "FREQ=YEARLY;BYMONTH=3;BYDAY=-1SU"),
        ("20071104T020000", "FREQ=YEARLY;BYMONTH=11;BYDAY=1SU"),
        ("20070311T020000", "FREQ=YEARLY;BYMONTH=3;BYDAY=2SU"),
        (
            "19870405T020000",
            "FREQ=YEARLY;BYMONTH=4;BYDAY=1SU;UNTIL=20060402T100000Z",
        ),
        ("19700101T000000", "FREQ=YEARLY"),
        ("19700315T000000", "FREQ=YEARLY;BYMONTH=3;BYDAY=+5SU"),
    ] {
        let dtstart = chrono::NaiveDateTime::parse_from_str(start, "%Y%m%dT%H%M%S").unwrap();
        let direct = yearly_transitions(dtstart, rule, from, to).expect(rule);
        let text = format!("DTSTART:{start}Z\nRRULE:{rule}\n");
        let by_crate = occurrences_for_tests(&text, from, to).unwrap();
        assert_eq!(direct, by_crate, "{rule}");
    }
}

/// A VTIMEZONE whose yearly rule never matches, named in many years: bounded, the event still shown.
#[test]
fn a_vtimezone_of_unusual_shape_is_bounded() {
    let ex: Vec<String> = (0..300)
        .map(|i| format!("{}0101T080000", 1700 + i))
        .collect();
    let raw = format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VTIMEZONE\r\nTZID:Odd\r\nBEGIN:STANDARD\r\nDTSTART:16010101T000000\r\nRRULE:FREQ=YEARLY;BYMONTH=2;BYMONTHDAY=30\r\nTZOFFSETFROM:+0100\r\nTZOFFSETTO:+0100\r\nEND:STANDARD\r\nEND:VTIMEZONE\r\nBEGIN:VEVENT\r\nUID:e1\r\nSUMMARY:S\r\nDTSTART;TZID=Odd:16990101T080000\r\nRRULE:FREQ=YEARLY\r\nEXDATE;TZID=Odd:{}\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",
        ex.join(",")
    );
    // the crate's own loop on a rule that never matches: the slowest case kept (MAX_ZONE_WALKS)
    let events = timed("odd VTIMEZONE", BUDGET * 3, || {
        CalDavClient::events_from_raw_ical(&raw, d(2026, 1, 1), d(2026, 1, 1))
    });
    assert_eq!(events.len(), 1);
}
