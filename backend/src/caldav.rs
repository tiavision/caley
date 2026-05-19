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

#[derive(Serialize, Deserialize, Clone)]
pub struct Alarm {
    pub trigger_type: String,    // "start" | "end" | "absolute"
    pub minutes: Option<i64>,    // for relative: negative = before, positive = after
    pub datetime: Option<String>, // for absolute: UTC ISO 8601 (e.g. "20260514T090000Z")
}

#[derive(Serialize, Deserialize, Clone, Default, PartialEq)]
#[serde(rename_all = "SCREAMING-KEBAB-CASE")]
pub enum PartStat {
    #[default]
    NeedsAction,
    Accepted,
    Declined,
    Tentative,
}

impl PartStat {
    pub fn from_ical(s: &str) -> Self {
        match s {
            "ACCEPTED" => Self::Accepted,
            "DECLINED" => Self::Declined,
            "TENTATIVE" => Self::Tentative,
            _ => Self::NeedsAction,
        }
    }

    pub fn to_ical(&self) -> &'static str {
        match self {
            Self::NeedsAction => "NEEDS-ACTION",
            Self::Accepted => "ACCEPTED",
            Self::Declined => "DECLINED",
            Self::Tentative => "TENTATIVE",
        }
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Attendee {
    pub cn: String,
    pub email: String,
    pub partstat: PartStat,
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
    pub alarms: Vec<Alarm>,
    pub attendees: Vec<Attendee>,
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

pub struct SyncResult {
    pub new_token: String,
    pub changed: Vec<(String, String, String)>, // (href, etag, raw_ical)
    pub deleted: Vec<String>,
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
    alarms: Vec<Alarm>,
    attendees: Vec<Attendee>,
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

    pub async fn get_display_name(&self) -> anyhow::Result<Option<String>> {
        let start_url = self.start_url().await;
        let principal_body = r#"<?xml version="1.0" encoding="utf-8"?>
<propfind xmlns="DAV:">
  <prop>
    <current-user-principal/>
  </prop>
</propfind>"#;
        let xml = self.propfind(&start_url, "0", principal_body).await?;
        let principal_hrefs = extract_hrefs(&xml, "current-user-principal")?;
        if principal_hrefs.is_empty() {
            return Ok(None);
        }
        let principal_url = resolve_url(&start_url, &principal_hrefs[0]);
        let display_body = r#"<?xml version="1.0" encoding="utf-8"?>
<propfind xmlns="DAV:">
  <prop>
    <displayname/>
  </prop>
</propfind>"#;
        let display_xml = self.propfind(&principal_url, "0", display_body).await?;
        Ok(extract_text_prop(&display_xml, "displayname"))
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

    /// Expand raw iCalendar text into `Event` structs for a date range.
    /// Used by the sync worker to convert cached blobs without touching CalDAV.
    pub fn events_from_raw_ical(raw_ical: &str, start: NaiveDate, end: NaiveDate) -> Vec<Event> {
        expand_events(parse_raw_events(raw_ical), start, end)
    }

    /// Fetch all VEVENTs from a calendar without a time-range filter.
    /// Returns `(href, etag, raw_ical)` tuples for cache population.
    pub async fn fetch_all_raw(&self, calendar_href: &str) -> anyhow::Result<Vec<(String, String, String)>> {
        let body = r#"<?xml version="1.0" encoding="utf-8"?>
<c:calendar-query xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav">
  <d:prop>
    <d:getetag/>
    <c:calendar-data/>
  </d:prop>
  <c:filter>
    <c:comp-filter name="VCALENDAR">
      <c:comp-filter name="VEVENT"/>
    </c:comp-filter>
  </c:filter>
</c:calendar-query>"#;
        let xml = self.report(calendar_href, body).await?;
        extract_events_with_hrefs(&xml, calendar_href)
    }

    /// PROPFIND for `getctag` (Apple) and `sync-token` (RFC 6578).
    /// Returns `(ctag, sync_token)` — either or both may be `None`.
    pub async fn get_ctag_and_token(&self, calendar_href: &str) -> anyhow::Result<(Option<String>, Option<String>)> {
        let body = r#"<?xml version="1.0" encoding="utf-8"?>
<d:propfind xmlns:d="DAV:" xmlns:cs="http://calendarserver.org/ns/">
  <d:prop>
    <cs:getctag/>
    <d:sync-token/>
  </d:prop>
</d:propfind>"#;
        let xml = self.propfind(calendar_href, "0", body).await?;
        Ok(extract_ctag_and_token(&xml))
    }

    /// RFC 6578 sync-collection REPORT.
    /// Pass `sync_token = None` for the initial sync (returns everything).
    pub async fn sync_collection(&self, calendar_href: &str, sync_token: Option<&str>) -> anyhow::Result<SyncResult> {
        let token_val = sync_token.unwrap_or("");
        let body = format!(
            r#"<?xml version="1.0" encoding="utf-8"?>
<d:sync-collection xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav">
  <d:sync-token>{token_val}</d:sync-token>
  <d:sync-level>1</d:sync-level>
  <d:prop>
    <d:getetag/>
    <c:calendar-data/>
  </d:prop>
</d:sync-collection>"#
        );
        let xml = self.report(calendar_href, &body).await?;
        extract_sync_result(&xml, calendar_href)
    }

    /// Register a BitFire WebDAV Push subscription (https://bitfire.at/webdav-push).
    /// POSTs a push-register document to the calendar URL. The server will POST an encrypted
    /// Web Push notification to `callback_url` when the calendar changes.
    ///
    /// `p256dh` — base64url-encoded EC P-256 uncompressed public key (65 bytes)
    /// `auth`   — base64url-encoded 16-byte auth secret
    ///
    /// Returns the expiry Unix timestamp on success, `None` if push isn't supported.
    pub async fn register_push(
        &self,
        calendar_href: &str,
        callback_url: &str,
        p256dh: &str,
        auth: &str,
    ) -> anyhow::Result<Option<i64>> {
        let expires_at_default = chrono::Utc::now().timestamp() + 86400;
        // IMF-fixdate as required by the BitFire WebDAV Push spec
        let expires_str = chrono::DateTime::from_timestamp(expires_at_default, 0)
            .unwrap()
            .format("%a, %d %b %Y %H:%M:%S GMT")
            .to_string();

        let body = format!(
            r#"<?xml version="1.0" encoding="utf-8" ?>
<push-register xmlns="https://bitfire.at/webdav-push" xmlns:D="DAV:">
  <subscription>
    <web-push-subscription>
      <push-resource>{callback_url}</push-resource>
      <content-encoding>aes128gcm</content-encoding>
      <subscription-public-key type="p256dh">{p256dh}</subscription-public-key>
      <auth-secret>{auth}</auth-secret>
    </web-push-subscription>
  </subscription>
  <trigger>
    <content-update>
      <D:depth>infinity</D:depth>
    </content-update>
  </trigger>
  <expires>{expires_str}</expires>
</push-register>"#
        );

        let resp = self
            .client
            .post(calendar_href)
            .header(CONTENT_TYPE, "application/xml")
            .basic_auth(&self.username, Some(&self.password))
            .body(body)
            .send()
            .await
            .context("push-register POST failed")?;

        let status = resp.status();
        if status.as_u16() == 405 || status.as_u16() == 501 || status.as_u16() == 404 {
            return Ok(None);
        }
        if status.is_client_error() {
            let body_text = resp.text().await.unwrap_or_default();
            bail!("push-register rejected ({status}): {body_text}");
        }
        if !status.is_success() && status.as_u16() != 201 {
            bail!("push-register returned {status}");
        }

        let xml = resp.text().await.unwrap_or_default();
        let expires_at = extract_push_expiry(&xml).unwrap_or(expires_at_default);
        Ok(Some(expires_at))
    }

    pub async fn create_calendar(&self, name: &str, color: Option<&str>) -> anyhow::Result<String> {
        let start = self.start_url().await;
        let home_sets = self.discover_calendar_home_sets(&start).await?;
        let home = home_sets
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("No calendar-home-set found"))?;

        let raw_slug: String = name
            .to_lowercase()
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { '-' })
            .collect();
        let raw_slug = raw_slug.trim_matches('-').to_string();
        let suffix = &Uuid::new_v4().to_string()[..8];
        let slug = if raw_slug.is_empty() {
            suffix.to_string()
        } else {
            format!("{raw_slug}-{suffix}")
        };

        let calendar_url = format!("{}/{}/", home.trim_end_matches('/'), slug);

        let color_prop = color
            .map(|c| format!("      <APPLE:calendar-color>{c}</APPLE:calendar-color>\n"))
            .unwrap_or_default();

        let escaped_name = name.replace('<', "&lt;").replace('>', "&gt;").replace('&', "&amp;");
        let body = format!(
            r#"<?xml version="1.0" encoding="utf-8"?>
<C:mkcalendar xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav"
              xmlns:APPLE="http://apple.com/ns/ical/">
  <D:set>
    <D:prop>
      <D:displayname>{escaped_name}</D:displayname>
{color_prop}      <C:supported-calendar-component-set>
        <C:comp name="VEVENT"/>
      </C:supported-calendar-component-set>
    </D:prop>
  </D:set>
</C:mkcalendar>"#
        );

        let method = reqwest::Method::from_bytes(b"MKCALENDAR")
            .context("invalid method MKCALENDAR")?;
        let resp = self
            .client
            .request(method, &calendar_url)
            .header(CONTENT_TYPE, "application/xml; charset=utf-8")
            .basic_auth(&self.username, Some(&self.password))
            .body(body)
            .send()
            .await
            .context("MKCALENDAR request failed")?;

        match resp.status().as_u16() {
            200 | 201 => Ok(calendar_url),
            403 => bail!("Calendar creation forbidden — server does not allow MKCALENDAR"),
            405 => bail!("Calendar creation not supported by this server (405)"),
            s => {
                let text = resp.text().await.unwrap_or_default();
                bail!("MKCALENDAR returned {s}: {text}")
            }
        }
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

fn extract_text_prop(xml: &str, prop_local_name: &str) -> Option<String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut in_prop = false;
    let mut text = String::new();
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf).ok()? {
            XmlEvent::Start(e) => {
                let local = std::str::from_utf8(e.local_name().as_ref()).ok()?.to_string();
                if local == prop_local_name {
                    in_prop = true;
                    text.clear();
                }
            }
            XmlEvent::Text(e) => {
                if in_prop {
                    if let Ok(s) = e.unescape() {
                        text.push_str(&s);
                    }
                }
            }
            XmlEvent::End(e) => {
                let local = std::str::from_utf8(e.local_name().as_ref()).ok()?.to_string();
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
                "ATTENDEE" => {
                    let raw = property.value.as_deref().unwrap_or("");
                    let email = raw.strip_prefix("mailto:").unwrap_or(raw).to_string();
                    if !email.is_empty() {
                        let mut cn = String::new();
                        let mut partstat = PartStat::default();
                        if let Some(params) = &property.params {
                            for (key, vals) in params {
                                match key.as_str() {
                                    "CN" => cn = vals.first().cloned().unwrap_or_default(),
                                    "PARTSTAT" => partstat = PartStat::from_ical(vals.first().map(|s| s.as_str()).unwrap_or("")),
                                    _ => {}
                                }
                            }
                        }
                        attendees.push(Attendee { cn, email, partstat });
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

        let mut alarms: Vec<Alarm> = Vec::new();
        for alarm in &event.alarms {
            let Some(trigger) = alarm.properties.iter().find(|p| p.name == "TRIGGER") else { continue };
            let Some(ref value) = trigger.value else { continue };

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
                alarms.push(Alarm { trigger_type: "absolute".to_string(), minutes: None, datetime: Some(value.clone()) });
            } else if let Some(mins) = parse_relative_trigger(value) {
                alarms.push(Alarm {
                    trigger_type: if is_end_relative { "end".to_string() } else { "start".to_string() },
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
            duration,
            rrule,
            rdates,
            exdates,
            recurrence_id,
            recurrence_id_tzid,
            alarms,
            attendees,
        });
    }

    results
}

fn parse_relative_trigger(s: &str) -> Option<i64> {
    let negative = s.starts_with('-');
    let s = s.trim_start_matches(['+', '-']);
    let s = s.strip_prefix('P')?;
    let minutes = if let Some(rest) = s.strip_prefix('T') {
        if let Some(h) = rest.strip_suffix('H') { h.parse::<i64>().ok()? * 60 }
        else if let Some(m) = rest.strip_suffix('M') { m.parse::<i64>().ok()? }
        else if let Some(sec) = rest.strip_suffix('S') { sec.parse::<i64>().ok()? / 60 }
        else { return None; }
    } else if let Some(d) = s.strip_suffix('D') {
        d.parse::<i64>().ok()? * 1440
    } else { return None; };
    Some(if negative { -minutes } else { minutes })
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
                        Some(dt + master.duration),
                        master.dtstart_is_date,
                        master.dtstart_tzid.as_deref().unwrap_or("UTC"),
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
                Some(master.dtstart + master.duration),
                master.dtstart_is_date,
                master.dtstart_tzid.as_deref().unwrap_or("UTC"),
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

/// Split a multi-VEVENT VCALENDAR blob into individual per-event (uid, vcalendar) pairs.
/// VTIMEZONE blocks and other VCALENDAR-level properties are preserved in each wrapper.
pub fn split_vcalendar_into_events(ical_content: &str) -> Vec<(String, String)> {
    let normalised = ical_content.replace("\r\n", "\n");
    let mut header_lines: Vec<&str> = Vec::new();
    let mut vevent_blocks: Vec<String> = Vec::new();
    let mut in_vevent = false;
    let mut current_vevent: Vec<&str> = Vec::new();

    for line in normalised.lines() {
        if line == "BEGIN:VEVENT" {
            in_vevent = true;
            current_vevent.clear();
            current_vevent.push(line);
        } else if line == "END:VEVENT" {
            current_vevent.push(line);
            vevent_blocks.push(current_vevent.join("\r\n"));
            current_vevent.clear();
            in_vevent = false;
        } else if in_vevent {
            current_vevent.push(line);
        } else if line != "BEGIN:VCALENDAR"
            && line != "END:VCALENDAR"
            && !line.starts_with("VERSION:")
            && !line.starts_with("PRODID:")
        {
            header_lines.push(line);
        }
    }

    let header = header_lines.join("\r\n");
    vevent_blocks
        .into_iter()
        .map(|vevent_block| {
            let uid = vevent_block
                .lines()
                .find(|l| l.starts_with("UID:"))
                .map(|l| l[4..].trim().to_string())
                .filter(|u| !u.is_empty())
                .unwrap_or_else(generate_new_uid);

            let header_section = if header.is_empty() {
                String::new()
            } else {
                format!("{}\r\n", header)
            };
            let wrapped = format!(
                "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Caley//Caley//EN\r\n{header_section}{vevent_block}\r\nEND:VCALENDAR\r\n"
            );
            (uid, wrapped)
        })
        .collect()
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
                    ical.push_str(&format!("TRIGGER;RELATED=END:{}PT{}M\r\n", sign, mins.unsigned_abs()));
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

fn extract_events_with_hrefs(xml: &str, base_url: &str) -> anyhow::Result<Vec<(String, String, String)>> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

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
                let local = std::str::from_utf8(lname.as_ref())?;
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
                let text = e.unescape()?;
                if in_calendar_data { current_ical.push_str(&text); }
                else if in_href { current_href.push_str(&text); }
                else if in_etag { current_etag.push_str(&text); }
            }
            XmlEvent::End(e) => {
                let lname = e.local_name();
                let local = std::str::from_utf8(lname.as_ref())?;
                match local {
                    "href" => in_href = false,
                    "getetag" => in_etag = false,
                    "calendar-data" => in_calendar_data = false,
                    "response" => {
                        if !current_href.is_empty() && !current_ical.is_empty() {
                            results.push((
                                resolve_url(base_url, current_href.trim()),
                                current_etag.trim().trim_matches('"').to_string(),
                                current_ical.clone(),
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

fn extract_sync_result(xml: &str, base_url: &str) -> anyhow::Result<SyncResult> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

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
                let local = std::str::from_utf8(lname.as_ref())?;
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
                let text = e.unescape()?;
                if in_sync_token_top { new_token.push_str(&text); }
                else if in_calendar_data { current_ical.push_str(&text); }
                else if in_href { current_href.push_str(&text); }
                else if in_etag { current_etag.push_str(&text); }
                else if in_status { current_status.push_str(&text); }
            }
            XmlEvent::End(e) => {
                let lname = e.local_name();
                let local = std::str::from_utf8(lname.as_ref())?;
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
                                current_ical.clone(),
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
    Ok(SyncResult { new_token, changed, deleted })
}

fn extract_ctag_and_token(xml: &str) -> (Option<String>, Option<String>) {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut ctag: Option<String> = None;
    let mut sync_token: Option<String> = None;
    let mut in_ctag = false;
    let mut in_token = false;
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(XmlEvent::Start(e)) => {
                let lname = e.local_name();
                let local = std::str::from_utf8(lname.as_ref()).unwrap_or("");
                match local {
                    "getctag" => in_ctag = true,
                    "sync-token" => in_token = true,
                    _ => {}
                }
            }
            Ok(XmlEvent::Text(e)) => {
                if let Ok(text) = e.unescape() {
                    let text = text.trim().to_string();
                    if !text.is_empty() {
                        if in_ctag { ctag = Some(text); }
                        else if in_token { sync_token = Some(text); }
                    }
                }
            }
            Ok(XmlEvent::End(e)) => {
                let lname = e.local_name();
                let local = std::str::from_utf8(lname.as_ref()).unwrap_or("");
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
/// Looks for `<expires>` text (ISO-8601 duration like "PT30M" or "PT1H", or a datetime).
/// Returns a Unix timestamp, or None if the response body cannot be parsed.
fn extract_push_expiry(xml: &str) -> Option<i64> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut in_expires = false;
    let mut expires_text = String::new();
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(XmlEvent::Start(e)) => {
                let lname = e.local_name();
                let local = std::str::from_utf8(lname.as_ref()).unwrap_or("");
                if local == "expires" {
                    in_expires = true;
                    expires_text.clear();
                }
            }
            Ok(XmlEvent::Text(e)) => {
                if in_expires {
                    if let Ok(t) = e.unescape() {
                        expires_text.push_str(t.as_ref());
                    }
                }
            }
            Ok(XmlEvent::End(e)) => {
                let lname = e.local_name();
                let local = std::str::from_utf8(lname.as_ref()).unwrap_or("");
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

    let now = chrono::Utc::now().timestamp();

    // ISO-8601 duration: "PT30M", "PT1H", "P1D", etc.
    if text.starts_with('P') {
        let seconds = parse_iso_duration_secs(text)?;
        return Some(now + seconds);
    }

    // Absolute datetime: try RFC 3339 / ISO 8601
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(text) {
        return Some(dt.timestamp());
    }

    // IMF-fixdate: "Sat, 17 May 2026 11:00:00 GMT" (used by BitFire/Rustical)
    if let Ok(dt) = chrono::DateTime::parse_from_str(text, "%a, %d %b %Y %H:%M:%S GMT") {
        return Some(dt.timestamp());
    }

    None
}

fn parse_iso_duration_secs(s: &str) -> Option<i64> {
    // Handles simple cases: PT30M, PT1H, P1D, PT1H30M, etc.
    let s = s.strip_prefix('P')?;
    let (date_part, time_part) = if let Some(t) = s.split_once('T') {
        (t.0, t.1)
    } else {
        (s, "")
    };

    let mut total: i64 = 0;

    // Parse date part: D
    let mut rest = date_part;
    if let Some(pos) = rest.find('D') {
        let days: i64 = rest[..pos].parse().ok()?;
        total += days * 86400;
        rest = &rest[pos + 1..];
    }
    let _ = rest;

    // Parse time part: H, M, S
    let mut rest = time_part;
    if let Some(pos) = rest.find('H') {
        let hours: i64 = rest[..pos].parse().ok()?;
        total += hours * 3600;
        rest = &rest[pos + 1..];
    }
    if let Some(pos) = rest.find('M') {
        let mins: i64 = rest[..pos].parse().ok()?;
        total += mins * 60;
        rest = &rest[pos + 1..];
    }
    if let Some(pos) = rest.find('S') {
        let secs: i64 = rest[..pos].parse().ok()?;
        total += secs;
    }

    if total > 0 { Some(total) } else { None }
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

/// Add EXDATE for one occurrence to the master VEVENT and drop any existing
/// override VEVENT for the same date.
/// `occurrence_dtstart` is in display format: "YYYY-MM-DD" for all-day or
/// "YYYY-MM-DDTHH:MM:SS..." for timed (the `data-dtstart` attribute value).
/// `tz` is the TZID name (e.g. "UTC", "Europe/Berlin").
pub fn add_exdate(ical_data: &str, occurrence_dtstart: &str, tz: &str) -> String {
    let ical = ical_data.replace("\r\n", "\n");
    let is_all_day = !occurrence_dtstart.contains('T');

    // Build the EXDATE line to insert into the master VEVENT.
    let exdate_line = if is_all_day {
        let date_compact = occurrence_dtstart.replace('-', "");
        format!("EXDATE;VALUE=DATE:{}", date_compact)
    } else {
        let parts: Vec<&str> = occurrence_dtstart.splitn(2, 'T').collect();
        let date_compact = parts[0].replace('-', "");
        let time_str = parts.get(1).copied().unwrap_or("000000Z");
        // Keep only the digit portion of the time component.
        let time_compact: String = time_str.chars().take_while(|c| c.is_ascii_digit()).collect();
        if occurrence_dtstart.ends_with('Z') || tz.is_empty() || tz == "UTC" {
            format!("EXDATE:{}T{}Z", date_compact, time_compact)
        } else {
            format!("EXDATE;TZID={}:{}T{}", tz, date_compact, time_compact)
        }
    };

    // 8-digit YYYYMMDD for matching existing override VEVENTs.
    let rid_date: String = occurrence_dtstart.replace('-', "").chars().take(8).collect();

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
                // Override VEVENT: drop if it matches our target date, keep otherwise.
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
                // Master VEVENT: inject EXDATE before END:VEVENT.
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

// ── Non-destructive event update ──────────────────────────────────────────────

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
        // First chunk: 75 bytes (walk back to UTF-8 boundary).
        let mut pos = 75;
        while !line.is_char_boundary(pos) {
            pos -= 1;
        }
        out.push_str(&line[..pos]);
        out.push_str("\r\n");
        let mut rest = &line[pos..];
        // Continuation chunks: 74 bytes of content + leading space.
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
/// Everything else (ORGANIZER, CLASS, STATUS, TRANSP, CATEGORIES, ATTACH,
/// RELATED-TO, RDATE, EXDATE, X-*, CREATED, …) is preserved from the original.
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
/// If PARTSTAT is absent, it is appended before the `:mailto:` value.
fn update_attendee_partstat(line: &str, new_partstat: &str) -> String {
    let Some(value_start) = line.rfind(":mailto:") else {
        return line.to_string();
    };
    let params = &line[..value_start];
    let value = &line[value_start..];
    let new_params = if let Some(ps) = params.find("PARTSTAT=") {
        let after = &params[ps + 9..];
        let end = after.find(';').unwrap_or(after.len());
        format!("{}PARTSTAT={}{}", &params[..ps], new_partstat, &after[end..])
    } else {
        format!("{};PARTSTAT={}", params, new_partstat)
    };
    format!("{}{}", new_params, value)
}

/// Rebuild one VEVENT block (already unfolded, LF-separated), replacing the
/// properties in REPLACE_PROPS and all VALARM sub-components with fresh values,
/// while preserving every other property verbatim.
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

    // Collect preserved lines, original ATTENDEE lines, and non-DISPLAY VALARM blocks.
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
            let is_display = current_valarm.iter()
                .any(|l| l.trim().eq_ignore_ascii_case("ACTION:DISPLAY"));
            if !is_display {
                let mut blk = String::from("BEGIN:VALARM\n");
                for vl in &current_valarm { blk.push_str(vl); blk.push('\n'); }
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
            // Collect separately for smart merge below.
            original_attendee_lines.push(line.to_string());
            continue;
        }
        if REPLACE_PROPS.contains(&pname.as_str()) {
            continue;
        }
        kept.push_str(line);
        kept.push('\n');
    }

    // Build the new VEVENT.
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
            out.push_str(&format!("RRULE:{}\n", recurrence_info_to_rrule(rec, tz, is_all_day)));
        }
    }

    if let Some((rid_date, rid_tz)) = target_recurrence_id {
        let is_date = rid_date.len() <= 10;
        let rid_ical = if is_date {
            rid_date.replace('-', "")
        } else if rid_tz.is_empty() || rid_tz == "UTC" {
            format!("{}Z", rid_date.replace('-', "").replace(':', ""))
        } else {
            rid_date.replace('-', "").replace(':', "")
        };
        if is_date && is_all_day {
            out.push_str(&format!("RECURRENCE-ID;VALUE=DATE:{}\n", rid_ical));
        } else if is_date {
            if rid_tz.is_empty() || rid_tz == "UTC" {
                out.push_str(&format!("RECURRENCE-ID:{}T000000Z\n", rid_ical));
            } else {
                out.push_str(&format!("RECURRENCE-ID;TZID={}:{}T000000\n", rid_tz, rid_ical));
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
            out.push_str(&update_attendee_partstat(orig_line, &att.partstat.to_ical()));
        } else {
            let cn = if !att.cn.is_empty() { format!(";CN={}", att.cn) } else { String::new() };
            out.push_str(&format!("ATTENDEE{};PARTSTAT={}:mailto:{}", cn, att.partstat.to_ical(), att.email));
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
                    out.push_str(&format!("TRIGGER;RELATED=END:{}PT{}M\n", sign, mins.unsigned_abs()));
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
/// Replaces only the properties the user can edit (dates, summary, recurrence,
/// attendees, alarms) while preserving everything else (ORGANIZER, CLASS,
/// STATUS, TRANSP, CATEGORIES, ATTACH, X-* extensions, EXDATE, RDATE, …).
///
/// `target_recurrence_id`: None = update the master VEVENT (series edit);
///   Some((rid_value, tz)) = update the override VEVENT for that occurrence.
///
/// Returns None if the target VEVENT is not found (caller should fall back to
/// generate_ical_event).
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

    let rid_date_prefix: Option<String> = target_recurrence_id
        .map(|(rv, _)| rv.replace('-', "").chars().take(8).collect());

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

        // Determine if this is the target VEVENT.
        let is_target = if !found_target {
            match &rid_date_prefix {
                None => {
                    // Master VEVENT: no RECURRENCE-ID line present.
                    !block.lines().any(|l| {
                        l.starts_with("RECURRENCE-ID:") || l.starts_with("RECURRENCE-ID;")
                    })
                }
                Some(rid_prefix) => {
                    // Override VEVENT: RECURRENCE-ID value starts with this date.
                    block.lines().any(|l| {
                        if !l.starts_with("RECURRENCE-ID:") && !l.starts_with("RECURRENCE-ID;") {
                            return false;
                        }
                        l.rsplit(':').next().map_or(false, |val| {
                            val.chars().take(8).collect::<String>() == *rid_prefix
                        })
                    })
                }
            }
        } else {
            false
        };

        if is_target {
            // Read SEQUENCE from original for incrementing.
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
