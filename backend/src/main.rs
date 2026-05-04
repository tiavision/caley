use anyhow::{bail, Context};
use chrono::{NaiveDate, NaiveDateTime};
use clap::Parser;
use dotenvy::dotenv;
use quick_xml::events::Event;
use quick_xml::Reader;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, CONTENT_TYPE};
use reqwest::Client;

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

    let client = Client::builder()
        .user_agent("caley/0.1.0")
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let base_url = args.base_url.trim_end_matches('/').to_string();

    let caldav_endpoint = discover_caldav_endpoint(&client, &base_url).await;
    let start_url = caldav_endpoint.as_deref().unwrap_or(&base_url);

    let calendar_home_sets = discover_calendar_home_set(&client, &args, &start_url).await?;
    let (calendar_url, _calendar_home_set) = find_calendar_in_home_sets(
        &client,
        &args,
        &calendar_home_sets,
        &args.calendar_name,
    )
    .await?;
    let events = fetch_events(&client, &args, &calendar_url).await?;

    for (name, dtstart, tz) in events {
        println!("{name} {dtstart} {tz}");
    }

    Ok(())
}

async fn discover_caldav_endpoint(_client: &Client, base_url: &str) -> Option<String> {
    let well_known = format!("{}/.well-known/caldav", base_url);

    let redirect_client = Client::builder()
        .user_agent("caley/0.1.0")
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .ok()?;

    let resp = redirect_client
        .get(&well_known)
        .send()
        .await
        .ok()?;

    let final_url = resp.url().to_string();

    Some(final_url.trim_end_matches('/').to_string())
}

async fn propfind(
    client: &Client,
    args: &Cli,
    url: &str,
    depth: &str,
    body: &str,
) -> anyhow::Result<String> {
    let mut url = url.to_string();
    for _ in 0..5 {
        let depth_hdr = HeaderName::from_static("depth");
        let mut headers = HeaderMap::new();
        headers.insert(depth_hdr, HeaderValue::from_str(depth)?);
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/xml"));

        let resp = client
            .request(reqwest::Method::from_bytes(b"PROPFIND")?, &url)
            .headers(headers)
            .basic_auth(&args.username, Some(&args.password))
            .body(body.to_string())
            .send()
            .await
            .context("PROPFIND request failed")?;

        let status = resp.status();

        if status.is_redirection() {
            if let Some(location) = resp.headers().get("location") {
                let location = location.to_str()?.to_string();
                url = location;
                continue;
            }
            bail!("PROPFIND redirect without Location header");
        }

        if !status.is_success() && status.as_u16() != 207 {
            bail!("PROPFIND returned {status}");
        }

        let text = resp.text().await.context("failed to read PROPFIND response")?;
        return Ok(text);
    }
    bail!("too many redirects for PROPFIND")
}

async fn report(
    client: &Client,
    args: &Cli,
    url: &str,
    body: &str,
) -> anyhow::Result<String> {
    let mut url = url.to_string();
    for _ in 0..5 {
        let depth_hdr = HeaderName::from_static("depth");
        let mut headers = HeaderMap::new();
        headers.insert(depth_hdr, HeaderValue::from_static("1"));
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/xml"));

        let resp = client
            .request(reqwest::Method::from_bytes(b"REPORT")?, &url)
            .headers(headers)
            .basic_auth(&args.username, Some(&args.password))
            .body(body.to_string())
            .send()
            .await
            .context("REPORT request failed")?;

        let status = resp.status();

        if status.is_redirection() {
            if let Some(location) = resp.headers().get("location") {
                let location = location.to_str()?.to_string();
                url = location;
                continue;
            }
            bail!("REPORT redirect without Location header");
        }

        if !status.is_success() && status.as_u16() != 207 {
            bail!("REPORT returned {status}");
        }

        let text = resp.text().await.context("failed to read REPORT response")?;
        return Ok(text);
    }
    bail!("too many redirects for REPORT")
}

async fn discover_calendar_home_set(
    client: &Client,
    args: &Cli,
    base_url: &str,
) -> anyhow::Result<Vec<String>> {
    let principal_body = r#"<?xml version="1.0" encoding="utf-8"?>
<propfind xmlns="DAV:">
  <prop>
    <current-user-principal/>
  </prop>
</propfind>"#;

    let xml = propfind(client, args, base_url, "0", principal_body).await?;

    match extract_hrefs(&xml, "current-user-principal") {
        Ok(hrefs) if !hrefs.is_empty() => {
            let principal_href = &hrefs[0];
            let principal_url = resolve_url(base_url, principal_href);

            let home_body = r#"<?xml version="1.0" encoding="utf-8"?>
<propfind xmlns="DAV:">
  <prop>
    <calendar-home-set xmlns="urn:ietf:params:xml:ns:caldav"/>
  </prop>
</propfind>"#;

            let home_xml = propfind(client, args, &principal_url, "0", home_body).await?;
            let hrefs = extract_hrefs(&home_xml, "calendar-home-set")
                .with_context(|| format!("calendar-home-set not found in response from {principal_url}"))?;
            Ok(hrefs.into_iter().map(|h| resolve_url(&principal_url, &h)).collect())
        }
        _ => {
            let home_body = r#"<?xml version="1.0" encoding="utf-8"?>
<propfind xmlns="DAV:">
  <prop>
    <calendar-home-set xmlns="urn:ietf:params:xml:ns:caldav"/>
  </prop>
</propfind>"#;

            let home_xml = propfind(client, args, base_url, "0", home_body).await?;
            let hrefs = extract_hrefs(&home_xml, "calendar-home-set")
                .with_context(|| format!("calendar-home-set not found in response from {base_url}"))?;
            Ok(hrefs.into_iter().map(|h| resolve_url(base_url, &h)).collect())
        }
    }
}

async fn find_calendar_in_home_sets(
    client: &Client,
    args: &Cli,
    home_sets: &[String],
    calendar_name: &str,
) -> anyhow::Result<(String, String)> {
    for home_set_url in home_sets {
        match find_calendar(client, args, home_set_url, calendar_name).await {
            Ok(url) => return Ok((url, home_set_url.clone())),
            Err(e) => {
                let _ = e;
            }
        }
    }
    bail!("Calendar '{calendar_name}' not found in any calendar home set")
}

async fn find_calendar(
    client: &Client,
    args: &Cli,
    home_set_url: &str,
    calendar_name: &str,
) -> anyhow::Result<String> {
    let body = r#"<?xml version="1.0" encoding="utf-8"?>
<propfind xmlns="DAV:">
  <prop>
    <displayname/>
  </prop>
</propfind>"#;

    let xml = propfind(client, args, home_set_url, "1", body).await?;
    find_calendar_by_name(&xml, calendar_name, home_set_url)
}

fn find_calendar_by_name(
    xml: &str,
    calendar_name: &str,
    base_url: &str,
) -> anyhow::Result<String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let base_url_resolved = resolve_url(base_url, "");
    let mut current_href: Option<String> = None;
    let mut current_displayname: Option<String> = None;
    let mut in_href = false;
    let mut in_displayname = false;
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(e) => {
                let local = e.local_name();
                let local = std::str::from_utf8(local.as_ref())?;
                match local {
                    "href" => in_href = true,
                    "displayname" => in_displayname = true,
                    _ => {}
                }
            }
            Event::Text(e) => {
                if in_href {
                    current_href = Some(e.unescape()?.to_string());
                } else if in_displayname {
                    current_displayname = Some(e.unescape()?.to_string());
                }
            }
            Event::End(e) => {
                let local = e.local_name();
                let local = std::str::from_utf8(local.as_ref())?;
                match local {
                    "href" => in_href = false,
                    "displayname" => in_displayname = false,
                    "response" => {
                        if let Some(dn) = &current_displayname {
                            if dn == calendar_name {
                                if let Some(href) = &current_href {
                                    let resolved = resolve_url(base_url, href);
                                    if resolved.trim_end_matches('/') != base_url_resolved.trim_end_matches('/') {
                                        return Ok(resolved);
                                    }
                                }
                            }
                        }
                        current_href = None;
                        current_displayname = None;
                    }
                    _ => {}
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }

    bail!("Calendar '{calendar_name}' not found in response")
}

fn resolve_url(current_url: &str, href: &str) -> String {
    if href.starts_with("http://") || href.starts_with("https://") {
        href.to_string()
    } else if href.starts_with('/') {
        let scheme_end = current_url.find("://").map(|i| i + 3).unwrap_or(0);
        let after_scheme = &current_url[scheme_end..];
        let host = after_scheme.split('/').next().unwrap_or("");
        let scheme = &current_url[..scheme_end - 3];
        format!("{scheme}://{host}{href}")
    } else {
        let base_trimmed = current_url.trim_end_matches('/');
        let href_trimmed = href.trim_start_matches('/');
        format!("{base_trimmed}/{href_trimmed}")
    }
}

fn extract_hrefs(xml: &str, element_local_name: &str) -> anyhow::Result<Vec<String>> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut in_target = false;
    let mut in_href = false;
    let mut hrefs = Vec::new();
    let mut current_href = String::new();
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(e) => {
                let local = e.local_name();
                let local = std::str::from_utf8(local.as_ref())?;
                if local == element_local_name {
                    in_target = true;
                } else if in_target && local == "href" {
                    in_href = true;
                    current_href.clear();
                }
            }
            Event::Text(e) => {
                if in_href {
                    current_href.push_str(&e.unescape()?);
                }
            }
            Event::End(e) => {
                let local = e.local_name();
                let local = std::str::from_utf8(local.as_ref())?;
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
            Event::Eof => break,
            _ => {}
        }
    }

    if hrefs.is_empty() {
        bail!("element '{element_local_name}' not found in response")
    }
    Ok(hrefs)
}

async fn fetch_events(
    client: &Client,
    args: &Cli,
    calendar_url: &str,
) -> anyhow::Result<Vec<(String, String, String)>> {
    let body = r#"<?xml version="1.0" encoding="utf-8"?>
<c:calendar-query xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav">
  <d:prop>
    <c:calendar-data/>
  </d:prop>
  <c:filter>
    <c:comp-filter name="VCALENDAR">
      <c:comp-filter name="VEVENT"/>
    </c:comp-filter>
  </c:filter>
</c:calendar-query>"#;

    let xml = report(client, args, calendar_url, body).await?;
    extract_events(&xml)
}

fn extract_events(xml: &str) -> anyhow::Result<Vec<(String, String, String)>> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut events = Vec::new();
    let mut in_calendar_data = false;
    let mut current_ical = String::new();
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(e) => {
                let local = e.local_name();
                let local = std::str::from_utf8(local.as_ref())?;
                if local == "calendar-data" {
                    in_calendar_data = true;
                    current_ical.clear();
                }
            }
            Event::Text(e) => {
                if in_calendar_data {
                    current_ical.push_str(&e.unescape()?);
                }
            }
            Event::End(e) => {
                let local = e.local_name();
                let local = std::str::from_utf8(local.as_ref())?;
                if local == "calendar-data" {
                    in_calendar_data = false;
                    if !current_ical.is_empty() {
                        if let Some(event) = parse_event(&current_ical) {
                            events.push(event);
                        }
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }

    Ok(events)
}

fn parse_event(ical_data: &str) -> Option<(String, String, String)> {
    let calendar = ical::IcalParser::new(ical_data.as_bytes()).next()?;
    let calendar = calendar.ok()?;

    for event in calendar.events {
        let mut summary: Option<String> = None;
        let mut dtstart_value: Option<String> = None;
        let mut dtstart_tzid: Option<String> = None;

        for property in event.properties {
            match property.name.as_str() {
                "SUMMARY" => summary = property.value,
                "DTSTART" => {
                    dtstart_value = property.value;
                    if let Some(params) = property.params {
                        for (key, vals) in params {
                            if key == "TZID" {
                                if let Some(first_val) = vals.first() {
                                    dtstart_tzid = Some(first_val.clone());
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        let name = summary.unwrap_or_default();
        let raw = dtstart_value.as_deref()?;

        let (date_str, tz_str) = if let Some(tzid) = &dtstart_tzid {
            let naive = parse_dtstart(raw)?;
            (naive.format("%Y-%m-%d %H:%M:%S").to_string(), tzid.clone())
        } else if raw.ends_with('Z') {
            let trimmed = raw.trim_end_matches('Z');
            let naive = parse_dtstart(trimmed)?;
            (naive.format("%Y-%m-%d %H:%M:%S").to_string(), "UTC".to_string())
        } else {
            let naive = parse_dtstart(raw)?;
            (naive.format("%Y-%m-%d %H:%M:%S").to_string(), "UTC".to_string())
        };

        return Some((name, date_str, tz_str));
    }

    None
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