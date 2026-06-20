use sqlx::SqlitePool;

use super::{DEMO_PERSONAL_ACCOUNT, DEMO_USER_ID, DEMO_WORK_ACCOUNT};

// chrono is used in build_mime for RFC 2822 date formatting

pub(super) async fn seed_imap_accounts(pool: &SqlitePool, key: &[u8; 32]) -> anyhow::Result<()> {
    // Account 1 — Work
    let enc_host = crate::crypto::encrypt("imap.work.example", key)?;
    let enc_user = crate::crypto::encrypt("demo@work.example", key)?;
    let enc_pass = crate::crypto::encrypt("demo", key)?;
    let enc_smtp = crate::crypto::encrypt("smtp.work.example", key)?;
    sqlx::query(
        "INSERT INTO imap_accounts
            (account_id, user_id, display_name, host, port, username, password,
             use_tls, smtp_host, smtp_port, sort_order)
         VALUES (?, ?, ?, ?, 993, ?, ?, 1, ?, 587, 0)
         ON CONFLICT(account_id) DO UPDATE SET
            host = excluded.host,
            username = excluded.username,
            password = excluded.password,
            smtp_host = excluded.smtp_host",
    )
    .bind(DEMO_WORK_ACCOUNT)
    .bind(DEMO_USER_ID)
    .bind("Work")
    .bind(enc_host)
    .bind(enc_user)
    .bind(enc_pass)
    .bind(enc_smtp)
    .execute(pool)
    .await?;

    // Account 2 — Personal
    let enc_host2 = crate::crypto::encrypt("mail.personal.example", key)?;
    let enc_user2 = crate::crypto::encrypt("me@personal.example", key)?;
    let enc_pass2 = crate::crypto::encrypt("demo", key)?;
    let enc_smtp2 = crate::crypto::encrypt("smtp.personal.example", key)?;
    sqlx::query(
        "INSERT INTO imap_accounts
            (account_id, user_id, display_name, host, port, username, password,
             use_tls, smtp_host, smtp_port, sort_order)
         VALUES (?, ?, ?, ?, 993, ?, ?, 1, ?, 587, 1)
         ON CONFLICT(account_id) DO UPDATE SET
            host = excluded.host,
            username = excluded.username,
            password = excluded.password,
            smtp_host = excluded.smtp_host",
    )
    .bind(DEMO_PERSONAL_ACCOUNT)
    .bind(DEMO_USER_ID)
    .bind("Personal")
    .bind(enc_host2)
    .bind(enc_user2)
    .bind(enc_pass2)
    .bind(enc_smtp2)
    .execute(pool)
    .await?;

    Ok(())
}

pub(super) async fn seed_mailboxes(pool: &SqlitePool) -> anyhow::Result<()> {
    let work_mailboxes = ["INBOX", "Sent", "Drafts", "Archive", "Trash", "Junk"];
    for (i, name) in work_mailboxes.iter().enumerate() {
        sqlx::query(
            "INSERT OR IGNORE INTO cached_mailboxes
                (account_id, name, delimiter, uidvalidity, last_synced, subscribed)
             VALUES (?, ?, '.', ?, strftime('%s','now'), 1)",
        )
        .bind(DEMO_WORK_ACCOUNT)
        .bind(name)
        .bind(2001 + i as i64)
        .execute(pool)
        .await?;
    }
    let personal_mailboxes = ["INBOX", "Sent", "Drafts", "Trash", "Newsletters"];
    for (i, name) in personal_mailboxes.iter().enumerate() {
        sqlx::query(
            "INSERT OR IGNORE INTO cached_mailboxes
                (account_id, name, delimiter, uidvalidity, last_synced, subscribed)
             VALUES (?, ?, '.', ?, strftime('%s','now'), 1)",
        )
        .bind(DEMO_PERSONAL_ACCOUNT)
        .bind(name)
        .bind(3001 + i as i64)
        .execute(pool)
        .await?;
    }
    Ok(())
}

struct MessageSeed {
    account: &'static str,
    uid: i64,
    mailbox: &'static str,
    message_id: &'static str,
    thread_id: Option<&'static str>,
    from_addr: &'static str,
    to_addrs: &'static str,
    subject: &'static str,
    days_ago: i64,
    flags: &'static str,
    body_text: &'static str,
    body_html: &'static str,
    is_unread: bool,
}

fn build_mime(m: &MessageSeed, sent_at: i64) -> String {
    use std::fmt::Write as _;
    // RFC 2822 date is approximated as UTC epoch string — good enough for demo.
    let date = chrono::DateTime::from_timestamp(sent_at, 0)
        .map(|dt| dt.format("%a, %d %b %Y %H:%M:%S +0000").to_string())
        .unwrap_or_else(|| "Thu, 01 Jan 1970 00:00:00 +0000".to_string());
    let mut s = String::new();
    let _ = write!(
        s,
        "MIME-Version: 1.0\r\nFrom: {}\r\nTo: {}\r\nSubject: {}\r\nDate: {}\r\nMessage-ID: {}\r\n",
        m.from_addr, m.to_addrs, m.subject, date, m.message_id,
    );
    if let Some(tid) = m.thread_id {
        let _ = write!(s, "In-Reply-To: {tid}\r\nReferences: {tid}\r\n");
    }
    let _ = write!(
        s,
        "Content-Type: multipart/alternative; boundary=\"demo-boundary\"\r\n\r\n\
         --demo-boundary\r\n\
         Content-Type: text/plain; charset=utf-8\r\n\r\n\
         {}\r\n\
         --demo-boundary\r\n\
         Content-Type: text/html; charset=utf-8\r\n\r\n\
         {}\r\n\
         --demo-boundary--\r\n",
        m.body_text, m.body_html,
    );
    s
}

pub(super) async fn seed_messages(
    pool: &SqlitePool,
    data_dir: &std::path::Path,
) -> anyhow::Result<()> {
    let messages: &[MessageSeed] = &[
        // ── Work INBOX ────────────────────────────────────────────────────────
        MessageSeed {
            account: DEMO_WORK_ACCOUNT,
            uid: 1,
            mailbox: "INBOX",
            message_id: "<welcome-work@demo.example>",
            thread_id: Some("<welcome-work@demo.example>"),
            from_addr: "\"IT Support\" <it@work.example>",
            to_addrs: "\"Demo User\" <demo@work.example>",
            subject: "Welcome to the team — setup checklist",
            days_ago: 0,
            flags: r#"[]"#,
            body_text: "Hi Demo,\n\nWelcome aboard! Please complete the onboarding checklist below. Key items: set up VPN, join #general on Slack, and book your first 1:1 with your manager.\n\n1. Install VPN client — see IT wiki\n2. Join Slack workspace and say hi in #intros\n3. Book 1:1 with Alice (use Calendly link)\n4. Complete security training (link in email)\n\nIT Support",
            body_html: r##"<div style="font-family:sans-serif;font-size:14px;line-height:1.6;color:#222"><p>Hi Demo,</p><p>Welcome aboard! Please complete the onboarding checklist below.</p><ol><li>Install VPN client — see IT wiki</li><li>Join Slack workspace and say hi in <strong>#intros</strong></li><li>Book 1:1 with Alice (use Calendly link)</li><li>Complete security training (link in email)</li></ol><p style="color:#666;margin-top:24px">IT Support<br>it@work.example</p></div>"##,
            is_unread: true,
        },
        MessageSeed {
            account: DEMO_WORK_ACCOUNT,
            uid: 2,
            mailbox: "INBOX",
            message_id: "<sprint-notes-1@work.example>",
            thread_id: Some("<sprint-thread-1@work.example>"),
            from_addr: "\"Alice Brennan\" <alice@acme.example>",
            to_addrs: "\"Demo User\" <demo@work.example>",
            subject: "Sprint 24 notes",
            days_ago: 1,
            flags: r#"["\\Seen"]"#,
            body_text: "Hi team,\n\nAttached are the notes from yesterday's sprint review. Please review and add any missing items by EOD Friday.\n\nHighlights:\n- Auth service refactor merged\n- Mobile perf regression still open (#341)\n- Q3 scope locked\n\nThanks,\nAlice",
            body_html: r##"<div style="font-family:sans-serif;font-size:14px;line-height:1.6;color:#222"><p>Hi team,</p><p>Attached are the notes from yesterday's sprint review.</p><ul><li>Auth service refactor merged ✓</li><li>Mobile perf regression still open (#341)</li><li>Q3 scope locked</li></ul><p>Thanks,<br><strong>Alice</strong></p></div>"##,
            is_unread: false,
        },
        MessageSeed {
            account: DEMO_WORK_ACCOUNT,
            uid: 3,
            mailbox: "INBOX",
            message_id: "<sprint-reply-bob@work.example>",
            thread_id: Some("<sprint-thread-1@work.example>"),
            from_addr: "\"Bob Chen\" <bob.chen@example.net>",
            to_addrs: "\"Demo User\" <demo@work.example>",
            subject: "Re: Sprint 24 notes",
            days_ago: 1,
            flags: r#"["\\Seen"]"#,
            body_text: "Looks good to me. I'll add the API performance items before the end of the day.\n\n— Bob",
            body_html: r##"<div style="font-family:sans-serif;font-size:14px;line-height:1.6;color:#222"><p>Looks good to me. I'll add the API performance items before the end of the day.</p><p style="color:#555">— Bob</p></div>"##,
            is_unread: false,
        },
        MessageSeed {
            account: DEMO_WORK_ACCOUNT,
            uid: 4,
            mailbox: "INBOX",
            message_id: "<invoice-may@work.example>",
            thread_id: None,
            from_addr: "\"Accounts\" <billing@acme.example>",
            to_addrs: "\"Demo User\" <demo@work.example>",
            subject: "Invoice #2405 — May Services",
            days_ago: 2,
            flags: r#"[]"#,
            body_text: "Please find attached invoice #2405 for May services.\n\nAmount due: EUR 1,200.00\nDue date: 30 June 2026\nPayment reference: INV-2405\n\nPlease process via the finance portal.",
            body_html: r##"<div style="font-family:sans-serif;font-size:14px;line-height:1.6;color:#222"><p>Please find attached invoice #2405 for May services.</p><table style="border-collapse:collapse;margin:16px 0;width:100%;max-width:400px"><tr style="background:#f5f5f5"><td style="padding:8px 12px;border:1px solid #ddd"><strong>Amount due</strong></td><td style="padding:8px 12px;border:1px solid #ddd">EUR 1,200.00</td></tr><tr><td style="padding:8px 12px;border:1px solid #ddd"><strong>Due date</strong></td><td style="padding:8px 12px;border:1px solid #ddd">30 June 2026</td></tr><tr style="background:#f5f5f5"><td style="padding:8px 12px;border:1px solid #ddd"><strong>Reference</strong></td><td style="padding:8px 12px;border:1px solid #ddd">INV-2405</td></tr></table></div>"##,
            is_unread: true,
        },
        MessageSeed {
            account: DEMO_WORK_ACCOUNT,
            uid: 5,
            mailbox: "INBOX",
            message_id: "<hr-policy@work.example>",
            thread_id: None,
            from_addr: "\"HR Team\" <hr@acme.example>",
            to_addrs: "\"All Staff\" <all@acme.example>",
            subject: "Updated Remote Work Policy — effective 1 July",
            days_ago: 3,
            flags: r#"["\\Seen"]"#,
            body_text: "Dear team,\n\nPlease review the updated remote work policy effective 1st July.\n\nKey changes:\n- Flexible core hours: 10:00–15:00\n- Quarterly in-office requirement: two weeks per quarter\n- Home office equipment budget increased to EUR 500/year\n\nBest,\nHR Team",
            body_html: r##"<div style="font-family:sans-serif;font-size:14px;line-height:1.6;color:#222"><p>Dear team,</p><p>Please review the updated remote work policy effective <strong>1st July</strong>.</p><ul><li>Flexible core hours: <strong>10:00–15:00</strong></li><li>Quarterly in-office requirement: two weeks per quarter</li><li>Home office equipment budget increased to <strong>EUR 500/year</strong></li></ul><p style="color:#666">Best,<br>HR Team</p></div>"##,
            is_unread: false,
        },
        MessageSeed {
            account: DEMO_WORK_ACCOUNT,
            uid: 6,
            mailbox: "INBOX",
            message_id: "<design-feedback@work.example>",
            thread_id: Some("<design-thread-1@work.example>"),
            from_addr: "\"Emma Johansson\" <emma@designstudio.example>",
            to_addrs: "\"Demo User\" <demo@work.example>",
            subject: "Homepage redesign — feedback needed",
            days_ago: 3,
            flags: r#"[]"#,
            body_text: "Hey,\n\nI've finished the v2 mockups for the homepage. Could you take a look and share your thoughts before our call Thursday?\n\nMain changes:\n- New hero section with video background\n- Consolidated navigation (7 items → 4)\n- Mobile-first grid layout\n- Refreshed colour palette\n\nEmma",
            body_html: r##"<div style="font-family:sans-serif;font-size:14px;line-height:1.6;color:#222"><p>Hey,</p><p>I've finished the v2 mockups for the homepage. Could you take a look?</p><ul><li>New hero section with video background</li><li>Consolidated navigation (7 → 4)</li><li>Mobile-first grid layout</li><li>Refreshed colour palette</li></ul><p>Emma</p></div>"##,
            is_unread: true,
        },
        MessageSeed {
            account: DEMO_WORK_ACCOUNT,
            uid: 7,
            mailbox: "INBOX",
            message_id: "<server-alert@work.example>",
            thread_id: None,
            from_addr: "\"Monitoring\" <alerts@ops.example>",
            to_addrs: "\"Demo User\" <demo@work.example>",
            subject: "[RESOLVED] High CPU on web-02",
            days_ago: 4,
            flags: r#"["\\Seen"]"#,
            body_text: "[RESOLVED] High CPU on web-02\n\nHost: web-02.prod\nAlert fired: 03:14 UTC\nResolved: 03:31 UTC\nDuration: 17 minutes\n\nRoot cause: runaway log rotation script. No user-facing impact.",
            body_html: r##"<div style="font-family:monospace;font-size:13px;background:#f8f8f8;padding:16px;border-left:4px solid #4caf50"><p style="font-family:sans-serif;font-size:15px"><strong style="color:#4caf50">[RESOLVED]</strong> High CPU on web-02</p><p style="font-family:sans-serif">Root cause: runaway log rotation script. No user-facing impact.</p></div>"##,
            is_unread: false,
        },
        MessageSeed {
            account: DEMO_WORK_ACCOUNT,
            uid: 8,
            mailbox: "INBOX",
            message_id: "<conf-registration@work.example>",
            thread_id: None,
            from_addr: "\"DevConf\" <registration@devconf.example>",
            to_addrs: "\"Demo User\" <demo@work.example>",
            subject: "Your DevConf ticket is confirmed",
            days_ago: 5,
            flags: r#"["\\Seen"]"#,
            body_text: "Your registration for DevConf is confirmed.\n\nOrder: DC-2026-08421\nTicket: Full Pass\nDate: 3–5 August 2026\nVenue: Convention Centre, Berlin",
            body_html: r##"<div style="font-family:sans-serif;font-size:14px;line-height:1.6;color:#222;max-width:500px"><div style="background:#1a1a2e;color:#fff;padding:24px;text-align:center;border-radius:8px 8px 0 0"><p style="font-size:22px;font-weight:700;margin:0">DevConf 2026</p></div><div style="border:1px solid #ddd;border-top:none;padding:24px;border-radius:0 0 8px 8px"><p>Ticket confirmed. Order DC-2026-08421 · Full Pass · 3–5 August 2026</p></div></div>"##,
            is_unread: false,
        },
        MessageSeed {
            account: DEMO_WORK_ACCOUNT,
            uid: 9,
            mailbox: "INBOX",
            message_id: "<github-pr@work.example>",
            thread_id: None,
            from_addr: "\"GitHub\" <notifications@github.example>",
            to_addrs: "\"Demo User\" <demo@work.example>",
            subject: "[demo-org/app] PR #142: Add dark mode support",
            days_ago: 0,
            flags: r#"[]"#,
            body_text: "David Kim opened pull request #142\n\nAdd dark mode support via CSS custom properties. Closes #87.\n\n2 files changed, 134 insertions(+), 12 deletions(-)",
            body_html: r##"<div style="font-family:sans-serif;font-size:14px;line-height:1.6;color:#222"><p><strong>David Kim</strong> opened PR <strong>#142</strong>: Add dark mode support</p><p>2 files changed · +134 · -12</p></div>"##,
            is_unread: true,
        },
        MessageSeed {
            account: DEMO_WORK_ACCOUNT,
            uid: 10,
            mailbox: "INBOX",
            message_id: "<customer-inquiry@work.example>",
            thread_id: Some("<customer-thread-1@work.example>"),
            from_addr: "\"James Wilson\" <james.wilson@citybank.example>",
            to_addrs: "\"Demo User\" <demo@work.example>",
            subject: "Integration question — webhook signatures",
            days_ago: 1,
            flags: r#"[]"#,
            body_text: "Hi,\n\nWe're integrating your API and have a question about webhook signature verification. The docs mention HMAC-SHA256 but don't specify the encoding — is the signature hex or base64?\n\nThanks,\nJames Wilson\nCity Bank",
            body_html: r##"<div style="font-family:sans-serif;font-size:14px;line-height:1.6;color:#222"><p>Hi,</p><p>We're integrating your API and have a question about webhook signature verification. Is the signature <strong>hex</strong> or <strong>base64</strong>?</p><p>Thanks,<br>James Wilson<br><span style="color:#555">City Bank</span></p></div>"##,
            is_unread: true,
        },
        MessageSeed {
            account: DEMO_WORK_ACCOUNT,
            uid: 11,
            mailbox: "INBOX",
            message_id: "<board-agenda@work.example>",
            thread_id: None,
            from_addr: "\"Barbara Williams\" <b.williams@acme.example>",
            to_addrs: "\"Leadership Team\" <leadership@acme.example>",
            subject: "Board meeting agenda — Thursday 10:00",
            days_ago: 2,
            flags: r#"["\\Seen"]"#,
            body_text: "Please find the agenda for Thursday's board meeting at 10:00.\n\nAgenda:\n1. Q2 financials (CFO, 20 min)\n2. Product roadmap H2 (CPO, 30 min)\n3. Headcount plan (HR, 15 min)\n4. AOB\n\nBarbara",
            body_html: r##"<div style="font-family:sans-serif;font-size:14px;line-height:1.6;color:#222"><p>Board meeting Thursday 10:00. Items: Q2 financials, Product roadmap, Headcount plan, AOB.</p><p style="color:#666">Barbara</p></div>"##,
            is_unread: false,
        },
        MessageSeed {
            account: DEMO_WORK_ACCOUNT,
            uid: 12,
            mailbox: "INBOX",
            message_id: "<deploy-success@work.example>",
            thread_id: None,
            from_addr: "\"CI/CD\" <ci@ops.example>",
            to_addrs: "\"Demo User\" <demo@work.example>",
            subject: "[SUCCESS] Production deploy v2.14.3",
            days_ago: 0,
            flags: r#"[]"#,
            body_text: "[SUCCESS] Production deploy v2.14.3\n\nCompleted: 14:32 UTC\nServices: api, worker, scheduler\nStatus: all health checks passing\nCommit: a3f1b9c",
            body_html: r##"<div style="font-family:monospace;font-size:13px;color:#222;background:#f0faf0;border-left:4px solid #4caf50;padding:16px"><p style="font-family:sans-serif;font-size:15px;font-weight:700;color:#2a7a2a;margin:0 0 12px">[SUCCESS] Production deploy v2.14.3</p><p style="font-family:sans-serif">All health checks passing. Commit a3f1b9c.</p></div>"##,
            is_unread: true,
        },
        MessageSeed {
            account: DEMO_WORK_ACCOUNT,
            uid: 13,
            mailbox: "INBOX",
            message_id: "<contract-renewal@work.example>",
            thread_id: Some("<contract-thread@work.example>"),
            from_addr: "\"Diana Reynolds\" <diana@reynoldslegal.example>",
            to_addrs: "\"Demo User\" <demo@work.example>",
            subject: "Contract renewal — action required by Friday",
            days_ago: 3,
            flags: r#"[]"#,
            body_text: "Hi,\n\nThe vendor contract for the data warehouse service expires on the 30th. Please review the renewal terms and confirm acceptance by Friday.\n\nKey points:\n- 12-month renewal, same terms\n- Price increase: +3% (EUR 14,400/year)\n- Auto-renewal clause removed\n\nRegards,\nDiana Reynolds",
            body_html: r##"<div style="font-family:sans-serif;font-size:14px;line-height:1.6;color:#222"><p>Hi,</p><p>Contract expires on the 30th. Please confirm acceptance by Friday.</p><ul><li>12-month renewal, same terms</li><li>Price increase: +3% (EUR 14,400/year)</li><li>Auto-renewal clause removed</li></ul><p>Diana Reynolds</p></div>"##,
            is_unread: true,
        },
        MessageSeed {
            account: DEMO_WORK_ACCOUNT,
            uid: 14,
            mailbox: "INBOX",
            message_id: "<weekly-digest@work.example>",
            thread_id: None,
            from_addr: "\"Analytics\" <analytics@acme.example>",
            to_addrs: "\"Demo User\" <demo@work.example>",
            subject: "Weekly metrics digest — Week 22",
            days_ago: 6,
            flags: r#"["\\Seen"]"#,
            body_text: "Week 22 highlights:\n\nDAU: 142k (+8% WoW)\nRevenue: EUR 280k (+3%)\nChurn: 1.2% (-0.3pp)\nP95 latency: 180ms (stable)",
            body_html: r##"<div style="font-family:sans-serif;font-size:14px;line-height:1.6;color:#222"><p><strong>Week 22 highlights</strong></p><table style="width:100%;max-width:400px;border-collapse:collapse;margin:8px 0"><tr style="background:#f5f5f5"><td style="padding:10px 12px;border:1px solid #ddd">DAU</td><td style="padding:10px 12px;border:1px solid #ddd;font-weight:600">142k +8%</td></tr><tr><td style="padding:10px 12px;border:1px solid #ddd">Revenue</td><td style="padding:10px 12px;border:1px solid #ddd;font-weight:600">EUR 280k +3%</td></tr></table></div>"##,
            is_unread: false,
        },
        // Work Sent
        MessageSeed {
            account: DEMO_WORK_ACCOUNT,
            uid: 1,
            mailbox: "Sent",
            message_id: "<sent-sprint-reply@work.example>",
            thread_id: Some("<sprint-thread-1@work.example>"),
            from_addr: "\"Demo User\" <demo@work.example>",
            to_addrs: "\"Alice Brennan\" <alice@acme.example>",
            subject: "Re: Sprint 24 notes",
            days_ago: 1,
            flags: r#"["\\Seen"]"#,
            body_text: "Thanks Alice, reviewed and LGTM. I've added the missing auth performance item.",
            body_html: r##"<div style="font-family:sans-serif;font-size:14px;line-height:1.6;color:#222"><p>Thanks Alice, reviewed and LGTM. I've added the missing auth performance item.</p></div>"##,
            is_unread: false,
        },
        MessageSeed {
            account: DEMO_WORK_ACCOUNT,
            uid: 2,
            mailbox: "Sent",
            message_id: "<sent-design@work.example>",
            thread_id: Some("<design-thread-1@work.example>"),
            from_addr: "\"Demo User\" <demo@work.example>",
            to_addrs: "\"Emma Johansson\" <emma@designstudio.example>",
            subject: "Re: Homepage redesign — feedback needed",
            days_ago: 2,
            flags: r#"["\\Seen"]"#,
            body_text: "Emma, the new hero section looks great. One suggestion: increase contrast on the CTA button (currently fails AA). Everything else is solid.",
            body_html: r##"<div style="font-family:sans-serif;font-size:14px;line-height:1.6;color:#222"><p>Emma, the new hero section looks great. One suggestion: increase contrast on the CTA button (currently fails AA). Everything else is solid.</p></div>"##,
            is_unread: false,
        },
        MessageSeed {
            account: DEMO_WORK_ACCOUNT,
            uid: 3,
            mailbox: "Sent",
            message_id: "<sent-customer@work.example>",
            thread_id: Some("<customer-thread-1@work.example>"),
            from_addr: "\"Demo User\" <demo@work.example>",
            to_addrs: "\"James Wilson\" <james.wilson@citybank.example>",
            subject: "Re: Integration question — webhook signatures",
            days_ago: 0,
            flags: r#"["\\Seen"]"#,
            body_text: "Hi James,\n\nThe signature is hex-encoded. You'll compute HMAC-SHA256 over the raw request body using your webhook secret, then compare to the X-Signature header value (hex string, no prefix).\n\nLet me know if you'd like a code snippet!",
            body_html: r##"<div style="font-family:sans-serif;font-size:14px;line-height:1.6;color:#222"><p>Hi James,</p><p>The signature is <strong>hex-encoded</strong>. Compute HMAC-SHA256 over the raw request body and compare to the <code>X-Signature</code> header (hex, no prefix).</p></div>"##,
            is_unread: false,
        },
        // ── Personal INBOX ────────────────────────────────────────────────────
        MessageSeed {
            account: DEMO_PERSONAL_ACCOUNT,
            uid: 1,
            mailbox: "INBOX",
            message_id: "<bank-statement@personal.example>",
            thread_id: None,
            from_addr: "\"City Bank\" <statements@citybank.example>",
            to_addrs: "\"Demo User\" <me@personal.example>",
            subject: "Your May statement is ready",
            days_ago: 1,
            flags: r#"[]"#,
            body_text: "Your May 2026 account statement is now available in online banking.\n\nAccount: **** 4821\nStatement period: 1–31 May 2026\nClosing balance: EUR 3,847.22",
            body_html: r##"<div style="font-family:sans-serif;font-size:14px;line-height:1.6;color:#222;max-width:480px"><div style="background:#003080;color:#fff;padding:20px;border-radius:8px 8px 0 0"><strong style="font-size:18px">City Bank</strong></div><div style="border:1px solid #ddd;border-top:none;padding:20px;border-radius:0 0 8px 8px"><p>Your May 2026 statement is ready. Closing balance: EUR 3,847.22</p></div></div>"##,
            is_unread: true,
        },
        MessageSeed {
            account: DEMO_PERSONAL_ACCOUNT,
            uid: 2,
            mailbox: "INBOX",
            message_id: "<henry-msg@personal.example>",
            thread_id: Some("<friends-trip@personal.example>"),
            from_addr: "\"Henry Park\" <henry.park@personal.example>",
            to_addrs: "\"Demo User\" <me@personal.example>",
            subject: "Summer trip plans!",
            days_ago: 1,
            flags: r#"["\\Seen"]"#,
            body_text: "Hey! Are you still up for the Portugal trip in July? I found some great flights — under 200 EUR return if we book this week. Katherine and Sarah are in too.",
            body_html: r##"<div style="font-family:sans-serif;font-size:14px;line-height:1.6;color:#222"><p>Hey! Are you still up for the Portugal trip in July? Found flights <strong>under 200 EUR return</strong> if we book this week. Katherine and Sarah are in too.</p></div>"##,
            is_unread: false,
        },
        MessageSeed {
            account: DEMO_PERSONAL_ACCOUNT,
            uid: 3,
            mailbox: "INBOX",
            message_id: "<katherine-reply@personal.example>",
            thread_id: Some("<friends-trip@personal.example>"),
            from_addr: "\"Katherine Hayes\" <kath.hayes@personal.example>",
            to_addrs: "\"Demo User\" <me@personal.example>",
            subject: "Re: Summer trip plans!",
            days_ago: 0,
            flags: r#"[]"#,
            body_text: "100% in! Porto is incredible. Can we do the 12th? That works best for me. Sarah?",
            body_html: r##"<div style="font-family:sans-serif;font-size:14px;line-height:1.6;color:#222"><p>100% in! Porto is incredible. Can we do the <strong>12th</strong>? That works best for me. Sarah?</p></div>"##,
            is_unread: true,
        },
        MessageSeed {
            account: DEMO_PERSONAL_ACCOUNT,
            uid: 4,
            mailbox: "INBOX",
            message_id: "<dentist-reminder@personal.example>",
            thread_id: None,
            from_addr: "\"Reed Dental\" <appointments@reeddental.example>",
            to_addrs: "\"Demo User\" <me@personal.example>",
            subject: "Appointment reminder — Tuesday 9:30",
            days_ago: 2,
            flags: r#"["\\Seen"]"#,
            body_text: "This is a reminder for your appointment with Dr. Reed on Tuesday at 9:30 AM. Please arrive 5 minutes early.",
            body_html: r##"<div style="font-family:sans-serif;font-size:14px;line-height:1.6;color:#222;max-width:400px"><div style="background:#e8f4fd;border-left:4px solid #2196f3;padding:16px;border-radius:4px"><p style="margin:0 0 8px;font-weight:600">Appointment Reminder</p><p style="margin:0;color:#555">Dr. Reed · Tuesday · <strong>9:30 AM</strong></p></div></div>"##,
            is_unread: false,
        },
        MessageSeed {
            account: DEMO_PERSONAL_ACCOUNT,
            uid: 5,
            mailbox: "INBOX",
            message_id: "<order-confirm@personal.example>",
            thread_id: None,
            from_addr: "\"Online Shop\" <orders@shop.example>",
            to_addrs: "\"Demo User\" <me@personal.example>",
            subject: "Order #78234 confirmed — delivery Thu 5 June",
            days_ago: 3,
            flags: r#"["\\Seen"]"#,
            body_text: "Thanks for your order! Your package will be delivered on Thursday 5 June between 08:00 and 18:00.\n\nOrder: #78234\nDelivery: Thu 5 June\nAddress: 12 Demo Street",
            body_html: r##"<div style="font-family:sans-serif;font-size:14px;line-height:1.6;color:#222;max-width:460px"><p>Order #78234 confirmed. Delivery Thu 5 June, 08:00–18:00, 12 Demo Street.</p></div>"##,
            is_unread: false,
        },
        MessageSeed {
            account: DEMO_PERSONAL_ACCOUNT,
            uid: 6,
            mailbox: "INBOX",
            message_id: "<tech-newsletter@personal.example>",
            thread_id: None,
            from_addr: "\"Tech Weekly\" <newsletter@techweekly.example>",
            to_addrs: "\"Demo User\" <me@personal.example>",
            subject: "Tech Weekly #312 — AI tools, Rust 2.0 preview, WebAssembly",
            days_ago: 6,
            flags: r#"["\\Seen"]"#,
            body_text: "Tech Weekly #312\n\n1. AI coding assistants hit 40% developer adoption\n2. Rust 2.0 RFC discussion begins\n3. WebAssembly components land in Firefox Nightly\n4. SQLite 3.47 adds JSON5 support",
            body_html: r##"<div style="font-family:sans-serif;font-size:14px;line-height:1.7;color:#222;max-width:540px"><div style="border-bottom:2px solid #222;padding-bottom:8px;margin-bottom:16px"><strong style="font-size:18px">Tech Weekly</strong> <span style="color:#888">#312</span></div><ol style="padding-left:20px"><li style="margin-bottom:10px"><strong>AI coding assistants hit 40% developer adoption</strong></li><li style="margin-bottom:10px"><strong>Rust 2.0 RFC discussion begins</strong></li><li style="margin-bottom:10px"><strong>WebAssembly components land in Firefox Nightly</strong></li><li style="margin-bottom:10px"><strong>SQLite 3.47 adds JSON5 support</strong></li></ol></div>"##,
            is_unread: false,
        },
        MessageSeed {
            account: DEMO_PERSONAL_ACCOUNT,
            uid: 7,
            mailbox: "INBOX",
            message_id: "<gym-invoice@personal.example>",
            thread_id: None,
            from_addr: "\"City Fitness\" <billing@cityfitness.example>",
            to_addrs: "\"Demo User\" <me@personal.example>",
            subject: "June membership invoice — EUR 45.00",
            days_ago: 4,
            flags: r#"[]"#,
            body_text: "Your June membership has been renewed.\n\nAmount: EUR 45.00\nCard: ending 4821\nMembership: All-Access Monthly",
            body_html: r##"<div style="font-family:sans-serif;font-size:14px;line-height:1.6;color:#222;max-width:400px"><p>June membership renewed. EUR 45.00 charged to card ending 4821.</p></div>"##,
            is_unread: true,
        },
        MessageSeed {
            account: DEMO_PERSONAL_ACCOUNT,
            uid: 8,
            mailbox: "INBOX",
            message_id: "<sarah-msg@personal.example>",
            thread_id: Some("<friends-trip@personal.example>"),
            from_addr: "\"Sarah Cohen\" <sarah.cohen@personal.example>",
            to_addrs: "\"Demo User\" <me@personal.example>",
            subject: "Re: Summer trip plans!",
            days_ago: 0,
            flags: r#"[]"#,
            body_text: "The 12th works perfectly! I'll look at Airbnb options for a 5-night stay. Budget around 80/night per room?",
            body_html: r##"<div style="font-family:sans-serif;font-size:14px;line-height:1.6;color:#222"><p>The 12th works perfectly! I'll look at Airbnb options for a 5-night stay. Budget around <strong>80/night</strong> per room?</p></div>"##,
            is_unread: true,
        },
        // Personal Sent
        MessageSeed {
            account: DEMO_PERSONAL_ACCOUNT,
            uid: 1,
            mailbox: "Sent",
            message_id: "<sent-henry@personal.example>",
            thread_id: Some("<friends-trip@personal.example>"),
            from_addr: "\"Demo User\" <me@personal.example>",
            to_addrs: "\"Henry Park\" <henry.park@personal.example>",
            subject: "Re: Summer trip plans!",
            days_ago: 1,
            flags: r#"["\\Seen"]"#,
            body_text: "Yes! I'm in. Let's do it. The 12th works for me too. Book those flights!",
            body_html: r##"<div style="font-family:sans-serif;font-size:14px;line-height:1.6;color:#222"><p>Yes! I'm in. Let's do it. The 12th works for me too. Book those flights!</p></div>"##,
            is_unread: false,
        },
    ];

    for m in messages {
        let sent_at: i64 =
            sqlx::query_scalar(&format!("SELECT unixepoch('now', '-{} days')", m.days_ago))
                .fetch_one(pool)
                .await?;

        let from_lower = extract_email_addr(m.from_addr);

        sqlx::query(
            "INSERT OR IGNORE INTO cached_messages
                (account_id, mailbox, uid, message_id, thread_id, from_addr, to_addrs,
                 subject, sent_at, flags, subject_idx, body_idx, from_idx, is_unread)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(m.account)
        .bind(m.mailbox)
        .bind(m.uid)
        .bind(m.message_id)
        .bind(m.thread_id)
        .bind(m.from_addr)
        .bind(m.to_addrs)
        .bind(m.subject)
        .bind(sent_at)
        .bind(m.flags)
        .bind(m.subject)
        .bind(m.body_text)
        .bind(&from_lower)
        .bind(m.is_unread as i64)
        .execute(pool)
        .await?;

        sqlx::query(
            "INSERT INTO cached_message_bodies
                (account_id, mailbox, uid, body_text, body_html, autocrypt_headers, is_signed)
             VALUES (?, ?, ?, ?, ?, '[]', 0)
             ON CONFLICT(account_id, mailbox, uid) DO UPDATE SET
                body_text = excluded.body_text,
                body_html = excluded.body_html",
        )
        .bind(m.account)
        .bind(m.mailbox)
        .bind(m.uid)
        .bind(m.body_text)
        .bind(m.body_html)
        .execute(pool)
        .await?;

        sqlx::query(
            "INSERT OR IGNORE INTO message_addresses (account_id, mailbox, uid, field, addr)
             VALUES (?, ?, ?, 'from', ?)",
        )
        .bind(m.account)
        .bind(m.mailbox)
        .bind(m.uid)
        .bind(&from_lower)
        .execute(pool)
        .await?;

        // Write the .eml file so the frontend can render via MIME parse.
        let path = crate::email_sync::eml_path(data_dir, m.account, m.mailbox, m.uid as u32);
        if !path.exists() {
            if let Some(dir) = path.parent() {
                tokio::fs::create_dir_all(dir).await?;
            }
            let mime = build_mime(m, sent_at);
            let compressed = zstd::encode_all(mime.as_bytes(), 3)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
            tokio::fs::write(&path, compressed).await?;
        }
    }
    Ok(())
}

fn extract_email_addr(display: &str) -> String {
    if let Some(start) = display.find('<') {
        if let Some(end) = display.find('>') {
            return display[start + 1..end].trim().to_ascii_lowercase();
        }
    }
    display.trim().to_ascii_lowercase()
}

#[allow(dead_code)]
fn strip_html(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for ch in html.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out
}
