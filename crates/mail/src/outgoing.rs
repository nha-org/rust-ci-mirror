//! Around a mail written elsewhere and sent from here by `submit` (task outbound-gate, `dash mail send`): the mail it answers,
//! read from that mail's own file or found by its subject, whether he already sent it from his own client, and where a file
//! is uploaded. Ported from klar-mail's answered, original_id and sent_by_him, over JMAP where they spoke IMAP.
use crate::jmap::{Http, SESSION, called, session};
use serde_json::{Value, json};

/// The mail a reply answers: its subject, its Message-ID and the ids its thread already lists, each without brackets.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Answered {
    pub subject: String,
    pub message_id: Option<String>,
    pub references: Vec<String>,
}

fn bare(id: &str) -> &str {
    id.trim().trim_start_matches('<').trim_end_matches('>')
}

fn base64(text: &str) -> Vec<u8> {
    let (mut out, mut acc, mut bits) = (vec![], 0u32, 0);
    for b in text.bytes() {
        let v = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => continue,
        };
        acc = (acc << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    out
}

/// A header's words with its RFC 2047 encoded words decoded: UTF-8, else each byte a character (Latin-1), which a subject
/// from an older client uses. ponytail: no other charset; one that shows up reads as Latin-1, legible in most of Europe.
fn decoded(value: &str) -> String {
    let mut out = String::new();
    let mut rest = value;
    let mut after_word = false;
    while let Some(at) = rest.find("=?") {
        let word = (|| {
            let mut parts = rest[at + 2..].splitn(3, '?');
            let (charset, kind, tail) = (parts.next()?, parts.next()?, parts.next()?);
            let end = tail.find("?=")?;
            let bytes = match kind {
                "B" | "b" => base64(&tail[..end]),
                "Q" | "q" => {
                    let (mut bytes, raw) = (vec![], &tail.as_bytes()[..end]);
                    let mut i = 0;
                    while i < raw.len() {
                        let hex = (raw[i] == b'=')
                            .then(|| tail.get(i + 1..i + 3))
                            .flatten()
                            .and_then(|h| u8::from_str_radix(h, 16).ok());
                        match (hex, raw[i]) {
                            (Some(b), _) => {
                                bytes.push(b);
                                i += 2;
                            }
                            (None, b'_') => bytes.push(b' '),
                            (None, b) => bytes.push(b),
                        }
                        i += 1;
                    }
                    bytes
                }
                _ => return None,
            };
            let text = match charset.eq_ignore_ascii_case("utf-8") {
                true => String::from_utf8_lossy(&bytes).into_owned(),
                false => bytes.iter().map(|b| *b as char).collect(),
            };
            Some((text, at + 2 + charset.len() + 1 + kind.len() + 1 + end + 2))
        })();
        let Some((text, end)) = word else { break };
        // the space between two encoded words is no part of the text
        if !(after_word && rest[..at].trim().is_empty()) {
            out.push_str(&rest[..at]);
        }
        out.push_str(&text);
        rest = &rest[end..];
        after_word = true;
    }
    out.push_str(rest);
    out
}

/// A saved mail's own headers: its subject, its Message-ID, and its References with that id last, so a reply keeps the
/// thread. A folded header is read whole; the body is not read.
pub fn answered(eml: &str) -> Answered {
    let mut headers: Vec<(String, String)> = vec![];
    for line in eml.lines() {
        if line.is_empty() {
            break;
        }
        match (line.starts_with([' ', '\t']), headers.last_mut()) {
            (true, Some(last)) => {
                last.1.push(' ');
                last.1.push_str(line.trim());
            }
            _ => {
                if let Some((name, value)) = line.split_once(':') {
                    headers.push((name.trim().to_ascii_lowercase(), value.trim().to_string()));
                }
            }
        }
    }
    let said = |name: &str| {
        let found = headers.iter().find(|(n, _)| n == name);
        found.map_or("", |(_, v)| v.as_str())
    };
    let message_id = Some(bare(said("message-id")).to_string()).filter(|id| !id.is_empty());
    let mut references: Vec<String> = (said("references").split_whitespace())
        .map(|r| bare(r).to_string())
        .collect();
    references.extend(message_id.clone());
    references.dedup();
    Answered {
        subject: decoded(said("subject")),
        message_id,
        references,
    }
}

/// The folder of `role`, if the account has one.
fn folder(http: Http, api: &str, account: &str, role: &str) -> Result<Option<String>, String> {
    let r = called(
        http,
        api,
        json!([["Mailbox/query", {"accountId": account, "filter": {"role": role}}, "b"]]),
    )?;
    Ok(r["b"]["ids"][0].as_str().map(String::from))
}

/// The mails of `folder` matching `filter`, newest first, each its messageId and subject.
fn found(
    http: Http,
    api: &str,
    account: &str,
    folder: &str,
    mut filter: Value,
) -> Result<Vec<Value>, String> {
    filter["inMailbox"] = json!(folder);
    let r = called(
        http,
        api,
        json!([
            ["Email/query", {"accountId": account, "filter": filter,
                             "sort": [{"property": "receivedAt", "isAscending": false}], "limit": 50}, "q"],
            ["Email/get", {"accountId": account, "properties": ["messageId", "subject"],
                           "#ids": {"resultOf": "q", "name": "Email/query", "path": "/ids"}}, "e"],
        ]),
    )?;
    Ok(r["e"]["list"].as_array().cloned().unwrap_or_default())
}

fn words(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The server matches a subject by its words: only the mail whose subject is this one exactly counts, never a later
/// « Re: » of it.
fn titled(mail: &Value, subject: &str) -> bool {
    words(mail["subject"].as_str().unwrap_or_default()) == words(subject)
}

/// The Message-ID of the newest mail with this subject, in the inbox, else in Sent (a reply to his own mail), so a reply
/// threads in the receiver's client. None found: none, and the reply goes unthreaded.
pub fn original_id(http: Http, subject: &str) -> Result<Option<String>, String> {
    let (api, account) = session(http)?;
    for role in ["inbox", "sent"] {
        let Some(id) = folder(http, &api, &account, role)? else {
            continue;
        };
        let mails = found(http, &api, &account, &id, json!({"subject": subject}))?;
        if let Some(m) = mails.iter().find(|m| titled(m, subject)) {
            return Ok(m["messageId"][0].as_str().map(String::from));
        }
    }
    Ok(None)
}

/// Our copy of a mail in his Drafts, as the mirror recorded it: its Message-ID, its receiver, its subject, and when it was
/// made (UTC, ISO).
#[derive(Clone, Debug, PartialEq)]
pub struct Copy {
    pub message_id: String,
    pub to: String,
    pub subject: String,
    pub made: String,
}

/// Whether he sent `copy` himself from his client: it left his Drafts (ours gone, and no other mail there to the same
/// address with its subject but the copies `others` names, each another mail's) while Sent got a mail to that address with
/// that subject since the copy was made. Approved on the phone, then sent from the Mac before the mirror saw it: not twice.
pub fn he_sent(http: Http, copy: &Copy, others: &[String]) -> Result<bool, String> {
    let (api, account) = session(http)?;
    let same = json!({"to": copy.to, "subject": copy.subject});
    if let Some(drafts) = folder(http, &api, &account, "drafts")? {
        let ours = |m: &Value| m["messageId"][0].as_str().unwrap_or_default().to_string();
        let kept = found(http, &api, &account, &drafts, same.clone())?;
        let another = |id: &String| others.iter().any(|o| bare(o) == id);
        if kept.iter().map(ours).any(|id| !another(&id)) {
            return Ok(false);
        }
    }
    sent_has(http, &api, &account, &copy.to, &copy.subject, &copy.made)
}

fn sent_has(
    http: Http,
    api: &str,
    account: &str,
    to: &str,
    subject: &str,
    since: &str,
) -> Result<bool, String> {
    let Some(sent) = folder(http, api, account, "sent")? else {
        return Ok(false);
    };
    let filter = json!({"to": to, "subject": subject, "after": since});
    let mails = found(http, api, account, &sent, filter)?;
    Ok(mails.iter().any(|m| titled(m, subject)))
}

/// Whether Sent holds a mail to `to` with exactly this subject since `since` (UTC, ISO): what a send that got no answer
/// from the server is checked against before it is tried again (`dash mail send`, /code-review of PR 1490).
pub fn sent_since(http: Http, to: &str, subject: &str, since: &str) -> Result<bool, String> {
    let (api, account) = session(http)?;
    sent_has(http, &api, &account, to, subject, since)
}

/// Where a file is uploaded for the mail account (RFC 8620 section 6.1): a POST of its bytes answers its `blobId`.
pub fn upload_url(http: Http) -> Result<String, String> {
    let (_, account) = session(http)?;
    let read: Value = serde_json::from_str(&http(SESSION, None)?)
        .map_err(|_| "jmap: the server's answer is not JSON".to_string())?;
    let url = read["uploadUrl"]
        .as_str()
        .ok_or("jmap: the session names no upload address")?;
    Ok(url.replace("{accountId}", &account))
}
