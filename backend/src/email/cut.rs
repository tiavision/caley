//! Cut messages (sinter-mail's docs/mail.md, "Large attachments: cut messages"): the raw message without the bodies of
//! its large attachments, and where they were (`holes`), so the reader renders the text without downloading a PDF
//! first and fetches a cut body by Range when it is opened. sinter-mail's reader checks every hole against its own
//! reading of the message, so this parser need not be its equal: a cut it does not accept costs a full download,
//! never a different reading. To be one it accepts, this follows its rules (its `mail.cut`, sinter/mime/cut.ts):
//! only leaf bodies of types the reader never reads, whose headers ended with a blank line, with no line in them
//! starting with `--` and not ending in a CR; and nothing at all in a message whose structure is anything but plain
//! (duplicate or unreadable structural fields, control bytes in them, a bad or nested-prefix boundary, an unclosed
//! multipart, an encoded multipart or message). A ciphertext the reader decrypts without asking is never cut (it would
//! fetch the whole message at once anyway): the encrypted structure at the message's root (multipart/encrypted, or
//! Exchange's multipart/mixed of a `Version: 1` application/pgp-encrypted part and an octet-stream one), or as the
//! first part of a root multipart/signed. One the reader opens only on request (in a forward, in a plain mail) is.

use serde::Serialize;

/// A body cut out: its range `[start, end)` in the whole message and its decoded size.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Hole {
    pub start: usize,
    pub end: usize,
    pub size: usize,
}

/// How much is cut (sinter-mail's defaults).
pub struct CutOptions {
    /// a body longer than this (encoded bytes) is cut
    pub over: usize,
    /// the same inside a multipart/related (an inline image)
    pub related_over: usize,
    /// at most this many holes, the largest bodies first
    pub max: usize,
}

impl Default for CutOptions {
    fn default() -> Self {
        Self { over: 64 * 1024, related_over: 512 * 1024, max: 32 }
    }
}

/// `raw` without the bodies of its large leaf parts, and where they were. No holes (and `raw` as it is) when there is
/// nothing to cut or the structure is anything but plain.
pub fn cut(raw: &[u8], options: &CutOptions) -> (Vec<u8>, Vec<Hole>) {
    let mut leaves = Vec::new();
    if scan(raw, 0, raw.len(), Context { parent: None, auto: true, ciphertext: false }, &mut Vec::new(), 0, &mut leaves).is_err() {
        return (raw.to_vec(), Vec::new());
    }
    let mut chosen: Vec<&Leaf> = leaves
        .iter()
        .filter(|l| {
            let limit = if l.in_related { options.related_over } else { options.over };
            l.end - l.body_start > limit && cuttable(raw, l.body_start, l.end)
        })
        .collect();
    chosen.sort_by(|a, b| (b.end - b.body_start).cmp(&(a.end - a.body_start)).then(a.body_start.cmp(&b.body_start)));
    chosen.truncate(options.max);
    chosen.sort_by_key(|l| l.body_start);
    let mut out = Vec::with_capacity(raw.len() - chosen.iter().map(|l| l.end - l.body_start).sum::<usize>());
    let mut holes = Vec::with_capacity(chosen.len());
    let mut from = 0;
    for l in chosen {
        out.extend_from_slice(&raw[from..l.body_start]);
        holes.push(Hole { start: l.body_start, end: l.end, size: decoded_size(&raw[l.body_start..l.end], l.encoding) });
        from = l.end;
    }
    out.extend_from_slice(&raw[from..]);
    (out, holes)
}

#[derive(Debug)]
struct Malformed;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Encoding {
    Identity,
    Base64,
    QuotedPrintable,
}

struct Leaf {
    body_start: usize,
    end: usize,
    encoding: Encoding,
    in_related: bool,
}

const MAX_DEPTH: usize = 32;

/// Where an entity is: its parent's type, whether an encrypted structure here is decrypted without asking (`auto`:
/// the message's root, or the first part of a root multipart/signed), and whether its parent is such a structure
/// (`ciphertext`: its octet-stream part is the ciphertext, never cut).
#[derive(Clone, Copy)]
struct Context<'a> {
    parent: Option<&'a str>,
    auto: bool,
    ciphertext: bool,
}

/// Reads the entity at `[start, end)`, adding the leaves that may be cut. Err for anything but plain structure.
fn scan(
    raw: &[u8],
    start: usize,
    end: usize,
    at: Context,
    boundaries: &mut Vec<Vec<u8>>,
    depth: usize,
    leaves: &mut Vec<Leaf>,
) -> Result<(), Malformed> {
    let parent_type = at.parent;
    if depth > MAX_DEPTH {
        return Err(Malformed);
    }
    let head = headers(raw, start, end)?;
    let (kind, params) = match &head.content_type {
        Some(v) => content_type(v)?,
        None if parent_type == Some("multipart/digest") => ("message/rfc822".to_owned(), Vec::new()),
        None => ("text/plain".to_owned(), Vec::new()),
    };
    let encoding = match head.encoding.as_deref().map(|e| e.trim().to_ascii_lowercase()) {
        None => Some(Encoding::Identity),
        Some(e) => match e.as_str() {
            "7bit" | "8bit" | "binary" => Some(Encoding::Identity),
            "base64" => Some(Encoding::Base64),
            "quoted-printable" => Some(Encoding::QuotedPrintable),
            _ => None,
        },
    };
    let body_start = head.body_start;

    if kind.starts_with("multipart/") {
        if encoding != Some(Encoding::Identity) || !head.ended {
            return Err(Malformed);
        }
        let boundary = params
            .iter()
            .find(|(k, _)| k == "boundary")
            .map(|(_, v)| v.as_bytes().to_vec())
            .ok_or(Malformed)?;
        if !valid_boundary(&boundary)
            || boundaries.iter().any(|b| b.starts_with(&boundary) || boundary.starts_with(b))
        {
            return Err(Malformed);
        }
        let parts = split_multipart(raw, body_start, end, &boundary)?;
        let ciphertext = at.auto && (kind == "multipart/encrypted" || (kind == "multipart/mixed" && exchange_encrypted(raw, &parts)));
        boundaries.push(boundary);
        for (i, (s, e)) in parts.into_iter().enumerate() {
            let auto = at.auto && kind == "multipart/signed" && i == 0;
            scan(raw, s, e, Context { parent: Some(&kind), auto, ciphertext }, boundaries, depth + 1, leaves)?;
        }
        boundaries.pop();
    } else if kind == "message/rfc822" || kind == "message/global" {
        if encoding != Some(Encoding::Identity) || !head.ended {
            return Err(Malformed);
        }
        scan(raw, body_start, end, Context { parent: None, auto: false, ciphertext: false }, boundaries, depth + 1, leaves)?;
    } else if let Some(encoding) = encoding
        && head.ended
        && !kind.starts_with("text/")
        && !kind.starts_with("message/")
        && kind != "application/pgp-encrypted"
        && kind != "application/pgp-signature"
        && !(at.ciphertext && kind == "application/octet-stream")
    {
        leaves.push(Leaf { body_start, end, encoding, in_related: parent_type == Some("multipart/related") });
    }
    Ok(())
}

/// Exchange's rewrite of PGP/MIME: a multipart/mixed whose parts include an application/pgp-encrypted one saying
/// `Version: 1` and an application/octet-stream one (the ciphertext).
fn exchange_encrypted(raw: &[u8], parts: &[(usize, usize)]) -> bool {
    let kind = |(s, e): (usize, usize)| {
        let head = headers(raw, s, e).ok()?;
        let (kind, _) = content_type(head.content_type.as_deref()?).ok()?;
        Some((kind, head.body_start, e))
    };
    let kinds: Vec<_> = parts.iter().filter_map(|p| kind(*p)).collect();
    kinds.iter().any(|(k, b, e)| {
        k == "application/pgp-encrypted" && String::from_utf8_lossy(&raw[*b..*e]).lines().any(|l| l.trim() == "Version: 1")
    }) && kinds.iter().any(|(k, _, _)| k == "application/octet-stream")
}

struct Head {
    content_type: Option<String>,
    encoding: Option<String>,
    /// just past the blank line, or `end` when the headers ran out
    body_start: usize,
    /// whether a blank line ended them
    ended: bool,
}

/// The entity's structural header fields (unfolded), and where its body starts.
fn headers(raw: &[u8], start: usize, end: usize) -> Result<Head, Malformed> {
    let mut head = Head { content_type: None, encoding: None, body_start: end, ended: false };
    let mut fields: Vec<(usize, usize)> = Vec::new(); // each field's [start, end) including continuations
    let mut at = start;
    while at < end {
        let line_end = memchr(b'\n', &raw[at..end]).map_or(end, |i| at + i + 1);
        let line = &raw[at..line_end];
        let content = strip_eol(line);
        if content.is_empty() {
            head.body_start = line_end;
            head.ended = true;
            break;
        }
        if content[0] == b' ' || content[0] == b'\t' {
            match fields.last_mut() {
                Some(f) => f.1 = line_end,
                None => return Err(Malformed),
            }
        } else {
            fields.push((at, line_end));
        }
        at = line_end;
    }
    for (s, e) in fields {
        let field = &raw[s..e];
        let colon = memchr(b':', field).ok_or(Malformed)?;
        let name = &field[..colon];
        if name.is_empty() || !name.iter().all(|&c| (0x21..=0x7e).contains(&c)) {
            return Err(Malformed);
        }
        let slot = if name.eq_ignore_ascii_case(b"content-type") {
            &mut head.content_type
        } else if name.eq_ignore_ascii_case(b"content-transfer-encoding") {
            &mut head.encoding
        } else if name.eq_ignore_ascii_case(b"content-disposition") {
            // read only for control bytes, as the reader would block on them
            unfold(&field[colon + 1..])?;
            continue;
        } else {
            continue;
        };
        if slot.is_some() {
            return Err(Malformed);
        }
        *slot = Some(unfold(&field[colon + 1..])?);
    }
    Ok(head)
}

/// A field's value without its line breaks; Err for other control bytes or non-ASCII in a structural field.
fn unfold(value: &[u8]) -> Result<String, Malformed> {
    let mut out = String::with_capacity(value.len());
    for &c in value {
        match c {
            b'\r' | b'\n' => {}
            b'\t' => out.push(' '),
            0x20..=0x7e => out.push(c as char),
            _ => return Err(Malformed),
        }
    }
    Ok(out)
}

/// `type/subtype` lowercased and the parameters (names lowercased, quoted values unquoted).
fn content_type(value: &str) -> Result<(String, Vec<(String, String)>), Malformed> {
    let mut pieces = split_params(value)?.into_iter();
    let kind = pieces.next().ok_or(Malformed)?.trim().to_ascii_lowercase();
    let (t, s) = kind.split_once('/').ok_or(Malformed)?;
    if !is_token(t) || !is_token(s) {
        return Err(Malformed);
    }
    let mut params = Vec::new();
    for p in pieces {
        let p = p.trim();
        if p.is_empty() {
            continue;
        }
        let (k, v) = p.split_once('=').ok_or(Malformed)?;
        let k = k.trim().to_ascii_lowercase();
        if k.contains('*') && k.starts_with("boundary") {
            return Err(Malformed); // RFC 2231 boundaries: not read here
        }
        let v = v.trim();
        let v = if let Some(q) = v.strip_prefix('"') {
            let q = q.strip_suffix('"').ok_or(Malformed)?;
            let mut out = String::new();
            let mut escaped = false;
            for ch in q.chars() {
                if escaped {
                    out.push(ch);
                    escaped = false;
                } else if ch == '\\' {
                    escaped = true;
                } else {
                    out.push(ch);
                }
            }
            out
        } else {
            v.to_owned()
        };
        if params.iter().any(|(n, _): &(String, String)| *n == k) {
            return Err(Malformed);
        }
        params.push((k, v));
    }
    Ok((kind, params))
}

/// `value` split at the semicolons outside quoted strings and comments are refused (none in a plain Content-Type).
fn split_params(value: &str) -> Result<Vec<&str>, Malformed> {
    let mut out = Vec::new();
    let (mut from, mut quoted, mut escaped) = (0, false, false);
    for (i, ch) in value.char_indices() {
        if escaped {
            escaped = false;
        } else if quoted && ch == '\\' {
            escaped = true;
        } else if ch == '"' {
            quoted = !quoted;
        } else if !quoted && ch == '(' {
            return Err(Malformed);
        } else if !quoted && ch == ';' {
            out.push(&value[from..i]);
            from = i + 1;
        }
    }
    if quoted {
        return Err(Malformed);
    }
    out.push(&value[from..]);
    Ok(out)
}

fn is_token(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|c| c.is_ascii_graphic() && !b"()<>@,;:\\\"/[]?=".contains(&c))
}

/// RFC 2046's boundary: 1 to 70 of its characters, not ending in a space.
fn valid_boundary(b: &[u8]) -> bool {
    (1..=70).contains(&b.len())
        && b.last() != Some(&b' ')
        && b.iter().all(|c| c.is_ascii_alphanumeric() || b"'()+_,-./:=? ".contains(c))
}

/// The parts of a multipart body: each `[start, end)`, end just before the line break of the delimiter after it.
/// Err without a close delimiter, without parts, or with a line that starts like a delimiter and is none.
fn split_multipart(raw: &[u8], body_start: usize, end: usize, boundary: &[u8]) -> Result<Vec<(usize, usize)>, Malformed> {
    let mut parts = Vec::new();
    let mut open: Option<usize> = None; // where the current part starts
    let mut at = body_start;
    while at < end {
        let line_end = memchr(b'\n', &raw[at..end]).map_or(end, |i| at + i + 1);
        let line = strip_eol(&raw[at..line_end]);
        if let Some(rest) = line.strip_prefix(b"--").and_then(|l| l.strip_prefix(boundary)) {
            let close = rest.starts_with(b"--");
            if !close && !rest.iter().all(|&c| c == b' ' || c == b'\t') {
                return Err(Malformed);
            }
            if let Some(s) = open {
                let mut e = at;
                if e > body_start && raw[e - 1] == b'\n' {
                    e -= 1;
                    if e > body_start && raw[e - 1] == b'\r' {
                        e -= 1;
                    }
                }
                parts.push((s, e.max(s)));
            }
            if close {
                return if parts.is_empty() { Err(Malformed) } else { Ok(parts) };
            }
            open = Some(line_end);
        }
        at = line_end;
    }
    Err(Malformed)
}

/// No line in the bytes starts with `--`, and they do not end in a CR (sinter-mail's `cuttable`): a cut cannot hide
/// or make a delimiter.
fn cuttable(raw: &[u8], start: usize, end: usize) -> bool {
    for k in start..end.saturating_sub(1) {
        if raw[k] == b'-' && raw[k + 1] == b'-' && (k == start || raw[k - 1] == b'\n' || raw[k - 1] == b'\r') {
            return false;
        }
    }
    !(end > start && raw[end - 1] == b'\r')
}

/// The size of the body decoded: base64's characters, quoted-printable's bytes without its escapes and soft breaks.
fn decoded_size(body: &[u8], encoding: Encoding) -> usize {
    match encoding {
        Encoding::Identity => body.len(),
        Encoding::Base64 => {
            // in quanta of four, each 3 bytes less its padding; a short last one without padding, one less than it
            let (mut n, mut quantum, mut pad) = (0usize, 0usize, 0usize);
            for &c in body {
                if c.is_ascii_alphanumeric() || c == b'+' || c == b'/' || c == b'=' {
                    quantum += 1;
                    pad += usize::from(c == b'=');
                    if quantum == 4 {
                        n += 3usize.saturating_sub(pad);
                        (quantum, pad) = (0, 0);
                    }
                }
            }
            n + quantum.saturating_sub(pad).saturating_sub(1)
        }
        Encoding::QuotedPrintable => {
            let (mut n, mut i) = (0, 0);
            while i < body.len() {
                if body[i] == b'=' {
                    if body.get(i + 1) == Some(&b'\r') && body.get(i + 2) == Some(&b'\n') {
                        i += 3;
                        continue;
                    }
                    if body.get(i + 1) == Some(&b'\n') {
                        i += 2;
                        continue;
                    }
                    if i + 2 < body.len() && body[i + 1].is_ascii_hexdigit() && body[i + 2].is_ascii_hexdigit() {
                        n += 1;
                        i += 3;
                        continue;
                    }
                }
                n += 1;
                i += 1;
            }
            n
        }
    }
}

fn strip_eol(line: &[u8]) -> &[u8] {
    let line = line.strip_suffix(b"\n").unwrap_or(line);
    line.strip_suffix(b"\r").unwrap_or(line)
}

fn memchr(needle: u8, hay: &[u8]) -> Option<usize> {
    hay.iter().position(|&c| c == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn small() -> CutOptions {
        CutOptions { over: 8, related_over: 8, max: 32 }
    }

    fn mail(attachment: &str) -> String {
        format!(
            "From: a@example.com\r\nSubject: s\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=\"b\"\r\n\r\n\
             --b\r\nContent-Type: text/plain\r\n\r\nHello there, a long enough text.\r\n\
             --b\r\nContent-Type: application/pdf\r\nContent-Transfer-Encoding: base64\r\n\r\n{attachment}\r\n--b--\r\n"
        )
    }

    #[test]
    fn cuts_an_attachment_body_but_not_the_text() {
        let raw = mail("QUJDREVGR0hJSktMTU5PUA==\r\nQUJDRA==");
        let (out, holes) = cut(raw.as_bytes(), &small());
        assert_eq!(holes.len(), 1);
        let h = &holes[0];
        assert_eq!(&raw[h.start..h.end], "QUJDREVGR0hJSktMTU5PUA==\r\nQUJDRA==");
        assert_eq!(h.size, 20); // 16 + 4 bytes, the padding not counted
        assert_eq!(out.len(), raw.len() - (h.end - h.start));
        assert!(String::from_utf8(out).unwrap().contains("Hello there"));
    }

    #[test]
    fn keeps_small_bodies_and_everything_of_malformed_structure() {
        let raw = mail("QUJD");
        assert!(cut(raw.as_bytes(), &CutOptions::default()).1.is_empty());
        let unclosed = mail("QUJDREVGR0hJSktMTU5PUA==").replace("--b--", "");
        assert!(cut(unclosed.as_bytes(), &small()).1.is_empty());
        let twice = mail("QUJDREVGR0hJSktMTU5PUA==")
            .replace("Content-Type: application/pdf", "Content-Type: application/pdf\r\nContent-Type: text/plain");
        assert!(cut(twice.as_bytes(), &small()).1.is_empty());
    }

    #[test]
    fn never_cuts_a_body_with_a_line_starting_with_two_dashes() {
        let raw = mail("QUJDREVGR0hJSktMTU5PUA==\r\n--not-a-boundary");
        assert!(cut(raw.as_bytes(), &small()).1.is_empty());
    }

    fn encrypted(root: &str) -> String {
        let cipher = "QUJDREVGR0hJSktMTU5PUA==";
        let inner = format!(
            "Content-Type: {root}; boundary=\"e\"\r\n\r\n--e\r\nContent-Type: application/pgp-encrypted\r\n\r\nVersion: 1\r\n\r\n\
             --e\r\nContent-Type: application/octet-stream\r\nContent-Transfer-Encoding: base64\r\n\r\n{cipher}\r\n--e--\r\n"
        );
        format!("From: a@example.com\r\nMIME-Version: 1.0\r\n{inner}")
    }

    #[test]
    fn never_cuts_a_ciphertext_the_reader_decrypts_at_once() {
        assert!(cut(encrypted("multipart/encrypted").as_bytes(), &small()).1.is_empty());
        assert!(cut(encrypted("multipart/mixed").as_bytes(), &small()).1.is_empty()); // Exchange's form
        let sig = "-----BEGIN PGP SIGNATURE-----\r\nx\r\n-----END PGP SIGNATURE-----";
        let signed = format!(
            "From: a@example.com\r\nContent-Type: multipart/signed; boundary=\"s\"\r\n\r\n--s\r\n{}\r\n--s\r\nContent-Type: application/pgp-signature\r\n\r\n{sig}\r\n--s--\r\n",
            encrypted("multipart/encrypted").trim_start_matches("From: a@example.com\r\nMIME-Version: 1.0\r\n").trim_end_matches("\r\n"),
        );
        assert!(cut(signed.as_bytes(), &small()).1.is_empty()); // sign over encrypt
    }

    #[test]
    fn cuts_a_ciphertext_opened_only_on_request() {
        let forward = encrypted("multipart/encrypted");
        let raw = format!(
            "Content-Type: multipart/mixed; boundary=\"m\"\r\n\r\n--m\r\nContent-Type: text/plain\r\n\r\nSee below.\r\n\
             --m\r\nContent-Type: message/rfc822\r\n\r\n{}\r\n--m--\r\n",
            forward.trim_end_matches("\r\n")
        );
        let (_, holes) = cut(raw.as_bytes(), &small());
        assert_eq!(holes.len(), 1);
        assert_eq!(&raw[holes[0].start..holes[0].end], "QUJDREVGR0hJSktMTU5PUA==");
    }

    #[test]
    fn the_largest_first_up_to_max() {
        let raw = "Content-Type: multipart/mixed; boundary=b\n\n--b\nContent-Type: application/octet-stream\n\n0123456789\n\
                   --b\nContent-Type: application/octet-stream\n\n0123456789abcdef\n--b--\n";
        let (_, holes) = cut(raw.as_bytes(), &CutOptions { over: 4, related_over: 4, max: 1 });
        assert_eq!(holes.len(), 1);
        assert_eq!(&raw[holes[0].start..holes[0].end], "0123456789abcdef");
    }
}

/// Cross-check against sinter-mail's reader: `CUT_CORPUS=<dir of .eml> CUT_OUT=<dir> cargo test cut_corpus` writes
/// each message cut with small limits (`<name>.cut`, `<name>.holes.json`), for a script that reads them with
/// `mail.mime(raw, { holes })` and expects `cut: 'valid'`.
#[cfg(test)]
#[test]
fn cut_corpus() {
    let (Ok(dir), Ok(out)) = (std::env::var("CUT_CORPUS"), std::env::var("CUT_OUT")) else { return };
    let mut stack = vec![std::path::PathBuf::from(dir)];
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().is_none_or(|e| e != "eml") {
                continue;
            }
            let raw = std::fs::read(&path).unwrap();
            let (cut_raw, holes) = cut(&raw, &CutOptions { over: 16, related_over: 16, max: 32 });
            let name = path.to_string_lossy().replace('/', "_");
            std::fs::write(format!("{out}/{name}.cut"), cut_raw).unwrap();
            std::fs::write(format!("{out}/{name}.holes.json"), serde_json::to_string(&holes).unwrap()).unwrap();
            std::fs::write(format!("{out}/{name}.src"), path.to_string_lossy().as_bytes()).unwrap();
        }
    }
}
