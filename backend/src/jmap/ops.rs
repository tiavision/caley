//! What the mail handlers do on a JMAP server: flags, moves and deletions of cached messages (Email/set), and
//! sending (an upload, Email/import into the Sent mailbox, EmailSubmission/set).

use std::collections::{HashMap, HashSet};

use anyhow::{Result, anyhow};
use serde_json::{Map, Value, json};
use sqlx::SqlitePool;

use super::{Client, JmapError, flag_to_keyword, pointer_escape};
use crate::email_db::jmap_db;

/// A change of one cached message.
pub enum Change<'a> {
    Flags {
        flags: &'a [String],
        set: bool,
    },
    Move {
        dest: &'a str,
    },
    /// out of this mailbox; an email in no other mailbox is destroyed
    Delete,
}

/// Makes the changes of an account's cached messages on its server in as few requests as `maxObjectsInSet` allows.
/// Returns the messages the server refused or the cache could not name ((mailbox, UID), reason); the caller logs
/// them (the next sync corrects the cache).
pub async fn apply_changes(
    client: &Client,
    db: &SqlitePool,
    account_id: &str,
    changes: &[(&str, u32, Change<'_>)],
) -> Result<Vec<((String, u32), String)>> {
    let mut unknown = Vec::new();
    let mut patches: HashMap<String, Map<String, Value>> = HashMap::new();
    let mut destroy: HashSet<String> = HashSet::new();
    let mut place_of: HashMap<String, (String, u32)> = HashMap::new();
    let mut mailbox_ids: HashMap<String, Option<String>> = HashMap::new();

    for (mailbox, uid, change) in changes {
        let Some((email_id, _)) = jmap_db::jmap_message(db, account_id, mailbox, *uid).await?
        else {
            unknown.push((
                ((*mailbox).to_owned(), *uid),
                "not a cached JMAP message".to_owned(),
            ));
            continue;
        };
        place_of.insert(email_id.clone(), ((*mailbox).to_owned(), *uid));
        match change {
            Change::Flags { flags, set } => {
                let patch = patches.entry(email_id).or_default();
                for keyword in flags.iter().filter_map(|f| flag_to_keyword(f)) {
                    let value = if *set { Value::Bool(true) } else { Value::Null };
                    patch.insert(format!("keywords/{}", pointer_escape(&keyword)), value);
                }
            }
            Change::Move { dest } => {
                let src = mailbox_id(db, account_id, mailbox, &mut mailbox_ids).await?;
                let dst = mailbox_id(db, account_id, dest, &mut mailbox_ids).await?;
                let (Some(src), Some(dst)) = (src, dst) else {
                    unknown.push((((*mailbox).to_owned(), *uid), format!("no mailbox {dest}")));
                    continue;
                };
                if src == dst {
                    continue;
                }
                let patch = patches.entry(email_id).or_default();
                patch.insert(format!("mailboxIds/{}", pointer_escape(&src)), Value::Null);
                patch.insert(
                    format!("mailboxIds/{}", pointer_escape(&dst)),
                    Value::Bool(true),
                );
            }
            Change::Delete => {
                let elsewhere = jmap_db::jmap_memberships(db, account_id, &email_id)
                    .await?
                    .iter()
                    .any(|(m, _)| m != mailbox);
                let src = mailbox_id(db, account_id, mailbox, &mut mailbox_ids).await?;
                match (elsewhere, src) {
                    (true, Some(src)) => {
                        patches
                            .entry(email_id)
                            .or_default()
                            .insert(format!("mailboxIds/{}", pointer_escape(&src)), Value::Null);
                    }
                    _ => {
                        destroy.insert(email_id);
                    }
                }
            }
        }
    }

    patches.retain(|id, patch| !destroy.contains(id) && !patch.is_empty());
    let update: Map<String, Value> = patches
        .into_iter()
        .map(|(id, patch)| (id, Value::Object(patch)))
        .collect();
    let failed = if update.is_empty() && destroy.is_empty() {
        Vec::new()
    } else {
        client
            .set_emails(update, destroy.into_iter().collect())
            .await?
    };
    Ok(unknown
        .into_iter()
        .chain(failed.into_iter().map(|(id, reason)| {
            let place = place_of.get(&id).cloned().unwrap_or_default();
            (place, reason)
        }))
        .collect())
}

async fn mailbox_id(
    db: &SqlitePool,
    account_id: &str,
    name: &str,
    known: &mut HashMap<String, Option<String>>,
) -> Result<Option<String>> {
    if let Some(id) = known.get(name) {
        return Ok(id.clone());
    }
    let id = jmap_db::jmap_mailbox_id(db, account_id, name).await?;
    known.insert(name.to_owned(), id.clone());
    Ok(id)
}

/// Why a message could not be sent.
#[derive(Debug)]
pub enum SendError {
    /// the server has no identity that may send as the address
    NoIdentity(String),
    /// the account has no mailbox to keep the sent message in
    NoSentMailbox,
}

impl std::fmt::Display for SendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SendError::NoIdentity(from) => {
                write!(f, "the JMAP server has no identity to send as {from}")
            }
            SendError::NoSentMailbox => write!(f, "the JMAP account has no Sent mailbox"),
        }
    }
}

impl std::error::Error for SendError {}

/// The id of the server's identity that sends as `from`: the one with that address, else a wildcard one
/// (`*@domain`, as Stalwart and Fastmail list catch-all identities).
pub fn identity_for(identities: &[(String, String)], from: &str) -> Option<String> {
    let from = from.to_ascii_lowercase();
    let domain = from.rsplit_once('@').map(|(_, d)| d.to_owned());
    identities
        .iter()
        .find(|(_, email)| email.to_ascii_lowercase() == from)
        .or_else(|| {
            identities.iter().find(|(_, email)| {
                let email = email.to_ascii_lowercase();
                domain.as_deref().is_some_and(|d| email == format!("*@{d}"))
            })
        })
        .map(|(id, _)| id.clone())
}

/// Sends a message as the client built it: uploads it, imports it into the Sent mailbox (read) and submits it to
/// the envelope's recipients. A submission the server refuses removes the imported copy again. Returns the name of
/// the mailbox the copy is in.
pub async fn send_raw(
    client: &Client,
    db: &SqlitePool,
    account_id: &str,
    from: &str,
    recipients: &[String],
    raw: Vec<u8>,
) -> Result<String> {
    let r = client
        .method(
            "Identity/get",
            json!({ "accountId": client.account_id, "ids": null }),
        )
        .await?;
    let identities: Vec<(String, String)> = r
        .get("list")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|i| {
                    Some((
                        i.get("id")?.as_str()?.to_owned(),
                        i.get("email")?.as_str()?.to_owned(),
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    let identity =
        identity_for(&identities, from).ok_or_else(|| SendError::NoIdentity(from.to_owned()))?;

    let (sent_id, sent_name) = match jmap_db::jmap_mailbox_by_role(db, account_id, "sent").await? {
        Some(sent) => sent,
        None => jmap_db::jmap_mailbox_by_role(db, account_id, "drafts")
            .await?
            .ok_or(SendError::NoSentMailbox)?,
    };

    let blob_id = client.upload(raw, "message/rfc822").await?;
    let rcpt_to: Vec<Value> = recipients.iter().map(|r| json!({ "email": r })).collect();
    let responses = client
        .call(vec![
            (
                "Email/import",
                json!({
                    "accountId": client.account_id,
                    "emails": { "m": {
                        "blobId": blob_id,
                        "mailboxIds": Value::Object(Map::from_iter([(sent_id.clone(), Value::Bool(true))])),
                        "keywords": { "$seen": true },
                    } },
                }),
            ),
            (
                "EmailSubmission/set",
                json!({
                    "accountId": client.account_id,
                    "create": { "s": {
                        "identityId": identity,
                        "emailId": "#m",
                        "envelope": { "mailFrom": { "email": from }, "rcptTo": rcpt_to },
                    } },
                }),
            ),
        ])
        .await?;
    let imported = &responses[0].1;
    let email_id = imported
        .pointer("/created/m/id")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let Some(email_id) = email_id else {
        return Err(set_error(
            imported,
            "m",
            "the server did not take the message",
        ));
    };
    let submitted = responses.get(1).map(|r| &r.1);
    if submitted.and_then(|s| s.pointer("/created/s")).is_none() {
        let err = match submitted {
            Some(s) => set_error(s, "s", "the server did not send the message"),
            None => anyhow!("the server did not send the message"),
        };
        if let Err(e) = client.set_emails(Map::new(), vec![email_id]).await {
            eprintln!("[jmap] removing the copy of an unsent message: {e}");
        }
        return Err(err);
    }
    Ok(sent_name)
}

/// The error of a Foo/set that did not create `key` (its SetError type and description).
fn set_error(response: &Value, key: &str, fallback: &str) -> anyhow::Error {
    let Some(err) = response.get("notCreated").and_then(|n| n.get(key)) else {
        return anyhow!("{fallback}");
    };
    JmapError::Method {
        kind: err
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("serverFail")
            .to_owned(),
        description: err
            .get("description")
            .and_then(Value::as_str)
            .map(str::to_owned),
    }
    .into()
}
