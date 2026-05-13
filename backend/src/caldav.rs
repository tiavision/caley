use anyhow::{bail, Context};
use chrono::{NaiveDate, NaiveDateTime, TimeDelta, TimeZone as _, Utc};
use quick_xml::events::Event as XmlEvent;
use quick_xml::Reader;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, CONTENT_TYPE};
use reqwest::Client;
use rrule::RRuleSet;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Serialize)]
pub struct Calendar {
    pub name: String,
    pub href: String,
    pub color: Option<String>,
}

#[derive(Serialize)]
pub struct Event {
    pub uid: String,
    pub summary: String,
    pub description: String,
    pub location: String,
    pub dtstart: String,
    pub dtend: Option<String>,
    pub date: NaiveDate,
    pub start_time: Option<String>,
    pub end_time: Option<String>,
    pub is_all_day: bool,
    pub tz: String,
    pub is_recurring: bool,
    pub recurrence: Option<RecurrenceInfo>,
    pub master_dtstart: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct RecurrenceInfo {
    pub freq: String,
    pub interval: u32,
    pub until: Option<String>,
    pub count: Option<u32>,
    pub by_day: Vec<String>,
    pub by_month_day: Vec<u32>,
    pub by_month: Vec<u32>,
    pub by_set_pos: Vec<i32>,
}

struct DateProperty {
    value: String,
    tzid: Option<String>,
}

struct RawIcalEvent {
    uid: String,
    summary: String,
    description: String,
    location: String,
    dtstart: NaiveDateTime,
    dtstart_is_date: bool,
    dtstart_tzid: Option<String>,
    duration: TimeDelta,
    rrule: Option<String>,
    rdates: Vec<DateProperty>,
    exdates: Vec<DateProperty>,
    recurrence_id: Option<NaiveDateTime>,
    recurrence_id_tzid: Option<String>,
}

#[derive(Clone)]
pub struct CalDavClient {
    client: Client,
    base_url: String,
    username: String,
    password: String,
}

impl CalDavClient {
    pub fn new(base_url: String, username: String, password: String) -> anyhow::Result<Self> {
        let client = Client::builder()
            .user_agent("caley/0.1.0")
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        Ok(Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
            username,
            password,
        })
    }

    pub async fn calendars(&self) -> anyhow::Result<Vec<Calendar>> {
        let start_url = self.start_url().await;
        let home_sets = self.discover_calendar_home_sets(&start_url).await?;
        let mut all = Vec::new();
        for hs in &home_sets {
            if let Ok(cals) = self.find_calendars_in_home_set(hs).await {
                all.extend(cals);
            }
        }
        Ok(all)
    }

    pub async fn events(
        &self,
        href: &str,
        start: NaiveDate,
        end: NaiveDate,
    ) -> anyhow::Result<Vec<Event>> {
        let raw_events = self.fetch_events(href, start, end).await?;
        Ok(expand_events(raw_events, start, end))
    }

    async fn start_url(&self) -> String {
        self.discover_caldav_endpoint()
            .await
            .unwrap_or_else(|| self.base_url.clone())
    }

    async fn discover_caldav_endpoint(&self) -> Option<String> {
        let well_known = format!("{}/.well-known/caldav", self.base_url);

        let resp = self
            .client
            .get(&well_known)
            .basic_auth(&self.username, Some(&self.password))
            .send()
            .await
            .ok()?;

        if resp.status().is_redirection() {
            if let Some(location) = resp.headers().get("location") {
                let location = location.to_str().ok()?.to_string();
                return Some(resolve_url(&well_known, &location).trim_end_matches('/').to_string());
            }
        }

        None
    }

    async fn discover_calendar_home_sets(
        &self,
        start_url: &str,
    ) -> anyhow::Result<Vec<String>> {
        let principal_body = r#"<?xml version="1.0" encoding="utf-8"?>
<propfind xmlns="DAV:">
  <prop>
    <current-user-principal/>
  </prop>
</propfind>"#;

        let xml = self.propfind(start_url, "0", principal_body).await?;

        match extract_hrefs(&xml, "current-user-principal") {
            Ok(hrefs) if !hrefs.is_empty() => {
                let principal_href = &hrefs[0];
                let principal_url = resolve_url(start_url, principal_href);

                let home_body = r#"<?xml version="1.0" encoding="utf-8"?>
<propfind xmlns="DAV:">
  <prop>
    <calendar-home-set xmlns="urn:ietf:params:xml:ns:caldav"/>
  </prop>
</propfind>"#;

                let home_xml = self.propfind(&principal_url, "0", home_body).await?;
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

                let home_xml = self.propfind(start_url, "0", home_body).await?;
                let hrefs = extract_hrefs(&home_xml, "calendar-home-set")
                    .with_context(|| format!("calendar-home-set not found in response from {start_url}"))?;
                Ok(hrefs.into_iter().map(|h| resolve_url(start_url, &h)).collect())
            }
        }
    }

    async fn find_calendars_in_home_set(
        &self,
        home_set_url: &str,
    ) -> anyhow::Result<Vec<Calendar>> {
        let body = r#"<?xml version="1.0" encoding="utf-8"?>
<propfind xmlns="DAV:" xmlns:APPLE="http://apple.com/ns/ical/">
  <prop>
    <displayname/>
    <APPLE:calendar-color/>
  </prop>
</propfind>"#;

        let xml = self.propfind(home_set_url, "1", body).await?;
        parse_all_calendars(&xml, home_set_url)
    }

    async fn fetch_events(
        &self,
        calendar_url: &str,
        start: NaiveDate,
        end: NaiveDate,
    ) -> anyhow::Result<Vec<RawIcalEvent>> {
        let start_str = format!("{}T000000Z", start.format("%Y%m%d"));
        let end_str = format!("{}T235959Z", end.format("%Y%m%d"));

        let body = format!(
            r#"<?xml version="1.0" encoding="utf-8"?>
<c:calendar-query xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav">
  <d:prop>
    <c:calendar-data/>
  </d:prop>
  <c:filter>
    <c:comp-filter name="VCALENDAR">
      <c:comp-filter name="VEVENT">
        <c:time-range start="{start_str}" end="{end_str}"/>
      </c:comp-filter>
    </c:comp-filter>
  </c:filter>
</c:calendar-query>"#
        );

        let xml = self.report(calendar_url, &body).await?;
        extract_raw_events(&xml)
    }

    async fn propfind(
        &self,
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

            let resp = self
                .client
                .request(reqwest::Method::from_bytes(b"PROPFIND")?, &url)
                .headers(headers)
                .basic_auth(&self.username, Some(&self.password))
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

    async fn report(&self, url: &str, body: &str) -> anyhow::Result<String> {
        let mut url = url.to_string();
        for _ in 0..5 {
            let depth_hdr = HeaderName::from_static("depth");
            let mut headers = HeaderMap::new();
            headers.insert(depth_hdr, HeaderValue::from_static("1"));
            headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/xml"));

            let resp = self
                .client
                .request(reqwest::Method::from_bytes(b"REPORT")?, &url)
                .headers(headers)
                .basic_auth(&self.username, Some(&self.password))
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

    pub async fn put_event(
        &self,
        calendar_href: &str,
        uid: &str,
        ical_data: &str,
    ) -> anyhow::Result<()> {
        let url = format!("{}/{}.ics", calendar_href.trim_end_matches('/'), uid);
        let mut url = url;
        for _ in 0..5 {
            let mut headers = HeaderMap::new();
            headers.insert(CONTENT_TYPE, HeaderValue::from_static("text/calendar; charset=utf-8"));

            let resp = self
                .client
                .put(&url)
                .headers(headers)
                .basic_auth(&self.username, Some(&self.password))
                .body(ical_data.to_string())
                .send()
                .await
                .context("PUT request failed")?;

            let status = resp.status();
            if status.is_redirection() {
                if let Some(location) = resp.headers().get("location") {
                    url = location.to_str()?.to_string();
                    continue;
                }
                bail!("PUT redirect without Location header");
            }
            if !status.is_success() {
                let text = resp.text().await.unwrap_or_default();
                bail!("PUT returned {status}: {text}");
            }
            return Ok(());
        }
        bail!("too many redirects for PUT")
    }

    pub async fn delete_event(&self, calendar_href: &str, uid: &str) -> anyhow::Result<()> {
        let url = format!("{}/{}.ics", calendar_href.trim_end_matches('/'), uid);
        let mut url = url;
        for _ in 0..5 {
            let resp = self
                .client
                .delete(&url)
                .basic_auth(&self.username, Some(&self.password))
                .send()
                .await
                .context("DELETE request failed")?;

            let status = resp.status();
            if status.is_redirection() {
                if let Some(location) = resp.headers().get("location") {
                    url = location.to_str()?.to_string();
                    continue;
                }
                bail!("DELETE redirect without Location header");
            }
            if !status.is_success() {
                bail!("DELETE returned {status}");
            }
            return Ok(());
        }
        bail!("too many redirects for DELETE")
    }

    pub async fn get_event(&self, calendar_href: &str, uid: &str) -> anyhow::Result<String> {
        let url = format!("{}/{}.ics", calendar_href.trim_end_matches('/'), uid);
        let mut url = url;
        for _ in 0..5 {
            let resp = self
                .client
                .get(&url)
                .basic_auth(&self.username, Some(&self.password))
                .send()
                .await
                .context("GET request failed")?;

            let status = resp.status();
            if status.is_redirection() {
                if let Some(location) = resp.headers().get("location") {
                    url = location.to_str()?.to_string();
                    continue;
                }
                bail!("GET redirect without Location header");
            }
            if !status.is_success() {
                bail!("GET returned {status}");
            }
            let text = resp.text().await.context("failed to read GET response")?;
            return Ok(text);
        }
        bail!("too many redirects for GET")
    }
}

fn resolve_url(current_url: &str, href: &str) -> String {
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
            XmlEvent::Start(e) => {
                let local = e.local_name();
                let local = std::str::from_utf8(local.as_ref())?;
                if local == element_local_name {
                    in_target = true;
                } else if in_target && local == "href" {
                    in_href = true;
                    current_href.clear();
                }
            }
            XmlEvent::Text(e) => {
                if in_href {
                    current_href.push_str(&e.unescape()?);
                }
            }
            XmlEvent::End(e) => {
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
            XmlEvent::Eof => break,
            _ => {}
        }
    }

    if hrefs.is_empty() {
        bail!("element '{element_local_name}' not found in response")
    }
    Ok(hrefs)
}

fn parse_all_calendars(xml: &str, base_url: &str) -> anyhow::Result<Vec<Calendar>> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

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
                let local = std::str::from_utf8(local.as_ref())?;
                match local {
                    "href" => in_href = true,
                    "displayname" => in_displayname = true,
                    "calendar-color" => in_color = true,
                    _ => {}
                }
            }
            XmlEvent::Text(e) => {
                if in_href {
                    current_href = Some(e.unescape()?.to_string());
                } else if in_displayname {
                    current_displayname = Some(e.unescape()?.to_string());
                } else if in_color {
                    current_color = Some(e.unescape()?.to_string());
                }
            }
            XmlEvent::End(e) => {
                let local = e.local_name();
                let local = std::str::from_utf8(local.as_ref())?;
                match local {
                    "href" => in_href = false,
                    "displayname" => in_displayname = false,
                    "calendar-color" => in_color = false,
                    "response" => {
                        if let Some(href) = &current_href {
                            let resolved = resolve_url(base_url, href);
                            if resolved.trim_end_matches('/')
                                != base_url_resolved.trim_end_matches('/')
                            {
                                let name = current_displayname.clone().unwrap_or_else(|| {
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
                                    color: current_color.clone(),
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

fn extract_raw_events(xml: &str) -> anyhow::Result<Vec<RawIcalEvent>> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut events = Vec::new();
    let mut in_calendar_data = false;
    let mut current_ical = String::new();
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf)? {
            XmlEvent::Start(e) => {
                let local = e.local_name();
                let local = std::str::from_utf8(local.as_ref())?;
                if local == "calendar-data" {
                    in_calendar_data = true;
                    current_ical.clear();
                }
            }
            XmlEvent::Text(e) => {
                if in_calendar_data {
                    current_ical.push_str(&e.unescape()?);
                }
            }
            XmlEvent::End(e) => {
                let local = e.local_name();
                let local = std::str::from_utf8(local.as_ref())?;
                if local == "calendar-data" {
                    in_calendar_data = false;
                    if !current_ical.is_empty() {
                        events.extend(parse_raw_events(&current_ical));
                    }
                }
            }
            XmlEvent::Eof => break,
            _ => {}
        }
    }

    Ok(events)
}

fn unescape_ical_text(s: &str) -> String {
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
        let mut _dtend_tzid: Option<String> = None;
        let mut duration_str: Option<String> = None;
        let mut rrule: Option<String> = None;
        let mut rdates: Vec<DateProperty> = Vec::new();
        let mut exdates: Vec<DateProperty> = Vec::new();
        let mut recurrence_id_value: Option<String> = None;
        let mut recurrence_id_tzid: Option<String> = None;

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
                    if let Some(params) = property.params {
                        for (key, vals) in params {
                            if key == "TZID" {
                                if let Some(first_val) = vals.first() {
                                    _dtend_tzid = Some(first_val.clone());
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

        let duration = if let Some(ref dtend_raw) = dtend_value {
            match parse_dtstart(dtend_raw) {
                Some(dtend) => dtend - dtstart,
                None => continue,
            }
        } else if let Some(ref dur) = duration_str {
            parse_ical_duration(dur).unwrap_or_else(|| {
                if dtstart_is_date { TimeDelta::days(1) } else { TimeDelta::zero() }
            })
        } else if dtstart_is_date {
            TimeDelta::days(1)
        } else {
            TimeDelta::zero()
        };

        let recurrence_id = recurrence_id_value.as_ref().and_then(|v| parse_dtstart(v));

        results.push(RawIcalEvent {
            uid,
            summary,
            description,
            location,
            dtstart,
            dtstart_is_date,
            dtstart_tzid,
            duration,
            rrule,
            rdates,
            exdates,
            recurrence_id,
            recurrence_id_tzid,
        });
    }

    results
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

fn expand_events(raw_events: Vec<RawIcalEvent>, start: NaiveDate, end: NaiveDate) -> Vec<Event> {
    let mut groups: HashMap<String, Vec<RawIcalEvent>> = HashMap::new();
    for event in raw_events {
        groups.entry(event.uid.clone()).or_default().push(event);
    }

    let mut events = Vec::new();

    for (_, group) in groups {
        let master = group.iter().find(|e| e.recurrence_id.is_none());
        let overrides: Vec<&RawIcalEvent> = group.iter().filter(|e| e.recurrence_id.is_some()).collect();

        let Some(master) = master else { continue };

        if master.rrule.is_some() || !master.rdates.is_empty() {
            let recurrence = master.rrule.as_ref().map(|r| parse_rrule_info(r));
            let expanded = expand_recurring(master, start, end);

            let master_dtstart_str = if master.dtstart_is_date {
                format_datetime_with_tz(master.dtstart, "UTC").split('T').next().unwrap_or("").to_string()
            } else {
                format_datetime_with_tz(master.dtstart, master.dtstart_tzid.as_deref().unwrap_or("UTC"))
            };

            for dt in expanded {
                let matched_override = overrides.iter().find(|o| {
                    o.recurrence_id.as_ref().map_or(false, |rid| {
                        let rid_ts = naive_to_utc_timestamp(*rid, &o.recurrence_id_tzid);
                        let exp_ts = naive_to_utc_timestamp(dt, &master.dtstart_tzid);
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
                        &override_event
                            .dtstart_tzid
                            .as_deref()
                            .unwrap_or("UTC"),
                        true,
                        recurrence.clone(),
                        master_dtstart_str.clone(),
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
                        master.dtstart_tzid.as_deref().unwrap_or("UTC"),
                        true,
                        recurrence.clone(),
                        master_dtstart_str.clone(),
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
                master.dtstart_tzid.as_deref().unwrap_or("UTC"),
                false,
                None,
                String::new(),
            ));
        }
    }

    events
}

fn expand_recurring(master: &RawIcalEvent, start: NaiveDate, end: NaiveDate) -> Vec<NaiveDateTime> {
    let mut rrule_str = String::new();

    if master.dtstart_is_date {
        rrule_str.push_str(&format!(
            "DTSTART;VALUE=DATE:{}\n",
            master.dtstart.format("%Y%m%d")
        ));
    } else if let Some(tzid) = &master.dtstart_tzid {
        rrule_str.push_str(&format!(
            "DTSTART;TZID={}:{}\n",
            tzid,
            master.dtstart.format("%Y%m%dT%H%M%S")
        ));
    } else {
        rrule_str.push_str(&format!(
            "DTSTART:{}Z\n",
            master.dtstart.format("%Y%m%dT%H%M%S")
        ));
    }

    if let Some(ref rrule) = master.rrule {
        rrule_str.push_str(&format!("RRULE:{rrule}\n"));
    }

    for rdate in &master.rdates {
        if let Some(tzid) = &rdate.tzid {
            rrule_str.push_str(&format!("RDATE;TZID={}:{}\n", tzid, rdate.value));
        } else if rdate.value.ends_with('Z') {
            rrule_str.push_str(&format!("RDATE;VALUE=DATE-TIME:{}\n", rdate.value));
        } else {
            rrule_str.push_str(&format!("RDATE;VALUE=DATE-TIME:{}\n", rdate.value));
        }
    }

    for exdate in &master.exdates {
        if let Some(tzid) = &exdate.tzid {
            rrule_str.push_str(&format!("EXDATE;TZID={}:{}\n", tzid, exdate.value));
        } else if exdate.value.ends_with('Z') {
            rrule_str.push_str(&format!("EXDATE;VALUE=DATE-TIME:{}\n", exdate.value));
        } else {
            rrule_str.push_str(&format!("EXDATE;VALUE=DATE-TIME:{}\n", exdate.value));
        }
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

fn format_datetime_with_tz(dt: NaiveDateTime, tz: &str) -> String {
    if tz == "UTC" {
        format!("{}Z", dt.format("%Y-%m-%dT%H:%M:%S"))
    } else if let Ok(tz) = tz.parse::<chrono_tz::Tz>() {
        match tz.from_local_datetime(&dt) {
            chrono::LocalResult::Single(local_dt) => {
                local_dt.format("%Y-%m-%dT%H:%M:%S%:z").to_string()
            }
            chrono::LocalResult::Ambiguous(local_dt, _) => {
                local_dt.format("%Y-%m-%dT%H:%M:%S%:z").to_string()
            }
            chrono::LocalResult::None => {
                format!("{}Z", dt.format("%Y-%m-%dT%H:%M:%S"))
            }
        }
    } else {
        format!("{}Z", dt.format("%Y-%m-%dT%H:%M:%S"))
    }
}

fn create_event(
    uid: &str,
    summary: &str,
    description: &str,
    location: &str,
    dtstart: NaiveDateTime,
    dtstart_is_date: bool,
    dtend: Option<NaiveDateTime>,
    dtend_is_date: bool,
    tz: &str,
    is_recurring: bool,
    recurrence: Option<RecurrenceInfo>,
    master_dtstart: String,
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
        is_all_day: dtstart_is_date,
        tz: tz.to_string(),
        is_recurring,
        recurrence,
        master_dtstart,
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
        let Some((key, val)) = part.split_once('=') else { continue };
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

fn naive_to_utc_timestamp(dt: NaiveDateTime, tzid: &Option<String>) -> i64 {
    match tzid {
        Some(tz_name) => {
            if let Ok(tz) = tz_name.parse::<chrono_tz::Tz>() {
                match tz.from_local_datetime(&dt) {
                    chrono::LocalResult::Single(local_dt) => local_dt.timestamp(),
                    chrono::LocalResult::Ambiguous(local_dt, _) => local_dt.timestamp(),
                    chrono::LocalResult::None => dt.and_utc().timestamp(),
                }
            } else {
                dt.and_utc().timestamp()
            }
        }
        None => dt.and_utc().timestamp(),
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

pub fn generate_new_uid() -> String {
    Uuid::new_v4().to_string()
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
    ical.push_str(&format!("DTSTAMP:{}\r\n", Utc::now().format("%Y%m%dT%H%M%SZ")));
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
            ical.push_str(&format!("RRULE:{}\r\n", recurrence_info_to_rrule(rec, tz, is_all_day)));
        }
    }

    if let Some((rid_date, rid_tz)) = recurrence_id {
        let is_date = rid_date.len() <= 10;
        let rid_ical = if is_date {
            rid_date.replace('-', "")
        } else {
            if rid_tz.is_empty() || rid_tz == "UTC" {
                format!("{}Z", rid_date.replace('-', "").replace(':', "").replace('T', "T"))
            } else {
                format!("{}", rid_date.replace('-', "").replace(':', "").replace('T', "T"))
            }
        };
        if is_date && is_all_day {
            ical.push_str(&format!("RECURRENCE-ID;VALUE=DATE:{}\r\n", rid_ical));
        } else if is_date {
            // Timed override of an all-day master: use midnight to keep same type as DTSTART
            if rid_tz.is_empty() || rid_tz == "UTC" {
                ical.push_str(&format!("RECURRENCE-ID:{}T000000Z\r\n", rid_ical));
            } else {
                ical.push_str(&format!("RECURRENCE-ID;TZID={}:{}T000000\r\n", rid_tz, rid_ical));
            }
        } else if rid_tz.is_empty() || rid_tz == "UTC" {
            ical.push_str(&format!("RECURRENCE-ID:{}\r\n", rid_ical));
        } else {
            ical.push_str(&format!("RECURRENCE-ID;TZID={}:{}\r\n", rid_tz, rid_ical));
        }
    }

    ical.push_str(&format!("SUMMARY:{}\r\n", escape_ical_text(summary)));
    if !description.is_empty() {
        ical.push_str(&format!("DESCRIPTION:{}\r\n", escape_ical_text(description)));
    }
    if !location.is_empty() {
        ical.push_str(&format!("LOCATION:{}\r\n", escape_ical_text(location)));
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
        parts.push(format!("BYMONTHDAY={}", info.by_month_day.iter().map(|d| d.to_string()).collect::<Vec<_>>().join(",")));
    }
    if !info.by_month.is_empty() {
        parts.push(format!("BYMONTH={}", info.by_month.iter().map(|d| d.to_string()).collect::<Vec<_>>().join(",")));
    }
    if !info.by_set_pos.is_empty() {
        parts.push(format!("BYSETPOS={}", info.by_set_pos.iter().map(|d| d.to_string()).collect::<Vec<_>>().join(",")));
    }
    if let Some(count) = info.count {
        parts.push(format!("COUNT={}", count));
    }
    if let Some(ref until) = info.until {
        parts.push(format!("UNTIL={}", format_until(until, tz, is_all_day)));
    }
    parts.join(";")
}

fn format_until(until: &str, tz: &str, is_all_day: bool) -> String {
    if is_all_day {
        if let Ok(d) = NaiveDate::parse_from_str(until, "%Y-%m-%d") {
            d.format("%Y%m%d").to_string()
        } else {
            until.to_string()
        }
    } else if let Ok(dt) = NaiveDateTime::parse_from_str(until, "%Y-%m-%d %H:%M:%S") {
        if tz.is_empty() || tz == "UTC" {
            format!("{}T{}Z", dt.format("%Y%m%d"), dt.format("%H%M%S"))
        } else {
            format!("{}T{}", dt.format("%Y%m%d"), dt.format("%H%M%S"))
        }
    } else if let Ok(d) = NaiveDate::parse_from_str(until, "%Y-%m-%d") {
        if tz.is_empty() || tz == "UTC" {
            format!("{}T235959Z", d.format("%Y%m%d"))
        } else {
            format!("{}T235959", d.format("%Y%m%d"))
        }
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
    let rid_date: String = recurrence_id_value.replace('-', "").chars().take(8).collect();

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
            let sep = if override_vevent.ends_with('\n') { "" } else { "\n" };
            result = format!("{}{}{}{}", before, override_vevent, sep, after);
        }
    }

    result.replace('\n', "\r\n")
}
