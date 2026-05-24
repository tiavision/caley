use std::time::Duration;

use quick_xml::events::Event as XmlEvent;
use quick_xml::Reader;

pub struct AutodiscoverResult {
    pub provider_name: Option<String>,
    pub imap_host: String,
    pub imap_port: u16,
    pub imap_use_tls: bool,
    pub imap_username: String,
    pub smtp_host: String,
    pub smtp_port: u16,
}

pub async fn autodiscover(
    email: &str,
    client: &reqwest::Client,
) -> Option<AutodiscoverResult> {
    let domain = email.split('@').nth(1)?;

    let urls = [
        format!("https://autoconfig.thunderbird.net/v1.1/{domain}"),
        format!("https://autoconfig.{domain}/mail/config-v1.1.xml"),
        format!("https://{domain}/.well-known/autoconfig/mail/config-v1.1.xml"),
        format!("http://autoconfig.{domain}/mail/config-v1.1.xml"),
    ];

    for url in &urls {
        if let Some(result) = try_url(url, email, client).await {
            return Some(result);
        }
    }
    None
}

async fn try_url(url: &str, email: &str, client: &reqwest::Client) -> Option<AutodiscoverResult> {
    let resp = client
        .get(url)
        .timeout(Duration::from_secs(5))
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let text = resp.text().await.ok()?;
    parse_autoconfig_xml(&text, email)
}

fn substitute(template: &str, email: &str) -> String {
    let local = email.split('@').next().unwrap_or("");
    let domain = email.split('@').nth(1).unwrap_or("");
    template
        .replace("%EMAILADDRESS%", email)
        .replace("%EMAILLOCALPART%", local)
        .replace("%EMAILDOMAIN%", domain)
}

fn parse_autoconfig_xml(xml: &str, email: &str) -> Option<AutodiscoverResult> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut buf = Vec::new();

    // Fields we collect
    let mut provider_name: Option<String> = None;
    let mut imap_host: Option<String> = None;
    let mut imap_port: Option<u16> = None;
    let mut imap_use_tls: bool = true;
    let mut imap_username_tpl: Option<String> = None;
    let mut smtp_host: Option<String> = None;
    let mut smtp_port: Option<u16> = None;

    // Parser state
    let mut in_incoming_imap = false;
    let mut in_outgoing_smtp = false;
    let mut current_tag: Option<String> = None;

    loop {
        match reader.read_event_into(&mut buf).ok()? {
            XmlEvent::Start(e) => {
                let name = std::str::from_utf8(e.local_name().as_ref())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                match name.as_str() {
                    "incomingserver" => {
                        let is_imap = e.attributes().flatten().any(|a| {
                            a.key.local_name().as_ref() == b"type"
                                && a.value.as_ref().eq_ignore_ascii_case(b"imap")
                        });
                        if is_imap && imap_host.is_none() {
                            in_incoming_imap = true;
                        }
                    }
                    "outgoingserver" => {
                        let is_smtp = e.attributes().flatten().any(|a| {
                            a.key.local_name().as_ref() == b"type"
                                && a.value.as_ref().eq_ignore_ascii_case(b"smtp")
                        });
                        if is_smtp && smtp_host.is_none() {
                            in_outgoing_smtp = true;
                        }
                    }
                    tag => {
                        current_tag = Some(tag.to_string());
                    }
                }
            }
            XmlEvent::Text(e) => {
                let text = e.unescape().unwrap_or_default().trim().to_string();
                if text.is_empty() {
                    continue;
                }
                match current_tag.as_deref() {
                    Some("displayname") if !in_incoming_imap && !in_outgoing_smtp => {
                        if provider_name.is_none() {
                            provider_name = Some(text);
                        }
                    }
                    Some("hostname") if in_incoming_imap => {
                        imap_host = Some(text);
                    }
                    Some("port") if in_incoming_imap => {
                        imap_port = text.parse().ok();
                    }
                    Some("sockettype") if in_incoming_imap => {
                        imap_use_tls = text.eq_ignore_ascii_case("ssl");
                    }
                    Some("loginname" | "username") if in_incoming_imap => {
                        if imap_username_tpl.is_none() {
                            imap_username_tpl = Some(text);
                        }
                    }
                    Some("hostname") if in_outgoing_smtp => {
                        smtp_host = Some(text);
                    }
                    Some("port") if in_outgoing_smtp => {
                        smtp_port = text.parse().ok();
                    }
                    _ => {}
                }
            }
            XmlEvent::End(e) => {
                let name = std::str::from_utf8(e.local_name().as_ref())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                match name.as_str() {
                    "incomingserver" => in_incoming_imap = false,
                    "outgoingserver" => in_outgoing_smtp = false,
                    _ => {}
                }
                current_tag = None;
            }
            XmlEvent::Eof => break,
            _ => {}
        }
        buf.clear();
    }

    let imap_host = imap_host?;
    let imap_port = imap_port?;
    let smtp_host = smtp_host.unwrap_or_else(|| imap_host.clone());
    let smtp_port = smtp_port.unwrap_or(587);
    let imap_username = substitute(
        imap_username_tpl.as_deref().unwrap_or("%EMAILADDRESS%"),
        email,
    );
    let provider_name = provider_name.map(|n| substitute(&n, email));

    Some(AutodiscoverResult {
        provider_name,
        imap_host,
        imap_port,
        imap_use_tls,
        imap_username,
        smtp_host,
        smtp_port,
    })
}
