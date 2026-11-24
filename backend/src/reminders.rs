//! Calendar reminders to the user's devices (notifications.rs), so they come while no page is open: once a
//! minute, for each user with a device subscribed, the alarms of the occurrences in their active calendars
//! (expanded as `GET /api/events` does) whose time has come go out, each once (`sent_reminders`). An alarm
//! relative to an all-day or floating event is timed in each device's timezone (the subscription's), the
//! others are the same instant everywhere. An alarm missed by up to GRACE (a restart) still goes out.
//! The calendar page shows the same reminders while it is open (event-reminders.ts), with the same tag.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use chrono::{NaiveDate, NaiveDateTime, NaiveTime, TimeZone};
use sqlx::Row;

use crate::caldav::{Alarm, CalDavClient, Event};
use crate::db::{self, Db};
use crate::notifications::{Notifier, Payload};

/// How late a reminder may still go out.
const GRACE: i64 = 10 * 60;
/// How far ahead events are read: an alarm may be up to this long before its event.
const AHEAD_DAYS: i64 = 15;
/// How long a sent reminder is remembered.
const KEEP: i64 = 3 * 24 * 60 * 60;

/// Starts the minute task.
pub fn spawn(db: Db, key: [u8; 32], notifier: Arc<Notifier>) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            if let Err(e) = tick(&db, &key, &notifier, chrono::Utc::now().timestamp()).await {
                eprintln!("[reminders] {e}");
            }
        }
    });
}

/// Sends the reminders due at `now` (epoch seconds); returns how many went to at least one device.
pub(crate) async fn tick(db: &Db, key: &[u8; 32], notifier: &Notifier, now: i64) -> anyhow::Result<usize> {
    sqlx::query("DELETE FROM sent_reminders WHERE fire_at < ?").bind(now - KEEP).execute(db).await?;

    // the users with a device, and their devices' timezones
    let mut users: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for row in sqlx::query("SELECT DISTINCT user_id, COALESCE(timezone, 'UTC') FROM notification_subscriptions")
        .fetch_all(db)
        .await?
    {
        users.entry(row.get(0)).or_default().insert(row.get(1));
    }

    let today = chrono::DateTime::from_timestamp(now, 0).unwrap_or_default().date_naive();
    let (start, end) = (today - chrono::TimeDelta::days(2), today + chrono::TimeDelta::days(AHEAD_DAYS));
    let mut sent = 0;
    for (user_id, timezones) in users {
        let timezones: Vec<(String, chrono_tz::Tz)> =
            timezones.into_iter().filter_map(|name| name.parse().ok().map(|tz| (name, tz))).collect();
        for (account_id, calendar_href, raw_ical) in active_events(db, key, &user_id, start, end).await? {
            for event in CalDavClient::events_from_raw_ical(&raw_ical, start, end) {
                for (i, alarm) in event.alarms.iter().enumerate() {
                    for (name, tz) in &timezones {
                        let Some(fire) = fire_at(&event, alarm, tz) else { continue };
                        if fire > now || fire <= now - GRACE {
                            continue;
                        }
                        let key = format!("{account_id}|{calendar_href}|{}|{}|{i}|{fire}|{name}", event.uid, event.dtstart);
                        let new = sqlx::query("INSERT OR IGNORE INTO sent_reminders(user_id, key, fire_at) VALUES(?, ?, ?)")
                            .bind(&user_id)
                            .bind(&key)
                            .bind(fire)
                            .execute(db)
                            .await?
                            .rows_affected()
                            == 1;
                        if new && notifier.send(&user_id, Some(name), &payload(&event, tz, fire)).await > 0 {
                            sent += 1;
                        }
                    }
                }
            }
        }
    }
    Ok(sent)
}

/// The raw events of the user's active calendars from `start` to `end`, as `GET /api/events` reads them.
async fn active_events(
    db: &Db,
    key: &[u8; 32],
    user_id: &str,
    start: NaiveDate,
    end: NaiveDate,
) -> anyhow::Result<Vec<(String, String, String)>> {
    let mut active = Vec::new();
    for calendar in db::get_cached_calendars(db, user_id).await? {
        let deactivated = db::get_deactivated_calendars(db, &calendar.account_id, key).await.unwrap_or_default();
        if !deactivated.contains(&calendar.href) {
            active.push((calendar.account_id, calendar.href));
        }
    }
    db::get_raw_events(db, &active, &start.format("%Y-%m-%d").to_string(), &end.format("%Y-%m-%d").to_string()).await
}

fn payload(event: &Event, tz: &chrono_tz::Tz, fire: i64) -> Payload {
    Payload::Reminder {
        summary: event.summary.clone(),
        location: event.location.clone(),
        start: (!event.is_all_day).then(|| instant(event, false, tz)).flatten(),
        end: (!event.is_all_day).then(|| instant(event, true, tz)).flatten(),
        all_day: event.is_all_day,
        date: event.date.format("%Y-%m-%d").to_string(),
        tag: reminder_tag(&event.uid, fire),
        url: "/calendar/".to_owned(),
    }
}

/// The tag of a reminder's notification, the calendar page's too (event-reminders.ts), so the page's and
/// the pushed one replace each other.
fn reminder_tag(uid: &str, fire: i64) -> String {
    format!("reminder|{uid}|{fire}")
}

/// When `alarm` of the occurrence `event` goes off (epoch seconds), its floating times in `tz`.
fn fire_at(event: &Event, alarm: &Alarm, tz: &chrono_tz::Tz) -> Option<i64> {
    match alarm.trigger_type.as_str() {
        "absolute" => {
            let at = NaiveDateTime::parse_from_str(alarm.datetime.as_deref()?.trim_end_matches('Z'), "%Y%m%dT%H%M%S").ok()?;
            Some(at.and_utc().timestamp())
        }
        "end" => Some(instant(event, true, tz)? + alarm.minutes? * 60),
        _ => Some(instant(event, false, tz)? + alarm.minutes? * 60),
    }
}

/// The occurrence's start (or end): its instant, or for an all-day or floating one its wall clock in `tz`
/// (an all-day event from midnight to the midnight after its last day).
fn instant(event: &Event, end: bool, tz: &chrono_tz::Tz) -> Option<i64> {
    let anchored = if end { event.end_ts } else { event.start_ts };
    if anchored.is_some() {
        return anchored;
    }
    let (date, time) = if end {
        let date = event
            .dtend
            .as_deref()
            .and_then(|s| NaiveDate::parse_from_str(s.get(..10)?, "%Y-%m-%d").ok())
            .unwrap_or(if event.is_all_day { event.date + chrono::TimeDelta::days(1) } else { event.date });
        (date, event.end_time.as_deref())
    } else {
        (event.date, event.start_time.as_deref())
    };
    let time = time.and_then(|t| NaiveTime::parse_from_str(t, "%H:%M").ok()).unwrap_or(NaiveTime::MIN);
    tz.from_local_datetime(&date.and_time(time)).earliest().map(|at| at.timestamp())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ICAL: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:test\r\n";

    fn event(body: &str) -> Event {
        let raw = format!("{ICAL}BEGIN:VEVENT\r\nUID:e1\r\nSUMMARY:Standup\r\n{body}END:VEVENT\r\nEND:VCALENDAR\r\n");
        let day = NaiveDate::from_ymd_opt(2026, 11, 25).unwrap();
        CalDavClient::events_from_raw_ical(&raw, day - chrono::TimeDelta::days(1), day + chrono::TimeDelta::days(1))
            .into_iter()
            .next()
            .expect("an occurrence")
    }

    fn utc(s: &str) -> i64 {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap().and_utc().timestamp()
    }

    #[test]
    fn alarm_times() {
        let berlin: chrono_tz::Tz = "Europe/Berlin".parse().unwrap();
        let tokyo: chrono_tz::Tz = "Asia/Tokyo".parse().unwrap();
        let alarm = |body: &str| format!("BEGIN:VALARM\r\nACTION:DISPLAY\r\nDESCRIPTION:x\r\n{body}END:VALARM\r\n");

        // anchored: 09:00 in Berlin (08:00 UTC), 15 minutes before; the same instant for every device
        let e = event(&format!(
            "DTSTART;TZID=Europe/Berlin:20261125T090000\r\nDTEND;TZID=Europe/Berlin:20261125T093000\r\n{}",
            alarm("TRIGGER:-PT15M\r\n")
        ));
        assert_eq!(fire_at(&e, &e.alarms[0], &berlin), Some(utc("2026-11-25 07:45")));
        assert_eq!(fire_at(&e, &e.alarms[0], &tokyo), Some(utc("2026-11-25 07:45")));

        // relative to the end
        let e = event(&format!(
            "DTSTART:20261125T080000Z\r\nDTEND:20261125T083000Z\r\n{}",
            alarm("TRIGGER;RELATED=END:PT5M\r\n")
        ));
        assert_eq!(fire_at(&e, &e.alarms[0], &berlin), Some(utc("2026-11-25 08:35")));

        // floating: 09:00 on each device's clock
        let e = event(&format!("DTSTART:20261125T090000\r\nDTEND:20261125T100000\r\n{}", alarm("TRIGGER:-PT10M\r\n")));
        assert_eq!(fire_at(&e, &e.alarms[0], &berlin), Some(utc("2026-11-25 07:50")));
        assert_eq!(fire_at(&e, &e.alarms[0], &tokyo), Some(utc("2026-11-24 23:50")));

        // all day: from midnight, a day before at 09:00 is -PT15H
        let e = event(&format!("DTSTART;VALUE=DATE:20261125\r\nDTEND;VALUE=DATE:20261126\r\n{}", alarm("TRIGGER:-PT15H\r\n")));
        assert_eq!(fire_at(&e, &e.alarms[0], &berlin), Some(utc("2026-11-24 08:00")));

        // absolute
        let e = event(&format!("DTSTART:20261125T080000Z\r\n{}", alarm("TRIGGER;VALUE=DATE-TIME:20261124T180000Z\r\n")));
        assert_eq!(fire_at(&e, &e.alarms[0], &berlin), Some(utc("2026-11-24 18:00")));
    }

    /// A reminder due now goes out once, to the device, encrypted as every notification; not again on the
    /// next tick, and not one whose time is long past.
    #[tokio::test]
    async fn due_reminders_go_out_once() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let dir = std::env::temp_dir().join(format!("caley-reminders-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = crate::db::init_db(dir.join("t.db").to_str().unwrap()).await.unwrap();
        let key = [9; 32];
        crate::db::ensure_user(&db, "u@example", None).await.unwrap();
        let settings = crate::db::UserSettings {
            caldav_base_url: "https://dav.example/".into(),
            caldav_username: "u".into(),
            caldav_password: "p".into(),
        };
        let account = crate::db::upsert_caldav_account(&db, "u@example", Some("acc"), None, &settings, &key)
            .await
            .unwrap();
        crate::db::upsert_cached_calendar(&db, &account, "/cal/", "Work", None).await.unwrap();

        // two events today: one with an alarm due now, one whose alarm was an hour ago
        let now = chrono::Utc::now().timestamp() / 60 * 60;
        let at = |ts: i64| chrono::DateTime::from_timestamp(ts, 0).unwrap().format("%Y%m%dT%H%M%SZ").to_string();
        for (uid, start) in [("due", now + 15 * 60), ("past", now - 45 * 60)] {
            let raw = format!(
                "{ICAL}BEGIN:VEVENT\r\nUID:{uid}\r\nSUMMARY:Event {uid}\r\nDTSTART:{}\r\nDTEND:{}\r\n\
                 BEGIN:VALARM\r\nACTION:DISPLAY\r\nDESCRIPTION:x\r\nTRIGGER:-PT15M\r\nEND:VALARM\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",
                at(start),
                at(start + 3600)
            );
            crate::db::upsert_cached_event_raw(&db, &account, "/cal/", &format!("/cal/{uid}.ics"), "1", &raw, &crate::search::SearchMode::Default)
                .await
                .unwrap();
        }

        // the device's push service
        let (tx, mut rx) = tokio::sync::mpsc::channel::<Vec<u8>>(4);
        let service = axum::Router::new().route(
            "/push",
            axum::routing::post(move |body: axum::body::Bytes| {
                let tx = tx.clone();
                async move {
                    tx.send(body.to_vec()).await.unwrap();
                    axum::http::StatusCode::CREATED
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/push", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, service).await.unwrap() });
        use base64::Engine as _;
        let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD;
        let device = p256::SecretKey::from_slice(&[3; 32]).unwrap();
        use p256::elliptic_curve::sec1::ToSec1Point as _;
        sqlx::query("INSERT INTO notification_subscriptions(endpoint, user_id, p256dh, auth, timezone, created_at) VALUES(?, ?, ?, ?, 'Europe/Berlin', 0)")
            .bind(&endpoint)
            .bind("u@example")
            .bind(b64.encode(device.public_key().to_sec1_point(false).as_bytes()))
            .bind(b64.encode([4u8; 16]))
            .execute(&db)
            .await
            .unwrap();

        let notifier = Notifier::new(db.clone(), key, "https://caley.example", true).unwrap();
        assert_eq!(tick(&db, &key, &notifier, now).await.unwrap(), 1);
        assert!(rx.recv().await.is_some());
        assert_eq!(tick(&db, &key, &notifier, now + 60).await.unwrap(), 0, "not twice");
        assert!(rx.try_recv().is_err());
    }
}
