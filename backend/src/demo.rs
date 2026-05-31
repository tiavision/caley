//! Demo mode seed data.
//!
//! Called once on startup when DEMO_MODE=true. Credentials and account rows use
//! INSERT … ON CONFLICT DO UPDATE so stale data from a previous run (e.g. before
//! an encryption-key change) is always corrected. Content rows (events, messages,
//! contacts) still use INSERT OR IGNORE so user edits survive a restart.

use sqlx::SqlitePool;

pub const DEMO_USER_ID: &str = "demo@demo.example";
const DEMO_CALDAV_ACCOUNT: &str = "demo-caldav";
const DEMO_CALDAV_URL: &str = "https://demo.caldav.example";
const DEMO_WORK_ACCOUNT: &str = "demo-work";
const DEMO_PERSONAL_ACCOUNT: &str = "demo-personal";
const DEMO_ADDRESSBOOK_HREF: &str = "/addressbooks/demo/contacts/";
const DEMO_WORK_CAL: &str = "/calendars/demo/work/";
const DEMO_PERSONAL_CAL: &str = "/calendars/demo/personal/";
const DEMO_FAMILY_CAL: &str = "/calendars/demo/family/";

pub async fn seed(pool: &SqlitePool, encryption_key: &[u8; 32]) -> anyhow::Result<()> {
    println!("[demo] seeding demo database for user {DEMO_USER_ID}");
    seed_user(pool).await?;
    seed_caldav_account(pool, encryption_key).await?;
    seed_calendars(pool).await?;
    seed_events(pool).await?;
    seed_addressbook(pool).await?;
    seed_contacts(pool).await?;
    seed_imap_accounts(pool, encryption_key).await?;
    seed_mailboxes(pool).await?;
    seed_messages(pool).await?;
    Ok(())
}

async fn seed_user(pool: &SqlitePool) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT OR IGNORE INTO user_settings (user_id, display_name) VALUES (?, ?)",
    )
    .bind(DEMO_USER_ID)
    .bind("Demo User")
    .execute(pool)
    .await?;
    Ok(())
}

async fn seed_caldav_account(pool: &SqlitePool, key: &[u8; 32]) -> anyhow::Result<()> {
    let enc_url = crate::crypto::encrypt(DEMO_CALDAV_URL, key)?;
    let enc_user = crate::crypto::encrypt("demo", key)?;
    let enc_pass = crate::crypto::encrypt("demo", key)?;
    // Upsert so re-keying (different ENCRYPTION_KEY) is always corrected.
    sqlx::query(
        "INSERT INTO caldav_accounts
            (account_id, user_id, display_name, caldav_base_url, caldav_username, caldav_password, sort_order)
         VALUES (?, ?, ?, ?, ?, ?, 0)
         ON CONFLICT(account_id) DO UPDATE SET
            caldav_base_url = excluded.caldav_base_url,
            caldav_username = excluded.caldav_username,
            caldav_password = excluded.caldav_password",
    )
    .bind(DEMO_CALDAV_ACCOUNT)
    .bind(DEMO_USER_ID)
    .bind("Demo CalDAV")
    .bind(enc_url)
    .bind(enc_user)
    .bind(enc_pass)
    .execute(pool)
    .await?;
    Ok(())
}

async fn seed_calendars(pool: &SqlitePool) -> anyhow::Result<()> {
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

async fn seed_events(pool: &SqlitePool) -> anyhow::Result<()> {
    let events: &[EventSeed] = &[
        // ── Work ──────────────────────────────────────────────────────────────
        EventSeed {
            uid: "demo-standup",
            calendar_href: DEMO_WORK_CAL,
            summary: "Morning Standup",
            description: "Daily team standup — share blockers and progress.",
            location: "Video call",
            dtstart_expr: "strftime('%Y%m%dT090000', datetime('now','weekday 1'))",
            dtend_expr:   "strftime('%Y%m%dT093000', datetime('now','weekday 1'))",
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
            dtend_expr:   "strftime('%Y%m%dT160000', datetime('now','+5 days'))",
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
            dtend_expr:   "strftime('%Y%m%dT113000', datetime('now','weekday 2'))",
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
            dtend_expr:   "strftime('%Y%m%d', datetime('now','+13 days'))",
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
            dtend_expr:   "strftime('%Y%m%dT180000', datetime('now','+20 days'))",
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
            dtend_expr:   "strftime('%Y%m%dT133000', datetime('now','+3 days'))",
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
            dtend_expr:   "strftime('%Y%m%dT170000', datetime('now','+8 days'))",
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
            dtend_expr:   "strftime('%Y%m%dT103000', datetime('now','+7 days'))",
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
            dtend_expr:   "strftime('%Y%m%dT083000', datetime('now','weekday 3'))",
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
            dtend_expr:   "strftime('%Y%m%d', datetime('now','+38 days'))",
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
            dtend_expr:   "strftime('%Y%m%d', datetime('now','+15 days'))",
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
            dtend_expr:   "strftime('%Y%m%dT073000', datetime('now','+1 days'))",
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
            dtend_expr:   "strftime('%Y%m%dT220000', datetime('now','+2 days'))",
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
            dtend_expr:   "strftime('%Y%m%dT173000', datetime('now','weekday 6'))",
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
            dtend_expr:   "strftime('%Y%m%dT110000', datetime('now','weekday 0'))",
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
            dtend_expr:   "strftime('%Y%m%dT200000', datetime('now','+10 days'))",
            is_all_day: false,
            is_recurring: false,
            rrule: None,
        },
    ];

    for e in events {
        let _rrule_line = e.rrule.map(|r| format!("\r\n{r}")).unwrap_or_default();
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
        .bind(format!("{}/{}.ics", e.calendar_href.trim_end_matches('/'), e.uid))
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
        format!("{}-{}-{}", &date_part[..4], &date_part[4..6], &date_part[6..8])
    } else {
        date_part.to_string()
    }
}

async fn seed_addressbook(pool: &SqlitePool) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT OR IGNORE INTO cached_addressbooks
            (account_id, href, name, sync_token, last_synced)
         VALUES (?, ?, 'Contacts', 'demo-token', strftime('%s','now'))",
    )
    .bind(DEMO_CALDAV_ACCOUNT)
    .bind(DEMO_ADDRESSBOOK_HREF)
    .execute(pool)
    .await?;
    Ok(())
}

struct ContactSeed {
    uid: &'static str,
    fn_name: &'static str,
    org: &'static str,
    emails: &'static str,
    phones: &'static str,
    vcard: &'static str,
}

async fn seed_contacts(pool: &SqlitePool) -> anyhow::Result<()> {
    let contacts: &[ContactSeed] = &[
        // A
        ContactSeed {
            uid: "demo-alice-brennan",
            fn_name: "Alice Brennan",
            org: "Acme Corp",
            emails: r#"[{"value":"alice@acme.example","type_label":"work"}]"#,
            phones: r#"[{"value":"+1 555 010 1001","type_label":"work"}]"#,
            vcard: "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:demo-alice-brennan\r\nFN:Alice Brennan\r\nN:Brennan;Alice;;;\r\nORG:Acme Corp\r\nEMAIL;TYPE=WORK:alice@acme.example\r\nTEL;TYPE=WORK:+1 555 010 1001\r\nEND:VCARD",
        },
        ContactSeed {
            uid: "demo-andrew-miller",
            fn_name: "Andrew Miller",
            org: "Acme Corp",
            emails: r#"[{"value":"a.miller@acme.example","type_label":"work"},{"value":"andrew.miller@personal.example","type_label":"home"}]"#,
            phones: r#"[{"value":"+1 555 010 1002","type_label":"work"}]"#,
            vcard: "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:demo-andrew-miller\r\nFN:Andrew Miller\r\nN:Miller;Andrew;;;\r\nORG:Acme Corp\r\nEMAIL;TYPE=WORK:a.miller@acme.example\r\nEMAIL;TYPE=HOME:andrew.miller@personal.example\r\nTEL;TYPE=WORK:+1 555 010 1002\r\nEND:VCARD",
        },
        ContactSeed {
            uid: "demo-aisha-patel",
            fn_name: "Aisha Patel",
            org: "Design Studio",
            emails: r#"[{"value":"aisha@designstudio.example","type_label":"work"}]"#,
            phones: r#"[{"value":"+44 7700 900010","type_label":"mobile"}]"#,
            vcard: "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:demo-aisha-patel\r\nFN:Aisha Patel\r\nN:Patel;Aisha;;;\r\nORG:Design Studio\r\nEMAIL;TYPE=WORK:aisha@designstudio.example\r\nTEL;TYPE=CELL:+44 7700 900010\r\nEND:VCARD",
        },
        // B
        ContactSeed {
            uid: "demo-bob-chen",
            fn_name: "Bob Chen",
            org: "Freelance",
            emails: r#"[{"value":"bob.chen@example.net","type_label":"home"}]"#,
            phones: r#"[{"value":"+44 7700 900001","type_label":"mobile"}]"#,
            vcard: "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:demo-bob-chen\r\nFN:Bob Chen\r\nN:Chen;Bob;;;\r\nORG:Freelance\r\nEMAIL;TYPE=HOME:bob.chen@example.net\r\nTEL;TYPE=CELL:+44 7700 900001\r\nEND:VCARD",
        },
        ContactSeed {
            uid: "demo-barbara-williams",
            fn_name: "Barbara Williams",
            org: "Acme Corp",
            emails: r#"[{"value":"b.williams@acme.example","type_label":"work"}]"#,
            phones: r#"[{"value":"+1 555 010 2001","type_label":"work"},{"value":"+1 555 010 2099","type_label":"mobile"}]"#,
            vcard: "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:demo-barbara-williams\r\nFN:Barbara Williams\r\nN:Williams;Barbara;;;\r\nORG:Acme Corp\r\nEMAIL;TYPE=WORK:b.williams@acme.example\r\nTEL;TYPE=WORK:+1 555 010 2001\r\nTEL;TYPE=CELL:+1 555 010 2099\r\nEND:VCARD",
        },
        // C
        ContactSeed {
            uid: "demo-carla-diaz",
            fn_name: "Carla Diaz",
            org: "City Hospital",
            emails: r#"[{"value":"c.diaz@hospital.example","type_label":"work"}]"#,
            phones: r#"[{"value":"+1 555 020 2002","type_label":"work"},{"value":"+1 555 020 2010","type_label":"mobile"}]"#,
            vcard: "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:demo-carla-diaz\r\nFN:Carla Diaz\r\nN:Diaz;Carla;;;\r\nORG:City Hospital\r\nEMAIL;TYPE=WORK:c.diaz@hospital.example\r\nTEL;TYPE=WORK:+1 555 020 2002\r\nTEL;TYPE=CELL:+1 555 020 2010\r\nEND:VCARD",
        },
        ContactSeed {
            uid: "demo-carlos-fernandez",
            fn_name: "Carlos Fernandez",
            org: "Startup Inc",
            emails: r#"[{"value":"carlos@startup.example","type_label":"work"}]"#,
            phones: r#"[{"value":"+34 612 345 678","type_label":"mobile"}]"#,
            vcard: "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:demo-carlos-fernandez\r\nFN:Carlos Fernandez\r\nN:Fernandez;Carlos;;;\r\nORG:Startup Inc\r\nEMAIL;TYPE=WORK:carlos@startup.example\r\nTEL;TYPE=CELL:+34 612 345 678\r\nEND:VCARD",
        },
        ContactSeed {
            uid: "demo-claire-nguyen",
            fn_name: "Claire Nguyen",
            org: "Marketing Agency",
            emails: r#"[{"value":"claire.nguyen@mktg.example","type_label":"work"},{"value":"claire.n@home.example","type_label":"home"}]"#,
            phones: r#"[{"value":"+1 555 020 3003","type_label":"work"}]"#,
            vcard: "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:demo-claire-nguyen\r\nFN:Claire Nguyen\r\nN:Nguyen;Claire;;;\r\nORG:Marketing Agency\r\nEMAIL;TYPE=WORK:claire.nguyen@mktg.example\r\nEMAIL;TYPE=HOME:claire.n@home.example\r\nTEL;TYPE=WORK:+1 555 020 3003\r\nEND:VCARD",
        },
        // D
        ContactSeed {
            uid: "demo-david-kim",
            fn_name: "David Kim",
            org: "Startup Inc",
            emails: r#"[{"value":"david@startup.example","type_label":"work"},{"value":"d.kim@personal.example","type_label":"home"}]"#,
            phones: r#"[{"value":"+82 10 1234 5678","type_label":"mobile"}]"#,
            vcard: "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:demo-david-kim\r\nFN:David Kim\r\nN:Kim;David;;;\r\nORG:Startup Inc\r\nEMAIL;TYPE=WORK:david@startup.example\r\nEMAIL;TYPE=HOME:d.kim@personal.example\r\nTEL;TYPE=CELL:+82 10 1234 5678\r\nEND:VCARD",
        },
        ContactSeed {
            uid: "demo-diana-reynolds",
            fn_name: "Diana Reynolds",
            org: "Reynolds Legal",
            emails: r#"[{"value":"diana@reynoldslegal.example","type_label":"work"}]"#,
            phones: r#"[{"value":"+1 555 030 1001","type_label":"work"},{"value":"+1 555 030 1099","type_label":"mobile"}]"#,
            vcard: "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:demo-diana-reynolds\r\nFN:Diana Reynolds\r\nN:Reynolds;Diana;;;\r\nORG:Reynolds Legal\r\nEMAIL;TYPE=WORK:diana@reynoldslegal.example\r\nTEL;TYPE=WORK:+1 555 030 1001\r\nTEL;TYPE=CELL:+1 555 030 1099\r\nEND:VCARD",
        },
        // E
        ContactSeed {
            uid: "demo-emma-johansson",
            fn_name: "Emma Johansson",
            org: "Design Studio",
            emails: r#"[{"value":"emma@designstudio.example","type_label":"work"}]"#,
            phones: r#"[{"value":"+46 70 123 45 67","type_label":"mobile"}]"#,
            vcard: "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:demo-emma-johansson\r\nFN:Emma Johansson\r\nN:Johansson;Emma;;;\r\nORG:Design Studio\r\nEMAIL;TYPE=WORK:emma@designstudio.example\r\nTEL;TYPE=CELL:+46 70 123 45 67\r\nEND:VCARD",
        },
        ContactSeed {
            uid: "demo-ethan-brooks",
            fn_name: "Ethan Brooks",
            org: "Acme Corp",
            emails: r#"[{"value":"e.brooks@acme.example","type_label":"work"}]"#,
            phones: r#"[{"value":"+1 555 040 1001","type_label":"work"}]"#,
            vcard: "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:demo-ethan-brooks\r\nFN:Ethan Brooks\r\nN:Brooks;Ethan;;;\r\nORG:Acme Corp\r\nEMAIL;TYPE=WORK:e.brooks@acme.example\r\nTEL;TYPE=WORK:+1 555 040 1001\r\nEND:VCARD",
        },
        // F
        ContactSeed {
            uid: "demo-felix-mueller",
            fn_name: "Felix Mueller",
            org: "Tech GmbH",
            emails: r#"[{"value":"felix.mueller@tech.example","type_label":"work"}]"#,
            phones: r#"[{"value":"+49 170 123 4567","type_label":"work"}]"#,
            vcard: "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:demo-felix-mueller\r\nFN:Felix Mueller\r\nN:Mueller;Felix;;;\r\nORG:Tech GmbH\r\nEMAIL;TYPE=WORK:felix.mueller@tech.example\r\nTEL;TYPE=WORK:+49 170 123 4567\r\nEND:VCARD",
        },
        ContactSeed {
            uid: "demo-fatima-al-rashid",
            fn_name: "Fatima Al-Rashid",
            org: "Global Consulting",
            emails: r#"[{"value":"fatima@globalconsulting.example","type_label":"work"}]"#,
            phones: r#"[{"value":"+971 50 123 4567","type_label":"mobile"}]"#,
            vcard: "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:demo-fatima-al-rashid\r\nFN:Fatima Al-Rashid\r\nN:Al-Rashid;Fatima;;;\r\nORG:Global Consulting\r\nEMAIL;TYPE=WORK:fatima@globalconsulting.example\r\nTEL;TYPE=CELL:+971 50 123 4567\r\nEND:VCARD",
        },
        // G
        ContactSeed {
            uid: "demo-grace-okafor",
            fn_name: "Grace Okafor",
            org: "NGO International",
            emails: r#"[{"value":"grace.okafor@ngo.example","type_label":"work"}]"#,
            phones: r#"[{"value":"+234 802 345 6789","type_label":"mobile"}]"#,
            vcard: "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:demo-grace-okafor\r\nFN:Grace Okafor\r\nN:Okafor;Grace;;;\r\nORG:NGO International\r\nEMAIL;TYPE=WORK:grace.okafor@ngo.example\r\nTEL;TYPE=CELL:+234 802 345 6789\r\nEND:VCARD",
        },
        ContactSeed {
            uid: "demo-george-thompson",
            fn_name: "George Thompson",
            org: "Thompson Consulting",
            emails: r#"[{"value":"george@tconsulting.example","type_label":"work"},{"value":"g.thompson@home.example","type_label":"home"}]"#,
            phones: r#"[{"value":"+1 555 050 1001","type_label":"work"}]"#,
            vcard: "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:demo-george-thompson\r\nFN:George Thompson\r\nN:Thompson;George;;;\r\nORG:Thompson Consulting\r\nEMAIL;TYPE=WORK:george@tconsulting.example\r\nEMAIL;TYPE=HOME:g.thompson@home.example\r\nTEL;TYPE=WORK:+1 555 050 1001\r\nEND:VCARD",
        },
        // H
        ContactSeed {
            uid: "demo-henry-park",
            fn_name: "Henry Park",
            org: "",
            emails: r#"[{"value":"henry.park@personal.example","type_label":"home"}]"#,
            phones: r#"[{"value":"+1 555 060 1001","type_label":"home"}]"#,
            vcard: "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:demo-henry-park\r\nFN:Henry Park\r\nN:Park;Henry;;;\r\nEMAIL;TYPE=HOME:henry.park@personal.example\r\nTEL;TYPE=HOME:+1 555 060 1001\r\nEND:VCARD",
        },
        ContactSeed {
            uid: "demo-hannah-schmidt",
            fn_name: "Hannah Schmidt",
            org: "Acme Corp",
            emails: r#"[{"value":"h.schmidt@acme.example","type_label":"work"}]"#,
            phones: r#"[{"value":"+49 151 234 56789","type_label":"mobile"}]"#,
            vcard: "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:demo-hannah-schmidt\r\nFN:Hannah Schmidt\r\nN:Schmidt;Hannah;;;\r\nORG:Acme Corp\r\nEMAIL;TYPE=WORK:h.schmidt@acme.example\r\nTEL;TYPE=CELL:+49 151 234 56789\r\nEND:VCARD",
        },
        // I–J
        ContactSeed {
            uid: "demo-isabella-romano",
            fn_name: "Isabella Romano",
            org: "PR Agency",
            emails: r#"[{"value":"i.romano@pragency.example","type_label":"work"}]"#,
            phones: r#"[{"value":"+39 347 123 4567","type_label":"mobile"}]"#,
            vcard: "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:demo-isabella-romano\r\nFN:Isabella Romano\r\nN:Romano;Isabella;;;\r\nORG:PR Agency\r\nEMAIL;TYPE=WORK:i.romano@pragency.example\r\nTEL;TYPE=CELL:+39 347 123 4567\r\nEND:VCARD",
        },
        ContactSeed {
            uid: "demo-james-wilson",
            fn_name: "James Wilson",
            org: "City Bank",
            emails: r#"[{"value":"james.wilson@citybank.example","type_label":"work"}]"#,
            phones: r#"[{"value":"+1 555 070 1001","type_label":"work"},{"value":"+1 555 070 9999","type_label":"mobile"}]"#,
            vcard: "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:demo-james-wilson\r\nFN:James Wilson\r\nN:Wilson;James;;;\r\nORG:City Bank\r\nEMAIL;TYPE=WORK:james.wilson@citybank.example\r\nTEL;TYPE=WORK:+1 555 070 1001\r\nTEL;TYPE=CELL:+1 555 070 9999\r\nEND:VCARD",
        },
        // K–L
        ContactSeed {
            uid: "demo-katherine-hayes",
            fn_name: "Katherine Hayes",
            org: "",
            emails: r#"[{"value":"kath.hayes@personal.example","type_label":"home"}]"#,
            phones: r#"[{"value":"+1 555 080 2002","type_label":"mobile"}]"#,
            vcard: "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:demo-katherine-hayes\r\nFN:Katherine Hayes\r\nN:Hayes;Katherine;;;\r\nEMAIL;TYPE=HOME:kath.hayes@personal.example\r\nTEL;TYPE=CELL:+1 555 080 2002\r\nEND:VCARD",
        },
        ContactSeed {
            uid: "demo-liam-obrien",
            fn_name: "Liam O'Brien",
            org: "Tech GmbH",
            emails: r#"[{"value":"liam.obrien@tech.example","type_label":"work"}]"#,
            phones: r#"[{"value":"+353 87 123 4567","type_label":"mobile"}]"#,
            vcard: "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:demo-liam-obrien\r\nFN:Liam O'Brien\r\nN:O'Brien;Liam;;;\r\nORG:Tech GmbH\r\nEMAIL;TYPE=WORK:liam.obrien@tech.example\r\nTEL;TYPE=CELL:+353 87 123 4567\r\nEND:VCARD",
        },
        // M
        ContactSeed {
            uid: "demo-maria-santos",
            fn_name: "Maria Santos",
            org: "City Hospital",
            emails: r#"[{"value":"m.santos@hospital.example","type_label":"work"}]"#,
            phones: r#"[{"value":"+55 11 9 1234 5678","type_label":"mobile"}]"#,
            vcard: "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:demo-maria-santos\r\nFN:Maria Santos\r\nN:Santos;Maria;;;\r\nORG:City Hospital\r\nEMAIL;TYPE=WORK:m.santos@hospital.example\r\nTEL;TYPE=CELL:+55 11 9 1234 5678\r\nEND:VCARD",
        },
        ContactSeed {
            uid: "demo-michael-chang",
            fn_name: "Michael Chang",
            org: "Startup Inc",
            emails: r#"[{"value":"michael@startup.example","type_label":"work"},{"value":"m.chang@home.example","type_label":"home"}]"#,
            phones: r#"[{"value":"+1 555 090 1001","type_label":"work"}]"#,
            vcard: "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:demo-michael-chang\r\nFN:Michael Chang\r\nN:Chang;Michael;;;\r\nORG:Startup Inc\r\nEMAIL;TYPE=WORK:michael@startup.example\r\nEMAIL;TYPE=HOME:m.chang@home.example\r\nTEL;TYPE=WORK:+1 555 090 1001\r\nEND:VCARD",
        },
        // N–O
        ContactSeed {
            uid: "demo-nina-petrov",
            fn_name: "Nina Petrov",
            org: "Global Consulting",
            emails: r#"[{"value":"nina.petrov@globalconsulting.example","type_label":"work"}]"#,
            phones: r#"[{"value":"+7 916 123 45 67","type_label":"mobile"}]"#,
            vcard: "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:demo-nina-petrov\r\nFN:Nina Petrov\r\nN:Petrov;Nina;;;\r\nORG:Global Consulting\r\nEMAIL;TYPE=WORK:nina.petrov@globalconsulting.example\r\nTEL;TYPE=CELL:+7 916 123 45 67\r\nEND:VCARD",
        },
        ContactSeed {
            uid: "demo-oliver-grant",
            fn_name: "Oliver Grant",
            org: "Acme Corp",
            emails: r#"[{"value":"o.grant@acme.example","type_label":"work"}]"#,
            phones: r#"[{"value":"+44 7700 900200","type_label":"work"}]"#,
            vcard: "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:demo-oliver-grant\r\nFN:Oliver Grant\r\nN:Grant;Oliver;;;\r\nORG:Acme Corp\r\nEMAIL;TYPE=WORK:o.grant@acme.example\r\nTEL;TYPE=WORK:+44 7700 900200\r\nEND:VCARD",
        },
        // R–S
        ContactSeed {
            uid: "demo-rafael-silva",
            fn_name: "Rafael Silva",
            org: "Design Studio",
            emails: r#"[{"value":"rafael@designstudio.example","type_label":"work"}]"#,
            phones: r#"[{"value":"+55 21 9 8765 4321","type_label":"mobile"}]"#,
            vcard: "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:demo-rafael-silva\r\nFN:Rafael Silva\r\nN:Silva;Rafael;;;\r\nORG:Design Studio\r\nEMAIL;TYPE=WORK:rafael@designstudio.example\r\nTEL;TYPE=CELL:+55 21 9 8765 4321\r\nEND:VCARD",
        },
        ContactSeed {
            uid: "demo-sarah-cohen",
            fn_name: "Sarah Cohen",
            org: "",
            emails: r#"[{"value":"sarah.cohen@personal.example","type_label":"home"}]"#,
            phones: r#"[{"value":"+1 555 100 1001","type_label":"home"}]"#,
            vcard: "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:demo-sarah-cohen\r\nFN:Sarah Cohen\r\nN:Cohen;Sarah;;;\r\nEMAIL;TYPE=HOME:sarah.cohen@personal.example\r\nTEL;TYPE=HOME:+1 555 100 1001\r\nEND:VCARD",
        },
        ContactSeed {
            uid: "demo-sophie-laurent",
            fn_name: "Sophie Laurent",
            org: "Marketing Agency",
            emails: r#"[{"value":"sophie.laurent@mktg.example","type_label":"work"}]"#,
            phones: r#"[{"value":"+33 6 12 34 56 78","type_label":"mobile"}]"#,
            vcard: "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:demo-sophie-laurent\r\nFN:Sophie Laurent\r\nN:Laurent;Sophie;;;\r\nORG:Marketing Agency\r\nEMAIL;TYPE=WORK:sophie.laurent@mktg.example\r\nTEL;TYPE=CELL:+33 6 12 34 56 78\r\nEND:VCARD",
        },
        // T
        ContactSeed {
            uid: "demo-tomoki-yamada",
            fn_name: "Tomoki Yamada",
            org: "Tech GmbH",
            emails: r#"[{"value":"t.yamada@tech.example","type_label":"work"}]"#,
            phones: r#"[{"value":"+81 90 1234 5678","type_label":"mobile"}]"#,
            vcard: "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:demo-tomoki-yamada\r\nFN:Tomoki Yamada\r\nN:Yamada;Tomoki;;;\r\nORG:Tech GmbH\r\nEMAIL;TYPE=WORK:t.yamada@tech.example\r\nTEL;TYPE=CELL:+81 90 1234 5678\r\nEND:VCARD",
        },
        // Z
        ContactSeed {
            uid: "demo-zara-ahmed",
            fn_name: "Zara Ahmed",
            org: "NGO International",
            emails: r#"[{"value":"zara.ahmed@ngo.example","type_label":"work"}]"#,
            phones: r#"[{"value":"+92 300 123 4567","type_label":"mobile"}]"#,
            vcard: "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:demo-zara-ahmed\r\nFN:Zara Ahmed\r\nN:Ahmed;Zara;;;\r\nORG:NGO International\r\nEMAIL;TYPE=WORK:zara.ahmed@ngo.example\r\nTEL;TYPE=CELL:+92 300 123 4567\r\nEND:VCARD",
        },
    ];

    for c in contacts {
        sqlx::query(
            "INSERT OR IGNORE INTO cached_contacts
                (account_id, addressbook_href, uid, etag, href, raw_vcard, fn_name, emails, phones, org)
             VALUES (?, ?, ?, 'demo', ?, ?, ?, ?, ?, ?)",
        )
        .bind(DEMO_CALDAV_ACCOUNT)
        .bind(DEMO_ADDRESSBOOK_HREF)
        .bind(c.uid)
        .bind(format!("{}{}.vcf", DEMO_ADDRESSBOOK_HREF, c.uid))
        .bind(c.vcard)
        .bind(c.fn_name)
        .bind(c.emails)
        .bind(c.phones)
        .bind(c.org)
        .execute(pool)
        .await?;
    }
    Ok(())
}

async fn seed_imap_accounts(pool: &SqlitePool, key: &[u8; 32]) -> anyhow::Result<()> {
    // Account 1 — Work
    let enc_host = crate::crypto::encrypt("imap.work.example", key)?;
    let enc_user = crate::crypto::encrypt("demo@work.example", key)?;
    let enc_pass = crate::crypto::encrypt("demo", key)?;
    let enc_smtp = crate::crypto::encrypt("smtp.work.example", key)?;
    sqlx::query(
        "INSERT INTO imap_accounts
            (account_id, user_id, display_name, host, port, username, password,
             use_tls, smtp_host, smtp_port, sort_order)
         VALUES (?, ?, ?, ?, 993, ?, ?, 1, ?, 587, 0)
         ON CONFLICT(account_id) DO UPDATE SET
            host = excluded.host,
            username = excluded.username,
            password = excluded.password,
            smtp_host = excluded.smtp_host",
    )
    .bind(DEMO_WORK_ACCOUNT)
    .bind(DEMO_USER_ID)
    .bind("Work")
    .bind(enc_host)
    .bind(enc_user)
    .bind(enc_pass)
    .bind(enc_smtp)
    .execute(pool)
    .await?;

    // Account 2 — Personal
    let enc_host2 = crate::crypto::encrypt("mail.personal.example", key)?;
    let enc_user2 = crate::crypto::encrypt("me@personal.example", key)?;
    let enc_pass2 = crate::crypto::encrypt("demo", key)?;
    let enc_smtp2 = crate::crypto::encrypt("smtp.personal.example", key)?;
    sqlx::query(
        "INSERT INTO imap_accounts
            (account_id, user_id, display_name, host, port, username, password,
             use_tls, smtp_host, smtp_port, sort_order)
         VALUES (?, ?, ?, ?, 993, ?, ?, 1, ?, 587, 1)
         ON CONFLICT(account_id) DO UPDATE SET
            host = excluded.host,
            username = excluded.username,
            password = excluded.password,
            smtp_host = excluded.smtp_host",
    )
    .bind(DEMO_PERSONAL_ACCOUNT)
    .bind(DEMO_USER_ID)
    .bind("Personal")
    .bind(enc_host2)
    .bind(enc_user2)
    .bind(enc_pass2)
    .bind(enc_smtp2)
    .execute(pool)
    .await?;

    Ok(())
}

async fn seed_mailboxes(pool: &SqlitePool) -> anyhow::Result<()> {
    let work_mailboxes = ["INBOX", "Sent", "Drafts", "Archive", "Trash", "Junk"];
    for (i, name) in work_mailboxes.iter().enumerate() {
        sqlx::query(
            "INSERT OR IGNORE INTO cached_mailboxes
                (account_id, name, delimiter, uidvalidity, last_synced, subscribed)
             VALUES (?, ?, '.', ?, strftime('%s','now'), 1)",
        )
        .bind(DEMO_WORK_ACCOUNT)
        .bind(name)
        .bind(2001 + i as i64)
        .execute(pool)
        .await?;
    }
    let personal_mailboxes = ["INBOX", "Sent", "Drafts", "Trash", "Newsletters"];
    for (i, name) in personal_mailboxes.iter().enumerate() {
        sqlx::query(
            "INSERT OR IGNORE INTO cached_mailboxes
                (account_id, name, delimiter, uidvalidity, last_synced, subscribed)
             VALUES (?, ?, '.', ?, strftime('%s','now'), 1)",
        )
        .bind(DEMO_PERSONAL_ACCOUNT)
        .bind(name)
        .bind(3001 + i as i64)
        .execute(pool)
        .await?;
    }
    Ok(())
}

struct MessageSeed {
    account: &'static str,
    uid: i64,
    mailbox: &'static str,
    message_id: &'static str,
    thread_id: Option<&'static str>,
    from_addr: &'static str,
    to_addrs: &'static str,
    subject: &'static str,
    days_ago: i64,
    flags: &'static str,
    /// Plain-text fallback / FTS index source
    body_text: &'static str,
    /// Full HTML body shown in the mail viewer iframe
    body_html: &'static str,
    is_unread: bool,
}

async fn seed_messages(pool: &SqlitePool) -> anyhow::Result<()> {
    let messages: &[MessageSeed] = &[
        // ── Work INBOX ────────────────────────────────────────────────────────
        MessageSeed {
            account: DEMO_WORK_ACCOUNT,
            uid: 1,
            mailbox: "INBOX",
            message_id: "<welcome-work@demo.example>",
            thread_id: Some("<welcome-work@demo.example>"),
            from_addr: "\"IT Support\" <it@work.example>",
            to_addrs: "\"Demo User\" <demo@work.example>",
            subject: "Welcome to the team — setup checklist",
            days_ago: 0,
            flags: r#"[]"#,
            body_text: "Hi Demo,\n\nWelcome aboard! Please complete the onboarding checklist below. Key items: set up VPN, join #general on Slack, and book your first 1:1 with your manager.\n\n1. Install VPN client — see IT wiki\n2. Join Slack workspace and say hi in #intros\n3. Book 1:1 with Alice (use Calendly link)\n4. Complete security training (link in email)\n\nIT Support",
            body_html: r##"<div style="font-family:sans-serif;font-size:14px;line-height:1.6;color:#222">
<p>Hi Demo,</p>
<p>Welcome aboard! Please complete the onboarding checklist below. Key items: set up VPN, join #general on Slack, and book your first 1:1 with your manager.</p>
<ol>
  <li>Install VPN client — see IT wiki</li>
  <li>Join Slack workspace and say hi in <strong>#intros</strong></li>
  <li>Book 1:1 with Alice (use Calendly link)</li>
  <li>Complete security training (link in email)</li>
</ol>
<p style="color:#666;margin-top:24px">IT Support<br>it@work.example</p>
</div>
            "##,
            is_unread: true,
        },
        MessageSeed {
            account: DEMO_WORK_ACCOUNT,
            uid: 2,
            mailbox: "INBOX",
            message_id: "<sprint-notes-1@work.example>",
            thread_id: Some("<sprint-thread-1@work.example>"),
            from_addr: "\"Alice Brennan\" <alice@acme.example>",
            to_addrs: "\"Demo User\" <demo@work.example>",
            subject: "Sprint 24 notes",
            days_ago: 1,
            flags: r#"["\\Seen"]"#,
            body_text: "Hi team,\n\nAttached are the notes from yesterday's sprint review. Please review and add any missing items by EOD Friday.\n\nHighlights:\n- Auth service refactor merged\n- Mobile perf regression still open (#341)\n- Q3 scope locked\n\nThanks,\nAlice",
            body_html: r##"<div style="font-family:sans-serif;font-size:14px;line-height:1.6;color:#222">
<p>Hi team,</p>
<p>Attached are the notes from yesterday's sprint review. Please review and add any missing items by EOD Friday.</p>
<p><strong>Highlights:</strong></p>
<ul>
  <li>Auth service refactor merged ✓</li>
  <li>Mobile perf regression still open (<a href="#">#341</a>)</li>
  <li>Q3 scope locked</li>
</ul>
<p>Thanks,<br><strong>Alice</strong></p>
</div>
            "##,
            is_unread: false,
        },
        MessageSeed {
            account: DEMO_WORK_ACCOUNT,
            uid: 3,
            mailbox: "INBOX",
            message_id: "<sprint-reply-bob@work.example>",
            thread_id: Some("<sprint-thread-1@work.example>"),
            from_addr: "\"Bob Chen\" <bob.chen@example.net>",
            to_addrs: "\"Demo User\" <demo@work.example>",
            subject: "Re: Sprint 24 notes",
            days_ago: 1,
            flags: r#"["\\Seen"]"#,
            body_text: "Looks good to me. I'll add the API performance items before the end of the day.\n\n— Bob",
            body_html: r##"<div style="font-family:sans-serif;font-size:14px;line-height:1.6;color:#222"><p>Looks good to me. I'll add the API performance items before the end of the day.</p><p style="color:#555">— Bob</p></div>
            "##,
            is_unread: false,
        },
        MessageSeed {
            account: DEMO_WORK_ACCOUNT,
            uid: 4,
            mailbox: "INBOX",
            message_id: "<invoice-may@work.example>",
            thread_id: None,
            from_addr: "\"Accounts\" <billing@acme.example>",
            to_addrs: "\"Demo User\" <demo@work.example>",
            subject: "Invoice #2405 — May Services",
            days_ago: 2,
            flags: r#"[]"#,
            body_text: "Please find attached invoice #2405 for May services.\n\nAmount due: EUR 1,200.00\nDue date: 30 June 2026\nPayment reference: INV-2405\n\nPlease process via the finance portal. Contact billing@acme.example with any questions.",
            body_html: r##"<div style="font-family:sans-serif;font-size:14px;line-height:1.6;color:#222">
<p>Please find attached invoice #2405 for May services.</p>
<table style="border-collapse:collapse;margin:16px 0;width:100%;max-width:400px">
  <tr style="background:#f5f5f5"><td style="padding:8px 12px;border:1px solid #ddd"><strong>Amount due</strong></td><td style="padding:8px 12px;border:1px solid #ddd">EUR 1,200.00</td></tr>
  <tr><td style="padding:8px 12px;border:1px solid #ddd"><strong>Due date</strong></td><td style="padding:8px 12px;border:1px solid #ddd">30 June 2026</td></tr>
  <tr style="background:#f5f5f5"><td style="padding:8px 12px;border:1px solid #ddd"><strong>Reference</strong></td><td style="padding:8px 12px;border:1px solid #ddd">INV-2405</td></tr>
</table>
<p>Please process via the finance portal. Contact <a href="mailto:billing@acme.example">billing@acme.example</a> with any questions.</p>
</div>
            "##,
            is_unread: true,
        },
        MessageSeed {
            account: DEMO_WORK_ACCOUNT,
            uid: 5,
            mailbox: "INBOX",
            message_id: "<hr-policy@work.example>",
            thread_id: None,
            from_addr: "\"HR Team\" <hr@acme.example>",
            to_addrs: "\"All Staff\" <all@acme.example>",
            subject: "Updated Remote Work Policy — effective 1 July",
            days_ago: 3,
            flags: r#"["\\Seen"]"#,
            body_text: "Dear team,\n\nPlease review the updated remote work policy effective 1st July.\n\nKey changes:\n- Flexible core hours: 10:00–15:00 (previously 09:00–17:00)\n- Quarterly in-office requirement: two weeks per quarter\n- Home office equipment budget increased to EUR 500/year\n\nThe full document is in the HR portal.\n\nBest,\nHR Team",
            body_html: r##"<div style="font-family:sans-serif;font-size:14px;line-height:1.6;color:#222">
<p>Dear team,</p>
<p>Please review the updated remote work policy effective <strong>1st July</strong>.</p>
<p><strong>Key changes:</strong></p>
<ul>
  <li>Flexible core hours: <strong>10:00–15:00</strong> (previously 09:00–17:00)</li>
  <li>Quarterly in-office requirement: two weeks per quarter</li>
  <li>Home office equipment budget increased to <strong>EUR 500/year</strong></li>
</ul>
<p>The full document is in the <a href="#">HR portal</a>.</p>
<p style="color:#666">Best,<br>HR Team</p>
</div>
            "##,
            is_unread: false,
        },
        MessageSeed {
            account: DEMO_WORK_ACCOUNT,
            uid: 6,
            mailbox: "INBOX",
            message_id: "<design-feedback@work.example>",
            thread_id: Some("<design-thread-1@work.example>"),
            from_addr: "\"Emma Johansson\" <emma@designstudio.example>",
            to_addrs: "\"Demo User\" <demo@work.example>",
            subject: "Homepage redesign — feedback needed",
            days_ago: 3,
            flags: r#"[]"#,
            body_text: "Hey,\n\nI've finished the v2 mockups for the homepage. Could you take a look and share your thoughts before our call Thursday?\n\nMain changes:\n- New hero section with video background\n- Consolidated navigation (7 items → 4)\n- Mobile-first grid layout\n- Refreshed colour palette\n\nFigma: [link]\n\nEmma",
            body_html: r##"<div style="font-family:sans-serif;font-size:14px;line-height:1.6;color:#222">
<p>Hey,</p>
<p>I've finished the v2 mockups for the homepage. Could you take a look and share your thoughts before our call Thursday?</p>
<p><strong>Main changes:</strong></p>
<ul>
  <li>New hero section with video background</li>
  <li>Consolidated navigation (7 items &rarr; 4)</li>
  <li>Mobile-first grid layout</li>
  <li>Refreshed colour palette</li>
</ul>
<p><a href="#" style="background:#0066cc;color:#fff;padding:8px 16px;border-radius:4px;text-decoration:none;display:inline-block">Open in Figma</a></p>
<p>Emma</p>
</div>
            "##,
            is_unread: true,
        },
        MessageSeed {
            account: DEMO_WORK_ACCOUNT,
            uid: 7,
            mailbox: "INBOX",
            message_id: "<server-alert@work.example>",
            thread_id: None,
            from_addr: "\"Monitoring\" <alerts@ops.example>",
            to_addrs: "\"Demo User\" <demo@work.example>",
            subject: "[RESOLVED] High CPU on web-02",
            days_ago: 4,
            flags: r#"["\\Seen"]"#,
            body_text: "[RESOLVED] High CPU on web-02\n\nHost: web-02.prod\nAlert fired: 03:14 UTC\nResolved: 03:31 UTC\nDuration: 17 minutes\n\nRoot cause: runaway log rotation script triggered at 03:14 UTC. No user-facing impact.\n\nPost-mortem: https://notion/postmortems/2026-05",
            body_html: r##"<div style="font-family:monospace;font-size:13px;line-height:1.5;color:#222;background:#f8f8f8;padding:16px;border-left:4px solid #4caf50">
<p style="font-size:15px;font-family:sans-serif"><strong style="color:#4caf50">[RESOLVED]</strong> High CPU on web-02</p>
<table style="margin:12px 0;border-collapse:collapse;font-family:sans-serif;font-size:13px">
  <tr><td style="padding:4px 16px 4px 0;color:#555">Host</td><td>web-02.prod</td></tr>
  <tr><td style="padding:4px 16px 4px 0;color:#555">Alert fired</td><td>03:14 UTC</td></tr>
  <tr><td style="padding:4px 16px 4px 0;color:#555">Resolved</td><td>03:31 UTC</td></tr>
  <tr><td style="padding:4px 16px 4px 0;color:#555">Duration</td><td>17 minutes</td></tr>
</table>
<p style="font-family:sans-serif">Root cause: runaway log rotation script. No user-facing impact.<br>Post-mortem: <a href="#">notion/postmortems/2026-05</a></p>
</div>
            "##,
            is_unread: false,
        },
        MessageSeed {
            account: DEMO_WORK_ACCOUNT,
            uid: 8,
            mailbox: "INBOX",
            message_id: "<conf-registration@work.example>",
            thread_id: None,
            from_addr: "\"DevConf\" <registration@devconf.example>",
            to_addrs: "\"Demo User\" <demo@work.example>",
            subject: "Your DevConf ticket is confirmed",
            days_ago: 5,
            flags: r#"["\\Seen"]"#,
            body_text: "Your registration for DevConf is confirmed.\n\nOrder: DC-2026-08421\nTicket: Full Pass\nDate: 3–5 August 2026\nVenue: Convention Centre, Berlin\n\nDownload your e-ticket from the attendee portal.",
            body_html: r##"<div style="font-family:sans-serif;font-size:14px;line-height:1.6;color:#222;max-width:500px">
<div style="background:#1a1a2e;color:#fff;padding:24px;text-align:center;border-radius:8px 8px 0 0">
  <p style="font-size:22px;font-weight:700;margin:0">DevConf 2026</p>
  <p style="color:#aaa;margin:4px 0 0">Your ticket is confirmed</p>
</div>
<div style="border:1px solid #ddd;border-top:none;padding:24px;border-radius:0 0 8px 8px">
  <table style="width:100%;border-collapse:collapse">
    <tr><td style="padding:8px 0;color:#555;width:40%">Order</td><td><strong>DC-2026-08421</strong></td></tr>
    <tr><td style="padding:8px 0;color:#555">Ticket type</td><td>Full Pass</td></tr>
    <tr><td style="padding:8px 0;color:#555">Dates</td><td>3–5 August 2026</td></tr>
    <tr><td style="padding:8px 0;color:#555">Venue</td><td>Convention Centre, Berlin</td></tr>
  </table>
  <p style="margin-top:20px"><a href="#" style="background:#1a1a2e;color:#fff;padding:10px 20px;border-radius:4px;text-decoration:none">Download e-ticket</a></p>
</div>
</div>
            "##,
            is_unread: false,
        },
        MessageSeed {
            account: DEMO_WORK_ACCOUNT,
            uid: 9,
            mailbox: "INBOX",
            message_id: "<github-pr@work.example>",
            thread_id: None,
            from_addr: "\"GitHub\" <notifications@github.example>",
            to_addrs: "\"Demo User\" <demo@work.example>",
            subject: "[demo-org/app] PR #142: Add dark mode support",
            days_ago: 0,
            flags: r#"[]"#,
            body_text: "David Kim opened pull request #142\n\nAdd dark mode support via CSS custom properties. Closes #87.\n\n2 files changed, 134 insertions(+), 12 deletions(-)\n\ndemo-org/app",
            body_html: r##"<div style="font-family:sans-serif;font-size:14px;line-height:1.6;color:#222">
<div style="border:1px solid #d1d5da;border-radius:6px;padding:16px;margin-bottom:16px">
  <p style="margin:0 0 4px"><strong>David Kim</strong> opened pull request <a href="#" style="color:#0066cc"><strong>#142</strong></a></p>
  <p style="font-size:18px;font-weight:600;margin:4px 0">Add dark mode support</p>
  <p style="color:#555;margin:4px 0 12px">Add dark mode support via CSS custom properties. Closes <a href="#" style="color:#0066cc">#87</a>.</p>
  <p style="font-size:12px;color:#888;margin:0">2 files changed &nbsp;·&nbsp; <span style="color:#2a8a2a">+134</span> &nbsp;·&nbsp; <span style="color:#cc0000">-12</span></p>
</div>
<p><a href="#" style="background:#2da44e;color:#fff;padding:8px 16px;border-radius:4px;text-decoration:none">Review pull request</a></p>
</div>
            "##,
            is_unread: true,
        },
        MessageSeed {
            account: DEMO_WORK_ACCOUNT,
            uid: 10,
            mailbox: "INBOX",
            message_id: "<customer-inquiry@work.example>",
            thread_id: Some("<customer-thread-1@work.example>"),
            from_addr: "\"James Wilson\" <james.wilson@citybank.example>",
            to_addrs: "\"Demo User\" <demo@work.example>",
            subject: "Integration question — webhook signatures",
            days_ago: 1,
            flags: r#"[]"#,
            body_text: "Hi,\n\nWe're integrating your API and have a question about webhook signature verification. The docs mention HMAC-SHA256 but don't specify the encoding — is the signature hex or base64?\n\nThanks,\nJames Wilson\nCity Bank",
            body_html: r##"<div style="font-family:sans-serif;font-size:14px;line-height:1.6;color:#222">
<p>Hi,</p>
<p>We're integrating your API and have a question about webhook signature verification. The docs mention HMAC-SHA256 but don't specify the encoding — is the signature <strong>hex</strong> or <strong>base64</strong>?</p>
<p>Thanks,<br>James Wilson<br><span style="color:#555">City Bank</span></p>
</div>
            "##,
            is_unread: true,
        },
        MessageSeed {
            account: DEMO_WORK_ACCOUNT,
            uid: 11,
            mailbox: "INBOX",
            message_id: "<board-agenda@work.example>",
            thread_id: None,
            from_addr: "\"Barbara Williams\" <b.williams@acme.example>",
            to_addrs: "\"Leadership Team\" <leadership@acme.example>",
            subject: "Board meeting agenda — Thursday 10:00",
            days_ago: 2,
            flags: r#"["\\Seen"]"#,
            body_text: "Please find the agenda for Thursday's board meeting at 10:00.\n\nAgenda:\n1. Q2 financials (CFO, 20 min)\n2. Product roadmap H2 (CPO, 30 min)\n3. Headcount plan (HR, 15 min)\n4. AOB\n\nPre-reads are in the shared Drive folder (link below).\n\nBarbara",
            body_html: r##"<div style="font-family:sans-serif;font-size:14px;line-height:1.6;color:#222">
<p>Please find the agenda for Thursday's board meeting at <strong>10:00</strong>.</p>
<table style="width:100%;border-collapse:collapse;margin:12px 0">
  <thead><tr style="background:#f0f0f0"><th style="text-align:left;padding:8px 12px;border:1px solid #ddd">#</th><th style="text-align:left;padding:8px 12px;border:1px solid #ddd">Item</th><th style="text-align:left;padding:8px 12px;border:1px solid #ddd">Duration</th></tr></thead>
  <tbody>
    <tr><td style="padding:8px 12px;border:1px solid #ddd">1</td><td style="padding:8px 12px;border:1px solid #ddd">Q2 financials (CFO)</td><td style="padding:8px 12px;border:1px solid #ddd">20 min</td></tr>
    <tr style="background:#fafafa"><td style="padding:8px 12px;border:1px solid #ddd">2</td><td style="padding:8px 12px;border:1px solid #ddd">Product roadmap H2 (CPO)</td><td style="padding:8px 12px;border:1px solid #ddd">30 min</td></tr>
    <tr><td style="padding:8px 12px;border:1px solid #ddd">3</td><td style="padding:8px 12px;border:1px solid #ddd">Headcount plan (HR)</td><td style="padding:8px 12px;border:1px solid #ddd">15 min</td></tr>
    <tr style="background:#fafafa"><td style="padding:8px 12px;border:1px solid #ddd">4</td><td style="padding:8px 12px;border:1px solid #ddd">AOB</td><td style="padding:8px 12px;border:1px solid #ddd">—</td></tr>
  </tbody>
</table>
<p>Pre-reads: <a href="#">Shared Drive folder</a></p>
<p style="color:#666">Barbara</p>
</div>
            "##,
            is_unread: false,
        },
        MessageSeed {
            account: DEMO_WORK_ACCOUNT,
            uid: 12,
            mailbox: "INBOX",
            message_id: "<deploy-success@work.example>",
            thread_id: None,
            from_addr: "\"CI/CD\" <ci@ops.example>",
            to_addrs: "\"Demo User\" <demo@work.example>",
            subject: "[SUCCESS] Production deploy v2.14.3",
            days_ago: 0,
            flags: r#"[]"#,
            body_text: "[SUCCESS] Production deploy v2.14.3\n\nCompleted: 14:32 UTC\nServices: api, worker, scheduler\nStatus: all health checks passing\nCommit: a3f1b9c",
            body_html: r##"<div style="font-family:monospace;font-size:13px;color:#222;background:#f0faf0;border-left:4px solid #4caf50;padding:16px">
<p style="font-family:sans-serif;font-size:15px;font-weight:700;color:#2a7a2a;margin:0 0 12px">[SUCCESS] Production deploy v2.14.3</p>
<table style="font-family:sans-serif;font-size:13px;border-collapse:collapse">
  <tr><td style="padding:3px 16px 3px 0;color:#555">Completed</td><td>14:32 UTC</td></tr>
  <tr><td style="padding:3px 16px 3px 0;color:#555">Services</td><td>api, worker, scheduler</td></tr>
  <tr><td style="padding:3px 16px 3px 0;color:#555">Status</td><td style="color:#2a7a2a;font-weight:600">All health checks passing</td></tr>
  <tr><td style="padding:3px 16px 3px 0;color:#555">Commit</td><td><a href="#" style="color:#0066cc">a3f1b9c</a></td></tr>
</table>
</div>
            "##,
            is_unread: true,
        },
        MessageSeed {
            account: DEMO_WORK_ACCOUNT,
            uid: 13,
            mailbox: "INBOX",
            message_id: "<contract-renewal@work.example>",
            thread_id: Some("<contract-thread@work.example>"),
            from_addr: "\"Diana Reynolds\" <diana@reynoldslegal.example>",
            to_addrs: "\"Demo User\" <demo@work.example>",
            subject: "Contract renewal — action required by Friday",
            days_ago: 3,
            flags: r#"[]"#,
            body_text: "Hi,\n\nThe vendor contract for the data warehouse service expires on the 30th. Please review the renewal terms and confirm acceptance by Friday so we can process in time.\n\nKey points:\n- 12-month renewal, same terms\n- Price increase: +3% (EUR 14,400/year)\n- Auto-renewal clause removed per our request\n\nRegards,\nDiana Reynolds\nReynolds Legal",
            body_html: r##"<div style="font-family:sans-serif;font-size:14px;line-height:1.6;color:#222">
<p>Hi,</p>
<p>The vendor contract for the data warehouse service expires on the <strong>30th</strong>. Please review the renewal terms and confirm acceptance by Friday so we can process in time.</p>
<p><strong>Key points:</strong></p>
<ul>
  <li>12-month renewal, same terms</li>
  <li>Price increase: +3% (<strong>EUR 14,400/year</strong>)</li>
  <li>Auto-renewal clause removed per our request</li>
</ul>
<p>Regards,<br><strong>Diana Reynolds</strong><br><span style="color:#555">Reynolds Legal</span></p>
</div>
            "##,
            is_unread: true,
        },
        MessageSeed {
            account: DEMO_WORK_ACCOUNT,
            uid: 14,
            mailbox: "INBOX",
            message_id: "<weekly-digest@work.example>",
            thread_id: None,
            from_addr: "\"Analytics\" <analytics@acme.example>",
            to_addrs: "\"Demo User\" <demo@work.example>",
            subject: "Weekly metrics digest — Week 22",
            days_ago: 6,
            flags: r#"["\\Seen"]"#,
            body_text: "Week 22 highlights:\n\nDAU: 142k (+8% WoW)\nRevenue: EUR 280k (+3%)\nChurn: 1.2% (-0.3pp)\nP95 latency: 180ms (stable)\n\nFull dashboard: analytics.acme.example",
            body_html: r##"<div style="font-family:sans-serif;font-size:14px;line-height:1.6;color:#222">
<p><strong>Week 22 highlights</strong></p>
<table style="width:100%;max-width:400px;border-collapse:collapse;margin:8px 0">
  <tr style="background:#f5f5f5"><td style="padding:10px 12px;border:1px solid #ddd">Daily Active Users</td><td style="padding:10px 12px;border:1px solid #ddd;font-weight:600">142k <span style="color:#2a7a2a;font-size:12px">+8%</span></td></tr>
  <tr><td style="padding:10px 12px;border:1px solid #ddd">Weekly Revenue</td><td style="padding:10px 12px;border:1px solid #ddd;font-weight:600">EUR 280k <span style="color:#2a7a2a;font-size:12px">+3%</span></td></tr>
  <tr style="background:#f5f5f5"><td style="padding:10px 12px;border:1px solid #ddd">Churn</td><td style="padding:10px 12px;border:1px solid #ddd;font-weight:600">1.2% <span style="color:#2a7a2a;font-size:12px">-0.3pp</span></td></tr>
  <tr><td style="padding:10px 12px;border:1px solid #ddd">P95 Latency</td><td style="padding:10px 12px;border:1px solid #ddd;font-weight:600">180ms <span style="color:#888;font-size:12px">stable</span></td></tr>
</table>
<p style="font-size:12px;color:#888"><a href="#">Full dashboard</a></p>
</div>
            "##,
            is_unread: false,
        },

        // Work Sent
        MessageSeed {
            account: DEMO_WORK_ACCOUNT,
            uid: 1,
            mailbox: "Sent",
            message_id: "<sent-sprint-reply@work.example>",
            thread_id: Some("<sprint-thread-1@work.example>"),
            from_addr: "\"Demo User\" <demo@work.example>",
            to_addrs: "\"Alice Brennan\" <alice@acme.example>",
            subject: "Re: Sprint 24 notes",
            days_ago: 1,
            flags: r#"["\\Seen"]"#,
            body_text: "Thanks Alice, reviewed and LGTM. I've added the missing auth performance item.",
            body_html: r##"<div style="font-family:sans-serif;font-size:14px;line-height:1.6;color:#222"><p>Thanks Alice, reviewed and LGTM. I've added the missing auth performance item.</p></div>
            "##,
            is_unread: false,
        },
        MessageSeed {
            account: DEMO_WORK_ACCOUNT,
            uid: 2,
            mailbox: "Sent",
            message_id: "<sent-design@work.example>",
            thread_id: Some("<design-thread-1@work.example>"),
            from_addr: "\"Demo User\" <demo@work.example>",
            to_addrs: "\"Emma Johansson\" <emma@designstudio.example>",
            subject: "Re: Homepage redesign — feedback needed",
            days_ago: 2,
            flags: r#"["\\Seen"]"#,
            body_text: "Emma, the new hero section looks great. One suggestion: increase contrast on the CTA button (currently fails AA). Everything else is solid.",
            body_html: r##"<div style="font-family:sans-serif;font-size:14px;line-height:1.6;color:#222"><p>Emma, the new hero section looks great. One suggestion: increase contrast on the CTA button (currently fails AA). Everything else is solid.</p></div>
            "##,
            is_unread: false,
        },
        MessageSeed {
            account: DEMO_WORK_ACCOUNT,
            uid: 3,
            mailbox: "Sent",
            message_id: "<sent-customer@work.example>",
            thread_id: Some("<customer-thread-1@work.example>"),
            from_addr: "\"Demo User\" <demo@work.example>",
            to_addrs: "\"James Wilson\" <james.wilson@citybank.example>",
            subject: "Re: Integration question — webhook signatures",
            days_ago: 0,
            flags: r#"["\\Seen"]"#,
            body_text: "Hi James,\n\nThe signature is hex-encoded. You'll compute HMAC-SHA256 over the raw request body using your webhook secret, then compare to the X-Signature header value (hex string, no prefix).\n\nLet me know if you'd like a code snippet!",
            body_html: r##"<div style="font-family:sans-serif;font-size:14px;line-height:1.6;color:#222">
<p>Hi James,</p>
<p>The signature is <strong>hex-encoded</strong>. You'll compute HMAC-SHA256 over the raw request body using your webhook secret, then compare to the <code style="background:#f0f0f0;padding:2px 5px;border-radius:3px">X-Signature</code> header value (hex string, no prefix).</p>
<p>Let me know if you'd like a code snippet!</p>
</div>
            "##,
            is_unread: false,
        },

        // ── Personal INBOX ────────────────────────────────────────────────────
        MessageSeed {
            account: DEMO_PERSONAL_ACCOUNT,
            uid: 1,
            mailbox: "INBOX",
            message_id: "<bank-statement@personal.example>",
            thread_id: None,
            from_addr: "\"City Bank\" <statements@citybank.example>",
            to_addrs: "\"Demo User\" <me@personal.example>",
            subject: "Your May statement is ready",
            days_ago: 1,
            flags: r#"[]"#,
            body_text: "Your May 2026 account statement is now available in online banking. Log in to view or download a PDF copy.\n\nAccount: **** 4821\nStatement period: 1–31 May 2026\nClosing balance: EUR 3,847.22",
            body_html: r##"<div style="font-family:sans-serif;font-size:14px;line-height:1.6;color:#222;max-width:480px">
<div style="background:#003080;color:#fff;padding:20px;border-radius:8px 8px 0 0">
  <strong style="font-size:18px">City Bank</strong>
</div>
<div style="border:1px solid #ddd;border-top:none;padding:20px;border-radius:0 0 8px 8px">
  <p>Your <strong>May 2026</strong> account statement is ready.</p>
  <table style="width:100%;border-collapse:collapse;margin:12px 0">
    <tr><td style="padding:6px 0;color:#555">Account</td><td>**** 4821</td></tr>
    <tr><td style="padding:6px 0;color:#555">Period</td><td>1–31 May 2026</td></tr>
    <tr><td style="padding:6px 0;color:#555;font-weight:600">Closing balance</td><td style="font-weight:600">EUR 3,847.22</td></tr>
  </table>
  <p><a href="#" style="background:#003080;color:#fff;padding:10px 20px;border-radius:4px;text-decoration:none;display:inline-block">View statement</a></p>
</div>
</div>
            "##,
            is_unread: true,
        },
        MessageSeed {
            account: DEMO_PERSONAL_ACCOUNT,
            uid: 2,
            mailbox: "INBOX",
            message_id: "<henry-msg@personal.example>",
            thread_id: Some("<friends-trip@personal.example>"),
            from_addr: "\"Henry Park\" <henry.park@personal.example>",
            to_addrs: "\"Demo User\" <me@personal.example>",
            subject: "Summer trip plans!",
            days_ago: 1,
            flags: r#"["\\Seen"]"#,
            body_text: "Hey! Are you still up for the Portugal trip in July? I found some great flights — under 200 EUR return if we book this week. Katherine and Sarah are in too.",
            body_html: r##"<div style="font-family:sans-serif;font-size:14px;line-height:1.6;color:#222"><p>Hey! Are you still up for the Portugal trip in July? I found some great flights — <strong>under 200 EUR return</strong> if we book this week. Katherine and Sarah are in too.</p></div>
            "##,
            is_unread: false,
        },
        MessageSeed {
            account: DEMO_PERSONAL_ACCOUNT,
            uid: 3,
            mailbox: "INBOX",
            message_id: "<katherine-reply@personal.example>",
            thread_id: Some("<friends-trip@personal.example>"),
            from_addr: "\"Katherine Hayes\" <kath.hayes@personal.example>",
            to_addrs: "\"Demo User\" <me@personal.example>",
            subject: "Re: Summer trip plans!",
            days_ago: 0,
            flags: r#"[]"#,
            body_text: "100% in! Porto is incredible. Can we do the 12th? That works best for me. Sarah?",
            body_html: r##"<div style="font-family:sans-serif;font-size:14px;line-height:1.6;color:#222"><p>100% in! Porto is incredible. Can we do the <strong>12th</strong>? That works best for me. Sarah?</p></div>
            "##,
            is_unread: true,
        },
        MessageSeed {
            account: DEMO_PERSONAL_ACCOUNT,
            uid: 4,
            mailbox: "INBOX",
            message_id: "<dentist-reminder@personal.example>",
            thread_id: None,
            from_addr: "\"Reed Dental\" <appointments@reeddental.example>",
            to_addrs: "\"Demo User\" <me@personal.example>",
            subject: "Appointment reminder — Tuesday 9:30",
            days_ago: 2,
            flags: r#"["\\Seen"]"#,
            body_text: "This is a reminder for your appointment with Dr. Reed on Tuesday at 9:30 AM. Please arrive 5 minutes early. Reply to this email to cancel or reschedule.",
            body_html: r##"<div style="font-family:sans-serif;font-size:14px;line-height:1.6;color:#222;max-width:400px">
<div style="background:#e8f4fd;border-left:4px solid #2196f3;padding:16px;border-radius:4px">
  <p style="margin:0 0 8px;font-weight:600">Appointment Reminder</p>
  <p style="margin:0;color:#555">Dr. Reed &nbsp;·&nbsp; Tuesday &nbsp;·&nbsp; <strong>9:30 AM</strong></p>
</div>
<p style="margin-top:16px">Please arrive 5 minutes early. Reply to this email to cancel or reschedule.</p>
<p><strong>Reed Dental</strong><br>45 Oak Ave</p>
</div>
            "##,
            is_unread: false,
        },
        MessageSeed {
            account: DEMO_PERSONAL_ACCOUNT,
            uid: 5,
            mailbox: "INBOX",
            message_id: "<order-confirm@personal.example>",
            thread_id: None,
            from_addr: "\"Online Shop\" <orders@shop.example>",
            to_addrs: "\"Demo User\" <me@personal.example>",
            subject: "Order #78234 confirmed — delivery Thu 5 June",
            days_ago: 3,
            flags: r#"["\\Seen"]"#,
            body_text: "Thanks for your order! Your package will be delivered on Thursday 5 June between 08:00 and 18:00.\n\nOrder: #78234\nDelivery: Thu 5 June, 08:00–18:00\nAddress: 12 Demo Street\n\nTrack your order with reference 78234.",
            body_html: r##"<div style="font-family:sans-serif;font-size:14px;line-height:1.6;color:#222;max-width:460px">
<p>Thanks for your order!</p>
<div style="background:#f9f9f9;border:1px solid #eee;border-radius:6px;padding:16px;margin:12px 0">
  <table style="width:100%;border-collapse:collapse">
    <tr><td style="padding:6px 0;color:#555">Order number</td><td><strong>#78234</strong></td></tr>
    <tr><td style="padding:6px 0;color:#555">Delivery</td><td>Thu 5 June, 08:00–18:00</td></tr>
    <tr><td style="padding:6px 0;color:#555">Address</td><td>12 Demo Street</td></tr>
  </table>
</div>
<p><a href="#" style="color:#0066cc">Track your order</a></p>
</div>
            "##,
            is_unread: false,
        },
        MessageSeed {
            account: DEMO_PERSONAL_ACCOUNT,
            uid: 6,
            mailbox: "INBOX",
            message_id: "<tech-newsletter@personal.example>",
            thread_id: None,
            from_addr: "\"Tech Weekly\" <newsletter@techweekly.example>",
            to_addrs: "\"Demo User\" <me@personal.example>",
            subject: "Tech Weekly #312 — AI tools, Rust 2.0 preview, WebAssembly",
            days_ago: 6,
            flags: r#"["\\Seen"]"#,
            body_text: "Tech Weekly #312\n\n1. AI coding assistants hit 40% developer adoption\n2. Rust 2.0 RFC discussion begins — memory model changes proposed\n3. WebAssembly components land in Firefox Nightly\n4. SQLite 3.47 adds JSON5 support\n\nRead more at techweekly.example",
            body_html: r##"<div style="font-family:sans-serif;font-size:14px;line-height:1.7;color:#222;max-width:540px">
<div style="border-bottom:2px solid #222;padding-bottom:8px;margin-bottom:16px">
  <strong style="font-size:18px">Tech Weekly</strong> <span style="color:#888">#312</span>
</div>
<ol style="padding-left:20px">
  <li style="margin-bottom:10px"><strong>AI coding assistants hit 40% developer adoption</strong><br><span style="color:#555;font-size:13px">GitHub survey shows rapid uptake across all experience levels.</span></li>
  <li style="margin-bottom:10px"><strong>Rust 2.0 RFC discussion begins</strong><br><span style="color:#555;font-size:13px">Memory model changes and async improvements proposed.</span></li>
  <li style="margin-bottom:10px"><strong>WebAssembly components land in Firefox Nightly</strong><br><span style="color:#555;font-size:13px">Component model spec now behind a flag in FF 130.</span></li>
  <li style="margin-bottom:10px"><strong>SQLite 3.47 adds JSON5 support</strong><br><span style="color:#555;font-size:13px">Comments, trailing commas, unquoted keys now accepted.</span></li>
</ol>
<p style="font-size:12px;color:#888;border-top:1px solid #eee;padding-top:12px;margin-top:16px">You're receiving this because you subscribed at techweekly.example &nbsp;·&nbsp; <a href="#" style="color:#888">Unsubscribe</a></p>
</div>
            "##,
            is_unread: false,
        },
        MessageSeed {
            account: DEMO_PERSONAL_ACCOUNT,
            uid: 7,
            mailbox: "INBOX",
            message_id: "<gym-invoice@personal.example>",
            thread_id: None,
            from_addr: "\"City Fitness\" <billing@cityfitness.example>",
            to_addrs: "\"Demo User\" <me@personal.example>",
            subject: "June membership invoice — EUR 45.00",
            days_ago: 4,
            flags: r#"[]"#,
            body_text: "Your June membership has been renewed.\n\nAmount: EUR 45.00\nCard: ending 4821\nMembership: All-Access Monthly\n\nReceipt available in your account portal.",
            body_html: r##"<div style="font-family:sans-serif;font-size:14px;line-height:1.6;color:#222;max-width:400px">
<p>Your June membership has been renewed.</p>
<table style="width:100%;border-collapse:collapse;margin:12px 0">
  <tr style="background:#f5f5f5"><td style="padding:8px 12px;border:1px solid #ddd">Amount charged</td><td style="padding:8px 12px;border:1px solid #ddd;font-weight:600">EUR 45.00</td></tr>
  <tr><td style="padding:8px 12px;border:1px solid #ddd">Card</td><td style="padding:8px 12px;border:1px solid #ddd">ending 4821</td></tr>
  <tr style="background:#f5f5f5"><td style="padding:8px 12px;border:1px solid #ddd">Membership</td><td style="padding:8px 12px;border:1px solid #ddd">All-Access Monthly</td></tr>
</table>
<p><a href="#">View receipt</a></p>
</div>
            "##,
            is_unread: true,
        },
        MessageSeed {
            account: DEMO_PERSONAL_ACCOUNT,
            uid: 8,
            mailbox: "INBOX",
            message_id: "<sarah-msg@personal.example>",
            thread_id: Some("<friends-trip@personal.example>"),
            from_addr: "\"Sarah Cohen\" <sarah.cohen@personal.example>",
            to_addrs: "\"Demo User\" <me@personal.example>",
            subject: "Re: Summer trip plans!",
            days_ago: 0,
            flags: r#"[]"#,
            body_text: "The 12th works perfectly! I'll look at Airbnb options for a 5-night stay. Budget around 80/night per room?",
            body_html: r##"<div style="font-family:sans-serif;font-size:14px;line-height:1.6;color:#222"><p>The 12th works perfectly! I'll look at Airbnb options for a 5-night stay. Budget around <strong>80/night</strong> per room?</p></div>
            "##,
            is_unread: true,
        },

        // Personal Sent
        MessageSeed {
            account: DEMO_PERSONAL_ACCOUNT,
            uid: 1,
            mailbox: "Sent",
            message_id: "<sent-henry@personal.example>",
            thread_id: Some("<friends-trip@personal.example>"),
            from_addr: "\"Demo User\" <me@personal.example>",
            to_addrs: "\"Henry Park\" <henry.park@personal.example>",
            subject: "Re: Summer trip plans!",
            days_ago: 1,
            flags: r#"["\\Seen"]"#,
            body_text: "Yes! I'm in. Let's do it. The 12th works for me too. Book those flights!",
            body_html: r##"<div style="font-family:sans-serif;font-size:14px;line-height:1.6;color:#222"><p>Yes! I'm in. Let's do it. The 12th works for me too. Book those flights!</p></div>
            "##,
            is_unread: false,
        },
    ];

    for m in messages {
        let sent_at: i64 = sqlx::query_scalar(&format!(
            "SELECT unixepoch('now', '-{} days')",
            m.days_ago
        ))
        .fetch_one(pool)
        .await?;

        let from_lower = extract_email_addr(m.from_addr);

        sqlx::query(
            "INSERT OR IGNORE INTO cached_messages
                (account_id, mailbox, uid, message_id, thread_id, from_addr, to_addrs,
                 subject, sent_at, flags, body_text, body_html, subject_idx, body_idx, from_idx, is_unread)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(m.account)
        .bind(m.mailbox)
        .bind(m.uid)
        .bind(m.message_id)
        .bind(m.thread_id)
        .bind(m.from_addr)
        .bind(m.to_addrs)
        .bind(m.subject)
        .bind(sent_at)
        .bind(m.flags)
        .bind(m.body_text)
        .bind(m.body_html)
        .bind(m.subject)
        .bind(m.body_text) // body_idx: plain text for FTS
        .bind(&from_lower)
        .bind(m.is_unread as i64)
        .execute(pool)
        .await?;

        sqlx::query(
            "INSERT OR IGNORE INTO message_addresses (account_id, mailbox, uid, field, addr)
             VALUES (?, ?, ?, 'from', ?)",
        )
        .bind(m.account)
        .bind(m.mailbox)
        .bind(m.uid)
        .bind(&from_lower)
        .execute(pool)
        .await?;
    }
    Ok(())
}

fn extract_email_addr(display: &str) -> String {
    if let Some(start) = display.find('<') {
        if let Some(end) = display.find('>') {
            return display[start + 1..end].trim().to_ascii_lowercase();
        }
    }
    display.trim().to_ascii_lowercase()
}

fn strip_html(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for ch in html.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out
}
