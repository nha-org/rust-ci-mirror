//! The mail plugin (rfc-rust-plugins-ui section 3, pick A): klar's mail server over JMAP (RFC 8620, RFC 8621), read as one of
//! his klar.im addresses, and /mail's list drawn from it, 50 to a page, newest first: the inbox, or a folder or a search the
//! older app's thread page links to. Ported from the older app's Dashboard.Jmap and Dashboard.Webmail.list. It depends on the
//! core alone and names nothing of the other plugins: the gate holds its source to that, his words and the banned ones in
//! the gate's crate, since naming them here would trip it. The caller brings the transport, signed in as the account it reads, so this crate
//! holds no password and no socket.
use serde_json::{Map, Value, json};
use std::collections::BTreeSet;

mod jmap;
mod thread;
pub use jmap::{Http, SESSION, encoded};
mod frame;
pub use frame::{Asked, LAST, Mailbox, OTHERS, OWN, PAGE, folder_links, long_when, shell};
use frame::{BOX_PROPERTIES, FOLDERS, mailboxes, when};
use jmap::{called, called_but, session};
pub use thread::{
    File, Mail, New, SCHEMA, Sent, Thread, compose_tree, reply, send_new, send_new_once, send_once,
    sent_already, thread, thread_tree,
};

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
    /// The sender's address: « me » on the page when it is the address read.
    pub address: String,
    /// The server's receivedAt, UTC.
    pub at: String,
    pub preview: String,
    pub unread: bool,
    /// Its mails, and whether one of them is starred (task rust-mail-parity).
    pub count: u64,
    pub starred: bool,
}

/// One page of conversations, and the account's folders with their counts, read in the same call.
#[derive(Clone, Debug, PartialEq)]
pub struct Listed {
    pub rows: Vec<Row>,
    /// The server's count of conversations.
    pub total: u64,
    pub boxes: Vec<Mailbox>,
}

/// What `asked` lists, newest first: the session, the folder's id, then its conversations and every folder's counts, three
/// calls (two for Starred, All mail and a label).
pub fn list(http: Http, asked: &Asked) -> Result<Listed, String> {
    let (api, account) = session(http)?;
    let (api, account) = (api.as_str(), account.as_str());
    let call = |calls: Value| called(http, api, calls);
    let filter = match (asked.folder.as_str(), asked.label()) {
        ("starred", _) => Some(json!({"hasKeyword": "$flagged"})),
        ("all", _) => None,
        (_, Some(id)) => Some(json!({"inMailbox": id})),
        (key, None) => {
            let role = FOLDERS.iter().find(|f| f.0 == key).map_or("inbox", |f| f.1);
            let boxes = call(
                json!([["Mailbox/query", {"accountId": account, "filter": {"role": role}}, "b"]]),
            )?;
            // a server with no such folder lists nothing, never everything
            let Some(id) = boxes["b"]["ids"][0].as_str() else {
                return Ok(Listed {
                    rows: vec![],
                    total: 0,
                    boxes: folders(http).unwrap_or_default(),
                });
            };
            Some(json!({"inMailbox": id}))
        }
    };
    let filter = match (filter, &asked.q) {
        (Some(f), Some(q)) => json!({"operator": "AND", "conditions": [f, {"text": q}]}),
        (Some(f), None) => f,
        (None, Some(q)) => json!({"text": q}),
        (None, None) => json!({}),
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
    // each shown mail's thread, and every mail of it, for its unread mark, its star and its count (as Dashboard.Webmail.list
    // reads it)
    let threads = json!({
        "accountId": account,
        "#ids": {"resultOf": "e", "name": "Email/get", "path": "/list/*/threadId"},
    });
    let all = json!({
        "accountId": account,
        "#ids": {"resultOf": "t", "name": "Thread/get", "path": "/list/*/emailIds"},
        "properties": ["id", "threadId", "keywords"],
    });
    let r = called_but(
        http,
        api,
        json!([
            ["Email/query", query, "q"],
            ["Email/get", get, "e"],
            ["Thread/get", threads, "t"],
            ["Email/get", all, "all"],
            ["Mailbox/get", {"accountId": account, "ids": null, "properties": BOX_PROPERTIES}, "m"]
        ]),
        &["m"],
    )?;
    let all: Vec<&Value> = r["all"]["list"].as_array().into_iter().flatten().collect();
    let threads_of = |key: &str, on: bool| -> BTreeSet<&str> {
        (all.iter())
            .filter(|m| (m["keywords"][key] == true) == on)
            .filter_map(|m| m["threadId"].as_str())
            .collect()
    };
    let marks = Marks {
        unseen: threads_of("$seen", false),
        starred: threads_of("$flagged", true),
        counts: (r["t"]["list"].as_array().into_iter().flatten())
            .filter_map(|t| Some((t["id"].as_str()?, t["emailIds"].as_array()?.len() as u64)))
            .collect(),
    };
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
        rows: mails.into_iter().map(|m| row(m, &marks)).collect(),
        total: r["q"]["total"].as_u64().unwrap_or(ids.len() as u64),
        boxes: mailboxes(&r["m"]),
    })
}

/// What a row reads off its thread's mails: the threads holding a mail not yet read, the ones holding a starred one, and each
/// thread's count of mails.
struct Marks<'a> {
    unseen: BTreeSet<&'a str>,
    starred: BTreeSet<&'a str>,
    counts: std::collections::BTreeMap<&'a str, u64>,
}

/// The account's folders and labels with their counts, one call after the session: Compose's side column.
pub fn folders(http: Http) -> Result<Vec<Mailbox>, String> {
    let (api, account) = session(http)?;
    let r = called(
        http,
        &api,
        json!([["Mailbox/get", {"accountId": account, "ids": null, "properties": BOX_PROPERTIES}, "m"]]),
    )?;
    Ok(mailboxes(&r["m"]))
}

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

fn row(mail: &Value, marks: &Marks) -> Row {
    let text = |key: &str| mail[key].as_str().unwrap_or_default().trim().to_string();
    let from = &mail["from"][0];
    let address = from["email"].as_str().unwrap_or_default().to_string();
    let from = [&from["name"], &from["email"]]
        .into_iter()
        .filter_map(Value::as_str)
        .find(|s| !s.trim().is_empty());
    let subject = text("subject");
    let thread = text("threadId");
    Row {
        subject: if subject.is_empty() {
            "(no subject)".into()
        } else {
            subject
        },
        from: from.unwrap_or("(no sender)").trim().to_string(),
        address,
        at: text("receivedAt"),
        preview: text("preview"),
        unread: mail["keywords"]["$seen"] != true || marks.unseen.contains(thread.as_str()),
        count: marks.counts.get(thread.as_str()).copied().unwrap_or(1),
        starred: mail["keywords"]["$flagged"] == true || marks.starred.contains(thread.as_str()),
        thread,
    }
}

/// /mail's UI tree: the frame, then the page asked, its bar and its rows, or why it could not be read. `today`: days since
/// 1970-01-01, UTC. A conversation opens on its thread page.
pub fn tree(
    account: &str,
    accounts: &[String],
    asked: &Asked,
    read: &Result<Listed, String>,
    today: i64,
) -> String {
    let mut elements = Map::new();
    let mut children = vec![];
    let boxes = read
        .as_ref()
        .map(|r| r.boxes.as_slice())
        .unwrap_or_default();
    let matching = asked
        .q
        .as_ref()
        .map_or(String::new(), |q| format!(" matching « {q} »"));
    let what = format!("{} of {account}{matching}", asked.name(boxes));
    let note = |text: String, elements: &mut Map<String, Value>, children: &mut Vec<String>| {
        elements.insert(
            "note".into(),
            json!({"type": "text", "props": {"text": text}}),
        );
        children.push("note".into());
    };
    match read {
        Err(e) => note(format!("{what}: {e}"), &mut elements, &mut children),
        Ok(read) if read.rows.is_empty() => note(
            format!("{what}: nothing on page {} of {}.", asked.page, read.total),
            &mut elements,
            &mut children,
        ),
        Ok(read) => {
            let first = (asked.page - 1) * PAGE;
            let last = first + read.rows.len() as u64;
            let newer = (asked.page > 1).then(|| asked.href(account, asked.page - 1));
            let older = (last < read.total).then(|| asked.href(account, asked.page + 1));
            elements.insert(
                "bar".into(),
                json!({"type": "mail-bar", "props": {
                "range": format!("{}–{last} of {}", first + 1, read.total),
                "refresh": asked.href(account, asked.page), "newer": newer, "older": older}}),
            );
            children.push("bar".into());
            // the address always: the thread page would read one his session kept from its own switch
            let thread = |r: &Row| {
                format!(
                    "/mail/t/{}?account={}",
                    encoded(&r.thread),
                    encoded(account)
                )
            };
            // /design?tab=mail's thread row, as the older app's /mail drew each conversation (task rust-mail-design-parts)
            let rows = read.rows.iter().map(|r| {
                let from = if r.address.eq_ignore_ascii_case(account) { "me" } else { r.from.as_str() };
                json!({"type": "thread-row", "props": {"from": from, "subject": r.subject, "snippet": r.preview,
                    "at": when(&r.at, today), "unread": r.unread, "href": thread(r), "count": r.count,
                    "starred": r.starred}})
            });
            listed("list", rows.collect(), &mut elements, &mut children);
        }
    }
    elements.insert(
        "page".into(),
        shell(
            account,
            accounts,
            boxes,
            &asked.folder,
            asked.q.as_deref(),
            children,
        ),
    );
    json!({"root": "page", "elements": elements}).to_string()
}

/// `list` as one list element under `id`, each row `<id>-<n>`, put on the page.
fn listed(
    id: &str,
    list: Vec<Value>,
    elements: &mut Map<String, Value>,
    children: &mut Vec<String>,
) {
    let ids: Vec<String> = (0..list.len()).map(|i| format!("{id}-{i}")).collect();
    for (key, value) in ids.iter().zip(list) {
        elements.insert(key.clone(), value);
    }
    elements.insert(id.into(), json!({"type": "list", "children": ids}));
    children.push(id.into());
}
