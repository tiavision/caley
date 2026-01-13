use std::{io::BufReader, str::FromStr};
use std::result::Result::Ok;

use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use chrono_tz::Tz;
use clap::Parser;
use dotenvy::dotenv;
use fast_dav_rs::CalDavClient;
use ical::IcalParser;

fn parse_timezone(tzid: &str) -> Option<chrono_tz::Tz> {
    chrono_tz::Tz::from_str(tzid).ok()
}

fn print_event_details(ical_str: &str) {
    let ical_bytes = ical_str.as_bytes();
    let parser = IcalParser::new(BufReader::new(ical_bytes));
    
    for calendar in parser {
        for event in calendar.unwrap().events {
            let mut summary = String::new();
            let mut dtstart = None;
            let mut dtend = None;
            let mut timezone: Option<chrono_tz::Tz> = None;
            
            for prop in event.properties {
                match prop.name.as_str() {
                    "SUMMARY" => if let Some(value) = prop.value { summary = value.to_string(); },
                    "DTSTART" => if let Some(value) = prop.value {
                        if let Some(params) = prop.params {
                            let tz = params.iter().filter(|x| x.0 == "TZID").map(|x| x.1.first().unwrap()).next();
                            if let Some(tz) = tz {
                                timezone = parse_timezone(tz);
                            }
                        }
                        dtstart = Some(parse_dt(&value, timezone.unwrap_or(Tz::UTC)));
                    },
                    "DTEND" => if let Some(value) = prop.value {
                        if let Some(params) = prop.params {
                            let tz = params.iter().filter(|x| x.0 == "TZID").map(|x| x.1.first().unwrap()).next();
                            if let Some(tz) = tz {
                                timezone = parse_timezone(tz);
                            }
                        }
                        dtend = Some(parse_dt(&value, timezone.unwrap_or(Tz::UTC)));
                    },
                    _ => { },
                }
            }
            
            println!("Title: {}", summary);
            if let Some(start) = dtstart {
                println!("Start: {}", start.format("%Y-%m-%d %H:%M (%Z)"));
            }
            if let Some(end) = dtend {
                println!("End: {}", end.format("%Y-%m-%d %H:%M (%Z)"));
            }
            println!();
        }
    }
}

fn parse_dt(value: &str, timezone: Tz) -> DateTime<Tz> {
    if value.len() == 8 {
        // DATE: 20260110 -> midnight UTC
        let date = NaiveDate::parse_from_str(value, "%Y%m%d").expect("DATE parse");
        date.and_time(NaiveTime::default()).and_local_timezone(timezone).unwrap()
    } else {
        // DATE-TIME: 20260110T170000Z
        if let Ok(dt) = DateTime::parse_from_str(value, "%Y%m%dT%H%M%SZ") {
            dt.with_timezone(&timezone)
        } else if let Ok(dt_offset) = DateTime::parse_from_rfc3339(value) {
            dt_offset.with_timezone(&timezone)
        } else {
            // parse values of type 20251230T090000 (local time, no timezone info)
            // let dt_local = DateTime::parse_from_str(value, "%Y%m%dT%H%M%S").unwrap();
            let dt_local = NaiveDateTime::parse_from_str(value, "%Y%m%dT%H%M%S").unwrap();
            dt_local.and_local_timezone(timezone).unwrap()
        }
    }
}


#[derive(clap::Parser, Debug)]
struct Cli {
    #[arg(env)]
    base_url: String,
    #[arg(env)]
    username: String,
    #[arg(env)]
    password: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenv().ok();
    let args = Cli::parse();

    // build http_client
    let client = CalDavClient::new(
        &args.base_url,
        Some(&args.username),
        Some(&args.password),
    )?;

    let principal = client.discover_current_user_principal().await.unwrap()
        .ok_or(anyhow::anyhow!("No principal")).unwrap();

    let homes = client.discover_calendar_home_set(&principal).await?;

    let mut termine: String = String::new();
    for home in &homes {
        let calendars = client.list_calendars(home).await.unwrap();
        for calendar in calendars {
            if let Some(displayname) = &calendar.displayname && displayname == "Termine" {
                termine = calendar.href.clone();
            }
            println!("Calendar: {:?} {:?}", calendar.displayname, calendar.href);
        }
    }

    println!();

    let now = Utc::now();
    let start = now.format("%Y%m%dT000000Z").to_string();
    let end = now.checked_add_signed(chrono::Duration::days(1))
        .map(|d| d.format("%Y%m%dT235959Z").to_string())
        .unwrap_or(start.clone());

    dbg!(&start);
    dbg!(&end);


    let events = client.calendar_query_timerange(&termine, "VEVENT", Some(&start), Some(&end), true).await.unwrap();


    for event in events {
        if let Some(ical_data) = event.calendar_data {
            dbg!(&ical_data);
            print_event_details(&ical_data);
        }
    }



    Ok(())

    // let app = Router::new().route("/", get(|| async { "Hello, World!" }));

    // // run our app with hyper, listening globally on port 3000
    // let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    // axum::serve(listener, app).await.unwrap();
}
