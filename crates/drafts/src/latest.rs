//! A contact's latest message re-evaluates what waits on it, Jev first (task rust-latest-reevaluates; @nha, 2026-11-12:
//! « if something is waiting a "latest triggers a re-evaluation" chepest first : JEV-first »). Each contact, lead and
//! address an open draft writes to holds its latest message from every channel pulled (mail, then the leads' timelines); a
//! waiting draft whose latest is newer than its last judgement gets Jev's five questions, and the verdict acts: the draft
//! moot (its ask leaves his list), flagged for update, or kept. Pure: the binary brings the mail, the store's rows, Jev and
//! the older app's writes (crates/dash/src/latest.rs). The Hostman draft that promised an account ID outlived his own mail
//! sending it (2026-11-09).
use dash_sdk::Connection;
use serde_json::{Value as Json, json};
use std::collections::{BTreeMap, BTreeSet};

/// Its rows in the drafts' file: `contact:<id>`, `lead:<id>`, `address:<email>` hold a latest message, `draft:<id>` the
/// last judgement of that draft, Jev's answers in it.
pub const TABLE: &str = "latest";
/// Its table, one record a row as the drafts' (`crate::SCHEMA`); the binary opens it beside theirs.
pub const SCHEMA: &str =
    "CREATE TABLE IF NOT EXISTS latest (id TEXT PRIMARY KEY, record TEXT) WITHOUT ROWID;";
/// The older app's open statuses (`Write.editable?/1` and approved): a draft that still waits to go.
pub const OPEN: [&str; 3] = ["pending", "approved-pending-changes", "approved"];
/// His own addresses: the send path's, which drops them from a copy, as a mail between two of them names nobody.
pub use crate::send::HIS;

/// One message of a channel, either way.
#[derive(Clone, Debug, PartialEq)]
pub struct Message {
    pub id: String,
    /// `email`, `linkedin`, `call`, as the channel names itself.
    pub channel: String,
    /// ISO UTC, or the day alone where the channel keeps no time (a lead's timeline): a day sorts before its times.
    pub at: String,
    pub from: String,
    /// Every address it went to, copies included.
    pub to: Vec<String>,
    /// The parties it names directly, with no address: `lead:<id>` for a timeline's event.
    pub parties: Vec<String>,
    pub subject: String,
    pub preview: String,
}

impl Message {
    fn touches(&self, key: &str, addresses: &[String]) -> bool {
        self.parties.iter().any(|p| p == key)
            || std::iter::once(&self.from)
                .chain(&self.to)
                .any(|a| addresses.contains(a))
    }

    fn to_json(&self) -> Json {
        json!({"id": self.id, "channel": self.channel, "at": self.at, "from": self.from, "to": self.to,
               "parties": self.parties, "subject": self.subject, "preview": self.preview})
    }

    fn from_json(j: &Json) -> Option<Message> {
        let s = |k: &str| j[k].as_str().map(str::to_string);
        let list = |k: &str| -> Vec<String> {
            (j[k].as_array().into_iter().flatten())
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        };
        Some(Message {
            id: s("id")?,
            channel: s("channel").unwrap_or_default(),
            at: s("at")?,
            from: s("from").unwrap_or_default(),
            to: list("to"),
            parties: list("parties"),
            subject: s("subject").unwrap_or_default(),
            preview: s("preview").unwrap_or_default(),
        })
    }
}

/// A draft's goal: its context from « Goal: » on, else the whole context (the drafts' list reads it too).
pub fn goal(context: &str) -> &str {
    context
        .find("Goal: ")
        .map_or(context, |at| &context[at + 6..])
}

/// The addresses in `text`, lowercased, each once, his own left out: each entry the send path's validator (`send::addresses`)
/// takes whole, so a draft is matched to the parties it is sent to (review of PR 1528); an entry that fails is no party, never
/// a guessed one (review of PR 1780).
pub fn addresses(text: &str) -> Vec<String> {
    kept(crate::send::addresses(text))
}

/// Validated addresses as parties key them: lowercased, each once, his own left out. The one normalization of a draft's
/// `to` and a contact's stored field (/code-review).
fn kept(each: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for a in each {
        let a = a.to_lowercase();
        if !HIS.contains(&a.as_str()) && !out.contains(&a) {
            out.push(a);
        }
    }
    out
}

/// One party: its key and the addresses it writes from.
pub type Party = (String, Vec<String>);

/// Each party's newest message among `messages`; a party none touches holds none.
pub fn latest(parties: &[Party], messages: &[Message]) -> BTreeMap<String, Message> {
    let mut out = BTreeMap::new();
    for (key, addresses) in parties {
        let newest = (messages.iter())
            .filter(|m| m.touches(key, addresses))
            .max_by(|a, b| (&a.at, &a.id).cmp(&(&b.at, &b.id)));
        if let Some(m) = newest {
            out.insert(key.clone(), m.clone());
        }
    }
    out
}

/// For each party, the newer of what it held and what this pass read: a mail that leaves the window read never takes a
/// party back to an older message (/code-review).
pub fn newer(
    held: &BTreeMap<String, Message>,
    mut now: BTreeMap<String, Message>,
) -> BTreeMap<String, Message> {
    for (k, h) in held {
        if now.get(k).is_none_or(|n| (&h.at, &h.id) > (&n.at, &n.id)) {
            now.insert(k.clone(), h.clone());
        }
    }
    now
}

/// The attempts on one message before a draft is tried only once every `RETRY_MS`: a draft Jev or the older app keeps
/// failing on never holds the pass's calls from the drafts after it (/code-review), and an outage longer than `TRIES` passes
/// never leaves a draft unjudged for good (review of PR 1528).
pub const TRIES: u64 = 3;
pub const RETRY_MS: i64 = 3_600_000;

/// The record of a failed judgement of `draft` on `m` at `at_ms`: its attempts on that message counted.
pub fn failed(held: &Held, draft: &str, m: &Message, why: &str, at_ms: i64) -> Json {
    let before = (held.judged.get(draft))
        .filter(|j| j["message"]["id"].as_str() == Some(m.id.as_str()))
        .and_then(|j| j["failed"].as_u64())
        .unwrap_or(0);
    json!({"message": {"id": m.id, "at": m.at}, "failed": before + 1, "why": why, "at_ms": at_ms})
}

/// Its row of when the latest pass last read the older app's drafts whole: a draft it found unchanged is read as of then,
/// with no write of its own (/code-review of PR 1662: a new read time in each draft rewrote them all every pass).
const PASSED: &str = "pass:drafts";

/// The pass read the older app's drafts at `at_ms`. Inside the caller's transaction.
pub fn passed(db: &Connection, at_ms: i64) -> Result<(), String> {
    crate::table::put(db, TABLE, PASSED, &json!({"at_ms": at_ms}).to_string())
}

/// When the pass last read the older app's drafts, `None` before its first.
pub fn passed_at(db: &Connection) -> Result<Option<i64>, String> {
    let row = crate::table::records(db, TABLE)?.remove(PASSED);
    Ok(row.and_then(|r| serde_json::from_str::<Json>(&r).ok()?["at_ms"].as_i64()))
}

/// What the table holds: each party's latest, and each draft's last judgement.
#[derive(Debug, Default, PartialEq)]
pub struct Held {
    pub latest: BTreeMap<String, Message>,
    pub judged: BTreeMap<String, Json>,
}

/// The table as it stands.
pub fn load(db: &Connection) -> Result<Held, String> {
    let mut held = Held::default();
    for (row, record) in crate::table::records(db, TABLE)? {
        let Ok(j) = serde_json::from_str::<Json>(&record) else {
            continue;
        };
        if row == PASSED {
            continue;
        }
        match row.strip_prefix("draft:") {
            Some(id) => {
                held.judged.insert(id.to_string(), j);
            }
            None => {
                if let Some(m) = Message::from_json(&j) {
                    held.latest.insert(row, m);
                }
            }
        }
    }
    Ok(held)
}

/// Writes each party's latest that differs from `held`'s, and says which parties have a new one. Inside the caller's
/// transaction.
pub fn save(
    db: &Connection,
    held: &Held,
    now: &BTreeMap<String, Message>,
) -> Result<Vec<String>, String> {
    let new: Vec<(&String, &Message)> = (now.iter())
        .filter(|(k, m)| held.latest.get(*k) != Some(*m))
        .collect();
    for (k, m) in &new {
        crate::table::put(db, TABLE, k, &m.to_json().to_string())?;
    }
    Ok(new.into_iter().map(|(k, _)| k.clone()).collect())
}

/// A draft's judgement kept: the message judged, Jev's answers, the verdict and what it did. Its changeset keeps every one.
pub fn judged(db: &Connection, draft: &str, record: Json) -> Result<(), String> {
    crate::table::put(db, TABLE, &format!("draft:{draft}"), &record.to_string())
}

/// A waiting draft, as the older app's `dash model drafts` gives it.
#[derive(Clone, Debug, PartialEq)]
pub struct Waiting {
    pub id: String,
    pub status: String,
    pub receiver: String,
    pub goal: String,
    pub text: String,
    /// The day it was written: a message before it, it knew.
    pub created: String,
    pub lead: Option<String>,
    /// The addresses its « cc <name> » entries name through a contact card (`named_copies`): its parties too, as a copy by
    /// address is (review of PR 1780).
    pub copies: Vec<String>,
}

impl Waiting {
    /// An open draft of the older app's list; none for a closed one.
    pub fn of(d: &Json) -> Option<Waiting> {
        let s = |k: &str| {
            let v = d[k].as_str().unwrap_or_default();
            v.strip_prefix("~:").unwrap_or(v).to_string()
        };
        let status = s("status");
        if !OPEN.contains(&status.as_str()) {
            return None;
        }
        Some(Waiting {
            id: s("id"),
            receiver: s("to"),
            goal: goal(&s("context")).to_string(),
            text: s("body"),
            created: s("created"),
            lead: Some(s("opportunity")).filter(|l| !l.is_empty()),
            status,
            copies: vec![],
        })
    }

    /// The parties it waits on: its addresses, its copies by name, its lead.
    pub fn parties(&self) -> Vec<String> {
        let mut keys: Vec<String> = (addresses(&self.receiver).into_iter())
            .chain(self.copies.iter().cloned())
            .map(|a| format!("address:{a}"))
            .collect();
        keys.extend(self.lead.iter().map(|l| format!("lead:{l}")));
        keys
    }
}

/// Each waiting draft with the message to judge it on: its parties' newest, when that is on or after the day it was written
/// and is not the message it was last judged on. A draft written after its latest already knew it: no call. Past `TRIES`
/// failures on that message, once every `RETRY_MS` as of `now_ms`.
pub fn due<'a>(
    waiting: &'a [Waiting],
    held: &'a Held,
    now_ms: i64,
) -> Vec<(&'a Waiting, &'a Message)> {
    let mut out = vec![];
    for w in waiting {
        let newest = (w.parties().iter())
            .filter_map(|k| held.latest.get(k))
            .max_by(|a, b| (&a.at, &a.id).cmp(&(&b.at, &b.id)));
        let Some(m) = newest else { continue };
        // to the second when both carry a time, else by day (/code-review of PR 1528: a reply written the same day was due)
        let (at, created) = match (m.at.len() > 10, w.created.len() > 10) {
            (true, true) => (m.at.as_str(), w.created.as_str()),
            _ => (
                m.at.get(..10).unwrap_or(&m.at),
                w.created.get(..10).unwrap_or(&w.created),
            ),
        };
        // an approved draft's own send lands before it is marked sent: his messages never re-judge it
        if w.status == "approved" && HIS.contains(&m.from.as_str()) {
            continue;
        }
        let last =
            (held.judged.get(&w.id)).filter(|j| j["message"]["id"].as_str() == Some(m.id.as_str()));
        // past TRIES, a wait that doubles with each failure, up to 64 times RETRY_MS: a draft that always fails costs a call
        // a few days apart, never one an hour for good (review of the recovery)
        let resting = |j: &Json| {
            let failed = j["failed"].as_u64().unwrap_or(0);
            let wait = RETRY_MS << failed.saturating_sub(TRIES).min(6);
            failed >= TRIES && now_ms - j["at_ms"].as_i64().unwrap_or(0) < wait
        };
        let seen = last.is_some_and(|j| j.get("verdict").is_some() || resting(j));
        if at >= created && !seen {
            out.push((w, m));
        }
    }
    out
}

/// Jev's five questions on a waiting draft and its party's latest message (Jev's question form, as rfc_judge's): each a
/// noul, true near 1.
pub const QUESTIONS: &str = r#"{
  "still_latest": {"type": "noul",
    "instructions": "`draft` is a message Nicolas has not sent yet to `draft.receiver`. `latest` is the newest message between them, from either side (`latest.from` is Nicolas when it is his). Is the draft still the right next message after `latest`?",
    "criteria": {"true": "`latest` is what the draft answers, or adds nothing the draft must take into account.", "false": "`latest` came after the draft was written and the draft no longer follows from it."}},
  "change": {"type": "noul",
    "instructions": "Does `latest` change the situation `draft.goal` describes: a new fact, a decision, a step done, a question asked or answered?",
    "criteria": {"true": "`latest` brings something the draft's goal or text did not know.", "false": "`latest` brings nothing new for the draft."}},
  "update": {"type": "noul",
    "instructions": "Should the text of `draft` be rewritten before it is sent, to take `latest` into account, while its goal still stands?",
    "criteria": {"true": "Some sentence of the draft is now wrong, stale or missing a point `latest` raises.", "false": "The draft can go as it is."}},
  "discard": {"type": "noul",
    "instructions": "Is the draft now pointless: `latest` already says or does what the draft says or promises, or makes its goal moot?",
    "criteria": {"true": "Sending the draft now would repeat `latest` or answer something already settled.", "false": "The draft still has a job to do."}},
  "answered": {"type": "noul",
    "instructions": "Does `latest` already give what the draft asks for or waits on (an answer, an ID, a document, a decision)?",
    "criteria": {"true": "What the draft asks or waits for is in `latest`.", "false": "The draft's ask is still open."}}
}"#;

/// The state Jev reads: the draft and the message.
pub fn state(w: &Waiting, m: &Message) -> Json {
    let his = HIS.contains(&m.from.as_str());
    json!({
        "draft": {"receiver": w.receiver, "goal": w.goal, "text": w.text, "written_on": w.created, "status": w.status},
        "latest": {"channel": m.channel, "at": m.at, "from": if his { "Nicolas" } else { m.from.as_str() },
                   "to": m.to, "subject": m.subject, "text": m.preview},
        "reader": "an agent deciding whether a draft still waiting to go to this person must be dropped or rewritten"
    })
}

/// What a judgement does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Verdict {
    /// The draft moot: its ask leaves his list.
    Moot,
    /// The draft flagged for rewriting; an approved one back to pending so it does not go out stale.
    Update,
    Keep,
}

impl Verdict {
    /// Its word in a judgement's line and record.
    pub fn name(self) -> &'static str {
        match self {
            Verdict::Moot => "moot",
            Verdict::Update => "update",
            Verdict::Keep => "kept",
        }
    }
}

/// Past this, a moot verdict acts: a moot draft can be set back to pending, so the bar is no higher.
pub const MOOT_BAR: f64 = 0.7;
pub const UPDATE_BAR: f64 = 0.5;

/// The verdict on Jev's answers, a missing answer read as 0.5 (no opinion). Discarded or answered first: a draft whose ask
/// is met goes before any rewrite.
pub fn verdict(answers: &BTreeMap<String, f64>) -> Verdict {
    let a = |k: &str| answers.get(k).copied().unwrap_or(0.5);
    if a("discard") >= MOOT_BAR || a("answered") >= MOOT_BAR {
        Verdict::Moot
    // « change » is context, never a rewrite alone: most replies bring something new (/code-review of PR 1528)
    // an update takes his approval back: it needs Jev's opinion past 0.5, never a missing answer's 0.5 (review of PR 1528)
    } else if a("update") > UPDATE_BAR || a("still_latest") < 1.0 - UPDATE_BAR {
        Verdict::Update
    } else {
        Verdict::Keep
    }
}

/// The older app's words for a verdict on `w`, none for a kept draft: moot through `dash draft status`, an update as a
/// `:why` key the next writer reads, an approved draft back to pending first.
pub fn acts(w: &Waiting, v: Verdict, m: &Message) -> Vec<Vec<String>> {
    let what = match m.subject.as_str() {
        "" => m.preview.chars().take(80).collect(),
        s => s.to_string(),
    };
    let whose = if HIS.contains(&m.from.as_str()) {
        "his own"
    } else {
        "their"
    };
    let line = format!("{} on {}: {whose} « {what} » ({})", m.channel, m.at, m.id);
    let words = |w: &[&str]| w.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    match v {
        Verdict::Keep => vec![],
        Verdict::Moot => vec![words(&["draft", "status", &w.id, "moot"])],
        Verdict::Update => {
            let mut out = vec![];
            // one approved with his corrections too: a session would apply them to the stale text (review of PR 1528)
            if w.status != "pending" {
                out.push(words(&["draft", "status", &w.id, "pending"]));
            }
            let why = format!("rewrite for the latest message, Jev's verdict: {line}");
            out.push(words(&["draft", "set", &w.id, "--why.update", &why]));
            out
        }
    }
}

/// A contact's stored email field, the store's own: a JSON list (its strings), a JSON string, or addresses a space, a line
/// break, `,` or `;` apart, each validated whole as the send path's (/code-review: the draft `to`'s split lost every address
/// of such a field; a JSON scalar or a list holding a null lost them all).
fn stored(text: &str) -> Vec<String> {
    let apart = |s: &str| -> Vec<String> {
        let cut = |c: char| c.is_whitespace() || c == ',' || c == ';';
        s.split(cut).map(String::from).collect()
    };
    let each = match serde_json::from_str::<Json>(text) {
        Ok(Json::Array(list)) => list
            .iter()
            .filter_map(Json::as_str)
            .flat_map(apart)
            .collect(),
        Ok(Json::String(one)) => apart(&one),
        _ => apart(text),
    };
    kept(
        each.iter()
            .filter_map(|e| crate::send::recipient(e))
            .map(String::from),
    )
}

/// Each waiting draft's « cc <name> » entries resolved through the contacts' cards, as the send path resolves them: a name
/// one card answers with a valid address is a copy, so a party (review of PR 1780: a copy's reply is a reply on the thread).
/// A name no card or more than one card answers, or a card holding junk, is no party: the send refuses it anyway.
fn named_copies(waiting: &mut [Waiting], rows: &[(String, String, String)]) {
    let cards: Vec<(String, String)> = (rows.iter())
        .filter(|(key, _, name)| key.starts_with("contact:") && !name.is_empty())
        .filter_map(|(_, text, name)| Some((name.to_lowercase(), stored(text).into_iter().next()?)))
        .collect();
    for w in waiting {
        let names = (w.receiver.split([',', ';']))
            .filter_map(|e| crate::send::copy_of(e.trim()))
            .filter(|rest| !rest.contains('@'));
        let found: Vec<String> = names
            .filter_map(|rest| match crate::send::named(rest, &cards).as_deref() {
                Ok([one]) => Some(one.to_lowercase()),
                _ => None,
            })
            .collect();
        w.copies = kept(found);
    }
}

/// The parties, each once: the store's contacts and leads as `rows` give them (a key and an email text, a lead once a contact),
/// then every address a waiting draft writes to and every lead it is on that no row names.
/// Each waiting draft's named copies are resolved here first (`named_copies`), so its own `parties` holds them too.
pub fn parties(rows: &[(String, String, String)], waiting: &mut [Waiting]) -> Vec<Party> {
    named_copies(waiting, rows);
    let mut out: Vec<Party> = vec![];
    for (key, text, _) in rows {
        let found = stored(text);
        match out.iter_mut().find(|(k, _)| k == key) {
            Some((_, have)) => {
                let more: Vec<String> = (found.into_iter()).filter(|a| !have.contains(a)).collect();
                have.extend(more);
            }
            None => out.push((key.clone(), found)),
        }
    }
    let mut seen: BTreeSet<String> = out.iter().map(|(k, _)| k.clone()).collect();
    for w in waiting {
        for a in addresses(&w.receiver)
            .into_iter()
            .chain(w.copies.iter().cloned())
        {
            let key = format!("address:{a}");
            if seen.insert(key.clone()) {
                out.push((key, vec![a]));
            }
        }
        // a lead whose contacts hold no email still holds its timeline's latest (review of PR 1528)
        if let Some(l) = &w.lead {
            let key = format!("lead:{l}");
            if seen.insert(key.clone()) {
                out.push((key, vec![]));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_sdk::{dash_meter, dash_test};

    fn mail(id: &str, at: &str, from: &str, to: &str) -> Message {
        Message {
            id: id.into(),
            channel: "email".into(),
            at: at.into(),
            from: from.into(),
            to: vec![to.into()],
            parties: vec![],
            subject: "Grant".into(),
            preview: format!("mail {id}"),
        }
    }

    /// Done-when 1, the pure part: a contact holds its newest message either way, his own included; a lead its timeline's
    /// newer event over an older mail; a party nobody wrote holds none; a second save of the same latest writes nothing.
    #[dash_test]
    fn each_party_holds_its_newest_message_either_way() {
        let contact: Party = ("contact:sam".into(), vec!["sam@example.com".into()]);
        let lead: Party = ("lead:acme".into(), vec!["sam@example.com".into()]);
        let quiet: Party = ("contact:nobody".into(), vec!["no@example.com".into()]);
        let mut event = mail("t1", "2026-11-10", "", "");
        (event.channel, event.parties, event.to) =
            ("linkedin".into(), vec!["lead:acme".into()], vec![]);
        let all = [
            mail(
                "m1",
                "2026-11-09T06:46:00Z",
                "sam@example.com",
                "nicolas@klar.im",
            ),
            mail(
                "m2",
                "2026-11-09T10:16:00Z",
                "nicolas@klar.im",
                "sam@example.com",
            ),
            event,
            mail(
                "m3",
                "2026-11-09T11:00:00Z",
                "other@example.org",
                "nicolas@klar.im",
            ),
        ];
        let now = latest(&[contact, lead, quiet], &all);
        assert_eq!(now["contact:sam"].id, "m2", "his own mail is the latest");
        assert_eq!(now["lead:acme"].id, "t1", "a later day's timeline event");
        assert!(!now.contains_key("contact:nobody"));
        let log = crate::table::memory(&[SCHEMA]);
        let held = load(&log).expect("empty");
        assert_eq!(
            save(&log, &held, &now).expect("saved"),
            ["contact:sam", "lead:acme"]
        );
        let held = load(&log).expect("read back");
        assert_eq!(held.latest, now);
        assert!(
            save(&log, &held, &now).expect("again").is_empty(),
            "nothing new"
        );
    }

    #[dash_test]
    fn a_receiver_line_gives_its_addresses_his_own_left_out() {
        let to = "Sam B. (Example grants), Sam.B@example.com, cc nicolas@klar.im, deciding on <x@y.org>.";
        // « deciding on <x@y.org>. » is no entry the validator takes whole: no party, never a guessed one
        assert_eq!(addresses(to), ["sam.b@example.com"]);
        assert!(addresses("Sam, on LinkedIn").is_empty());
        // an address is never cut inside: an accented letter and a local part's `&` stay; a list still splits at « ; »
        // (review of PR 1780)
        assert_eq!(
            addresses("josé.martin@x.fr; a&b@corp.com"),
            ["josé.martin@x.fr", "a&b@corp.com"]
        );
    }

    fn waiting(id: &str, created: &str) -> Waiting {
        Waiting {
            id: id.into(),
            status: "pending".into(),
            receiver: "Sam (Example), sam@example.com".into(),
            goal: "get the test account topped up".into(),
            text: "I will create the account and send you its ID.".into(),
            created: created.into(),
            lead: None,
            copies: vec![],
        }
    }

    /// Cheapest first: a draft written after its latest is not asked, one already judged on that message neither.
    #[dash_test]
    fn only_a_draft_older_than_its_latest_and_not_judged_on_it_is_due() {
        let m = mail(
            "m2",
            "2026-11-09T10:16:00Z",
            "nicolas@klar.im",
            "sam@example.com",
        );
        let mut held = Held::default();
        held.latest.insert("address:sam@example.com".into(), m);
        let ws = [waiting("old", "2026-11-09"), waiting("newer", "2026-11-10")];
        let due_ids = |h: &Held| {
            due(&ws, h, 0)
                .iter()
                .map(|(w, _)| w.id.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(due_ids(&held), ["old"]);
        held.judged.insert(
            "old".into(),
            json!({"message": {"id": "m2"}, "verdict": "kept"}),
        );
        assert!(due_ids(&held).is_empty(), "judged on it");
        // to the second when both carry a time: a reply written the same day after the message knew it (/code-review of PR 1528)
        let later = [waiting("same-day", "2026-11-09T15:00:00Z")];
        assert!(
            due(&later, &held, 0).is_empty(),
            "written after it, same day"
        );
        // an approved draft's own send is his latest until it is marked sent: never judged on it
        let approved = [Waiting {
            status: "approved".into(),
            ..waiting("sending", "2026-11-09")
        }];
        assert!(due(&approved, &held, 0).is_empty(), "its own send");
    }

    /// A lead no contact row names (its contacts hold no email) is a party all the same, so its timeline is read (review of
    /// PR 1528).
    #[dash_test]
    fn a_waiting_draft_s_lead_is_a_party_without_a_contact_row() {
        let w = Waiting {
            lead: Some("acme".into()),
            ..waiting("d", "2026-11-09")
        };
        let keys: Vec<String> = parties(&[], &mut [w]).into_iter().map(|(k, _)| k).collect();
        assert_eq!(keys, ["address:sam@example.com", "lead:acme"]);
        // a contact's stored field, a JSON list or addresses a space or a line apart, and a draft's copy are parties too
        // (/code-review)
        let row =
            |k: &str, text: &str, name: &str| (k.to_string(), text.to_string(), name.to_string());
        let rows = [
            row("contact:a", r#"["a@x.com","b@y.com",null]"#, "Ann"),
            row("contact:c", "c@x.com\nd@y.com e@z.com", "Carl Dupont"),
            row("contact:s", r#""s@x.com""#, "Sam"),
            row("contact:b1", "ben@x.com", "Ben"),
            row("contact:b2", "ben@y.com", "Ben"),
        ];
        // a « cc <name> » copy one card answers is a party; one more cards answer is none, as the send refuses it
        let copied = Waiting {
            receiver: "bob@x.org, cc Carl Dupont; cc Ben".into(),
            ..waiting("d", "2026-11-09")
        };
        let mut drafts = [copied];
        let got = parties(&rows, &mut drafts);
        assert_eq!(got[0].1, ["a@x.com", "b@y.com"], "a list with a null");
        assert_eq!(got[1].1, ["c@x.com", "d@y.com", "e@z.com"]);
        assert_eq!(got[2].1, ["s@x.com"], "a JSON string");
        assert_eq!(drafts[0].copies, ["c@x.com"]);
        assert!(drafts[0].parties().contains(&"address:c@x.com".to_string()));
        assert_eq!(
            addresses("bob@x.org, cc carl@y.org"),
            ["bob@x.org", "carl@y.org"]
        );
    }

    /// A latest never moves back to an older message (a mail out of the window read); a draft failing on one message is
    /// tried `TRIES` times, then once an hour, never dropped for good; a `created` with a time compares by its day
    /// (/code-review, review of PR 1528).
    #[dash_test]
    fn a_latest_never_moves_back_and_a_failing_draft_is_tried_three_times_then_hourly() {
        let new = mail(
            "m3",
            "2026-11-09T10:31:00Z",
            "sam@example.com",
            "nicolas@klar.im",
        );
        let old = mail("t0", "2026-11-08", "", "");
        let held = BTreeMap::from([("address:sam@example.com".to_string(), new.clone())]);
        let now = BTreeMap::from([("address:sam@example.com".to_string(), old)]);
        assert_eq!(newer(&held, now)["address:sam@example.com"], new);
        let mut h = Held::default();
        h.latest
            .insert("address:sam@example.com".into(), new.clone());
        let ws = [waiting("d", "2026-11-09T09:00:00Z")];
        let at = 1_000;
        for n in 1..TRIES {
            let rec = failed(&h, "d", &new, "Jev answered 500", at);
            assert_eq!(rec["failed"], n);
            h.judged.insert("d".into(), rec);
            assert_eq!(due(&ws, &h, at).len(), 1, "tried again after {n}");
        }
        h.judged
            .insert("d".into(), failed(&h, "d", &new, "Jev answered 500", at));
        assert!(
            due(&ws, &h, at + RETRY_MS - 1).is_empty(),
            "rests after {TRIES}"
        );
        assert_eq!(due(&ws, &h, at + RETRY_MS).len(), 1, "then after an hour");
        h.judged
            .insert("d".into(), failed(&h, "d", &new, "Jev answered 500", at));
        assert!(due(&ws, &h, at + RETRY_MS).is_empty(), "then twice as long");
        assert_eq!(due(&ws, &h, at + 2 * RETRY_MS).len(), 1);
    }

    /// Done-when 2 and 3: each verdict and what it does in the older app.
    #[dash_test]
    fn each_verdict_acts_through_the_older_app() {
        let a = |pairs: &[(&str, f64)]| pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect();
        let m = mail(
            "m2",
            "2026-11-09T10:16:00Z",
            "nicolas@klar.im",
            "sam@example.com",
        );
        let w = waiting("d", "2026-11-09");
        let moot = verdict(&a(&[("discard", 0.9), ("answered", 0.8), ("update", 0.7)]));
        assert_eq!(moot, Verdict::Moot);
        assert_eq!(acts(&w, moot, &m), [["draft", "status", "d", "moot"]]);
        let up = verdict(&a(&[
            ("discard", 0.2),
            ("answered", 0.1),
            ("update", 0.8),
            ("still_latest", 0.4),
        ]));
        assert_eq!(up, Verdict::Update);
        let approved = Waiting {
            status: "approved".into(),
            ..w.clone()
        };
        let words = acts(&approved, up, &m);
        assert_eq!(
            words[0],
            ["draft", "status", "d", "pending"],
            "never sent stale"
        );
        assert_eq!(words[1][..4], ["draft", "set", "d", "--why.update"]);
        assert!(
            words[1][4].contains("his own « Grant » (m2)"),
            "{}",
            words[1][4]
        );
        let keep = verdict(&a(&[
            ("discard", 0.1),
            ("answered", 0.1),
            ("update", 0.2),
            ("change", 0.2),
            ("still_latest", 0.9),
        ]));
        assert_eq!(keep, Verdict::Keep);
        assert!(acts(&w, keep, &m).is_empty());
        // review of PR 1528: his corrections are never applied to a stale text, and no opinion takes no approval back
        let corrected = Waiting {
            status: "approved-pending-changes".into(),
            ..w.clone()
        };
        assert_eq!(
            acts(&corrected, up, &m)[0],
            ["draft", "status", "d", "pending"]
        );
        let unsure = a(&[("discard", 0.1), ("answered", 0.1), ("still_latest", 0.8)]);
        assert_eq!(
            verdict(&unsure),
            Verdict::Keep,
            "a missing update reads 0.5"
        );
        // something new alone is no rewrite: Jev says the draft can go as it is (/code-review of PR 1528)
        let news = verdict(&a(&[
            ("discard", 0.1),
            ("answered", 0.1),
            ("update", 0.2),
            ("change", 0.9),
            ("still_latest", 0.9),
        ]));
        assert_eq!(news, Verdict::Keep);
    }

    #[dash_test]
    fn the_questions_are_jev_s_five_nouls() {
        let q: Json = serde_json::from_str(QUESTIONS).expect("JSON");
        let mut names: Vec<&String> = q.as_object().expect("map").keys().collect();
        names.sort();
        assert_eq!(
            names,
            ["answered", "change", "discard", "still_latest", "update"]
        );
        assert!(
            q.as_object()
                .expect("map")
                .values()
                .all(|v| v["type"] == "noul")
        );
    }
}
