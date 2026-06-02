use anyhow::{Context, bail};
use chrono::Utc;
use reqwest::Client;
use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};
use uuid::Uuid;

use crate::version;

pub(crate) mod generation;
pub(crate) mod parsing;
pub mod types;

pub use generation::{
    add_exdate, add_recurrence_override, generate_ical_event, generate_new_uid,
    split_vcalendar_into_events, update_vcalendar_event,
};
pub use parsing::preview_events_from_raw_ical;
pub use types::{Alarm, Attendee, Calendar, Event, PartStat, RecurrenceInfo, SyncResult};
pub(crate) use parsing::{
    extract_ctag_and_token, extract_hrefs, resolve_url, unescape_ical_text,
};

#[derive(Clone)]
pub struct CalDavClient {
    client: Client,
    base_url: String,
    username: String,
    password: String,
    pub demo_mode: bool,
}

impl CalDavClient {
    pub fn new(base_url: String, username: String, password: String) -> anyhow::Result<Self> {
        let client = Client::builder()
            .user_agent(version::USER_AGENT)
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        Ok(Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
            username,
            password,
            demo_mode: false,
        })
    }

    pub fn demo() -> Self {
        Self {
            client: Client::new(),
            base_url: String::new(),
            username: String::new(),
            password: String::new(),
            demo_mode: true,
        }
    }

    pub(crate) fn base_url(&self) -> &str {
        &self.base_url
    }
    pub(crate) fn username(&self) -> &str {
        &self.username
    }
    pub(crate) fn password(&self) -> &str {
        &self.password
    }
    pub(crate) fn http_client(&self) -> &Client {
        &self.client
    }

    pub async fn calendars(&self) -> anyhow::Result<Vec<Calendar>> {
        if self.demo_mode {
            return Ok(vec![]);
        }
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
        if self.demo_mode {
            return Ok(Some("Demo User".to_string()));
        }
        let start_url = self.start_url().await;
        let principal_body = r#"<?xml version="1.0" encoding="utf-8"?>
<propfind xmlns="DAV:">
  <prop>
    <current-user-principal/>
  </prop>
</propfind>"#;
        let xml = self.propfind(&start_url, "0", principal_body).await?;
        let principal_hrefs = parsing::extract_hrefs(&xml, "current-user-principal")?;
        if principal_hrefs.is_empty() {
            return Ok(None);
        }
        let principal_url = parsing::resolve_url(&start_url, &principal_hrefs[0]);
        let display_body = r#"<?xml version="1.0" encoding="utf-8"?>
<propfind xmlns="DAV:">
  <prop>
    <displayname/>
  </prop>
</propfind>"#;
        let display_xml = self.propfind(&principal_url, "0", display_body).await?;
        Ok(parsing::extract_text_prop(&display_xml, "displayname"))
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
                return Some(
                    parsing::resolve_url(&well_known, &location)
                        .trim_end_matches('/')
                        .to_string(),
                );
            }
        }

        None
    }

    async fn discover_calendar_home_sets(&self, start_url: &str) -> anyhow::Result<Vec<String>> {
        let principal_body = r#"<?xml version="1.0" encoding="utf-8"?>
<propfind xmlns="DAV:">
  <prop>
    <current-user-principal/>
  </prop>
</propfind>"#;

        let xml = self.propfind(start_url, "0", principal_body).await?;

        match parsing::extract_hrefs(&xml, "current-user-principal") {
            Ok(hrefs) if !hrefs.is_empty() => {
                let principal_href = &hrefs[0];
                let principal_url = parsing::resolve_url(start_url, principal_href);

                let home_body = r#"<?xml version="1.0" encoding="utf-8"?>
<propfind xmlns="DAV:">
  <prop>
    <calendar-home-set xmlns="urn:ietf:params:xml:ns:caldav"/>
  </prop>
</propfind>"#;

                let home_xml = self.propfind(&principal_url, "0", home_body).await?;
                let hrefs = parsing::extract_hrefs(&home_xml, "calendar-home-set")
                    .with_context(|| {
                        format!(
                            "calendar-home-set not found in response from {principal_url}"
                        )
                    })?;
                Ok(hrefs
                    .into_iter()
                    .map(|h| parsing::resolve_url(&principal_url, &h))
                    .collect())
            }
            _ => {
                let home_body = r#"<?xml version="1.0" encoding="utf-8"?>
<propfind xmlns="DAV:">
  <prop>
    <calendar-home-set xmlns="urn:ietf:params:xml:ns:caldav"/>
  </prop>
</propfind>"#;

                let home_xml = self.propfind(start_url, "0", home_body).await?;
                let hrefs = parsing::extract_hrefs(&home_xml, "calendar-home-set")
                    .with_context(|| {
                        format!("calendar-home-set not found in response from {start_url}")
                    })?;
                Ok(hrefs
                    .into_iter()
                    .map(|h| parsing::resolve_url(start_url, &h))
                    .collect())
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
        parsing::parse_all_calendars(&xml, home_set_url)
    }

    pub(crate) async fn propfind(
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

            let text = resp
                .text()
                .await
                .context("failed to read PROPFIND response")?;
            return Ok(text);
        }
        bail!("too many redirects for PROPFIND")
    }

    pub(crate) async fn report(&self, url: &str, body: &str) -> anyhow::Result<String> {
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

            let text = resp
                .text()
                .await
                .context("failed to read REPORT response")?;
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
        if self.demo_mode {
            return Ok(());
        }
        let url = format!("{}/{}.ics", calendar_href.trim_end_matches('/'), uid);
        let mut url = url;
        for _ in 0..5 {
            let mut headers = HeaderMap::new();
            headers.insert(
                CONTENT_TYPE,
                HeaderValue::from_static("text/calendar; charset=utf-8"),
            );

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
        if self.demo_mode {
            return Ok(());
        }
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
    pub fn events_from_raw_ical(raw_ical: &str, start: chrono::NaiveDate, end: chrono::NaiveDate) -> Vec<Event> {
        parsing::parse_and_expand_events(raw_ical, start, end)
    }

    /// Fetch all VEVENTs from a calendar without a time-range filter.
    pub async fn fetch_all_raw(
        &self,
        calendar_href: &str,
    ) -> anyhow::Result<Vec<(String, String, String)>> {
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
        parsing::extract_events_with_hrefs(&xml, calendar_href)
    }

    /// PROPFIND for `getctag` (Apple) and `sync-token` (RFC 6578).
    pub async fn get_ctag_and_token(
        &self,
        calendar_href: &str,
    ) -> anyhow::Result<(Option<String>, Option<String>)> {
        let body = r#"<?xml version="1.0" encoding="utf-8"?>
<d:propfind xmlns:d="DAV:" xmlns:cs="http://calendarserver.org/ns/">
  <d:prop>
    <cs:getctag/>
    <d:sync-token/>
  </d:prop>
</d:propfind>"#;
        let xml = self.propfind(calendar_href, "0", body).await?;
        Ok(parsing::extract_ctag_and_token(&xml))
    }

    /// RFC 6578 sync-collection REPORT.
    pub async fn sync_collection(
        &self,
        calendar_href: &str,
        sync_token: Option<&str>,
    ) -> anyhow::Result<SyncResult> {
        if self.demo_mode {
            return Ok(SyncResult {
                new_token: "demo-token".into(),
                changed: vec![],
                deleted: vec![],
            });
        }
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
        parsing::extract_sync_result(&xml, calendar_href)
    }

    /// Register a BitFire WebDAV Push subscription.
    pub async fn register_push(
        &self,
        calendar_href: &str,
        callback_url: &str,
        p256dh: &str,
        auth: &str,
    ) -> anyhow::Result<Option<i64>> {
        if self.demo_mode {
            return Ok(None);
        }
        let expires_at_default = Utc::now().timestamp() + 86400;
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
        let expires_at = parsing::extract_push_expiry(&xml).unwrap_or(expires_at_default);
        Ok(Some(expires_at))
    }

    pub async fn create_calendar(&self, name: &str, color: Option<&str>) -> anyhow::Result<String> {
        if self.demo_mode {
            anyhow::bail!("Cannot create calendars in demo mode");
        }
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

        let escaped_name = name
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('&', "&amp;");
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

        let method =
            reqwest::Method::from_bytes(b"MKCALENDAR").context("invalid method MKCALENDAR")?;
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
