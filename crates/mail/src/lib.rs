//! The mail plugin (rfc-rust-plugins-ui section 3, pick A): klar's mail server over JMAP (RFC 8620, RFC 8621), read as one of
//! his klar.im addresses, and /mail's list drawn from it, 50 to a page, newest first: the inbox, or a folder or a search the
//! older app's thread page links to. Ported from the older app's Dashboard.Jmap and Dashboard.Webmail.list. It depends on the
//! core alone and names nothing of the other plugins: the gate holds its source to that, his words and the banned ones in
//! the gate's crate, since naming them here would trip it. The caller brings the transport, signed in as the account it reads, so this crate
//! holds no password and no socket.
use dash_core::days;
use serde_json::{Map, Value, json};
use std::collections::BTreeSet;

mod jmap;
mod thread;
pub use jmap::{Http, SESSION, encoded};
use jmap::{called, session};
pub use thread::{
    File, Mail, SCHEMA, Sent, Thread, reply, send_once, sent_already, thread, thread_tree,
};

/// His own address: the one /mail reads with no switch.
pub const OWN: &str = "nicolas@klar.im";
/// klar's other human mailboxes (klar company/mail/accounts.py ACCOUNTS); the switch shows each one whose password he keeps.
pub const OTHERS: [&str; 3] = ["agent@klar.im", "support@klar.im", "incorrect@klar.im"];
/// Gmail's 50 to a page.
pub const PAGE: u64 = 50;
/// The last page a link may ask: 5 million conversations, so no page number overflows a position.
pub const LAST: u64 = 100_000;

/// The addresses /mail can switch to (@nha, 2026-10-31, on /mail: « we also need the account switcher »): his own first, then
/// each other one `has_password` finds. klar.im only, never his Gmail addresses.
pub fn accounts(has_password: impl Fn(&str) -> bool) -> Vec<String> {
    let others = OTHERS.into_iter().filter(|a| has_password(a));
    std::iter::once(OWN)
        .chain(others)
        .map(String::from)
        .collect()
}

/// One conversation of the inbox, as its newest mail there shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub thread: String,
    pub subject: String,
    pub from: String,
    /// The server's receivedAt, UTC.
    pub at: String,
    pub preview: String,
    pub unread: bool,
}

/// The folders the older app's links name (`/mail?box=sent`), each one's JMAP role and its name; Starred is a keyword.
/// ponytail: its labels and All mail are not read here: their link shows the inbox, as an unknown box always did.
const FOLDERS: [(&str, &str, &str); 6] = [
    ("inbox", "inbox", "Inbox"),
    ("starred", "", "Starred"),
    ("sent", "sent", "Sent"),
    ("drafts", "drafts", "Drafts"),
    ("spam", "junk", "Spam"),
    ("trash", "trash", "Trash"),
];

/// What /mail is asked for: a folder of `FOLDERS`, a search of the server's full text, a page from 1.
#[derive(Clone, Debug, PartialEq)]
pub struct Asked {
    pub folder: &'static str,
    pub q: Option<String>,
    pub page: u64,
}

impl Asked {
    /// The inbox for a folder not in `FOLDERS`, no search for a blank one, page 1 for none, the last page `LAST` past it.
    pub fn of(folder: Option<&str>, q: Option<&str>, page: Option<u64>) -> Asked {
        let folder = FOLDERS
            .iter()
            .find(|f| Some(f.0) == folder)
            .map_or("inbox", |f| f.0);
        let q = q.map(str::trim).filter(|q| !q.is_empty()).map(String::from);
        Asked {
            folder,
            q,
            page: page.unwrap_or(1).clamp(1, LAST),
        }
    }

    fn name(&self) -> &'static str {
        FOLDERS
            .iter()
            .find(|f| f.0 == self.folder)
            .map_or("Inbox", |f| f.2)
    }

    /// /mail's link to `page` of this, as `account` reads it: no parameter that says the default.
    fn href(&self, account: &str, page: u64) -> String {
        let mut query = vec![];
        if account != OWN {
            query.push(format!("account={}", encoded(account)));
        }
        if self.folder != "inbox" {
            query.push(format!("box={}", self.folder));
        }
        if let Some(q) = &self.q {
            query.push(format!("q={}", encoded(q)));
        }
        if page > 1 {
            query.push(format!("page={page}"));
        }
        if query.is_empty() {
            "/mail".into()
        } else {
            format!("/mail?{}", query.join("&"))
        }
    }
}

/// One page of conversations.
#[derive(Clone, Debug, PartialEq)]
pub struct Listed {
    pub rows: Vec<Row>,
    /// The server's count of conversations.
    pub total: u64,
}

/// What `asked` lists, newest first: the session, the folder's id, then its conversations, three calls (two for Starred).
pub fn list(http: Http, asked: &Asked) -> Result<Listed, String> {
    let (api, account) = session(http)?;
    let (api, account) = (api.as_str(), account.as_str());
    let call = |calls: Value| called(http, api, calls);
    let role = FOLDERS
        .iter()
        .find(|f| f.0 == asked.folder)
        .map_or("inbox", |f| f.1);
    let filter = if role.is_empty() {
        json!({"hasKeyword": "$flagged"})
    } else {
        let boxes = call(
            json!([["Mailbox/query", {"accountId": account, "filter": {"role": role}}, "b"]]),
        )?;
        // a server with no such folder lists nothing, never everything
        let Some(id) = boxes["b"]["ids"][0].as_str() else {
            return Ok(Listed {
                rows: vec![],
                total: 0,
            });
        };
        json!({"inMailbox": id})
    };
    let filter = match &asked.q {
        Some(q) => json!({"operator": "AND", "conditions": [filter, {"text": q}]}),
        None => filter,
    };
    let query = json!({
        "accountId": account,
        "filter": filter,
        "sort": [{"property": "receivedAt", "isAscending": false}],
        "collapseThreads": true,
        "position": (asked.page - 1) * PAGE,
        "limit": PAGE,
        "calculateTotal": true,
    });
    let get = json!({
        "accountId": account,
        "#ids": {"resultOf": "q", "name": "Email/query", "path": "/ids"},
        "properties": ["id", "threadId", "subject", "preview", "from", "receivedAt", "keywords"],
    });
    // each shown mail's thread, and every mail of it, for its unread mark (as Dashboard.Webmail.list reads it)
    let threads = json!({
        "accountId": account,
        "#ids": {"resultOf": "e", "name": "Email/get", "path": "/list/*/threadId"},
    });
    let all = json!({
        "accountId": account,
        "#ids": {"resultOf": "t", "name": "Thread/get", "path": "/list/*/emailIds"},
        "properties": ["id", "threadId", "keywords"],
    });
    let r = call(json!([
        ["Email/query", query, "q"],
        ["Email/get", get, "e"],
        ["Thread/get", threads, "t"],
        ["Email/get", all, "all"]
    ]))?;
    let all = r["all"]["list"].as_array().into_iter().flatten();
    let unseen: BTreeSet<&str> = all
        .filter(|m| m["keywords"]["$seen"] != true)
        .filter_map(|m| m["threadId"].as_str())
        .collect();
    let ids: Vec<&str> = r["q"]["ids"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let mut mails: Vec<&Value> = r["e"]["list"].as_array().into_iter().flatten().collect();
    // Email/get's list keeps no order of its own (RFC 8620 5.1): the query's is newest first
    mails.sort_by_key(|m| ids.iter().position(|id| m["id"].as_str() == Some(id)));
    Ok(Listed {
        rows: mails.into_iter().map(|m| row(m, &unseen)).collect(),
        total: r["q"]["total"].as_u64().unwrap_or(ids.len() as u64),
    })
}

/// `unseen`: the threads holding a mail not yet read.
/// The unread conversations of the inbox read for replies, newest first: /mail's page, every mail of which the server already
/// gives in one call, so no limit on a get is nearer than the inbox page's (/code-review).
/// ponytail: a count past this many shows this many; a count the server sums if his inbox ever holds more unread.
pub const REPLIES_READ: u64 = PAGE;

/// The replies waiting: the inbox's unread conversations that hold a mail of his Sent folder, so an answer to him, never a
/// newsletter. Two calls after the session: the two folders' ids, then the unread conversations and every mail of them.
pub fn replies_waiting(http: Http) -> Result<u64, String> {
    let (api, account) = session(http)?;
    let boxes = called(
        http,
        &api,
        json!([
            ["Mailbox/query", {"accountId": account, "filter": {"role": "inbox"}}, "i"],
            ["Mailbox/query", {"accountId": account, "filter": {"role": "sent"}}, "s"]
        ]),
    )?;
    // no inbox or no Sent folder is said, never counted as no reply (/code-review)
    let (Some(inbox), Some(sent)) = (boxes["i"]["ids"][0].as_str(), boxes["s"]["ids"][0].as_str())
    else {
        return Err("jmap: the account names no inbox or no Sent folder".into());
    };
    let unread =
        json!({"operator": "AND", "conditions": [{"inMailbox": inbox}, {"notKeyword": "$seen"}]});
    let query = json!({"accountId": account, "filter": unread, "collapseThreads": true,
        "sort": [{"property": "receivedAt", "isAscending": false}], "limit": REPLIES_READ});
    let r = called(
        http,
        &api,
        json!([
            ["Email/query", query, "q"],
            ["Email/get", {"accountId": account, "properties": ["threadId"],
                "#ids": {"resultOf": "q", "name": "Email/query", "path": "/ids"}}, "e"],
            ["Thread/get", {"accountId": account,
                "#ids": {"resultOf": "e", "name": "Email/get", "path": "/list/*/threadId"}}, "t"],
            ["Email/get", {"accountId": account, "properties": ["threadId", "mailboxIds"],
                "#ids": {"resultOf": "t", "name": "Thread/get", "path": "/list/*/emailIds"}}, "all"]
        ]),
    )?;
    let all = r["all"]["list"].as_array().into_iter().flatten();
    let answered: BTreeSet<&str> = all
        .filter(|m| m["mailboxIds"][sent] == true)
        .filter_map(|m| m["threadId"].as_str())
        .collect();
    Ok(answered.len() as u64)
}

/// The mail plugin's line in the daily's « today » slot. The binary brings the count: `None` while its first read has not ended.
pub struct RepliesWaiting<F>(pub F);

impl<F: Fn() -> Result<Option<u64>, String>> dash_core::registry::Slot for RepliesWaiting<F> {
    fn line(&self) -> dash_core::registry::Line {
        dash_core::registry::Line {
            says: (self.0)().map(|n| {
                n.map(|n| match n {
                    1 => "1 reply waiting".into(),
                    n => format!("{n} replies waiting"),
                })
            }),
            what: "unread answers in his inbox".into(),
            href: "/mail".into(),
        }
    }
}

fn row(mail: &Value, unseen: &BTreeSet<&str>) -> Row {
    let text = |key: &str| mail[key].as_str().unwrap_or_default().trim().to_string();
    let from = &mail["from"][0];
    let from = [&from["name"], &from["email"]]
        .into_iter()
        .filter_map(Value::as_str)
        .find(|s| !s.trim().is_empty());
    let subject = text("subject");
    Row {
        thread: text("threadId"),
        subject: if subject.is_empty() {
            "(no subject)".into()
        } else {
            subject
        },
        from: from.unwrap_or("(no sender)").trim().to_string(),
        at: text("receivedAt"),
        preview: text("preview"),
        unread: mail["keywords"]["$seen"] != true
            || mail["threadId"]
                .as_str()
                .is_some_and(|t| unseen.contains(t)),
    }
}

/// A day's mail by its time, an older one by its date, as Gmail's list shows them; UTC, as the server keeps it.
fn when(at: &str, today: i64) -> String {
    match (days(at), at.get(11..16)) {
        (Some(day), Some(time)) if day == today => format!("{time} UTC"),
        _ => at.get(..10).unwrap_or(at).to_string(),
    }
}

/// /mail's UI tree: the switch when there is more than one address, the page asked or why it could not be read, and the
/// pages around it. `today`: days since 1970-01-01, UTC. A conversation opens on the older app's thread page.
pub fn tree(
    account: &str,
    accounts: &[String],
    asked: &Asked,
    read: &Result<Listed, String>,
    today: i64,
) -> String {
    let mut elements = Map::new();
    let mut children = vec!["title".to_string(), "note".to_string()];
    elements.insert(
        "title".into(),
        json!({"type": "heading", "props": {"text": "Mail"}}),
    );
    let card = |title: &str, line: &str, href: String| json!({"type": "card", "props": {"title": title, "line": line, "href": href}});
    if accounts.len() > 1 {
        let inbox = Asked::of(None, None, None);
        let each = accounts
            .iter()
            .map(|a| card(a, if a == account { "shown" } else { "" }, inbox.href(a, 1)));
        cards("accounts", each.collect(), &mut elements, &mut children);
    }
    let matching = asked
        .q
        .as_ref()
        .map_or(String::new(), |q| format!(" matching « {q} »"));
    let what = format!("{} of {account}{matching}", asked.name());
    let note = match read {
        Err(e) => format!("{what}: {e}"),
        Ok(read) if read.rows.is_empty() => {
            format!("{what}: nothing on page {} of {}.", asked.page, read.total)
        }
        Ok(read) => {
            let first = (asked.page - 1) * PAGE;
            let last = first + read.rows.len() as u64;
            // the address always: the thread page would read one his session kept from its own switch
            let thread = |r: &Row| {
                format!(
                    "/mail/t/{}?account={}",
                    encoded(&r.thread),
                    encoded(account)
                )
            };
            let rows = read.rows.iter().map(|r| {
                let unread = if r.unread { " · unread" } else { "" };
                let preview: String = r.preview.chars().take(90).collect();
                card(
                    &r.subject,
                    &format!("{} · {}{unread} · {preview}", r.from, when(&r.at, today)),
                    thread(r),
                )
            });
            cards("list", rows.collect(), &mut elements, &mut children);
            let mut pages = vec![];
            if asked.page > 1 {
                pages.push(card("Newer", "", asked.href(account, asked.page - 1)));
            }
            if last < read.total {
                pages.push(card("Older", "", asked.href(account, asked.page + 1)));
            }
            if !pages.is_empty() {
                cards("pages", pages, &mut elements, &mut children);
            }
            format!("{what}: {} to {last} of {}.", first + 1, read.total)
        }
    };
    elements.insert(
        "note".into(),
        json!({"type": "text", "props": {"text": note}}),
    );
    elements.insert(
        "page".into(),
        json!({"type": "stack", "children": children}),
    );
    json!({"root": "page", "elements": elements}).to_string()
}

/// `list` as one cards element under `id`, each card `<id>-<n>`, put on the page.
fn cards(
    id: &str,
    list: Vec<Value>,
    elements: &mut Map<String, Value>,
    children: &mut Vec<String>,
) {
    let ids: Vec<String> = (0..list.len()).map(|i| format!("{id}-{i}")).collect();
    for (key, value) in ids.iter().zip(list) {
        elements.insert(key.clone(), value);
    }
    elements.insert(id.into(), json!({"type": "cards", "children": ids}));
    children.push(id.into());
}
