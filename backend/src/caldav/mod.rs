use anyhow::{Context, bail};
use chrono::Utc;
use std::sync::Arc;

use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue, LOCATION};
use reqwest::{Client, Method};
use tokio::sync::OnceCell;
use url::Url;
use uuid::Uuid;

use crate::version;

pub(crate) mod generation;
pub(crate) mod parsing;
pub(crate) mod pin;
pub mod types;
pub(crate) mod windows_tz;

#[cfg(test)]
mod limits_tests;
#[cfg(test)]
mod pin_tests;

pub use generation::{
    add_exdate, add_recurrence_override, generate_ical_event, generate_new_uid,
    split_vcalendar_into_events, update_vcalendar_event,
};
pub use parsing::preview_events_from_raw_ical;
pub(crate) use parsing::{extract_ctag_and_token, extract_hrefs, unescape_ical_text};

pub use types::{Alarm, Attendee, Calendar, Event, PartStat, RecurrenceInfo, SyncResult};

/// The resource name (one path segment, without extension) Caley gives a new calendar object or
/// contact: its UID when that is a conservative name (letters, digits, `-`, `_`, `@`, `.` not first, at
/// most 128), else the SHA-256 of the UID in hex. A UID from a file or an invite cannot name another
/// path (`..`, `/`, `?`, `#`, `%`); the same UID always names the same resource. The UID inside the
/// object stays as it is.
pub fn resource_name(uid: &str) -> String {
    let plain = !uid.is_empty()
        && uid.len() <= 128
        && !uid.starts_with('.')
        && uid
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'@' | b'.'));
    if plain {
        uid.to_owned()
    } else {
        use sha2::{Digest, Sha256};
        Sha256::digest(uid.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }
}

/// Where a new calendar object with `uid` goes in `calendar_href` (an existing one keeps the href the
/// server gave it: `cached_events.href`).
pub fn new_event_href(calendar_href: &str, uid: &str) -> String {
    format!(
        "{}/{}.ics",
        calendar_href.trim_end_matches('/'),
        resource_name(uid)
    )
}

/// URL path segment for a new calendar: the lowercased name with non-alphanumerics
/// replaced by `-`, plus a random suffix so repeated names don't collide.
fn calendar_slug(name: &str) -> String {
    let raw_slug: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    let raw_slug = raw_slug.trim_matches('-').to_string();
    let suffix = &Uuid::new_v4().to_string()[..8];
    if raw_slug.is_empty() {
        suffix.to_string()
    } else {
        format!("{raw_slug}-{suffix}")
    }
}

#[derive(Clone)]
pub struct CalDavClient {
    client: Client,
    base_url: String,
    username: String,
    password: String,
    /// Where the `.well-known/caldav` and `.well-known/carddav` discovery led (`pin::well_known_target`),
    /// asked once per client: the servers besides the account URL's that credentials may go to.
    well_known: Arc<[OnceCell<Option<Url>>; 2]>,
    pub demo_mode: bool,
}

/// The `.well-known` services a DAV account is discovered by.
#[derive(Clone, Copy)]
pub(crate) enum Service {
    CalDav,
    CardDav,
}

impl Service {
    fn name(self) -> &'static str {
        match self {
            Service::CalDav => "caldav",
            Service::CardDav => "carddav",
        }
    }
}

impl CalDavClient {
    pub fn new(base_url: String, username: String, password: String) -> anyhow::Result<Self> {
        let client = Client::builder()
            .user_agent(version::USER_AGENT)
            .use_preconfigured_tls(crate::imap_client::tls_config())
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        Ok(Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
            username,
            password,
            well_known: Arc::default(),
            demo_mode: false,
        })
    }

    pub fn demo() -> Self {
        Self {
            client: Client::new(),
            base_url: String::new(),
            username: String::new(),
            password: String::new(),
            well_known: Arc::default(),
            demo_mode: true,
        }
    }

    /// Where `service`'s `.well-known` URL led on the account's server (`pin::well_known_target`), asked
    /// once; `None` when it does not redirect (or not to a server credentials may go to).
    async fn well_known_target(&self, service: Service) -> Option<Url> {
        self.well_known[service as usize]
            .get_or_init(|| async {
                let well_known =
                    Url::parse(&format!("{}/.well-known/{}", self.base_url, service.name()))
                        .ok()?;
                let resp = self
                    .client
                    .get(well_known.clone())
                    .basic_auth(&self.username, Some(&self.password))
                    .send()
                    .await
                    .ok()?;
                if !resp.status().is_redirection() {
                    return None;
                }
                let location = resp.headers().get(LOCATION)?.to_str().ok()?;
                pin::well_known_target(&well_known, location)
            })
            .await
            .clone()
    }

    /// Where discovery of `service` starts: where its `.well-known` URL led, else the account URL.
    pub(crate) async fn start_url_for(&self, service: Service) -> String {
        match self.well_known_target(service).await {
            Some(url) => url.as_str().trim_end_matches('/').to_string(),
            None => self.base_url.clone(),
        }
    }

    /// The servers a request to `url` may go to: the account URL's, and where the `.well-known`
    /// discovery led when `url` is not on the account URL's (asked then, once).
    async fn servers_for(&self, url: &Url) -> Vec<Url> {
        let mut servers: Vec<Url> = Url::parse(&self.base_url).into_iter().collect();
        if !servers.iter().any(|s| pin::same_server(s, url)) {
            for service in [Service::CalDav, Service::CardDav] {
                servers.extend(self.well_known_target(service).await);
            }
        }
        servers
    }

    /// Sends `method` to `url` with the account's credentials, following redirects (at most 5) on the
    /// account's servers only (`pin`): a URL or a redirect elsewhere is an error, and nothing (no
    /// credentials) is sent there.
    pub(crate) async fn send(
        &self,
        method: Method,
        url: &str,
        headers: HeaderMap,
        body: Option<&str>,
        what: &str,
    ) -> anyhow::Result<reqwest::Response> {
        let mut url = Url::parse(url).with_context(|| format!("{what}: invalid URL {url:?}"))?;
        let servers = self.servers_for(&url).await;
        if !servers.iter().any(|s| pin::same_server(s, &url)) {
            bail!(
                "{what} to {}, which is not the account's server: refused",
                url.host_str().unwrap_or("?")
            );
        }
        for _ in 0..5 {
            let mut request = self
                .client
                .request(method.clone(), url.clone())
                .headers(headers.clone())
                .basic_auth(&self.username, Some(&self.password));
            if let Some(body) = body {
                request = request.body(body.to_owned());
            }
            let resp = request
                .send()
                .await
                .with_context(|| format!("{what} request failed"))?;
            if !resp.status().is_redirection() {
                return Ok(resp);
            }
            let Some(location) = resp.headers().get(LOCATION) else {
                bail!("{what} redirect without Location header");
            };
            let location = location.to_str()?.to_owned();
            let servers = match url.join(&location) {
                Ok(next) => self.servers_for(&next).await,
                Err(_) => servers.clone(),
            };
            url = pin::redirect_target(&url, &location, &servers, what)?;
        }
        bail!("too many redirects for {what}")
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
        let Some(principal_url) = principal_hrefs
            .first()
            .and_then(|h| pin::resolve_href(&start_url, h))
        else {
            return Ok(None);
        };
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
        self.start_url_for(Service::CalDav).await
    }

    async fn discover_calendar_home_sets(&self, start_url: &str) -> anyhow::Result<Vec<String>> {
        let principal_body = r#"<?xml version="1.0" encoding="utf-8"?>
<propfind xmlns="DAV:">
  <prop>
    <current-user-principal/>
  </prop>
</propfind>"#;

        let xml = self.propfind(start_url, "0", principal_body).await?;

        let principal_url = match parsing::extract_hrefs(&xml, "current-user-principal") {
            Ok(hrefs) if !hrefs.is_empty() => pin::resolve_href(start_url, &hrefs[0])
                .context("the current-user-principal is not on the account's server")?,
            _ => start_url.to_owned(),
        };
        let home_body = r#"<?xml version="1.0" encoding="utf-8"?>
<propfind xmlns="DAV:">
  <prop>
    <calendar-home-set xmlns="urn:ietf:params:xml:ns:caldav"/>
  </prop>
</propfind>"#;

        let home_xml = self.propfind(&principal_url, "0", home_body).await?;
        let hrefs = parsing::extract_hrefs(&home_xml, "calendar-home-set").with_context(|| {
            format!("calendar-home-set not found in response from {principal_url}")
        })?;
        let home_sets: Vec<String> = hrefs
            .iter()
            .filter_map(|h| pin::resolve_href(&principal_url, h))
            .collect();
        if home_sets.is_empty() {
            bail!("no calendar-home-set on the account's server");
        }
        Ok(home_sets)
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
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("depth"),
            HeaderValue::from_str(depth)?,
        );
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/xml"));
        let resp = self
            .send(
                Method::from_bytes(b"PROPFIND")?,
                url,
                headers,
                Some(body),
                "PROPFIND",
            )
            .await?;
        let status = resp.status();
        if !status.is_success() && status.as_u16() != 207 {
            bail!("PROPFIND returned {status}");
        }
        resp.text()
            .await
            .context("failed to read PROPFIND response")
    }

    pub(crate) async fn report(&self, url: &str, body: &str) -> anyhow::Result<String> {
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("depth"),
            HeaderValue::from_static("1"),
        );
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/xml"));
        let resp = self
            .send(
                Method::from_bytes(b"REPORT")?,
                url,
                headers,
                Some(body),
                "REPORT",
            )
            .await?;
        let status = resp.status();
        if !status.is_success() && status.as_u16() != 207 {
            bail!("REPORT returned {status}");
        }
        resp.text().await.context("failed to read REPORT response")
    }

    /// PUTs the calendar object at `event_href` (`new_event_href`, or the href it has).
    pub async fn put_event(&self, event_href: &str, ical_data: &str) -> anyhow::Result<()> {
        self.put_event_with(event_href, ical_data, HeaderMap::new())
            .await
    }

    /// `put_event` with `extra` headers besides the content type.
    pub async fn put_event_with(
        &self,
        event_href: &str,
        ical_data: &str,
        extra: HeaderMap,
    ) -> anyhow::Result<()> {
        if self.demo_mode {
            return Ok(());
        }
        let mut headers = extra;
        headers.insert(
            CONTENT_TYPE,
            HeaderValue::from_static("text/calendar; charset=utf-8"),
        );
        let resp = self
            .send(Method::PUT, event_href, headers, Some(ical_data), "PUT")
            .await?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            bail!("PUT returned {status}: {text}");
        }
        Ok(())
    }

    pub async fn delete_event(&self, event_href: &str) -> anyhow::Result<()> {
        if self.demo_mode {
            return Ok(());
        }
        let resp = self
            .send(Method::DELETE, event_href, HeaderMap::new(), None, "DELETE")
            .await?;
        let status = resp.status();
        if !status.is_success() {
            bail!("DELETE returned {status}");
        }
        Ok(())
    }

    pub async fn get_event(&self, event_href: &str) -> anyhow::Result<String> {
        let resp = self
            .send(Method::GET, event_href, HeaderMap::new(), None, "GET")
            .await?;
        let status = resp.status();
        if !status.is_success() {
            bail!("GET returned {status}");
        }
        resp.text().await.context("failed to read GET response")
    }

    /// Expand raw iCalendar text into `Event` structs for a date range.
    pub fn events_from_raw_ical(
        raw_ical: &str,
        start: chrono::NaiveDate,
        end: chrono::NaiveDate,
    ) -> Vec<Event> {
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

        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/xml"));
        let resp = self
            .send(
                Method::POST,
                calendar_href,
                headers,
                Some(&body),
                "push-register POST",
            )
            .await?;

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
            // Demo mode: no MKCALENDAR; the caller caches the calendar locally.
            return Ok(format!("/calendars/demo/{}/", calendar_slug(name)));
        }
        let start = self.start_url().await;
        let home_sets = self.discover_calendar_home_sets(&start).await?;
        let home = home_sets
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("No calendar-home-set found"))?;
        let slug = calendar_slug(name);

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

        let method = Method::from_bytes(b"MKCALENDAR").context("invalid method MKCALENDAR")?;
        let mut headers = HeaderMap::new();
        headers.insert(
            CONTENT_TYPE,
            HeaderValue::from_static("application/xml; charset=utf-8"),
        );
        let resp = self
            .send(method, &calendar_url, headers, Some(&body), "MKCALENDAR")
            .await?;

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

#[cfg(test)]
mod demo_tests {
    use super::*;

    /// `reqwest::Client::new()` needs the rustls provider that `main` installs.
    fn demo_client() -> CalDavClient {
        let _ = rustls::crypto::ring::default_provider().install_default();
        CalDavClient::demo()
    }

    #[test]
    fn calendar_slug_sanitizes_and_suffixes() {
        let slug = calendar_slug("Side Project!");
        let (base, suffix) = slug.rsplit_once('-').unwrap();
        assert_eq!(base, "side-project");
        assert_eq!(suffix.len(), 8);
        assert_eq!(calendar_slug("!!!").len(), 8);
        assert_ne!(calendar_slug("Work"), calendar_slug("Work"));
    }

    #[tokio::test]
    async fn demo_writes_are_stubbed() {
        let client = demo_client();
        let href = client
            .create_calendar("Side Project", Some("#123456"))
            .await
            .expect("demo create_calendar must succeed without MKCALENDAR");
        assert!(href.starts_with("/calendars/demo/side-project-"), "{href}");
        assert!(href.ends_with('/'));
        let event = new_event_href(&href, "uid-1");
        client
            .put_event(&event, "BEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n")
            .await
            .unwrap();
        client.delete_event(&event).await.unwrap();
    }

    /// A UID names one resource in the calendar and nothing else.
    #[test]
    fn resource_names_stay_in_the_collection() {
        let cal = "https://dav.example/remote.php/dav/calendars/alice/personal/";
        assert_eq!(
            new_event_href(cal, "040000008200E00074C5B7101A82E008@example.com"),
            "https://dav.example/remote.php/dav/calendars/alice/personal/040000008200E00074C5B7101A82E008@example.com.ics"
        );
        for uid in [
            "../other/victim",
            "../../../files/alice/Documents/report.docx?",
            "x#frag",
            "%2e%2e/%2e%2e/files/alice/a.txt?",
            "..",
            ".hidden",
            "a b",
            "caf\u{e9}",
            "",
        ] {
            let href = new_event_href(cal, uid);
            let url = reqwest::Url::parse(&href).unwrap();
            assert_eq!(url.query(), None, "{uid:?}");
            assert_eq!(url.fragment(), None, "{uid:?}");
            let name = url
                .path()
                .strip_prefix("/remote.php/dav/calendars/alice/personal/")
                .unwrap();
            assert!(
                !name.contains('/') && !name.contains('%'),
                "{uid:?}: {name}"
            );
            assert_eq!(name.len(), 64 + 4, "{uid:?}");
            assert_eq!(href, new_event_href(cal, uid), "stable");
        }
    }
}
