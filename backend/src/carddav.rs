use anyhow::{bail, Context};
use quick_xml::events::Event as XmlEvent;
use quick_xml::Reader;
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE};
use serde::{Deserialize, Serialize};

use crate::caldav::{extract_ctag_and_token, extract_hrefs, resolve_url, CalDavClient};

#[derive(Serialize, Clone)]
pub struct Addressbook {
    pub href: String,
    pub name: String,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct TypedEntry {
    pub value: String,
    pub type_label: String,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct AddressEntry {
    pub street: String,
    pub city: String,
    pub region: String,
    pub postal: String,
    pub country: String,
    pub type_label: String,
}

pub struct ContactFields {
    pub uid: String,
    pub fn_name: String,
    pub emails: Vec<TypedEntry>,
    pub phones: Vec<TypedEntry>,
    pub addresses: Vec<AddressEntry>,
    pub org: String,
    pub photo: Option<String>,
}

pub struct CardSyncResult {
    pub new_token: String,
    pub changed: Vec<(String, String, String)>, // (href, etag, raw_vcard)
    pub deleted: Vec<String>,
}

// ── Discovery ─────────────────────────────────────────────────────────────────

pub async fn discover_addressbooks(client: &CalDavClient) -> anyhow::Result<Vec<Addressbook>> {
    let start_url = client_start_url(client).await;

    // Step 1: find current-user-principal
    let principal_body = r#"<?xml version="1.0" encoding="utf-8"?>
<propfind xmlns="DAV:">
  <prop>
    <current-user-principal/>
  </prop>
</propfind>"#;

    let principal_xml = client.propfind(&start_url, "0", principal_body).await?;

    let principal_url = match extract_hrefs(&principal_xml, "current-user-principal") {
        Ok(hrefs) if !hrefs.is_empty() => resolve_url(&start_url, &hrefs[0]),
        _ => start_url.clone(),
    };

    // Step 2: find addressbook-home-set
    let home_body = r#"<?xml version="1.0" encoding="utf-8"?>
<propfind xmlns="DAV:" xmlns:card="urn:ietf:params:xml:ns:carddav">
  <prop>
    <card:addressbook-home-set/>
  </prop>
</propfind>"#;

    let home_xml = client.propfind(&principal_url, "0", home_body).await?;

    let home_hrefs = extract_hrefs(&home_xml, "addressbook-home-set")
        .with_context(|| "addressbook-home-set not found")?;

    let mut all = Vec::new();
    for h in &home_hrefs {
        let home_url = resolve_url(&principal_url, h);
        if let Ok(books) = find_addressbooks_in_home(client, &home_url).await {
            all.extend(books);
        }
    }
    Ok(all)
}

async fn client_start_url(client: &CalDavClient) -> String {
    // Try .well-known/carddav redirect; fall back to base_url
    let well_known = format!("{}/.well-known/carddav", client.base_url());
    let resp = client
        .http_client()
        .get(&well_known)
        .basic_auth(client.username(), Some(client.password()))
        .send()
        .await;
    if let Ok(r) = resp {
        if r.status().is_redirection() {
            if let Some(loc) = r.headers().get("location") {
                if let Ok(s) = loc.to_str() {
                    return resolve_url(&well_known, s).trim_end_matches('/').to_string();
                }
            }
        }
    }
    client.base_url().to_string()
}

async fn find_addressbooks_in_home(
    client: &CalDavClient,
    home_url: &str,
) -> anyhow::Result<Vec<Addressbook>> {
    let body = r#"<?xml version="1.0" encoding="utf-8"?>
<propfind xmlns="DAV:" xmlns:card="urn:ietf:params:xml:ns:carddav">
  <prop>
    <displayname/>
    <resourcetype/>
  </prop>
</propfind>"#;
    let xml = client.propfind(home_url, "1", body).await?;
    parse_addressbooks(&xml, home_url)
}

fn parse_addressbooks(xml: &str, base_url: &str) -> anyhow::Result<Vec<Addressbook>> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let base_resolved = resolve_url(base_url, "");
    let mut books = Vec::new();
    let mut current_href: Option<String> = None;
    let mut current_name: Option<String> = None;
    let mut is_addressbook = false;
    let mut in_href = false;
    let mut in_displayname = false;
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf)? {
            XmlEvent::Start(e) => {
                let lname = e.local_name();
                let local = std::str::from_utf8(lname.as_ref())?;
                match local {
                    "href" => in_href = true,
                    "displayname" => in_displayname = true,
                    "addressbook" => is_addressbook = true,
                    _ => {}
                }
            }
            XmlEvent::Empty(e) => {
                let lname = e.local_name();
                let local = std::str::from_utf8(lname.as_ref())?;
                if local == "addressbook" {
                    is_addressbook = true;
                }
            }
            XmlEvent::Text(e) => {
                let text = e.unescape()?.to_string();
                if in_href { current_href = Some(text); }
                else if in_displayname { current_name = Some(text); }
            }
            XmlEvent::End(e) => {
                let lname = e.local_name();
                let local = std::str::from_utf8(lname.as_ref())?;
                match local {
                    "href" => in_href = false,
                    "displayname" => in_displayname = false,
                    "response" => {
                        if is_addressbook {
                            if let Some(href) = &current_href {
                                let resolved = resolve_url(base_url, href);
                                if resolved.trim_end_matches('/') != base_resolved.trim_end_matches('/') {
                                    let name = current_name.clone().unwrap_or_else(|| {
                                        resolved.trim_end_matches('/').rsplit('/').next().unwrap_or("Contacts").to_string()
                                    });
                                    books.push(Addressbook { href: resolved, name });
                                }
                            }
                        }
                        current_href = None;
                        current_name = None;
                        is_addressbook = false;
                    }
                    _ => {}
                }
            }
            XmlEvent::Eof => break,
            _ => {}
        }
    }
    Ok(books)
}

// ── Sync / fetch ──────────────────────────────────────────────────────────────

pub async fn get_addressbook_ctag_and_token(
    client: &CalDavClient,
    href: &str,
) -> anyhow::Result<(Option<String>, Option<String>)> {
    let body = r#"<?xml version="1.0" encoding="utf-8"?>
<d:propfind xmlns:d="DAV:" xmlns:cs="http://calendarserver.org/ns/">
  <d:prop>
    <cs:getctag/>
    <d:sync-token/>
  </d:prop>
</d:propfind>"#;
    let xml = client.propfind(href, "0", body).await?;
    Ok(extract_ctag_and_token(&xml))
}

pub async fn fetch_all_raw_contacts(
    client: &CalDavClient,
    addressbook_href: &str,
) -> anyhow::Result<Vec<(String, String, String)>> {
    let body = r#"<?xml version="1.0" encoding="utf-8"?>
<d:propfind xmlns:d="DAV:" xmlns:card="urn:ietf:params:xml:ns:carddav">
  <d:prop>
    <d:getetag/>
    <card:address-data/>
  </d:prop>
</d:propfind>"#;
    let xml = client.propfind(addressbook_href, "1", body).await?;
    extract_contacts_with_hrefs(&xml, addressbook_href)
}

pub async fn sync_addressbook_collection(
    client: &CalDavClient,
    addressbook_href: &str,
    sync_token: Option<&str>,
) -> anyhow::Result<CardSyncResult> {
    let token_val = sync_token.unwrap_or("");
    let body = format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<d:sync-collection xmlns:d="DAV:" xmlns:card="urn:ietf:params:xml:ns:carddav">
  <d:sync-token>{token_val}</d:sync-token>
  <d:sync-level>1</d:sync-level>
  <d:prop>
    <d:getetag/>
    <card:address-data/>
  </d:prop>
</d:sync-collection>"#
    );
    let xml = client.report(addressbook_href, &body).await?;
    extract_card_sync_result(&xml, addressbook_href)
}

fn extract_contacts_with_hrefs(xml: &str, base_url: &str) -> anyhow::Result<Vec<(String, String, String)>> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut results = Vec::new();
    let mut in_response = false;
    let mut in_href = false;
    let mut in_etag = false;
    let mut in_address_data = false;
    let mut current_href = String::new();
    let mut current_etag = String::new();
    let mut current_vcard = String::new();
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
                        current_vcard.clear();
                    }
                    "href" if in_response => in_href = true,
                    "getetag" => in_etag = true,
                    "address-data" => {
                        in_address_data = true;
                        current_vcard.clear();
                    }
                    _ => {}
                }
            }
            XmlEvent::Text(e) => {
                let text = e.unescape()?;
                if in_address_data { current_vcard.push_str(&text); }
                else if in_href { current_href.push_str(&text); }
                else if in_etag { current_etag.push_str(&text); }
            }
            XmlEvent::End(e) => {
                let lname = e.local_name();
                let local = std::str::from_utf8(lname.as_ref())?;
                match local {
                    "href" => in_href = false,
                    "getetag" => in_etag = false,
                    "address-data" => in_address_data = false,
                    "response" => {
                        if !current_href.is_empty() && !current_vcard.is_empty() {
                            results.push((
                                resolve_url(base_url, current_href.trim()),
                                current_etag.trim().trim_matches('"').to_string(),
                                current_vcard.clone(),
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

fn extract_card_sync_result(xml: &str, base_url: &str) -> anyhow::Result<CardSyncResult> {
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
    let mut in_address_data = false;
    let mut in_status = false;

    let mut current_href = String::new();
    let mut current_etag = String::new();
    let mut current_vcard = String::new();
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
                        current_vcard.clear();
                        current_status.clear();
                    }
                    "href" if in_response => in_href = true,
                    "getetag" => in_etag = true,
                    "address-data" => {
                        in_address_data = true;
                        current_vcard.clear();
                    }
                    "status" => in_status = true,
                    _ => {}
                }
            }
            XmlEvent::Text(e) => {
                let text = e.unescape()?;
                if in_sync_token_top { new_token.push_str(&text); }
                else if in_address_data { current_vcard.push_str(&text); }
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
                    "address-data" => in_address_data = false,
                    "status" => in_status = false,
                    "response" => {
                        let href = resolve_url(base_url, current_href.trim());
                        if current_status.contains("404") {
                            deleted.push(href);
                        } else if !current_vcard.is_empty() {
                            changed.push((
                                href,
                                current_etag.trim().trim_matches('"').to_string(),
                                current_vcard.clone(),
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
    Ok(CardSyncResult { new_token, changed, deleted })
}

// ── CRUD ──────────────────────────────────────────────────────────────────────

pub async fn put_contact(
    client: &CalDavClient,
    contact_href: &str,
    raw_vcard: &str,
) -> anyhow::Result<()> {
    let mut url = contact_href.to_string();
    for _ in 0..5 {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("text/vcard; charset=utf-8"));

        let resp = client
            .http_client()
            .put(&url)
            .headers(headers)
            .basic_auth(client.username(), Some(client.password()))
            .body(raw_vcard.to_string())
            .send()
            .await
            .context("PUT contact request failed")?;

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
            bail!("PUT contact returned {status}: {text}");
        }
        return Ok(());
    }
    bail!("too many redirects for PUT contact")
}

pub async fn delete_contact(client: &CalDavClient, contact_href: &str) -> anyhow::Result<()> {
    let mut url = contact_href.to_string();
    for _ in 0..5 {
        let resp = client
            .http_client()
            .delete(&url)
            .basic_auth(client.username(), Some(client.password()))
            .send()
            .await
            .context("DELETE contact request failed")?;

        let status = resp.status();
        if status.is_redirection() {
            if let Some(location) = resp.headers().get("location") {
                url = location.to_str()?.to_string();
                continue;
            }
            bail!("DELETE redirect without Location header");
        }
        if !status.is_success() {
            bail!("DELETE contact returned {status}");
        }
        return Ok(());
    }
    bail!("too many redirects for DELETE contact")
}

// ── vCard parsing / building ──────────────────────────────────────────────────

fn extract_type_label(prop: &ical::property::Property) -> String {
    let raw = prop
        .params
        .as_ref()
        .and_then(|p| p.iter().find(|(k, _)| k == "TYPE"))
        .and_then(|(_, vals)| vals.first())
        .map(|s| s.to_uppercase());
    match raw.as_deref() {
        Some("HOME") => "home".into(),
        Some("WORK") => "work".into(),
        Some("CELL") | Some("MOBILE") => "mobile".into(),
        Some("FAX") => "fax".into(),
        Some("PAGER") => "pager".into(),
        _ => "other".into(),
    }
}

pub fn parse_vcard(raw: &str) -> ContactFields {
    let reader = ical::VcardParser::new(raw.as_bytes());
    let mut uid = String::new();
    let mut fn_name = String::new();
    let mut emails = Vec::new();
    let mut phones = Vec::new();
    let mut addresses = Vec::new();
    let mut org = String::new();
    let mut photo: Option<String> = None;

    if let Some(Ok(contact)) = reader.into_iter().next() {
        for prop in &contact.properties {
            match prop.name.as_str() {
                "UID" => uid = prop.value.clone().unwrap_or_default(),
                "FN" => fn_name = prop.value.clone().unwrap_or_default(),
                "EMAIL" => {
                    if let Some(v) = &prop.value {
                        if !v.is_empty() {
                            emails.push(TypedEntry {
                                value: v.clone(),
                                type_label: extract_type_label(prop),
                            });
                        }
                    }
                }
                "TEL" => {
                    if let Some(v) = &prop.value {
                        if !v.is_empty() {
                            phones.push(TypedEntry {
                                value: v.clone(),
                                type_label: extract_type_label(prop),
                            });
                        }
                    }
                }
                "ADR" => {
                    if let Some(v) = &prop.value {
                        // ADR format: POBox;ExtAddr;Street;City;Region;Postal;Country
                        let parts: Vec<&str> = v.splitn(7, ';').collect();
                        let street = parts.get(2).copied().unwrap_or("").trim().to_string();
                        let city = parts.get(3).copied().unwrap_or("").trim().to_string();
                        let region = parts.get(4).copied().unwrap_or("").trim().to_string();
                        let postal = parts.get(5).copied().unwrap_or("").trim().to_string();
                        let country = parts.get(6).copied().unwrap_or("").trim().to_string();
                        if !street.is_empty() || !city.is_empty() || !country.is_empty() {
                            addresses.push(AddressEntry {
                                street,
                                city,
                                region,
                                postal,
                                country,
                                type_label: extract_type_label(prop),
                            });
                        }
                    }
                }
                "ORG" => {
                    if let Some(v) = &prop.value {
                        org = v.split(';').next().unwrap_or("").to_string();
                    }
                }
                "N" if fn_name.is_empty() => {
                    if let Some(v) = &prop.value {
                        // N format: Family;Given;Additional;Prefix;Suffix
                        let parts: Vec<&str> = v.split(';').collect();
                        let family = parts.first().copied().unwrap_or("").trim();
                        let given = parts.get(1).copied().unwrap_or("").trim();
                        fn_name = match (given.is_empty(), family.is_empty()) {
                            (false, false) => format!("{given} {family}"),
                            (true, false) => family.to_string(),
                            (false, true) => given.to_string(),
                            _ => String::new(),
                        };
                    }
                }
                "PHOTO" => {
                    if let Some(v) = &prop.value {
                        if !v.is_empty() {
                            let params = prop.params.as_deref().unwrap_or(&[]);
                            let is_uri = params.iter().any(|(k, vals)| {
                                k == "VALUE" && vals.iter().any(|v| v.eq_ignore_ascii_case("uri"))
                            });
                            let is_b64 = params.iter().any(|(k, vals)| {
                                k == "ENCODING" && vals.iter().any(|v| {
                                    v.eq_ignore_ascii_case("b") || v.eq_ignore_ascii_case("base64")
                                })
                            });
                            if v.starts_with("data:") || is_uri || (!is_b64 && (v.starts_with("http://") || v.starts_with("https://"))) {
                                photo = Some(v.clone());
                            } else if is_b64 {
                                let mime = params.iter()
                                    .find(|(k, _)| k == "TYPE")
                                    .and_then(|(_, vals)| vals.first())
                                    .map(|t| t.to_lowercase())
                                    .unwrap_or_else(|| "jpeg".to_string());
                                let mime = if mime.contains('/') { mime } else { format!("image/{mime}") };
                                photo = Some(format!("data:{mime};base64,{v}"));
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    ContactFields { uid, fn_name, emails, phones, addresses, org, photo }
}

pub fn build_vcard(
    uid: &str,
    fn_name: &str,
    emails: &[TypedEntry],
    phones: &[TypedEntry],
    addresses: &[AddressEntry],
    org: &str,
) -> String {
    let mut lines = vec![
        "BEGIN:VCARD".to_string(),
        "VERSION:3.0".to_string(),
        format!("UID:{uid}"),
        format!("FN:{}", escape_vcard(fn_name)),
    ];
    lines.push(format!("N:;{};;;", escape_vcard(fn_name)));
    for e in emails {
        let type_param = vcard_type_param(&e.type_label);
        lines.push(format!("EMAIL{type_param}:{}", escape_vcard(&e.value)));
    }
    for p in phones {
        let type_param = vcard_type_param(&p.type_label);
        lines.push(format!("TEL{type_param}:{}", escape_vcard(&p.value)));
    }
    for a in addresses {
        let type_param = vcard_type_param(&a.type_label);
        lines.push(format!(
            "ADR{type_param}:;;{};{};{};{};{}",
            escape_vcard(&a.street),
            escape_vcard(&a.city),
            escape_vcard(&a.region),
            escape_vcard(&a.postal),
            escape_vcard(&a.country),
        ));
    }
    if !org.is_empty() {
        lines.push(format!("ORG:{}", escape_vcard(org)));
    }
    lines.push("END:VCARD".to_string());
    lines.join("\r\n") + "\r\n"
}

pub fn build_vcard_preserving(
    existing_raw: Option<&str>,
    uid: &str,
    fn_name: &str,
    emails: &[TypedEntry],
    phones: &[TypedEntry],
    addresses: &[AddressEntry],
    org: &str,
) -> String {
    let Some(raw) = existing_raw else {
        return build_vcard(uid, fn_name, emails, phones, addresses, org);
    };

    // Unfold continuation lines (RFC 6350 §3.2)
    let unfolded = raw
        .replace("\r\n ", "")
        .replace("\r\n\t", "")
        .replace("\n ", "")
        .replace("\n\t", "");

    const MANAGED: &[&str] = &["BEGIN", "END", "VERSION", "UID", "FN", "N", "EMAIL", "TEL", "ADR", "ORG"];

    let preserved: Vec<&str> = unfolded
        .lines()
        .filter(|line| {
            let prop = line.split(|c| c == ':' || c == ';').next().unwrap_or("").to_uppercase();
            !MANAGED.contains(&prop.as_str())
        })
        .collect();

    let mut lines = vec![
        "BEGIN:VCARD".to_string(),
        "VERSION:3.0".to_string(),
        format!("UID:{uid}"),
        format!("FN:{}", escape_vcard(fn_name)),
        format!("N:;{};;;", escape_vcard(fn_name)),
    ];
    for e in emails {
        let type_param = vcard_type_param(&e.type_label);
        lines.push(format!("EMAIL{type_param}:{}", escape_vcard(&e.value)));
    }
    for p in phones {
        let type_param = vcard_type_param(&p.type_label);
        lines.push(format!("TEL{type_param}:{}", escape_vcard(&p.value)));
    }
    for a in addresses {
        let type_param = vcard_type_param(&a.type_label);
        lines.push(format!(
            "ADR{type_param}:;;{};{};{};{};{}",
            escape_vcard(&a.street),
            escape_vcard(&a.city),
            escape_vcard(&a.region),
            escape_vcard(&a.postal),
            escape_vcard(&a.country),
        ));
    }
    if !org.is_empty() {
        lines.push(format!("ORG:{}", escape_vcard(org)));
    }
    for line in preserved {
        lines.push(line.to_string());
    }
    lines.push("END:VCARD".to_string());
    lines.join("\r\n") + "\r\n"
}

fn vcard_type_param(type_label: &str) -> String {
    let upper = match type_label {
        "home" => "HOME",
        "work" => "WORK",
        "mobile" => "CELL",
        "fax" => "FAX",
        "pager" => "PAGER",
        _ => return String::new(),
    };
    format!(";TYPE={upper}")
}

fn escape_vcard(s: &str) -> String {
    s.replace('\\', "\\\\").replace(',', "\\,").replace(';', "\\;").replace('\n', "\\n")
}

/// Derive the contact href from addressbook href and uid.
pub fn contact_href(addressbook_href: &str, uid: &str) -> String {
    format!("{}/{}.vcf", addressbook_href.trim_end_matches('/'), uid)
}
