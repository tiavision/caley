use sqlx::SqlitePool;

use super::{DEMO_CALDAV_ACCOUNT, DEMO_FAMILY_CAL, DEMO_PERSONAL_CAL, DEMO_WORK_CAL};

pub(super) async fn seed_calendars(pool: &SqlitePool) -> anyhow::Result<()> {
    let calendars = [
        (DEMO_WORK_CAL, "Work", "#4CAF50"),
        (DEMO_PERSONAL_CAL, "Personal", "#2196F3"),
        (DEMO_FAMILY_CAL, "Family", "#FF9800"),
    ];
    for (href, name, color) in &calendars {
        sqlx::query(
            "INSERT OR IGNORE INTO cached_calendars
                (account_id, href, name, color, sync_token, last_synced)
             VALUES (?, ?, ?, ?, 'demo-token', strftime('%s','now'))",
        )
        .bind(DEMO_CALDAV_ACCOUNT)
        .bind(href)
        .bind(name)
        .bind(color)
        .execute(pool)
        .await?;
    }
    Ok(())
}

struct EventSeed {
    uid: &'static str,
    calendar_href: &'static str,
    summary: &'static str,
    description: &'static str,
    location: &'static str,
    dtstart_expr: &'static str,
    dtend_expr: &'static str,
    is_all_day: bool,
    is_recurring: bool,
    rrule: Option<&'static str>,
}

pub(super) async fn seed_events(pool: &SqlitePool) -> anyhow::Result<()> {
    let events: &[EventSeed] = &[
        // ── Work ──────────────────────────────────────────────────────────────
        EventSeed {
            uid: "demo-standup",
            calendar_href: DEMO_WORK_CAL,
            summary: "Morning Standup",
            description: "Daily team standup — share blockers and progress.",
            location: "Video call",
            dtstart_expr: "strftime('%Y%m%dT090000', datetime('now','weekday 1'))",
            dtend_expr: "strftime('%Y%m%dT093000', datetime('now','weekday 1'))",
            is_all_day: false,
            is_recurring: true,
            rrule: Some("RRULE:FREQ=WEEKLY;BYDAY=MO,WE,FR"),
        },
        EventSeed {
            uid: "demo-sprint-review",
            calendar_href: DEMO_WORK_CAL,
            summary: "Sprint Review & Retro",
            description: "Biweekly sprint review with stakeholders.",
            location: "Conference Room B",
            dtstart_expr: "strftime('%Y%m%dT140000', datetime('now','+5 days'))",
            dtend_expr: "strftime('%Y%m%dT160000', datetime('now','+5 days'))",
            is_all_day: false,
            is_recurring: false,
            rrule: None,
        },
        EventSeed {
            uid: "demo-1on1-alice",
            calendar_href: DEMO_WORK_CAL,
            summary: "1:1 with Alice",
            description: "Weekly check-in.",
            location: "",
            dtstart_expr: "strftime('%Y%m%dT110000', datetime('now','weekday 2'))",
            dtend_expr: "strftime('%Y%m%dT113000', datetime('now','weekday 2'))",
            is_all_day: false,
            is_recurring: true,
            rrule: Some("RRULE:FREQ=WEEKLY;BYDAY=TU"),
        },
        EventSeed {
            uid: "demo-deadline",
            calendar_href: DEMO_WORK_CAL,
            summary: "Q3 Report Deadline",
            description: "Final Q3 numbers due to finance.",
            location: "",
            dtstart_expr: "strftime('%Y%m%d', datetime('now','+12 days'))",
            dtend_expr: "strftime('%Y%m%d', datetime('now','+13 days'))",
            is_all_day: true,
            is_recurring: false,
            rrule: None,
        },
        EventSeed {
            uid: "demo-conf-keynote",
            calendar_href: DEMO_WORK_CAL,
            summary: "DevConf Keynote",
            description: "Annual developers conference — keynote and networking.",
            location: "Convention Centre, Main Hall",
            dtstart_expr: "strftime('%Y%m%dT090000', datetime('now','+20 days'))",
            dtend_expr: "strftime('%Y%m%dT180000', datetime('now','+20 days'))",
            is_all_day: false,
            is_recurring: false,
            rrule: None,
        },
        EventSeed {
            uid: "demo-lunch-team",
            calendar_href: DEMO_WORK_CAL,
            summary: "Team Lunch",
            description: "Monthly team lunch out.",
            location: "The Garden Bistro, 12 Maple Street",
            dtstart_expr: "strftime('%Y%m%dT120000', datetime('now','+3 days'))",
            dtend_expr: "strftime('%Y%m%dT133000', datetime('now','+3 days'))",
            is_all_day: false,
            is_recurring: false,
            rrule: None,
        },
        EventSeed {
            uid: "demo-board-prep",
            calendar_href: DEMO_WORK_CAL,
            summary: "Board Meeting Prep",
            description: "Prepare slides and talking points for Thursday board meeting.",
            location: "",
            dtstart_expr: "strftime('%Y%m%dT150000', datetime('now','+8 days'))",
            dtend_expr: "strftime('%Y%m%dT170000', datetime('now','+8 days'))",
            is_all_day: false,
            is_recurring: false,
            rrule: None,
        },
        // ── Personal ──────────────────────────────────────────────────────────
        EventSeed {
            uid: "demo-dentist",
            calendar_href: DEMO_PERSONAL_CAL,
            summary: "Dentist Appointment",
            description: "Annual check-up at Dr. Reed's office.",
            location: "Reed Dental, 45 Oak Ave",
            dtstart_expr: "strftime('%Y%m%dT093000', datetime('now','+7 days'))",
            dtend_expr: "strftime('%Y%m%dT103000', datetime('now','+7 days'))",
            is_all_day: false,
            is_recurring: false,
            rrule: None,
        },
        EventSeed {
            uid: "demo-yoga",
            calendar_href: DEMO_PERSONAL_CAL,
            summary: "Yoga Class",
            description: "Morning flow yoga.",
            location: "Sunrise Yoga Studio",
            dtstart_expr: "strftime('%Y%m%dT073000', datetime('now','weekday 3'))",
            dtend_expr: "strftime('%Y%m%dT083000', datetime('now','weekday 3'))",
            is_all_day: false,
            is_recurring: true,
            rrule: Some("RRULE:FREQ=WEEKLY;BYDAY=WE,SA"),
        },
        EventSeed {
            uid: "demo-vacation",
            calendar_href: DEMO_PERSONAL_CAL,
            summary: "Summer Holiday",
            description: "Vacation in Porto. Flight TK 422.",
            location: "Porto, Portugal",
            dtstart_expr: "strftime('%Y%m%d', datetime('now','+30 days'))",
            dtend_expr: "strftime('%Y%m%d', datetime('now','+38 days'))",
            is_all_day: true,
            is_recurring: false,
            rrule: None,
        },
        EventSeed {
            uid: "demo-birthday-mom",
            calendar_href: DEMO_PERSONAL_CAL,
            summary: "Mom's Birthday",
            description: "",
            location: "",
            dtstart_expr: "strftime('%Y%m%d', datetime('now','+14 days'))",
            dtend_expr: "strftime('%Y%m%d', datetime('now','+15 days'))",
            is_all_day: true,
            is_recurring: false,
            rrule: None,
        },
        EventSeed {
            uid: "demo-gym",
            calendar_href: DEMO_PERSONAL_CAL,
            summary: "Gym",
            description: "Upper body day.",
            location: "City Fitness",
            dtstart_expr: "strftime('%Y%m%dT063000', datetime('now','+1 days'))",
            dtend_expr: "strftime('%Y%m%dT073000', datetime('now','+1 days'))",
            is_all_day: false,
            is_recurring: true,
            rrule: Some("RRULE:FREQ=WEEKLY;BYDAY=MO,WE,FR"),
        },
        EventSeed {
            uid: "demo-dinner-out",
            calendar_href: DEMO_PERSONAL_CAL,
            summary: "Dinner with Friends",
            description: "Birthday dinner for Jo.",
            location: "Trattoria Bella, 8 Vine Lane",
            dtstart_expr: "strftime('%Y%m%dT193000', datetime('now','+2 days'))",
            dtend_expr: "strftime('%Y%m%dT220000', datetime('now','+2 days'))",
            is_all_day: false,
            is_recurring: false,
            rrule: None,
        },
        // ── Family ────────────────────────────────────────────────────────────
        EventSeed {
            uid: "demo-soccer",
            calendar_href: DEMO_FAMILY_CAL,
            summary: "Kid's Soccer Practice",
            description: "",
            location: "Riverside Park, Field 3",
            dtstart_expr: "strftime('%Y%m%dT163000', datetime('now','weekday 6'))",
            dtend_expr: "strftime('%Y%m%dT173000', datetime('now','weekday 6'))",
            is_all_day: false,
            is_recurring: true,
            rrule: Some("RRULE:FREQ=WEEKLY;BYDAY=SA"),
        },
        EventSeed {
            uid: "demo-grocery",
            calendar_href: DEMO_FAMILY_CAL,
            summary: "Weekly Grocery Run",
            description: "Check pantry list before going.",
            location: "SuperFresh Market",
            dtstart_expr: "strftime('%Y%m%dT100000', datetime('now','weekday 0'))",
            dtend_expr: "strftime('%Y%m%dT110000', datetime('now','weekday 0'))",
            is_all_day: false,
            is_recurring: true,
            rrule: Some("RRULE:FREQ=WEEKLY;BYDAY=SU"),
        },
        EventSeed {
            uid: "demo-school-concert",
            calendar_href: DEMO_FAMILY_CAL,
            summary: "School Spring Concert",
            description: "Year 4 music performance.",
            location: "Lincoln Primary School, Main Hall",
            dtstart_expr: "strftime('%Y%m%dT183000', datetime('now','+10 days'))",
            dtend_expr: "strftime('%Y%m%dT200000', datetime('now','+10 days'))",
            is_all_day: false,
            is_recurring: false,
            rrule: None,
        },
    ];

    for e in events {
        let dtstart_value_type = if e.is_all_day { ";VALUE=DATE" } else { "" };

        let dtstart_dt: String = sqlx::query_scalar(&format!("SELECT {}", e.dtstart_expr))
            .fetch_one(pool)
            .await?;
        let dtend_dt: String = sqlx::query_scalar(&format!("SELECT {}", e.dtend_expr))
            .fetch_one(pool)
            .await?;
        let dtstart_date = ical_dt_to_date(&dtstart_dt);
        let dtend_date = ical_dt_to_date(&dtend_dt);

        let rrule_line = e.rrule.map(|r| format!("{r}\r\n")).unwrap_or_default();
        let raw_ical = format!(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Caley Demo//EN\r\n\
             BEGIN:VEVENT\r\n\
             UID:{uid}\r\n\
             SUMMARY:{summary}\r\n\
             DESCRIPTION:{description}\r\n\
             LOCATION:{location}\r\n\
             DTSTART{vtype}:{dtstart}\r\n\
             DTEND{vtype}:{dtend}\r\n\
             {rrule}\
             END:VEVENT\r\n\
             END:VCALENDAR",
            uid = e.uid,
            summary = e.summary,
            description = e.description,
            location = e.location,
            vtype = dtstart_value_type,
            dtstart = dtstart_dt,
            dtend = dtend_dt,
            rrule = rrule_line,
        );

        sqlx::query(
            "INSERT OR IGNORE INTO cached_events
                (account_id, calendar_href, uid, etag, href, raw_ical, summary, dtstart, dtend,
                 description, location, is_all_day, is_recurring, last_modified,
                 summary_idx, description_idx, location_idx)
             VALUES (?, ?, ?, 'demo', ?, ?, ?, ?, ?, ?, ?, ?, ?, strftime('%s','now'), ?, ?, ?)",
        )
        .bind(DEMO_CALDAV_ACCOUNT)
        .bind(e.calendar_href)
        .bind(e.uid)
        .bind(format!(
            "{}/{}.ics",
            e.calendar_href.trim_end_matches('/'),
            e.uid
        ))
        .bind(&raw_ical)
        .bind(e.summary)
        .bind(&dtstart_date)
        .bind(&dtend_date)
        .bind(e.description)
        .bind(e.location)
        .bind(e.is_all_day as i64)
        .bind(e.is_recurring as i64)
        .bind(e.summary)
        .bind(e.description)
        .bind(e.location)
        .execute(pool)
        .await?;
    }
    Ok(())
}

fn ical_dt_to_date(s: &str) -> String {
    let date_part = s.split('T').next().unwrap_or(s).trim();
    if date_part.len() >= 8 {
        format!(
            "{}-{}-{}",
            &date_part[..4],
            &date_part[4..6],
            &date_part[6..8]
        )
    } else {
        date_part.to_string()
    }
}
