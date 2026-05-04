mod caldav;

use caldav::CalDavClient;
use clap::Parser;
use dotenvy::dotenv;

#[derive(clap::Parser, Debug)]
struct Cli {
    #[arg(env = "CALDAV_BASE_URL")]
    base_url: String,
    #[arg(env = "CALDAV_USERNAME")]
    username: String,
    #[arg(env = "CALDAV_PASSWORD")]
    password: String,
    #[arg(env = "CALDAV_CALENDAR_NAME")]
    calendar_name: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenv().ok();
    let args = Cli::parse();

    let client = CalDavClient::new(args.base_url, args.username, args.password)?;

    let calendars = client.calendars().await?;
    for cal in &calendars {
        println!("{}: {}", cal.name, cal.url);
    }

    let events = client.events(&args.calendar_name).await?;
    for event in &events {
        println!("{} {} {}", event.summary, event.dtstart, event.tz);
    }

    Ok(())
}