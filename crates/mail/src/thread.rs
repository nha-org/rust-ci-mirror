//! /mail/t/<thread>: one conversation read over JMAP, oldest first, the older mails folded and the latest open, and his reply
//! sent from the address read (rfc-rust-plugins-ui section 3: mail's thread and compose box). Ported from the older app's
//! Dashboard.Webmail.thread and send. Each send is logged once, by a key of its words and the mail it answers, so a form
//! posted twice sends once.
use crate::jmap::{Http, MAIL, USING, answers, encoded, failed, responses, session};
use dash_core::{FieldWrite, Fnv, OpLog, Value as Field};
use serde_json::{Map, Value, json};

const SUBMISSION: &str = "urn:ietf:params:jmap:submission";
/// The send log's table: a row a reply sent, keyed by `key`.
pub const SCHEMA: &str =
    "CREATE TABLE IF NOT EXISTS mail_sent (id TEXT PRIMARY KEY, record TEXT) WITHOUT ROWID;";
const TABLE: &str = "mail_sent";
const FIELD: &str = "record";
const PAGE: i64 = 1000;

/// An attachment: its name, its type and the server's blob of it.
#[derive(Clone, Debug, PartialEq)]
pub struct File {
    pub name: String,
    pub kind: String,
    pub blob: String,
}

/// One mail of a conversation.
#[derive(Clone, Debug, PartialEq)]
pub struct Mail {
    pub id: String,
    /// The sender's name, else the address.
    pub from: String,
    /// The sender's address, which a reply goes to.
    pub address: String,
    pub to: Vec<String>,
    /// The server's receivedAt, UTC.
    pub at: String,
    pub subject: String,
    pub text: String,
    pub html: Option<String>,
    /// Its Message-ID, bare, for a reply's In-Reply-To.
    pub message_id: Option<String>,
    pub references: Vec<String>,
    pub files: Vec<File>,
}

/// A conversation, its mails oldest first.
#[derive(Clone, Debug, PartialEq)]
pub struct Thread {
    pub id: String,
    pub subject: String,
    pub mails: Vec<Mail>,
}

/// The conversation `id`: the session, then the thread and its mails with their bodies, one call.
pub fn thread(http: Http, id: &str) -> Result<Thread, String> {
    let (api, account) = session(http)?;
    let get = json!({
        "accountId": account,
        "#ids": {"resultOf": "t", "name": "Thread/get", "path": "/list/*/emailIds"},
        "properties": ["id", "messageId", "references", "from", "to", "cc", "subject", "receivedAt", "textBody", "htmlBody",
            "bodyValues", "attachments"],
        "fetchTextBodyValues": true,
        "fetchHTMLBodyValues": true,
        "maxBodyValueBytes": 2_000_000,
    });
    let calls =
        json!([["Thread/get", {"accountId": account, "ids": [id]}, "t"], ["Email/get", get, "e"]]);
    let body = json!({"using": USING, "methodCalls": calls}).to_string();
    let r = answers(&http(&api, Some(&body))?)?;
    let mut mails: Vec<Mail> = r["e"]["list"]
        .as_array()
        .into_iter()
        .flatten()
        .map(mail)
        .collect();
    if mails.is_empty() {
        return Err(format!("no conversation {id} on this server"));
    }
    mails.sort_by(|a, b| a.at.cmp(&b.at));
    let subject = mails[0].subject.trim();
    Ok(Thread {
        id: id.to_string(),
        subject: if subject.is_empty() {
            "(no subject)"
        } else {
            subject
        }
        .to_string(),
        mails,
    })
}

fn mail(e: &Value) -> Mail {
    let text = |v: &Value| v.as_str().unwrap_or_default().to_string();
    let addresses = |v: &Value| -> Vec<String> {
        let each = v.as_array().into_iter().flatten();
        each.filter_map(|a| a["email"].as_str())
            .map(String::from)
            .collect()
    };
    let sender = &e["from"][0];
    let address = text(&sender["email"]);
    let name = text(&sender["name"]);
    let mut to = addresses(&e["to"]);
    to.extend(addresses(&e["cc"]));
    let files = e["attachments"].as_array().into_iter().flatten();
    Mail {
        id: text(&e["id"]),
        from: if name.trim().is_empty() {
            address.clone()
        } else {
            name.trim().to_string()
        },
        address,
        to,
        at: text(&e["receivedAt"]),
        subject: text(&e["subject"]),
        text: body(e, "textBody", "text/plain").unwrap_or_default(),
        html: body(e, "htmlBody", "text/html"),
        message_id: e["messageId"][0].as_str().map(String::from),
        references: addresses_or_ids(&e["references"]),
        files: files
            .map(|a| File {
                name: a["name"].as_str().unwrap_or("file").to_string(),
                kind: a["type"]
                    .as_str()
                    .unwrap_or("application/octet-stream")
                    .to_string(),
                blob: text(&a["blobId"]),
            })
            .collect(),
    }
}

fn addresses_or_ids(v: &Value) -> Vec<String> {
    let each = v.as_array().into_iter().flatten();
    each.filter_map(Value::as_str).map(String::from).collect()
}

/// The body's parts of `kind`, joined: the server's own decoding, never ours.
fn body(e: &Value, key: &str, kind: &str) -> Option<String> {
    let parts: Vec<&str> = (e[key].as_array().into_iter().flatten())
        .filter(|p| p["type"] == kind)
        .filter_map(|p| e["bodyValues"][p["partId"].as_str()?]["value"].as_str())
        .collect();
    (!parts.is_empty()).then(|| parts.join("\n"))
}

/// Whom a reply from `from` goes to, as Gmail's Reply: the latest sender who is not `from`, else those the latest mail went
/// to. A mail with no sender's address (a server's notice) answers nobody. The box's button names them, so none is dropped
/// unseen. ponytail: no Reply all or Cc; add them when he asks.
fn recipients(from: &str, thread: &Thread) -> Vec<String> {
    let theirs = |a: &str| !a.trim().is_empty() && !a.eq_ignore_ascii_case(from);
    let latest = thread.mails.iter().rev().find(|m| theirs(&m.address));
    match latest {
        Some(m) => vec![m.address.clone()],
        None => (thread.mails.last().into_iter())
            .flat_map(|m| m.to.iter().filter(|a| theirs(a)).cloned())
            .collect(),
    }
}

fn subject(thread: &Thread) -> String {
    match thread.subject.get(..3) {
        Some(re) if re.eq_ignore_ascii_case("re:") => thread.subject.clone(),
        _ => format!("Re: {}", thread.subject),
    }
}

/// His reply to the latest mail of `thread`, sent from `from` through JMAP: made in Drafts, submitted, then moved to Sent
/// (RFC 8621 7.5, onSuccessUpdateEmail). The sent mail's id, or why it did not go; never retried, so it never goes twice.
/// ponytail: plain text only, no files; the older app's formatting bar and attachments when he asks for them here.
pub fn reply(http: Http, from: &str, thread: &Thread, text: &str) -> Result<String, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("an empty reply is not sent".into());
    }
    let latest = thread.mails.last().ok_or("no mail to answer")?;
    let to = recipients(from, thread);
    if to.is_empty() {
        return Err("no address to answer".into());
    }
    let (api, account) = session(http)?;
    let call = |calls: Value| {
        let using = [USING[0], MAIL, SUBMISSION];
        http(
            &api,
            Some(&json!({"using": using, "methodCalls": calls}).to_string()),
        )
    };
    let r = answers(&call(json!([
        ["Identity/get", {"accountId": account}, "i"],
        ["Mailbox/query", {"accountId": account, "filter": {"role": "drafts"}}, "d"],
        ["Mailbox/query", {"accountId": account, "filter": {"role": "sent"}}, "s"],
    ]))?)?;
    let identity = (r["i"]["list"].as_array().into_iter().flatten())
        .find(|i| {
            i["email"]
                .as_str()
                .is_some_and(|e| e.eq_ignore_ascii_case(from))
        })
        .ok_or(format!("jmap: {from} has no identity to send from"))?;
    let drafts = r["d"]["ids"][0]
        .as_str()
        .ok_or("jmap: this account has no Drafts folder")?;
    let sent = r["s"]["ids"][0]
        .as_str()
        .ok_or("jmap: this account has no Sent folder")?;
    let mut email = json!({
        "mailboxIds": {drafts: true},
        "keywords": {"$draft": true, "$seen": true},
        "from": [{"name": identity["name"], "email": identity["email"]}],
        "to": to.iter().map(|a| json!({"email": a})).collect::<Vec<_>>(),
        "subject": subject(thread),
        "textBody": [{"partId": "body", "type": "text/plain"}],
        "bodyValues": {"body": {"value": text}},
    });
    if let Some(id) = &latest.message_id {
        let mut references = latest.references.clone();
        references.push(id.clone());
        email["inReplyTo"] = json!([id]);
        email["references"] = json!(references);
    }
    let mut moved = Map::new();
    moved.insert(format!("mailboxIds/{drafts}"), Value::Null);
    moved.insert(format!("mailboxIds/{sent}"), Value::Bool(true));
    moved.insert("keywords/$draft".into(), Value::Null);
    let r = responses(&call(json!([
        ["Email/set", {"accountId": account, "create": {"draft": email}}, "e"],
        ["EmailSubmission/set", {
            "accountId": account,
            "create": {"send": {"identityId": identity["id"], "emailId": "#draft"}},
            "onSuccessUpdateEmail": {"#send": moved},
        }, "s"],
    ]))?)?;
    let made = r
        .get("e")
        .and_then(|e| e["created"]["draft"]["id"].as_str());
    let submitted = r.get("s").is_some_and(|s| s["created"]["send"].is_object());
    match (made, submitted) {
        (Some(id), true) => Ok(id.to_string()),
        (Some(id), false) => {
            // made but not sent: it leaves Drafts, so another try adds none there
            let _ = call(json!([["Email/set", {"accountId": account, "destroy": [id]}, "x"]]));
            Err(format!(
                "jmap: the server did not send it: {}",
                why(&r, "s")
            ))
        }
        (None, _) => Err(format!(
            "jmap: the server refused the mail: {}",
            why(&r, "e")
        )),
    }
}

/// Why call `id` made nothing: its method's error, else its notCreated.
fn why(r: &Map<String, Value>, id: &str) -> String {
    let result = r.get(id).cloned().unwrap_or_default();
    failed(id, &result)
        .unwrap_or_else(|| result["notCreated"].to_string().chars().take(300).collect())
}

/// A reply's key in the send log: the same words to the same mail from the same address are one send.
fn key(from: &str, thread: &Thread, text: &str) -> String {
    let latest = thread.mails.last().map_or("", |m| m.id.as_str());
    let mut hash = Fnv::new();
    for part in [from, "\0", &thread.id, "\0", latest, "\0", text.trim()] {
        hash.bytes(part.as_bytes());
    }
    format!("{:016x}", hash.finish())
}

/// Whether the send log holds `key`.
/// ponytail: the whole log read on each send, fine at a few thousand replies; a read port once the core has one.
pub fn sent_already(log: &dyn OpLog, key: &str) -> Result<bool, String> {
    let mut after: Option<(i64, String)> = None;
    loop {
        let ops = log.ops_after(after.as_ref().map(|(s, w)| (*s, w.as_str())), PAGE)?;
        let Some(last) = ops.last() else {
            return Ok(false);
        };
        if ops
            .iter()
            .any(|op| op.write.table == TABLE && op.write.row == key)
        {
            return Ok(true);
        }
        after = Some((last.stamp, last.writer.clone()));
    }
}

/// A reply that went: the mail made, the send log's record of it (the older app's outbound line, same fields), and whether
/// the log took it.
#[derive(Debug, PartialEq)]
pub struct Sent {
    pub email: String,
    pub record: Value,
    pub logged: Result<(), String>,
}

/// `reply`, once: refused when the log holds the same reply, logged once it went. `at`: now, UTC, as the log keeps it. The
/// caller holds one lock across it, so two posts of the same form never both pass the check. A send that went is `Ok` even
/// when its log failed (`Sent::logged`): an error would read as not sent, and he would send it again.
pub fn send_once(
    http: Http,
    log: &mut dyn OpLog,
    from: &str,
    thread: &Thread,
    text: &str,
    at: &str,
) -> Result<Sent, String> {
    let key = key(from, thread, text);
    if sent_already(log, &key)? {
        return Err("this reply already went (the send log): nothing sent again".into());
    }
    let email = reply(http, from, thread, text)?;
    let record = json!({
        "date": at.get(..10).unwrap_or(at),
        "at": at,
        "channel": "email",
        "tool": "mail-page",
        "from": from,
        "to": recipients(from, thread).join(", "),
        "subject": subject(thread),
        "text": text.trim(),
        "thread": thread.id,
        "email": email,
    });
    let row = FieldWrite {
        table: TABLE.into(),
        row: key,
        field: FIELD.into(),
        value: Field::Text(record.to_string()),
    };
    let logged = log.write(&[row]).map(|_| ());
    Ok(Sent {
        email,
        record,
        logged,
    })
}

/// /mail/t/<thread>'s UI tree: its subject, each mail oldest first, folded but the latest, then his reply box, or why it
/// could not be read. `said`: what his last send came to.
pub fn thread_tree(
    account: &str,
    id: &str,
    read: &Result<Thread, String>,
    said: Option<&str>,
) -> String {
    let mut elements = Map::new();
    let mut children = vec!["title".to_string(), "note".to_string()];
    let (title, note) = match read {
        Err(e) => (
            "Mail".to_string(),
            format!("Conversation {id} of {account}: {e}"),
        ),
        Ok(t) => {
            let n = t.mails.len();
            let mails = if n == 1 {
                "1 mail".to_string()
            } else {
                format!("{n} mails")
            };
            (
                t.subject.clone(),
                format!("{mails}, as {account} reads them."),
            )
        }
    };
    elements.insert(
        "title".into(),
        json!({"type": "heading", "props": {"text": title}}),
    );
    elements.insert(
        "note".into(),
        json!({"type": "text", "props": {"text": note}}),
    );
    if let Some(said) = said {
        elements.insert(
            "said".into(),
            json!({"type": "text", "props": {"text": said}}),
        );
        children.push("said".into());
    }
    if let Ok(t) = read {
        let last = t.mails.len() - 1;
        for (i, m) in t.mails.iter().enumerate() {
            let files: Vec<(String, String)> = (m.files.iter())
                .map(|f| {
                    let href = format!(
                        "/mail/blob/{}?name={}&type={}&account={}",
                        encoded(&f.blob),
                        encoded(&f.name),
                        encoded(&f.kind),
                        encoded(account)
                    );
                    (f.name.clone(), href)
                })
                .collect();
            let props = json!({"from": m.from, "to": m.to.join(", "), "at": m.at, "text": m.text, "html": m.html,
                "open": i == last, "files": files});
            elements.insert(format!("mail-{i}"), json!({"type": "mail", "props": props}));
            children.push(format!("mail-{i}"));
        }
        let to = recipients(account, t).join(", ");
        let compose = json!({"type": "compose", "props": {
            "action": format!("/mail/t/{}/reply", encoded(id)),
            "send": format!("Send to {to} from {account}"),
            "placeholder": "Your reply",
            "hidden": [["account", account]],
        }});
        elements.insert("reply".into(), compose);
        children.push("reply".into());
    }
    elements.insert(
        "page".into(),
        json!({"type": "stack", "children": children}),
    );
    json!({"root": "page", "elements": elements}).to_string()
}
