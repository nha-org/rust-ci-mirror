//! A reply draft's card under the mail it answers (rfc-rust-plugins-ui sections 3 and 4, task draft-from-is-address): its goal,
//! sender and receiver first, every check of its channel run before he sees it, then its words in the channel's compose box
//! with Send. A draft failing a check shows no Send, and goes back to its agent when the fault is its own (a check that did not
//! answer only waits). Send waits `UNDO_MS` on the host's clock, so Undo stops it any time before it begins; then it goes,
//! once, and is logged once. Drafts reaches the channel, the compose
//! box and the thread by key through the core, never by import (principle 7).
use crate::{Draft, load, open};
use dash_sdk::{Answer, Asked, Check, FieldWrite, Host, Message, OpLog, TEXT, Tree, Value, json};
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
/// Request corrections' post, `<CORRECT><draft>`: his words on the text's words, filed by the comments plugin.
pub const CORRECT: &str = "/drafts/correct/";
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
    match dash_sdk::channel(&d.channel) {
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
    // a correction is on the version's words as they would leave, the ones `correct` finds a range in, never a send's record
    let own = (format!("{id}-text"), "quote", json!({"text": m.text}));
    let due = last["due"].as_i64().unwrap_or(0);
    let waits = said_("state") == "waiting" && host.now_ms() < due;
    let (mut roots, mut correcting) = (vec![], false);
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
        // his correction waits on the agent's new version: no Send on the words he asked changed
        (_, "corrected") if last["version"] == d.versions.len() => {
            parts.push((
                format!("{id}-back"),
                "alert",
                json!({"title": "Back to its agent: your correction", "text": "A new version comes back here, its checks run again."}),
            ));
            parts.push(own.clone());
            correcting = true;
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
            parts.push(own.clone());
            correcting = true;
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
            // the words read first, each word its range for a correction, then the box that sends them, his edit included
            parts.push(own.clone());
            let compose = dash_sdk::element("mail/compose")?;
            roots = (compose.draw)(host, &props, &format!("{id}-compose"), tree)?;
            correcting = true;
        }
    }
    let mut children: Vec<String> = parts.iter().map(|(k, ..)| k.clone()).collect();
    for (k, kind, props) in parts {
        tree.add(&k, kind, props, &[]);
    }
    // the text by its lasting anchor, every version's comments drawn on it by the plugins placed there, just under it
    let text = format!("{id}-text");
    tree.anchor(&text, &anchor(&d.id));
    if let Some(at) = children.iter().position(|k| *k == text) {
        let shown = if correcting { &m.text } else { &words };
        let on = dash_sdk::on_anchor(host, &anchor(&d.id), shown, &format!("{id}-on"), tree);
        children.splice(at + 1..at + 1, on);
    }
    // the compose box last, its text the box's, then Request corrections beside Send
    children.extend(roots);
    if correcting {
        let props = json!({"action": format!("{CORRECT}{}", segment(&d.id)), "send": "Request corrections",
            "placeholder": "What to change", "fields": [{"name": "quote", "label": "On the words (all of them when empty)"}],
            "hidden": [["thread", key], ["account", account]]});
        tree.add(&format!("{id}-correct"), "compose", props, &[]);
        children.push(format!("{id}-correct"));
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
    let same = ["returned", "corrected"].contains(&last["state"].as_str().unwrap_or_default())
        && last["version"] == version;
    if in_flight(host, &last) || same {
        return Ok(());
    }
    write(
        &mut *log,
        &d.id,
        json!({"state": "returned", "version": version, "why": why.join("; ")}),
    )
}

/// Why draft `id` went back to its agent, a failing check or his correction, as last recorded; `None` when it did not, or a
/// later step superseded it: a send, or a new version, which the card checks again on its next draw.
pub fn back_to_agent(log: &dyn OpLog, id: &str) -> Result<Option<String>, String> {
    let last = record(log, id)?;
    let back = ["returned", "corrected"].contains(&last["state"].as_str().unwrap_or_default())
        && last["version"] == versions(log, id)?;
    Ok(back
        .then(|| last["why"].as_str().map(String::from))
        .flatten())
}

/// How many versions draft `id` has, 0 when it is not kept.
fn versions(log: &dyn OpLog, id: &str) -> Result<usize, String> {
    Ok(load(log)?.drafts.get(id).map_or(0, |d| d.versions.len()))
}

/// Whether a new version of `d` may be written (`dash draft revise`): it is open and no send of it waits, goes or went, so
/// the words his Send recorded are the ones that leave.
pub fn revisable(host: &dyn Host, log: &dyn OpLog, d: &Draft) -> Result<(), String> {
    if !crate::OPEN.contains(&d.status.as_str()) {
        return Err(format!("{}: it is {}, not open", d.id, d.status));
    }
    if in_flight(host, &record(log, &d.id)?) {
        return Err(format!("{}: its send is waiting, going or gone", d.id));
    }
    Ok(())
}

/// Draft `id`'s text's lasting anchor, the core's prop its comments attach to, the same across its versions.
pub fn anchor(id: &str) -> String {
    format!("drafts/{id}/text")
}

/// Whether a send of draft `id` is waiting, going or gone, as `last` records it: his then, never taken back by a correction.
fn in_flight(host: &dyn Host, last: &Json) -> bool {
    ["sent", "sending"].contains(&last["state"].as_str().unwrap_or_default())
        || (last["state"] == "waiting" && host.now_ms() < last["due"].as_i64().unwrap_or(0))
}

/// His Request corrections on draft `id`, answering thread `key` read as `account`: `words` filed on the words `quote` of its
/// last version as it would leave (all of it when empty; at byte `at` when given and they are there, else their first place)
/// by the comments plugin, through the core, where the queue wakes the agent; then the draft back to its agent for that
/// version, with no Send until a new one comes. Words not in the text, a closed draft, or a send waiting, going or gone, file
/// nothing. The lock is held from the check to the record, the filing a local write: no send begins between them.
// ponytail: no page fills `at` yet (a picker on the quote's word ranges is the upgrade), so typed words take their first place.
pub fn correct(
    host: &dyn Host,
    id: &str,
    (key, account): (&str, &str),
    (quote, at): (&str, Option<usize>),
    words: &str,
) -> Result<String, String> {
    let d = load(&*crate::log(host, &host.data())?)?
        .drafts
        .remove(id)
        .ok_or(format!("no draft {id}"))?;
    if !crate::OPEN.contains(&d.status.as_str()) {
        return Err(format!("it is {}, not open", d.status));
    }
    let text = message(&d, key, "").text;
    let quote = match quote.trim() {
        "" => text.as_str(),
        q => q,
    };
    let there = at.filter(|&at| text.get(at..).is_some_and(|t| t.starts_with(quote)));
    let from = there
        .or_else(|| text.find(quote))
        .filter(|_| !quote.is_empty())
        .ok_or("those words are not in its text")?;
    let form = [
        ("anchor", anchor(id)),
        ("from", from.to_string()),
        ("to", (from + quote.len()).to_string()),
        ("quote", quote.into()),
        ("text", words.into()),
        ("page", back(key, account)),
    ];
    let form: Vec<String> = form
        .iter()
        .map(|(k, v)| format!("{k}={}", segment(v)))
        .collect();
    let asked = Asked {
        path: dash_sdk::COMMENT_ON_ANCHOR,
        query: "",
        html: false,
        held: None,
        host,
    };
    let filed = dash_sdk::post(dash_sdk::COMMENT_ON_ANCHOR).ok_or("no plugin files comments")?;
    let _one = lock();
    let mut log = crate::log(host, &host.data())?;
    if in_flight(host, &record(&*log, id)?) {
        return Err("its send is waiting, going or gone".into());
    }
    let comment = match (filed.serve)(&asked, &form.join("&")) {
        Answer::Body(status, _, id) if status.starts_with("200") => id,
        Answer::Body(_, _, e) => return Err(e),
        Answer::Drawn(_) => return Err("the comments plugin answered a page".into()),
    };
    let why = format!("his correction on « {quote} »: {}", words.trim());
    let version = d.versions.len();
    write(
        &mut *log,
        id,
        json!({"state": "corrected", "version": version, "comment": comment, "why": why}),
    )?;
    Ok(comment)
}

/// His Send on draft `id`, answering thread `key`, `text` his words as the box posts them: every check run again (outside the
/// lock: they read the network), then the send waits `UNDO_MS` on the host's clock, the host handed what sends it then. A
/// second tap while it waits queues nothing.
pub fn send(host: &dyn Host, id: &str, key: &str, text: &str) -> Result<(), String> {
    let d = load(&*crate::log(host, &host.data())?)?
        .drafts
        .remove(id)
        .ok_or(format!("no draft {id}"))?;
    let corrected =
        |last: &Json| last["state"] == "corrected" && last["version"] == d.versions.len();
    if corrected(&record(&*crate::log(host, &host.data())?, id)?) {
        return Err(
            "you asked corrections on this version: its agent writes a new one first".into(),
        );
    }
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
        // a correction filed between the first read and the lock
        _ if corrected(&last) => {
            return Err(
                "you asked corrections on this version: its agent writes a new one first".into(),
            );
        }
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
    let sent = dash_sdk::channel(&d.channel).and_then(|c| (c.send)(host, &m));
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
    let field = |n| dash_sdk::param(form, n).unwrap_or_default();
    let (key, account, words) = (field("thread"), field("account"), field("text"));
    answered(
        send(a.host, &a.id(), &key, &words),
        (&key, &account),
        "Not sent",
        &words,
    )
}

fn correct_post(a: &Asked, form: &str) -> Answer {
    let field = |n| dash_sdk::param(form, n).unwrap_or_default();
    let (key, account, words) = (field("thread"), field("account"), field("text"));
    let at = field("at").parse().ok();
    let done = correct(
        a.host,
        &a.id(),
        (&key, &account),
        (&field("quote"), at),
        &words,
    );
    answered(done.map(drop), (&key, &account), "Not filed", &words)
}

fn undo_post(a: &Asked, _: &str) -> Answer {
    let (key, account) = (
        a.param("thread").unwrap_or_default(),
        a.param("account").unwrap_or_default(),
    );
    answered(undo(a.host, &a.id()), (&key, &account), "Not undone", "")
}

inventory::submit! { dash_sdk::Element { key: PLACED, draw: under_thread } }
inventory::submit! { dash_sdk::Post { path: "/drafts/send/*", step: "route_draft_send", serve: send_post } }
inventory::submit! { dash_sdk::Post { path: "/drafts/undo/*", step: "route_draft_undo", serve: undo_post } }
inventory::submit! { dash_sdk::Post { path: "/drafts/correct/*", step: "route_draft_correct", serve: correct_post } }
