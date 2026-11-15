//! A reply draft's card under the mail it answers (rfc-rust-plugins-ui sections 3 and 4, task draft-from-is-address): its goal,
//! sender and receiver first, every check of its channel run before he sees it, then its words in the channel's compose box
//! with Send. A draft failing a check shows no Send, and goes back to its agent when the fault is its own (a check that did not
//! answer only waits). Send waits `UNDO_MS` on the host's clock, so Undo stops it any time before it begins; then it goes,
//! once, and is logged once. Drafts reaches the channel, the compose
//! box and the thread by key through the core, never by import (principle 7).
use crate::{Draft, load, open};
use dash_core::{Answer, Asked, Check, FieldWrite, Host, Message, OpLog, TEXT, Tree, Value, json};
use serde_json::Value as Json;
use std::collections::BTreeMap;
use std::sync::Mutex;

const TABLE: &str = "draft_sends";
const FIELD: &str = "record";
const PAGE: i64 = 1000;
/// Send's window for an Undo, as Gmail's: a tap by mistake is taken back inside it.
pub const UNDO_MS: i64 = 10_000;
/// Send's post, `<SEND><draft>`, and Undo's.
pub const SEND: &str = "/drafts/send/";
pub const UNDO: &str = "/drafts/undo/";
/// Where the card is placed: under a conversation's mails, drafts' order among the plugins placing there.
pub const PLACED: &str = "mail/thread/10-drafts";

/// One read and write of a send's record at a time: a second tap, an Undo and the timer's send never cross. Never held across
/// a network read or a send.
static ONE: Mutex<()> = Mutex::new(());

/// Each draft's last send record, by its id.
/// ponytail: the whole log read, as `load` reads it; a read of the one row once the core has a read port.
fn records(log: &dyn OpLog) -> Result<BTreeMap<String, Json>, String> {
    let (mut last, mut after) = (BTreeMap::new(), None::<(i64, String)>);
    loop {
        let ops = log.ops_after(after.as_ref().map(|(s, w)| (*s, w.as_str())), PAGE)?;
        let Some(op) = ops.last() else {
            return Ok(last);
        };
        after = Some((op.stamp, op.writer.clone()));
        for w in ops.iter().map(|op| &op.write) {
            if let (TABLE, FIELD, Value::Text(t)) = (w.table.as_str(), w.field.as_str(), &w.value) {
                last.insert(w.row.clone(), serde_json::from_str(t).unwrap_or_default());
            }
        }
    }
}

/// Draft `id`'s last send record, null with none.
fn record(log: &dyn OpLog, id: &str) -> Result<Json, String> {
    Ok(records(log)?.remove(id).unwrap_or_default())
}

/// Each send a card made or began, as the outbound log's lines name a draft: `dash mail send` reads them with the log's, so a
/// draft sent from its card never goes a second time, and is marked sent in the older app there. A send begun and never
/// recorded as done (its record's write failed after the mail went) counts as gone: a second mail is the worse mistake.
pub fn sent(log: &dyn OpLog) -> Result<Vec<Json>, String> {
    let all = records(log)?
        .into_iter()
        .filter(|(_, r)| r["state"] == "sent" || r["state"] == "sending");
    let lines = all.map(|(id, r)| json!({"draft": id, "email": r["email"], "channel": "email", "tool": "draft-card"}));
    Ok(lines.collect())
}

/// `dash mail send`'s refusal of draft `id` at `now_ms` while its card's Send waits its Undo window (and a timer's second
/// after it).
/// ponytail: two processes, two locks; the window between this read and the card's own is the send's few milliseconds.
pub fn waits(log: &dyn OpLog, id: &str, now_ms: i64) -> Result<(), String> {
    let last = record(log, id)?;
    let due = last["due"].as_i64().unwrap_or(0);
    if last["state"] == "waiting" && now_ms < due + UNDO_MS {
        return Err(format!(
            "{id}: its card's Send waits its Undo window on /mail; nothing sent"
        ));
    }
    Ok(())
}

/// The outbound log's `line` of a mail `dash mail send` sent: its draft's card shows it went, never its Send again. A line
/// naming no draft or no mail writes nothing.
pub fn went(log: &mut dyn OpLog, line: &Json) -> Result<(), String> {
    let (Some(id), Some(email)) = (line["draft"].as_str(), line["email"].as_str()) else {
        return Ok(());
    };
    let record = json!({"state": "sent", "email": email, "by": "dash mail send"});
    write(log, id, record).map_err(|e| {
        format!("draft {id}: sent as {email}, but its card's record did not write: {e}")
    })
}

fn write(log: &mut dyn OpLog, id: &str, record: Json) -> Result<(), String> {
    let row = FieldWrite {
        table: TABLE.into(),
        row: id.into(),
        field: FIELD.into(),
        value: Value::Text(record.to_string()),
    };
    log.write(&[row]).map(drop)
}

fn lock() -> std::sync::MutexGuard<'static, ()> {
    ONE.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// `d` as its channel would send it, answering thread `key`, its words `text` (its last version's when empty), as plain
/// text: a `**bold**` marker goes as its words, as `dash mail send`'s text part (the channel sends no HTML part).
fn message(d: &Draft, key: &str, text: &str) -> Message {
    let last = d.versions.last().map_or("", |v| v.text.as_str());
    let text = if text.trim().is_empty() { last } else { text };
    Message {
        key: format!("{}:v{}", d.id, d.versions.len()),
        from: d.sender.trim().into(),
        to: (d
            .receiver
            .split(',')
            .map(str::trim)
            .filter(|a| !a.is_empty())
            .map(String::from))
        .collect(),
        subject: String::new(),
        text: text.replace("**", "").trim().into(),
        answers: Some(key.into()),
    }
}

/// Every failed check of `m`: the three things first (a draft missing one shows what is missing), then its channel's, a
/// channel no plugin registers failing it.
fn failed(host: &dyn Host, d: &Draft, m: &Message) -> Vec<Check> {
    let missing = |name, value: &str| Check {
        name,
        fails: value.trim().is_empty().then(|| "missing".into()),
        retry: false,
    };
    let shut = (!crate::OPEN.contains(&d.status.as_str()))
        .then(|| format!("it is {}, not open", d.status));
    let mut checks = vec![
        missing("goal", &d.goal),
        missing("sender", &d.sender),
        missing("receiver", &d.receiver),
        Check {
            name: "status",
            fails: shut,
            retry: false,
        },
    ];
    match dash_core::channel(&d.channel) {
        Ok(c) => checks.extend((c.checks)(host, m)),
        Err(e) => checks.push(Check {
            name: "channel",
            fails: Some(e),
            retry: false,
        }),
    }
    checks.retain(|c| c.fails.is_some());
    checks
}

/// Each check of `checks` as « name: why », `held` alone the ones held against the draft (no retry).
fn said(checks: &[Check], held: bool) -> Vec<String> {
    let each = checks.iter().filter(|c| !held || !c.retry);
    each.map(|c| format!("{}: {}", c.name, c.fails.as_deref().unwrap_or_default()))
        .collect()
}

/// Whether `d` answers the thread of server key `key`, titled `subject`: by its key, or by the subject its older record names,
/// each less its « Re: ».
/// ponytail: two threads of one subject both show the draft; the older app names no key, so a key wins once it does.
fn answers(d: &Draft, key: &str, subject: &str) -> bool {
    let base = |s: &str| crate::send::base(s.trim()).trim().to_lowercase();
    let named = d.thread.as_deref().map(str::trim);
    named.is_some_and(|t| t == key || (!subject.is_empty() && base(t) == base(subject)))
}

/// The cards of the open drafts answering `props`' thread (its `thread` key, its `subject`, read as its `account`), each under
/// `id`. No lock while its checks read the network: a card writes only a draft sent back, under the lock (rust/CLAUDE.md rule
/// 5, one slow request never stalls the others).
/// ponytail: each card reads its thread again for its receivers' check; the thread read once in its props when it is slow.
fn under_thread(
    host: &dyn Host,
    props: &Json,
    id: &str,
    tree: &mut Tree,
) -> Result<Vec<String>, String> {
    let key = props["thread"].as_str().ok_or("no thread named")?;
    let subject = props["subject"].as_str().unwrap_or_default();
    let account = props["account"].as_str().unwrap_or_default();
    let log = crate::log(host, &host.data())?;
    let kept: Vec<Draft> = load(&*log)?.drafts.into_values().collect();
    let sends = records(&*log)?;
    let answering = open(&kept).into_iter().filter(|d| answers(d, key, subject));
    let cards = answering.enumerate().map(|(i, d)| {
        let last = sends.get(&d.id).cloned().unwrap_or_default();
        card(host, d, (key, account, &last), &format!("{id}-{i}"), tree)
    });
    cards.collect()
}

/// Draft `d`'s card as `id`, answering thread `key` read as `account`, `last` its send's record: what it is (goal, sender,
/// receiver, its checks' state), then that it went or is going, its Undo while it waits, why it has no Send, or its Send
/// (with why the last did not go, if one did not).
fn card(
    host: &dyn Host,
    d: &Draft,
    (key, account, last): (&str, &str, &Json),
    id: &str,
    tree: &mut Tree,
) -> Result<String, String> {
    let m = message(d, key, "");
    let failed = failed(host, d, &m);
    let shown = |v: &str| {
        if v.trim().is_empty() {
            "nobody yet".to_string()
        } else {
            v.trim().to_string()
        }
    };
    let state = if failed.is_empty() {
        "every check passes"
    } else {
        "a check fails"
    };
    let mut parts = vec![(
        format!("{id}-goal"),
        "text",
        json!({"text": format!("Goal: {}", d.goal_line())}),
    )];
    let who = format!(
        "From {} · To {} · {state}",
        shown(&d.sender),
        shown(&d.receiver)
    );
    parts.push((format!("{id}-who"), "text", json!({"text": who})));
    let said_ = |k: &str| last[k].as_str().unwrap_or_default().to_string();
    // the words that went or wait to go, his edit included, else the draft's
    let words = Some(said_("text"))
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| m.text.clone());
    let quote = (format!("{id}-text"), "quote", json!({"text": words}));
    let due = last["due"].as_i64().unwrap_or(0);
    let waits = said_("state") == "waiting" && host.now_ms() < due;
    let mut roots = vec![];
    match (failed.is_empty(), said_("state").as_str()) {
        (_, "sent") => {
            parts.push((
                format!("{id}-sent"),
                "text",
                json!({"text": format!("Sent ({}).", said_("email"))}),
            ));
            parts.push(quote);
        }
        // begun and not recorded done: it may have gone, so no Send; the thread above shows it once it went
        (_, "sending") => {
            parts.push((
                format!("{id}-going"),
                "text",
                json!({"text": "Going: the conversation above shows it once it went."}),
            ));
            parts.push(quote);
        }
        // a send in its window is his: a check failing on a redraw (a read that timed out) never drops it
        _ if waits => {
            let s = (due - host.now_ms() + 999) / 1000;
            parts.push((
                format!("{id}-wait"),
                "text",
                json!({"text": format!("Sending in {s} s.")}),
            ));
            let undo = format!(
                "{UNDO}{}?thread={}&account={}",
                segment(&d.id),
                segment(key),
                segment(account)
            );
            parts.push((
                format!("{id}-undo"),
                "action",
                json!({"action": undo, "label": "Undo"}),
            ));
            parts.push(quote);
        }
        (false, _) => {
            let held = said(&failed, true);
            // only a fault of the draft goes back to its agent; a check that did not answer may pass on the next view
            let title = if held.is_empty() {
                "No Send yet: a check did not answer"
            } else {
                returned(host, d, &held)?;
                "Back to its agent: no Send"
            };
            let why = said(&failed, false).join("\n");
            parts.push((
                format!("{id}-back"),
                "alert",
                json!({"title": title, "text": why}),
            ));
            parts.push(quote);
        }
        (true, state) => {
            let why = match state {
                "failed" => Some(format!("Not sent: {}", said_("why"))),
                "waiting" => Some("Not sent: the server stopped inside its window.".into()),
                _ => None,
            };
            if let Some(why) = why {
                parts.push((
                    format!("{id}-failed"),
                    "alert",
                    json!({"title": "Send again?", "text": why}),
                ));
            }
            let to = m.to.join(", ");
            let props = json!({"action": format!("{SEND}{}", segment(&d.id)), "send": format!("Send to {to} from {}", m.from),
                "words": m.text, "from": m.from, "to": to, "placeholder": "The reply",
                "hidden": [["thread", key], ["account", account]]});
            let compose = dash_core::element("mail/compose")?;
            roots = (compose.draw)(host, &props, &format!("{id}-compose"), tree)?;
        }
    }
    let mut children: Vec<String> = parts.iter().map(|(k, ..)| k.clone()).collect();
    // the compose box last, its text the box's
    children.extend(roots);
    for (k, kind, props) in parts {
        tree.add(&k, kind, props, &[]);
    }
    tree.add(id, "section", json!({"title": "Reply draft"}), &children);
    Ok(id.to_string())
}

/// `d` back to its agent: `why`, its faults, recorded once for its last version (a reason worded anew on a redraw writes
/// nothing more), where `dash draft show` reads them; a send waiting, going or gone is his, never overwritten.
/// ponytail: the agent reads it there; the comments plugin's queue wakes it in slice 2 (Corrections).
fn returned(host: &dyn Host, d: &Draft, why: &[String]) -> Result<(), String> {
    let _one = lock();
    let mut log = crate::log(host, &host.data())?;
    let last = record(&*log, &d.id)?;
    let version = d.versions.len();
    let in_flight = ["sent", "sending"].contains(&last["state"].as_str().unwrap_or_default())
        || (last["state"] == "waiting" && host.now_ms() < last["due"].as_i64().unwrap_or(0));
    if in_flight || (last["state"] == "returned" && last["version"] == version) {
        return Ok(());
    }
    write(
        &mut *log,
        &d.id,
        json!({"state": "returned", "version": version, "why": why.join("; ")}),
    )
}

/// Why draft `id` went back to its agent, as last recorded; `None` when it did not, or a later step superseded it.
pub fn back_to_agent(log: &dyn OpLog, id: &str) -> Result<Option<String>, String> {
    let last = record(log, id)?;
    let back = (last["state"] == "returned").then(|| last["why"].as_str().map(String::from));
    Ok(back.flatten())
}

/// His Send on draft `id`, answering thread `key`, `text` his words as the box posts them: every check run again (outside the
/// lock: they read the network), then the send waits `UNDO_MS` on the host's clock, the host handed what sends it then. A
/// second tap while it waits queues nothing.
pub fn send(host: &dyn Host, id: &str, key: &str, text: &str) -> Result<(), String> {
    let d = load(&*crate::log(host, &host.data())?)?
        .drafts
        .remove(id)
        .ok_or(format!("no draft {id}"))?;
    let m = message(&d, key, text);
    let failed = failed(host, &d, &m);
    let held = said(&failed, true);
    if !held.is_empty() {
        returned(host, &d, &held)?;
        return Err(format!(
            "{}; back to its agent",
            said(&failed, false).join("; ")
        ));
    }
    if !failed.is_empty() {
        return Err(format!("{}; nothing sent", said(&failed, false).join("; ")));
    }
    let _one = lock();
    let mut log = crate::log(host, &host.data())?;
    let last = record(&*log, id)?;
    match last["state"].as_str() {
        Some("sent") => {
            return Err(format!(
                "it went already ({})",
                last["email"].as_str().unwrap_or_default()
            ));
        }
        Some("sending") => return Err("it is going now".into()),
        Some("waiting") if host.now_ms() < last["due"].as_i64().unwrap_or(0) => return Ok(()),
        _ => {}
    }
    let due = host.now_ms() + UNDO_MS;
    write(
        &mut *log,
        id,
        json!({"state": "waiting", "due": due, "thread": key, "text": m.text}),
    )?;
    if let Err(e) = later(host, id, UNDO_MS) {
        write(&mut *log, id, json!({"state": "undone", "why": e}))?;
        return Err(format!("{e}: nothing sent"));
    }
    Ok(())
}

/// Whether draft `id`'s send still waits to go.
fn waiting(host: &dyn Host, id: &str) -> bool {
    let read = crate::log(host, &host.data()).and_then(|log| record(&*log, id));
    read.is_ok_and(|r| r["state"] == "waiting")
}

/// Draft `id`'s send run `ms` from now, the host handed what runs it then: a timer woken before its window ends (a clock
/// stepped back) waits again.
fn later(host: &dyn Host, id: &str, ms: i64) -> Result<(), String> {
    let id = id.to_string();
    host.after(
        ms,
        Box::new(move |h| match go(h, &id) {
            Ok(None) if waiting(h, &id) => {
                if let Err(e) = later(h, &id, 1000) {
                    eprintln!("draft {id}: not sent: {e}");
                }
            }
            Ok(_) => {}
            Err(e) => eprintln!("draft {id}: not sent: {e}"),
        }),
    )
}

/// His Undo on draft `id`: a send that has not begun never goes, inside its window or past it (a restart lost its timer, and
/// Undo never turns into a send). One begun or gone says so.
pub fn undo(host: &dyn Host, id: &str) -> Result<(), String> {
    let _one = lock();
    let mut log = crate::log(host, &host.data())?;
    let last = record(&*log, id)?;
    match last["state"].as_str() {
        Some("waiting") => write(&mut *log, id, json!({"state": "undone"})),
        Some("sending") => Err("past its window: it is going now".into()),
        Some("sent") => Err(format!(
            "past its window: it went ({})",
            last["email"].as_str().unwrap_or_default()
        )),
        _ => Err("nothing of it waits to go".into()),
    }
}

/// Draft `id`'s send once its window passed and no Undo came: « sending » recorded before the channel's send, so a record that
/// fails after the mail went never offers it again, then its outcome. The lock is held for the records, never across the
/// send (seconds of JMAP). `None`: nothing of it waits, or its window is not over.
pub fn go(host: &dyn Host, id: &str) -> Result<Option<String>, String> {
    let (d, m, key) = {
        let _one = lock();
        let mut log = crate::log(host, &host.data())?;
        let last = record(&*log, id)?;
        let due = last["due"].as_i64().unwrap_or(i64::MAX);
        if last["state"] != "waiting" || host.now_ms() < due {
            return Ok(None);
        }
        let d = load(&*log)?
            .drafts
            .remove(id)
            .ok_or(format!("no draft {id}"))?;
        let key = last["thread"].as_str().unwrap_or_default().to_string();
        let m = message(&d, &key, last["text"].as_str().unwrap_or_default());
        write(
            &mut *log,
            id,
            json!({"state": "sending", "thread": key, "text": m.text}),
        )?;
        (d, m, key)
    };
    let sent = dash_core::channel(&d.channel).and_then(|c| (c.send)(host, &m));
    let record = match &sent {
        Ok(email) => json!({"state": "sent", "email": email, "thread": key, "text": m.text}),
        Err(e) => json!({"state": "failed", "why": e}),
    };
    let _one = lock();
    // a write that fails leaves it « sending »: never offered again, and `sent` names it to `dash mail send`
    write(&mut *crate::log(host, &host.data())?, id, record)?;
    sent.map(Some)
}

/// `s` as one segment of a path or a query's value: letters, digits, - _ . ~ as they are, every other byte %-encoded.
fn segment(s: &str) -> String {
    let each = s.bytes().map(|b| match b {
        b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
            (b as char).to_string()
        }
        _ => format!("%{b:02X}"),
    });
    each.collect()
}

/// Back to the thread the card was on, as the account it was read as (a JMAP thread id is one account's); none named goes to
/// /mail.
fn back(key: &str, account: &str) -> String {
    match (key, account) {
        ("", _) => "/mail".into(),
        (key, "") => format!("/mail/t/{}", segment(key)),
        (key, account) => format!("/mail/t/{}?account={}", segment(key), segment(account)),
    }
}

/// A post's answer: back to its thread, or why not, his words given back so none is lost.
fn answered(
    done: Result<(), String>,
    (key, account): (&str, &str),
    what: &str,
    words: &str,
) -> Answer {
    match done {
        Ok(()) => Answer::Body("303 See Other", TEXT, back(key, account)),
        Err(e) if words.trim().is_empty() => {
            Answer::Body("409 Conflict", TEXT, format!("{what}: {e}\n"))
        }
        Err(e) => Answer::Body(
            "409 Conflict",
            TEXT,
            format!("{what}: {e}\n\nYour words:\n{words}\n"),
        ),
    }
}

fn send_post(a: &Asked, form: &str) -> Answer {
    let field = |n| dash_core::param(form, n).unwrap_or_default();
    let (key, account, words) = (field("thread"), field("account"), field("text"));
    answered(
        send(a.host, &a.id(), &key, &words),
        (&key, &account),
        "Not sent",
        &words,
    )
}

fn undo_post(a: &Asked, _: &str) -> Answer {
    let (key, account) = (
        a.param("thread").unwrap_or_default(),
        a.param("account").unwrap_or_default(),
    );
    answered(undo(a.host, &a.id()), (&key, &account), "Not undone", "")
}

inventory::submit! { dash_core::Element { key: PLACED, draw: under_thread } }
inventory::submit! { dash_core::Post { path: "/drafts/send/*", step: "route_draft_send", serve: send_post } }
inventory::submit! { dash_core::Post { path: "/drafts/undo/*", step: "route_draft_undo", serve: undo_post } }
