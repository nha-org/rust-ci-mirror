//! /mail's frame, below the list and the thread that draw in it (task rust-mail-parity): his address, the folders and labels with
//! their counts, what a list is asked for, Gmail's dates, and the side column around a page.
use crate::jmap::encoded;
use dash_core::days;
use serde_json::{Value, json};

/// His own address: the one /mail reads with no switch.
pub const OWN: &str = "nicolas@klar.im";
/// klar's other human mailboxes (klar company/mail/accounts.py ACCOUNTS); the switch shows each one whose password he keeps.
pub const OTHERS: [&str; 3] = ["agent@klar.im", "support@klar.im", "incorrect@klar.im"];
/// Gmail's 50 to a page.
pub const PAGE: u64 = 50;
/// The last page a link may ask: 5 million conversations, so no page number overflows a position.
pub const LAST: u64 = 100_000;

/// A folder or label of the account, as `Mailbox/get` gives it: its conversations unread and in all.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mailbox {
    pub id: String,
    pub name: String,
    /// Its JMAP role (inbox, sent, drafts, junk, trash), empty for a label of his own.
    pub role: String,
    pub unread: u64,
    pub total: u64,
}

/// The side column's folders in the older app's order (`/mail?box=sent`): each one's key, its JMAP role and its name.
/// Starred is a keyword, All mail every mail; a label of his own is asked as `label:<its id>`, and listed after these.
pub(crate) const FOLDERS: [(&str, &str, &str); 7] = [
    ("inbox", "inbox", "Inbox"),
    ("starred", "", "Starred"),
    ("sent", "sent", "Sent"),
    ("drafts", "drafts", "Drafts"),
    ("spam", "junk", "Spam"),
    ("trash", "trash", "Trash"),
    ("all", "", "All mail"),
];

/// The properties a folder's line needs, which `Mailbox/get` is asked for.
pub(crate) const BOX_PROPERTIES: [&str; 5] =
    ["id", "name", "role", "unreadThreads", "totalThreads"];

/// The folders and labels a `Mailbox/get` answer holds.
pub(crate) fn mailboxes(answer: &Value) -> Vec<Mailbox> {
    let each = answer["list"].as_array().into_iter().flatten();
    each.map(|b| Mailbox {
        id: b["id"].as_str().unwrap_or_default().into(),
        name: b["name"].as_str().unwrap_or_default().into(),
        role: b["role"].as_str().unwrap_or_default().into(),
        unread: b["unreadThreads"].as_u64().unwrap_or(0),
        total: b["totalThreads"].as_u64().unwrap_or(0),
    })
    .filter(|b| !b.id.is_empty())
    .collect()
}

/// What /mail is asked for: a folder of `FOLDERS` or a label, a search of the server's full text, a page from 1.
#[derive(Clone, Debug, PartialEq)]
pub struct Asked {
    pub folder: String,
    pub q: Option<String>,
    pub page: u64,
}

/// A label's id as a link may carry it: JMAP's id characters (RFC 8620 1.2), 255 at most.
fn an_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() < 256
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

impl Asked {
    /// The inbox for a folder neither in `FOLDERS` nor a label's, no search for a blank one, page 1 for none, the last page
    /// `LAST` past it.
    pub fn of(folder: Option<&str>, q: Option<&str>, page: Option<u64>) -> Asked {
        let known = |f: &&str| {
            FOLDERS.iter().any(|k| k.0 == *f) || f.strip_prefix("label:").is_some_and(an_id)
        };
        let folder = folder.filter(known).unwrap_or("inbox").to_string();
        let q = q.map(str::trim).filter(|q| !q.is_empty()).map(String::from);
        Asked {
            folder,
            q,
            page: page.unwrap_or(1).clamp(1, LAST),
        }
    }

    pub(crate) fn label(&self) -> Option<&str> {
        self.folder.strip_prefix("label:")
    }

    /// Its name: the folder's, or the label's among `boxes`.
    pub(crate) fn name(&self, boxes: &[Mailbox]) -> String {
        if let Some(id) = self.label() {
            return boxes
                .iter()
                .find(|b| b.id == id)
                .map_or("Label".into(), |b| b.name.clone());
        }
        FOLDERS
            .iter()
            .find(|f| f.0 == self.folder)
            .map_or("Inbox", |f| f.2)
            .into()
    }

    /// /mail's link to `page` of this, as `account` reads it: no parameter that says the default.
    pub(crate) fn href(&self, account: &str, page: u64) -> String {
        let mut query = vec![];
        if account != OWN {
            query.push(format!("account={}", encoded(account)));
        }
        if self.folder != "inbox" {
            query.push(format!("box={}", encoded(&self.folder)));
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

use dash_core::MONTHS;

/// A date as Gmail's list shows it: a day's mail by its time, this year's by its day and month (« 10 Nov »), an older one with
/// its year; UTC, as the server keeps it.
pub(crate) fn when(at: &str, today: i64) -> String {
    let (Some(day), Some(time)) = (days(at), at.get(11..16)) else {
        return at.get(..10).unwrap_or(at).to_string();
    };
    if day == today {
        return time.to_string();
    }
    let Some((d, month, year)) = dated(day) else {
        return dash_core::date(day);
    };
    if dated(today).is_some_and(|t| t.2 == year) {
        format!("{d} {month}")
    } else {
        format!("{d} {month} {year}")
    }
}

/// A day since 1970-01-01 as its day of the month, its month's short name and its year.
fn dated(day: i64) -> Option<(u32, &'static str, String)> {
    let date = dash_core::date(day);
    let month = date.get(5..7)?.parse::<usize>().ok()?;
    Some((
        date.get(8..10)?.parse().ok()?,
        MONTHS.get(month.checked_sub(1)?)?,
        date.get(..4)?.into(),
    ))
}

/// A role the side column names among its folders: any other (archive, important) is listed with his labels, so every
/// folder is reachable (/code-review of rust-mail-parity).
pub(crate) fn named(role: &str) -> bool {
    !role.is_empty() && FOLDERS.iter().any(|f| f.1 == role)
}

/// A mail's date as the older app's conversation showed it: « Mon 9 Nov 2026, 21:49 UTC », UTC as the server keeps it.
pub fn long_when(at: &str) -> String {
    let (Some(day), Some(time)) = (days(at), at.get(11..16)) else {
        return at.to_string();
    };
    // 1970-01-01 was a Thursday
    let weekday = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"][day.rem_euclid(7) as usize];
    match dated(day) {
        Some((d, month, year)) => format!("{weekday} {d} {month} {year}, {time} UTC"),
        None => at.to_string(),
    }
}

/// The side column's lines, as `account` reads them: the folders in the older app's order, each with its count (the
/// unread conversations; Drafts and Spam, all of them), then his labels by name, `current` marked (task rust-mail-parity).
pub fn folder_links(account: &str, boxes: &[Mailbox], current: &str) -> Vec<Value> {
    let by_role = |role: &str| boxes.iter().find(|b| !role.is_empty() && b.role == role);
    let mut links: Vec<Value> = FOLDERS
        .iter()
        .map(|(key, role, name)| {
            // Drafts and Spam count all their conversations, the inbox its unread, as the older app's Webmail.folders did
            // (count.("junk", :total)); the side-by-side comparison with :7799 caught Spam's unread count
            let count = by_role(role).map_or(0, |b| if matches!(*key, "drafts" | "spam") { b.total } else { b.unread });
            let count = if matches!(*key, "inbox" | "drafts" | "spam") { count } else { 0 };
            let kind = if role.is_empty() { *key } else { *role };
            let href = Asked::of(Some(key), None, None).href(account, 1);
            json!({"name": name, "href": href, "count": count, "kind": kind, "current": current == *key})
        })
        .collect();
    let mut labels: Vec<&Mailbox> = boxes.iter().filter(|b| !named(&b.role)).collect();
    labels.sort_by_key(|b| b.name.to_lowercase());
    links.extend(labels.into_iter().map(|b| {
        let key = format!("label:{}", b.id);
        let href = Asked::of(Some(&key), None, None).href(account, 1);
        json!({"name": b.name, "href": href, "count": b.unread, "kind": "label", "current": current == key})
    }));
    links
}

/// /mail's frame around `inside`, as `account` reads it (mail-shell): the switch's addresses, Compose, the folders with
/// `current` marked, and the search, which keeps the account and `folder`.
pub fn shell(
    account: &str,
    accounts: &[String],
    boxes: &[Mailbox],
    current: &str,
    search: Option<&str>,
    inside: Vec<String>,
) -> Value {
    let switch = accounts.iter().map(|a| {
        json!({"name": a, "href": Asked::of(None, None, None).href(a, 1), "current": a == account})
    });
    let mut hidden = vec![];
    if account != OWN {
        hidden.push(json!(["account", account]));
    }
    if current != "inbox" && FOLDERS.iter().any(|f| f.0 == current) || current.starts_with("label:")
    {
        hidden.push(json!(["box", current]));
    }
    let compose = if account == OWN {
        "/mail/compose".to_string()
    } else {
        format!("/mail/compose?account={}", encoded(account))
    };
    json!({"type": "mail-shell", "props": {"account": account, "accounts": switch.collect::<Vec<_>>(),
        "compose": compose, "folders": folder_links(account, boxes, current), "search": search.unwrap_or_default(),
        "hidden": hidden}, "children": inside})
}
