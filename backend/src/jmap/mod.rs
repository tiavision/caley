//! A JMAP client (RFC 8620 core, RFC 8621 mail, RFC 8620 §7.3 push over EventSource), only the methods Caley uses:
//! JSON over HTTP with reqwest. Written against Stalwart (its session's URLs are relative: they are resolved against
//! the session URL the server answered from), and kept to the RFCs so other servers work too.
//!
//! - `sync.rs`: a JMAP account's mail into the cache (the same tables and `.eml` files an IMAP account fills).
//! - `ops.rs`: flags, moves, deletions and sending, for the handlers.
//! - `fake.rs` (tests): an in-memory JMAP server the tests run against; `stalwart_tests.rs` runs the client against
//!   a real Stalwart when `JMAP_TEST_URL` is set.

use std::collections::HashMap;
use std::fmt;
use std::time::Duration;

use anyhow::{Result, anyhow};
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::{Value, json};

pub(crate) mod oauth;
pub(crate) mod ops;
pub(crate) mod sync;

#[cfg(test)]
pub(crate) mod api_tests;
#[cfg(test)]
pub(crate) mod fake;
#[cfg(test)]
mod oauth_tests;
#[cfg(test)]
mod stalwart_tests;
#[cfg(test)]
pub(crate) mod tests;

pub const CORE: &str = "urn:ietf:params:jmap:core";
pub const MAIL: &str = "urn:ietf:params:jmap:mail";
pub const SUBMISSION: &str = "urn:ietf:params:jmap:submission";
/// The OAuth scope of mail access (draft-ietf-mailmaint-oauth-public; Stalwart 1.x gives a token without it no mail
/// account).
pub const MAIL_SCOPE: &str = "urn:ietf:params:oauth:scope:mail";

/// Keywords the server sets about the message itself (RFC 8621 and the IANA registry: `$hasattachment` and
/// `$hasnoattachment`, which Stalwart sets): not flags a user sets, and the cache finds attachments itself.
const SERVER_KEYWORDS: &[&str] = &["$hasattachment", "$hasnoattachment"];

/// The properties of an email the sync reads: its mailboxes and keywords, and its raw header fields, which the IMAP
/// path's header parser reads as they are (so both protocols cache the same columns from the same parser).
pub const EMAIL_PROPERTIES: &[&str] = &[
    "id",
    "blobId",
    "threadId",
    "mailboxIds",
    "keywords",
    "size",
    "receivedAt",
    "headers",
];

/// How a JMAP account signs in.
#[derive(Clone)]
pub enum Auth {
    /// HTTP Basic: a password or an app password.
    Basic { username: String, password: String },
    /// A bearer token: an API token, or the user's OIDC access token.
    Bearer(String),
}

impl fmt::Debug for Auth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Auth::Basic { username, .. } => write!(f, "Basic({username})"),
            Auth::Bearer(_) => write!(f, "Bearer"),
        }
    }
}

impl Auth {
    fn apply(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match self {
            Auth::Basic { username, password } => req.basic_auth(username, Some(password)),
            Auth::Bearer(token) => req.bearer_auth(token),
        }
    }
}

/// A failure the caller may act on: the server refused the credentials (401/403), or a method failed with a JMAP
/// error type (`cannotCalculateChanges`, `notFound`, …). Anything else is an `anyhow` error.
#[derive(Debug)]
pub enum JmapError {
    Unauthorized,
    Method {
        kind: String,
        description: Option<String>,
    },
}

impl fmt::Display for JmapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            JmapError::Unauthorized => write!(f, "the JMAP server refused the sign-in"),
            JmapError::Method { kind, description } => match description {
                Some(d) => write!(f, "JMAP error {kind}: {d}"),
                None => write!(f, "JMAP error {kind}"),
            },
        }
    }
}

impl std::error::Error for JmapError {}

/// The JMAP error type of a method failure, if `e` is one.
pub fn method_error(e: &anyhow::Error) -> Option<&str> {
    match e.downcast_ref::<JmapError>() {
        Some(JmapError::Method { kind, .. }) => Some(kind),
        _ => None,
    }
}

pub fn is_unauthorized(e: &anyhow::Error) -> bool {
    matches!(e.downcast_ref::<JmapError>(), Some(JmapError::Unauthorized))
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    #[serde(default)]
    pub capabilities: HashMap<String, Value>,
    #[serde(default)]
    pub primary_accounts: HashMap<String, String>,
    pub api_url: String,
    pub download_url: String,
    pub upload_url: String,
    #[serde(default)]
    pub event_source_url: Option<String>,
    #[serde(default)]
    pub username: Option<String>,
}

impl Session {
    /// `maxObjectsInGet` of the core capability (the RFC's minimum, 500, when the server does not say).
    pub fn max_objects_in_get(&self) -> usize {
        self.core_limit("maxObjectsInGet").unwrap_or(500)
    }

    pub fn max_objects_in_set(&self) -> usize {
        self.core_limit("maxObjectsInSet").unwrap_or(500)
    }

    fn core_limit(&self, name: &str) -> Option<usize> {
        self.capabilities
            .get(CORE)
            .and_then(|c| c.get(name))
            .and_then(Value::as_u64)
            .map(|n| n.clamp(1, 5000) as usize)
    }
}

/// A mailbox as Mailbox/get gives it.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Mailbox {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default = "yes")]
    pub is_subscribed: bool,
}

fn yes() -> bool {
    true
}

/// One raw header field of an email (`headers`, its value in Raw form: as in the message, folding included).
#[derive(Debug, Clone, Deserialize)]
pub struct Header {
    pub name: String,
    pub value: String,
}

/// An email with `EMAIL_PROPERTIES`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Email {
    pub id: String,
    pub blob_id: String,
    #[serde(default)]
    pub mailbox_ids: HashMap<String, bool>,
    #[serde(default)]
    pub keywords: HashMap<String, bool>,
    #[serde(default)]
    pub size: Option<i64>,
    #[serde(default)]
    pub headers: Vec<Header>,
}

impl Email {
    /// The header section the fields came from, for the IMAP path's header parser.
    pub fn header_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for h in &self.headers {
            out.extend_from_slice(h.name.as_bytes());
            out.push(b':');
            out.extend_from_slice(h.value.as_bytes());
            out.extend_from_slice(b"\r\n");
        }
        out.extend_from_slice(b"\r\n");
        out
    }

    /// The keywords as IMAP flags (`$seen` → `\Seen`), the form the cache and the frontend keep.
    pub fn flags(&self) -> Vec<String> {
        let mut flags: Vec<String> = self
            .keywords
            .iter()
            .filter(|(k, set)| **set && !SERVER_KEYWORDS.contains(&k.to_ascii_lowercase().as_str()))
            .map(|(k, _)| keyword_to_flag(k))
            .collect();
        flags.sort();
        flags.dedup();
        flags
    }
}

/// What Foo/changes reports since a state (all of its pages).
#[derive(Debug, Default)]
pub struct Changes {
    pub new_state: String,
    pub created: Vec<String>,
    pub updated: Vec<String>,
    pub destroyed: Vec<String>,
}

/// A JMAP keyword as an IMAP flag (RFC 8621 §4.1.1: `$seen` is `\Seen`, …); other keywords stay as they are.
pub fn keyword_to_flag(keyword: &str) -> String {
    match keyword.to_ascii_lowercase().as_str() {
        "$seen" => "\\Seen".to_owned(),
        "$flagged" => "\\Flagged".to_owned(),
        "$answered" => "\\Answered".to_owned(),
        "$draft" => "\\Draft".to_owned(),
        "$forwarded" => "$Forwarded".to_owned(),
        _ => keyword.to_owned(),
    }
}

/// An IMAP flag as a JMAP keyword; `None` for the system flags JMAP has none for (`\Deleted`, `\Recent`).
pub fn flag_to_keyword(flag: &str) -> Option<String> {
    match flag.to_ascii_lowercase().as_str() {
        "\\seen" => Some("$seen".to_owned()),
        "\\flagged" => Some("$flagged".to_owned()),
        "\\answered" => Some("$answered".to_owned()),
        "\\draft" => Some("$draft".to_owned()),
        "\\deleted" | "\\recent" => None,
        "" => None,
        other if other.starts_with('\\') => None,
        other => Some(other.to_owned()),
    }
}

/// The JSON Pointer escaping of one path segment (RFC 6901), for patch keys such as `keywords/$seen`.
pub fn pointer_escape(segment: &str) -> String {
    segment.replace('~', "~0").replace('/', "~1")
}

/// A connected JMAP account: its session and the mail account the requests act on.
#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    auth: Auth,
    /// the URL the session was answered from (after redirects): relative URLs in it resolve against this
    base: url::Url,
    pub session: Session,
    /// the primary mail account
    pub account_id: String,
}

/// The reqwest client for JMAP: the app's TLS roots, timeouts, the user agent.
pub fn http_client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .user_agent(crate::version::USER_AGENT)
        .use_preconfigured_tls(crate::imap_client::tls_config())
        .connect_timeout(Duration::from_secs(10))
        .build()?)
}

/// The session URL to use for what a user typed: a URL as it is; a host (or `host:port`) gets
/// `https://…/.well-known/jmap` (RFC 8620 §2.2).
pub fn session_url(input: &str) -> Result<url::Url> {
    let input = input.trim();
    if input.is_empty() {
        return Err(anyhow!("no JMAP server"));
    }
    let with_scheme = if input.contains("://") {
        input.to_owned()
    } else {
        format!("https://{input}")
    };
    let mut url = url::Url::parse(&with_scheme).map_err(|e| anyhow!("invalid JMAP URL: {e}"))?;
    if !matches!(url.scheme(), "https" | "http") {
        return Err(anyhow!("a JMAP URL is https:// (or http://)"));
    }
    if url.path() == "/" || url.path().is_empty() {
        url.set_path("/.well-known/jmap");
    }
    Ok(url)
}

impl Client {
    /// Fetches the session resource and picks the primary mail account.
    pub async fn connect(http: &reqwest::Client, url: &str, auth: Auth) -> Result<Client> {
        let url = session_url(url)?;
        let resp = auth
            .apply(http.get(url.clone()))
            .header("accept", "application/json")
            .timeout(Duration::from_secs(30))
            .send()
            .await
            .map_err(|e| anyhow!("JMAP session request failed: {e}"))?;
        let status = resp.status();
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            return Err(JmapError::Unauthorized.into());
        }
        if !status.is_success() {
            return Err(anyhow!("JMAP session request answered {status}"));
        }
        let base = resp.url().clone();
        let body = resp.bytes().await?;
        let session: Session = serde_json::from_slice(&body)
            .map_err(|e| anyhow!("not a JMAP session resource: {e}"))?;
        let account_id = session
            .primary_accounts
            .get(MAIL)
            .cloned()
            .ok_or_else(|| {
                anyhow!(
                    "the JMAP server has no mail account for this sign-in (an OAuth token needs the scope \
                     {MAIL_SCOPE}: OIDC_SCOPES)"
                )
            })?;
        Ok(Client {
            http: http.clone(),
            auth,
            base,
            session,
            account_id,
        })
    }

    fn resolve(&self, url: &str) -> Result<url::Url> {
        let resolved = self
            .base
            .join(url)
            .map_err(|e| anyhow!("invalid URL {url} in the JMAP session: {e}"))?;
        #[cfg(test)]
        let resolved = test_rewrite(resolved);
        Ok(resolved)
    }

    /// Runs method calls in one request; each response in order (`[name, arguments, call id]`). A method that
    /// answered `error` fails the whole call with its type.
    pub async fn call(&self, calls: Vec<(&str, Value)>) -> Result<Vec<(String, Value)>> {
        let method_calls: Vec<Value> = calls
            .iter()
            .enumerate()
            .map(|(i, (name, args))| json!([name, args, format!("c{i}")]))
            .collect();
        let body = json!({ "using": [CORE, MAIL, SUBMISSION], "methodCalls": method_calls });
        let resp = self
            .auth
            .apply(self.http.post(self.resolve(&self.session.api_url)?))
            .header("content-type", "application/json")
            .header("accept", "application/json")
            .body(serde_json::to_vec(&body)?)
            .timeout(Duration::from_secs(120))
            .send()
            .await
            .map_err(|e| anyhow!("JMAP request failed: {e}"))?;
        let status = resp.status();
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            return Err(JmapError::Unauthorized.into());
        }
        let bytes = resp.bytes().await?;
        if !status.is_success() {
            // a request-level error is a problem details object (RFC 8620 §3.6.1)
            let detail = serde_json::from_slice::<Value>(&bytes)
                .ok()
                .and_then(|v| v.get("detail").or(v.get("type")).cloned())
                .map(|v| v.to_string())
                .unwrap_or_default();
            return Err(anyhow!("JMAP request answered {status} {detail}"));
        }
        let parsed: Value = serde_json::from_slice(&bytes)
            .map_err(|e| anyhow!("JMAP response is not JSON: {e}"))?;
        let responses = parsed
            .get("methodResponses")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow!("JMAP response without methodResponses"))?;
        let mut out = Vec::with_capacity(responses.len());
        for r in responses {
            let name = r.get(0).and_then(Value::as_str).unwrap_or_default();
            let args = r.get(1).cloned().unwrap_or(Value::Null);
            if name == "error" {
                return Err(JmapError::Method {
                    kind: args
                        .get("type")
                        .and_then(Value::as_str)
                        .unwrap_or("serverFail")
                        .to_owned(),
                    description: args
                        .get("description")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                }
                .into());
            }
            out.push((name.to_owned(), args));
        }
        Ok(out)
    }

    /// One method call; its response's arguments.
    pub async fn method(&self, name: &str, args: Value) -> Result<Value> {
        let mut responses = self.call(vec![(name, args)]).await?;
        if responses.is_empty() {
            return Err(anyhow!("no response to {name}"));
        }
        Ok(responses.remove(0).1)
    }

    /// Every mailbox, and the Mailbox state.
    pub async fn mailboxes(&self) -> Result<(Vec<Mailbox>, String)> {
        let r = self
            .method(
                "Mailbox/get",
                json!({
                    "accountId": self.account_id,
                    "ids": null,
                    "properties": ["id", "name", "parentId", "role", "isSubscribed"],
                }),
            )
            .await?;
        let list: Vec<Mailbox> =
            serde_json::from_value(r.get("list").cloned().unwrap_or_default())?;
        Ok((list, state_of(&r)))
    }

    /// The ids of every email `filter` matches (all of them with `null`), newest first, page by page.
    pub async fn query_ids(&self, filter: Value) -> Result<Vec<String>> {
        let page = self.session.max_objects_in_get();
        let mut ids: Vec<String> = Vec::new();
        loop {
            let r = self
                .method(
                    "Email/query",
                    json!({
                        "accountId": self.account_id,
                        "filter": filter,
                        "sort": [{ "property": "receivedAt", "isAscending": false }],
                        "position": ids.len(),
                        "limit": page,
                        "calculateTotal": true,
                    }),
                )
                .await?;
            let batch: Vec<String> =
                serde_json::from_value(r.get("ids").cloned().unwrap_or_default())?;
            let total = r.get("total").and_then(Value::as_u64).map(|t| t as usize);
            let done = batch.is_empty() || total.is_some_and(|t| ids.len() + batch.len() >= t);
            ids.extend(batch);
            if done {
                return Ok(ids);
            }
        }
    }

    /// The emails of `ids` with `EMAIL_PROPERTIES`, in batches of `maxObjectsInGet`, and the Email state of the
    /// first batch (`state` alone with no ids). Ids the server no longer has are left out.
    pub async fn emails(&self, ids: &[String]) -> Result<(Vec<Email>, String)> {
        let mut out = Vec::with_capacity(ids.len());
        let mut state = String::new();
        if ids.is_empty() {
            let r = self
                .method(
                    "Email/get",
                    json!({ "accountId": self.account_id, "ids": [], "properties": ["id"] }),
                )
                .await?;
            return Ok((out, state_of(&r)));
        }
        for chunk in ids.chunks(self.session.max_objects_in_get()) {
            let r = self
                .method(
                    "Email/get",
                    json!({
                        "accountId": self.account_id,
                        "ids": chunk,
                        "properties": EMAIL_PROPERTIES,
                    }),
                )
                .await?;
            if state.is_empty() {
                state = state_of(&r);
            }
            let list: Vec<Email> =
                serde_json::from_value(r.get("list").cloned().unwrap_or_default())?;
            out.extend(list);
        }
        Ok((out, state))
    }

    /// The current state of a type (`Email`, `Mailbox`): Foo/get with no ids.
    pub async fn state(&self, kind: &str) -> Result<String> {
        let r = self
            .method(
                &format!("{kind}/get"),
                json!({ "accountId": self.account_id, "ids": [], "properties": ["id"] }),
            )
            .await?;
        Ok(state_of(&r))
    }

    /// Foo/changes since `since`, every page of it. `cannotCalculateChanges` is a `JmapError::Method`.
    pub async fn changes(&self, kind: &str, since: &str) -> Result<Changes> {
        let mut out = Changes::default();
        let mut since = since.to_owned();
        loop {
            let r = self
                .method(
                    &format!("{kind}/changes"),
                    json!({
                        "accountId": self.account_id,
                        "sinceState": since,
                        "maxChanges": self.session.max_objects_in_get(),
                    }),
                )
                .await?;
            let list = |key: &str| -> Vec<String> {
                serde_json::from_value(r.get(key).cloned().unwrap_or_default()).unwrap_or_default()
            };
            out.created.extend(list("created"));
            out.updated.extend(list("updated"));
            out.destroyed.extend(list("destroyed"));
            let new_state = r
                .get("newState")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let more = r
                .get("hasMoreChanges")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            out.new_state = new_state.clone();
            if !more || new_state.is_empty() || new_state == since {
                return Ok(out);
            }
            since = new_state;
        }
    }

    /// Email/set with `update` (id → patch) and `destroy`: the ids the server did not change (with its reason).
    pub async fn set_emails(
        &self,
        update: serde_json::Map<String, Value>,
        destroy: Vec<String>,
    ) -> Result<Vec<(String, String)>> {
        let mut failed = Vec::new();
        let max = self.session.max_objects_in_set();
        let updates: Vec<(String, Value)> = update.into_iter().collect();
        for chunk in updates.chunks(max) {
            let patch: serde_json::Map<String, Value> = chunk.iter().cloned().collect();
            let r = self
                .method(
                    "Email/set",
                    json!({ "accountId": self.account_id, "update": patch }),
                )
                .await?;
            failed.extend(set_failures(&r, "notUpdated"));
        }
        for chunk in destroy.chunks(max) {
            let r = self
                .method(
                    "Email/set",
                    json!({ "accountId": self.account_id, "destroy": chunk }),
                )
                .await?;
            failed.extend(set_failures(&r, "notDestroyed"));
        }
        Ok(failed)
    }

    /// A blob's bytes (an email's `blobId`: its whole message).
    pub async fn download(&self, blob_id: &str) -> Result<Vec<u8>> {
        let url = self
            .session
            .download_url
            .replace("{accountId}", &urlencode(&self.account_id))
            .replace("{blobId}", &urlencode(blob_id))
            .replace("{name}", "message.eml")
            .replace("{type}", "message%2Frfc822");
        let resp = self
            .auth
            .apply(self.http.get(self.resolve(&url)?))
            .timeout(Duration::from_secs(300))
            .send()
            .await
            .map_err(|e| anyhow!("JMAP download failed: {e}"))?;
        let status = resp.status();
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            return Err(JmapError::Unauthorized.into());
        }
        if !status.is_success() {
            return Err(anyhow!("JMAP download of {blob_id} answered {status}"));
        }
        Ok(resp.bytes().await?.to_vec())
    }

    /// Uploads bytes; the new blob's id.
    pub async fn upload(&self, bytes: Vec<u8>, content_type: &str) -> Result<String> {
        let url = self
            .session
            .upload_url
            .replace("{accountId}", &urlencode(&self.account_id));
        let resp = self
            .auth
            .apply(self.http.post(self.resolve(&url)?))
            .header("content-type", content_type)
            .body(bytes)
            .timeout(Duration::from_secs(300))
            .send()
            .await
            .map_err(|e| anyhow!("JMAP upload failed: {e}"))?;
        let status = resp.status();
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            return Err(JmapError::Unauthorized.into());
        }
        let body: Value = serde_json::from_slice(&resp.bytes().await?).unwrap_or_default();
        if !status.is_success() {
            return Err(anyhow!("JMAP upload answered {status}"));
        }
        body.get("blobId")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| anyhow!("JMAP upload answered no blobId"))
    }

    /// The EventSource push URL for `types` (RFC 8620 §7.3), if the server offers push.
    pub fn event_source_url(&self, types: &str, ping_secs: u32) -> Option<Result<url::Url>> {
        let template = self.session.event_source_url.as_deref()?;
        let url = template
            .replace("{types}", types)
            .replace("{closeafter}", "no")
            .replace("{ping}", &ping_secs.to_string());
        Some(self.resolve(&url))
    }

    /// Opens the push stream; `on_change` runs for each StateChange that names this account. Returns when the
    /// server closes the stream, or with an error (a stream that stays silent for `3 × ping` is an error).
    pub async fn listen(
        &self,
        types: &str,
        ping_secs: u32,
        mut on_change: impl FnMut(&HashMap<String, String>),
    ) -> Result<()> {
        let url = self
            .event_source_url(types, ping_secs)
            .ok_or_else(|| anyhow!("the JMAP server offers no push"))??;
        let resp = self
            .auth
            .apply(self.http.get(url))
            .header("accept", "text/event-stream")
            .send()
            .await?;
        let status = resp.status();
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            return Err(JmapError::Unauthorized.into());
        }
        if !status.is_success() {
            return Err(anyhow!("JMAP event source answered {status}"));
        }
        let mut stream = resp.bytes_stream();
        let mut parser = EventStreamParser::default();
        let silence = Duration::from_secs(u64::from(ping_secs.max(10)) * 3);
        loop {
            let chunk = match tokio::time::timeout(silence, stream.next()).await {
                Err(_) => return Err(anyhow!("JMAP event source went silent")),
                Ok(None) => return Ok(()),
                Ok(Some(chunk)) => chunk?,
            };
            for event in parser.push(&chunk) {
                if let Some(changed) = state_change(&event, &self.account_id) {
                    on_change(&changed);
                }
            }
        }
    }
}

/// Tests against a Stalwart in a container (`stalwart_tests.rs`): Stalwart advertises `https://<its hostname>/…` in
/// the session, which the container does not serve; `JMAP_TEST_REWRITE=https://mail.example.com=http://localhost:18081`
/// sends what is advertised on that origin to the container.
#[cfg(test)]
fn test_rewrite(url: url::Url) -> url::Url {
    let Ok(rule) = std::env::var("JMAP_TEST_REWRITE") else {
        return url;
    };
    let Some((from, to)) = rule.split_once('=') else {
        return url;
    };
    match url.as_str().strip_prefix(from.trim_end_matches('/')) {
        Some(rest) => {
            url::Url::parse(&format!("{}{rest}", to.trim_end_matches('/'))).unwrap_or(url)
        }
        None => url,
    }
}

/// A URL the server gave (an endpoint in its metadata), as requested: in the tests against Stalwart in a container,
/// rewritten as the session's are (`test_rewrite`); as it is otherwise.
pub(crate) fn rewrite(url: String) -> String {
    #[cfg(test)]
    if let Ok(parsed) = url::Url::parse(&url) {
        return test_rewrite(parsed).to_string();
    }
    url
}

fn urlencode(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

fn state_of(r: &Value) -> String {
    r.get("state")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn set_failures(r: &Value, key: &str) -> Vec<(String, String)> {
    r.get(key)
        .and_then(Value::as_object)
        .map(|m| {
            m.iter()
                .map(|(id, err)| {
                    let kind = err
                        .get("type")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown")
                        .to_owned();
                    (id.clone(), kind)
                })
                .collect()
        })
        .unwrap_or_default()
}

/// One server-sent event: its `event:` name and `data:` lines joined.
#[derive(Debug, Default, PartialEq)]
pub struct ServerEvent {
    pub event: String,
    pub data: String,
}

/// The text/event-stream format, fed chunk by chunk (a chunk may end in the middle of a line or of a UTF-8
/// sequence).
#[derive(Default)]
pub struct EventStreamParser {
    buf: Vec<u8>,
    event: String,
    data: Vec<String>,
}

impl EventStreamParser {
    pub fn push(&mut self, chunk: &[u8]) -> Vec<ServerEvent> {
        self.buf.extend_from_slice(chunk);
        let mut out = Vec::new();
        while let Some(pos) = self.buf.iter().position(|&b| b == b'\n') {
            let mut line: Vec<u8> = self.buf.drain(..=pos).collect();
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            let line = String::from_utf8_lossy(&line);
            if line.is_empty() {
                if !self.data.is_empty() || !self.event.is_empty() {
                    out.push(ServerEvent {
                        event: std::mem::take(&mut self.event),
                        data: std::mem::take(&mut self.data).join("\n"),
                    });
                }
                continue;
            }
            if line.starts_with(':') {
                continue;
            }
            let (field, value) = match line.split_once(':') {
                Some((f, v)) => (f, v.strip_prefix(' ').unwrap_or(v)),
                None => (line.as_ref(), ""),
            };
            match field {
                "event" => self.event = value.to_owned(),
                "data" => self.data.push(value.to_owned()),
                _ => {}
            }
        }
        out
    }
}

/// The types that changed for `account_id` in a `state` event (a StateChange object), if it is one.
pub fn state_change(event: &ServerEvent, account_id: &str) -> Option<HashMap<String, String>> {
    if !(event.event.is_empty() || event.event == "state") {
        return None;
    }
    let v: Value = serde_json::from_str(&event.data).ok()?;
    if v.get("@type").and_then(Value::as_str) != Some("StateChange") {
        return None;
    }
    let changed = v.get("changed")?.get(account_id)?.as_object()?;
    Some(
        changed
            .iter()
            .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_owned())))
            .collect(),
    )
}

/// The names the cache knows a JMAP account's mailboxes by: their paths, `/`-separated, the inbox (role `inbox`,
/// at the top) `INBOX` as in IMAP, so that what looks for `INBOX` finds it. Two mailboxes whose paths are equal get
/// their ids appended. A mailbox whose parent is missing (or a cycle) is put at the top.
pub fn mailbox_names(mailboxes: &[Mailbox]) -> HashMap<String, String> {
    let by_id: HashMap<&str, &Mailbox> = mailboxes.iter().map(|m| (m.id.as_str(), m)).collect();
    let mut sorted: Vec<&Mailbox> = mailboxes.iter().collect();
    // the inbox first, then by id: a stable order for who keeps a contested name
    sorted.sort_by(|a, b| {
        (a.role.as_deref() != Some("inbox"), &a.id)
            .cmp(&(b.role.as_deref() != Some("inbox"), &b.id))
    });
    let mut names: HashMap<String, String> = HashMap::new();
    let mut taken: std::collections::HashSet<String> = std::collections::HashSet::new();
    for mb in sorted {
        let mut segments: Vec<String> = Vec::new();
        let mut cur = Some(mb);
        let mut seen = std::collections::HashSet::new();
        while let Some(m) = cur {
            if !seen.insert(m.id.as_str()) {
                break;
            }
            let top = m.parent_id.as_deref().and_then(|p| by_id.get(p)).is_none();
            let name = if top && m.role.as_deref() == Some("inbox") {
                "INBOX".to_owned()
            } else {
                m.name.replace('/', "∕")
            };
            segments.push(name);
            cur = m.parent_id.as_deref().and_then(|p| by_id.get(p).copied());
        }
        segments.reverse();
        let mut name = segments.join("/");
        if name.eq_ignore_ascii_case("INBOX") && mb.role.as_deref() != Some("inbox") {
            name = format!("{name} ({})", mb.id);
        }
        if !taken.insert(name.to_ascii_lowercase()) {
            name = format!("{name} ({})", mb.id);
            taken.insert(name.to_ascii_lowercase());
        }
        names.insert(mb.id.clone(), name);
    }
    names
}
