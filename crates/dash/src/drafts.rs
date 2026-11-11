//! The binary's wiring of the drafts plugin: its table in `drafts.sqlite` on the op log, the older app's export read from a
//! folder holding drafts.json (`dash model drafts`), leads.json (`dash store files opportunities`) and jev.jsonl (Jev's log,
//! private/ats/jev.jsonl), and an evidence/ folder of labelled files (private/evidence/drafts/*.md). `dash draft
//! import|new|show|rates`.
use crate::fresh::{self, Read};
use dash_core::registry::{Contribution, Slot};
use dash_drafts::{self as drafts, Draft, outbox};
use dash_store::oplog::Replica;
use dash_surface as surface;
use serde_json::{Map, Value, json};
use std::fs::File;
use std::io::{BufReader, Write};
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn log(dir: &Path) -> Result<Replica, String> {
    let db = dash_store::connect(&dir.join("drafts.sqlite")).map_err(|e| e.to_string())?;
    db.execute_batch(drafts::SCHEMA)
        .map_err(|e| e.to_string())?;
    Replica::new(db, "dash").map_err(|e| e.to_string())
}

/// Each labelled file of `folder`, in name order; none when the folder is not there, a refusal when it cannot be read: a
/// skipped file would drop his labels with no word.
fn evidence(folder: &Path) -> Result<Vec<drafts::Labels>, String> {
    let said = |e: std::io::Error| format!("{}: {e}", folder.display());
    let entries = match std::fs::read_dir(folder) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        read => read.map_err(said)?,
    };
    let paths: Vec<_> = entries
        .map(|e| e.map(|e| e.path()))
        .collect::<Result<_, _>>()
        .map_err(said)?;
    let mut files: Vec<_> = paths
        .into_iter()
        .filter(|p| p.extension().is_some_and(|x| x == "md"))
        .collect();
    files.sort();
    files.iter().map(|f| labelled_file(f)).collect()
}

fn labelled_file(file: &Path) -> Result<drafts::Labels, String> {
    let text = std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
    drafts::labels(&text).map_err(|e| format!("{}: {e}", file.display()))
}

/// The export's drafts, each one a labelled file names given its labels, and how many files there were. A file naming a
/// draft the store does not hold adds it, as `new --file` alone does.
/// ponytail: `new --file` labels last only until the next import; a labelled file is kept by copying
/// private/evidence/drafts into `<export>/evidence`, and the count printed shows one missing.
fn export(from: &Path) -> Result<(Vec<Draft>, usize), String> {
    with_labels(store_export(from)?, &from.join("evidence"))
}

fn with_labels(mut all: Vec<Draft>, evidence_in: &Path) -> Result<(Vec<Draft>, usize), String> {
    let files = evidence(evidence_in)?;
    let n = files.len();
    for l in files {
        match all.iter().position(|d| d.id == l.id) {
            Some(at) => {
                let base = all.remove(at);
                all.insert(at, drafts::labelled(Some(base), l));
            }
            None => all.push(drafts::labelled(None, l)),
        }
    }
    Ok((all, n))
}

fn store_export(from: &Path) -> Result<Vec<Draft>, String> {
    let json = |name: &str| -> Result<Value, String> {
        let text = std::fs::read_to_string(from.join(name)).map_err(|e| format!("{name}: {e}"))?;
        serde_json::from_str(&text).map_err(|e| format!("{name}: {e}"))
    };
    let jev = File::open(from.join("jev.jsonl")).map_err(|e| format!("jev.jsonl: {e}"))?;
    drafts::from_store(
        &json("drafts.json")?,
        &json("leads.json")?,
        BufReader::new(jev),
    )
}

/// UTC's day, the one rule `surface::components::today` holds (its ponytail note says the Paris hour it misses).
fn today() -> String {
    drafts::date(surface::components::today())
}

fn done(said: Result<String, String>, mut out: File) {
    match said {
        Ok(text) => writeln!(out, "{text}").expect("stdout"),
        Err(e) => {
            eprintln!("draft: {e}");
            std::process::exit(1);
        }
    }
}

/// What the table holds: each draft as kept, and a row whose record does not read named on stderr, so the read goes on.
fn kept(log: &Replica) -> Result<drafts::Kept, String> {
    let kept = drafts::load(log)?;
    for id in &kept.unread {
        eprintln!(
            "draft {id}: its record does not read, left out; dash draft import writes it again"
        );
    }
    Ok(kept)
}

/// Every draft of the export, kept; a line of counts by outcome. The table also keeps a draft the export no longer holds.
pub fn import(dir: &Path, from: &Path, out: File) {
    let said = export(from).and_then(|(all, labelled)| {
        let mut log = log(dir)?;
        let before = kept(&log)?;
        let written = drafts::save(&mut log, &before, &all)?;
        let gone = before
            .drafts
            .keys()
            .filter(|id| !all.iter().any(|d| &d.id == *id));
        let today = today();
        let mut counts = std::collections::BTreeMap::new();
        for d in &all {
            *counts.entry(d.outcome(&today)).or_insert(0) += 1;
        }
        let counts: Vec<String> = counts.iter().map(|(o, n)| format!("{n} {o}")).collect();
        Ok(format!(
            "{} drafts in the export, {labelled} labelled files, {written} written, {} kept from before only: {}",
            all.len(),
            gone.count(),
            counts.join(", ")
        ))
    });
    done(said, out);
}

/// A labelled file's draft, kept, then shown: on the export's record of it with `--from`, else on the one kept, else on
/// the file alone.
pub fn new(dir: &Path, file: &Path, from: Option<&Path>, out: File) {
    let said = labelled_file(file).and_then(|l| {
        let mut log = log(dir)?;
        let before = kept(&log)?;
        let base = match from {
            Some(from) => store_export(from)?.into_iter().find(|d| d.id == l.id),
            None => before.drafts.get(&l.id).cloned(),
        };
        let d = drafts::labelled(base, l);
        drafts::save(&mut log, &before, std::slice::from_ref(&d))?;
        Ok(drafts::show(&d, &today()))
    });
    done(said, out);
}

pub fn show(dir: &Path, id: &str, out: File) {
    let said = log(dir).and_then(|log| {
        let kept = kept(&log)?.drafts;
        let d = kept.get(id).ok_or(format!("{id}: not kept"))?;
        Ok(drafts::show(d, &today()))
    });
    done(said, out);
}

pub fn rates(dir: &Path, out: File) {
    let said = log(dir).and_then(|log| {
        let kept: Vec<Draft> = kept(&log)?.drafts.into_values().collect();
        Ok(drafts::rates(&kept, &today()))
    });
    done(said, out);
}

/// The older app's read of its drafts or its leads: some 0.2 s on 2026-11-09.
const OLDER_CALL: Duration = Duration::from_secs(60);

/// What a refresh gives the page: the drafts to list, and what its note adds (a read or a write that failed).
#[derive(Clone)]
struct Listed {
    drafts: Vec<Draft>,
    said: String,
    /// The drafts the outbox sends now, in its order; `None` when the export was not read, so no count is known.
    to_send: Option<Vec<String>>,
}

/// One of the older app's reads as JSON: `dash model drafts`, `dash store files opportunities`.
fn read(args: &[&str]) -> Result<Value, String> {
    let text = dash_process::run(&crate::tasks::older_app().bin, args, None, OLDER_CALL)?;
    serde_json::from_str(&text).map_err(|e| format!("dash {}: {e}", args.join(" ")))
}

/// The older app's drafts, where drafts are written today: `dash model drafts`, `dash store files opportunities`, Jev's
/// log and the labelled files, as `dash draft import` reads them from an export.
/// ponytail: reads Jev's whole log (43 MB on 2026-11-09) each time, off the routing thread and at most every five minutes;
/// a read from the log's last offset if its time ever shows.
/// The drafts, and the outbox's ids that go now, or why they are not known: an export with no `ready` still lists its drafts
/// (/code-review).
fn exported() -> Result<(Vec<Draft>, ToSendIds), String> {
    let repo = crate::tasks::repo();
    let jev = repo.join("private/ats/jev.jsonl");
    let jev = File::open(&jev).map_err(|e| format!("{}: {e}", jev.display()))?;
    let leads = read(&["store", "files", "opportunities"])?;
    let export = read(&["model", "drafts"])?;
    let to_send = outbox::outbox(&export).map(|o| o.now.into_iter().map(|r| r.id).collect());
    let all = drafts::from_store(&export, &leads, BufReader::new(jev))?;
    let all = with_labels(all, &repo.join("private/evidence/drafts"))?.0;
    Ok((all, to_send))
}

/// The outbox's ids that go now, or why they are not known.
type ToSendIds = Result<Vec<String>, String>;

/// `dash outbox` (task rust-daily-today): the older app's export read now, never a page's last read, since a draft sent a minute
/// ago must not be listed to send again.
pub fn outbox_lines() -> Result<String, String> {
    Ok(outbox::outbox(&read(&["model", "drafts"])?)?.lines())
}

/// How many drafts go now, from the page's last read: `None` while none has ended. A read that failed, or one from the table
/// alone, is its error: a count never falls back to zero.
fn to_send(dir: &Path) -> Result<Option<usize>, String> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH);
    let now = now.map_err(|e| e.to_string())?.as_secs();
    let at = dir.to_path_buf();
    match fresh::bounded(&READ, now, "drafts", move || refreshed(&at))? {
        None => Ok(None),
        Some((Err(e), _)) => Err(e),
        Some((
            Ok(Listed {
                to_send: Some(ids), ..
            }),
            _,
        )) => Ok(Some(ids.len())),
        Some((Ok(Listed { said, .. }), _)) => Err(format!("no count.{said}")),
    }
}

/// The drafts' line's key: first in « today », since drafts to send come first (the task's title: « drafts to send first »).
pub const SLOT: &str = "today/10-drafts";

/// The drafts plugin's line in the daily's « today » (rfc-rust-plugins-ui section 3, the Slot port).
pub fn slot(dir: &Path) -> Contribution {
    let dir = dir.to_path_buf();
    let line: Box<dyn Slot> = Box::new(outbox::ToSend(move || to_send(&dir)));
    Contribution {
        slots: vec![(SLOT.into(), line)],
        ..Default::default()
    }
}

/// A refresh: the export listed and kept in the table of `dir`. The page lists the export, never the table, so a draft
/// the older app no longer holds leaves the list (the table keeps it: data is gold). With no export, the table's last
/// kept, named as such; with neither, or a table holding no open draft, the read's error: never « no draft waiting ».
fn refreshed(dir: &Path) -> Result<Listed, String> {
    match exported() {
        Ok((all, to_send)) => {
            // each with the versions the table kept, as /drafts/<id> keeps them: a text he asked corrections on outlives the
            // rewrite (/code-review: a plain save of the export dropped it)
            let saved = log(dir).and_then(|mut log| {
                let before = kept(&log)?;
                let merged: Vec<Draft> = (all.iter().cloned())
                    .map(|d| drafts::with_kept_versions(before.drafts.get(&d.id), d))
                    .collect();
                drafts::save(&mut log, &before, &merged)
            });
            let mut said = saved.err().map_or(String::new(), |e| {
                format!(" The table did not keep it: {e}")
            });
            if let Err(e) = &to_send {
                said += &format!(" Which drafts go now is not known: {e}");
            }
            Ok(Listed {
                drafts: all,
                said,
                to_send: to_send.ok(),
            })
        }
        Err(e) => {
            let drafts: Vec<Draft> = kept(&log(dir)?)?.drafts.into_values().collect();
            if drafts::open(&drafts).is_empty() {
                return Err(e);
            }
            let said =
                format!(" The older app's read failed, these are the table's last kept: {e}");
            Ok(Listed {
                drafts,
                said,
                to_send: None,
            })
        }
    }
}

/// The screen at /drafts as a UI tree (task rust-drafts-list, rfc-rust-plugins-ui section 2's drafts waiting): a section a
/// status as `drafts::OPEN` orders them, each draft a message draft listed (its receiver, its goal in one line, Jev's answers),
/// opening its page on the older app. A section's name keeps the older page's anchor (`/drafts#stage-approved`). `None`: no read
/// has ended, so no list, and no « no draft waiting » either.
fn tree(all: Option<&[Draft]>, note: &str) -> String {
    let titles = ["Corrections in progress", "To review", "To send"];
    let mut elements = Map::new();
    let text = |t: &str| json!({"type": "text", "props": {"text": t}});
    elements.insert(
        "title".into(),
        json!({"type": "heading", "props": {"text": "Drafts"}}),
    );
    elements.insert("note".into(), text(note));
    let mut children = vec!["title".to_string(), "note".to_string()];
    let open = drafts::open(all.unwrap_or_default());
    if open.iter().any(|d| d.status == "pending") {
        let series = json!({"name": "Review in series, the simplest first", "href": "/review"});
        elements.insert(
            "tools".into(),
            json!({"type": "links", "props": {"links": [series]}}),
        );
        children.push("tools".into());
    }
    for (status, title) in drafts::OPEN.iter().zip(titles) {
        let stage: Vec<&Draft> = open
            .iter()
            .copied()
            .filter(|d| d.status == *status)
            .collect();
        if stage.is_empty() {
            continue;
        }
        let (section, list) = (format!("stage-{status}"), format!("drafts-{status}"));
        elements.insert(list.clone(), table(&stage));
        let part = json!({"type": "section", "props": {"title": title}, "children": [list]});
        elements.insert(section.clone(), part);
        children.push(section);
    }
    if all.is_some() && open.is_empty() {
        elements.insert("none".into(), text("No draft waiting."));
        children.push("none".into());
    }
    elements.insert(
        "page".into(),
        json!({"type": "stack", "children": children}),
    );
    json!({"root": "page", "elements": elements}).to_string()
}

/// Drafts as the design system's table, a row a draft opening its page (task rust-drafts-table; @nha, 2026-11-09: « propose a
/// different UI not a sea of cards. »): its receiver, channel, goal and Jev's scores; its words open at /drafts/<id>.
fn table(drafts: &[&Draft]) -> Value {
    let rows: Vec<Value> = (drafts.iter())
        .map(|d| {
            // a blank receiver says so, as the message draft's template did
            let receiver = match d.receiver.trim() {
                "" => "no receiver".to_string(),
                r => r.to_string(),
            };
            let cells = [receiver, d.channel.clone(), d.goal_line(), d.jev_line()];
            json!({"cells": cells, "href": moved(&d.id)})
        })
        .collect();
    json!({"type": "table", "props": {"columns": ["Receiver", "Channel", "Goal", "Scores"], "rows": rows}})
}

/// /drafts?show=to-send, where the daily's « N to send » leads: the outbox's drafts that go now, in its order (oldest
/// approval first). `None`: no read has ended, or the last one did not read the export, so which drafts go is not known: no
/// list, never « nothing to send ».
fn to_send_tree(listed: Option<(&[Draft], &[String])>, note: &str) -> String {
    let mut elements = Map::new();
    let text = |t: &str| json!({"type": "text", "props": {"text": t}});
    elements.insert(
        "title".into(),
        json!({"type": "heading", "props": {"text": "Drafts to send"}}),
    );
    elements.insert("note".into(), text(note));
    let mut children = vec!["title".to_string(), "note".to_string()];
    let back = json!({"name": "All drafts, by stage", "href": "/drafts"});
    elements.insert(
        "tools".into(),
        json!({"type": "links", "props": {"links": [back]}}),
    );
    children.push("tools".into());
    if let Some((all, ids)) = listed {
        let go: Vec<&Draft> = (ids.iter())
            .filter_map(|id| all.iter().find(|d| &d.id == id))
            .collect();
        if go.is_empty() {
            elements.insert("none".into(), text("Nothing to send."));
            children.push("none".into());
        } else {
            elements.insert("to-send".into(), table(&go));
            let part =
                json!({"type": "section", "props": {"title": "To send"}, "children": ["to-send"]});
            elements.insert("stage-to-send".into(), part);
            children.push("stage-to-send".into());
        }
    }
    elements.insert(
        "page".into(),
        json!({"type": "stack", "children": children}),
    );
    json!({"root": "page", "elements": elements}).to_string()
}

/// Draft `id` as the older app holds it now, its text read alone (no Jev log), whether Claude sends it, and what it answers:
/// `None` when it holds none of that id.
fn older_draft(id: &str) -> Result<Option<(Draft, bool, Answers)>, String> {
    let all = read(&["model", "drafts"])?;
    let Some(d) = all.as_array().into_iter().flatten().find(|d| d["id"] == id) else {
        return Ok(None);
    };
    let draft = drafts::from_store(&json!([d]), &json!({}), &b""[..])?.pop();
    // each store read only when the draft names what it finds, the two at once: a page waits on the slower alone (/code-review)
    let names = |k: &str| d[k].is_string();
    let (postings, leads) = std::thread::scope(|s| {
        let postings = (names("posting") || names("opportunity"))
            .then(|| s.spawn(|| read(&["store", "files", "postings"])));
        let leads = names("opportunity").then(|| read(&["store", "files", "opportunities"]));
        let joined = |h: std::thread::ScopedJoinHandle<'_, _>| {
            h.join()
                .unwrap_or_else(|_| Err("its read stopped".to_string()))
        };
        (postings.map(joined), leads)
    });
    let mut answers = answers(d, postings, leads);
    // the steps it waits on, from the older app's database as its own page read them; unread, the page says it is not
    // known, never « waits on nothing » (/code-review)
    let waits = dash_store::older::waits(&crate::tasks::older_db(), &format!("drafts:{id}"))
        .map_err(|e| answers.1.push(format!("waits: {e}")))
        .ok();
    answers.0.extend(stated(d, by_claude(d), waits.as_deref()));
    Ok(draft.map(|draft| (draft, by_claude(d), answers)))
}

/// The days its decisions were made, as State lists them, in the older page's order.
const DATED: [(&str, &str); 4] = [
    (
        "approved-pending-changes-on",
        "Approved with his corrections",
    ),
    ("approved-on", "Approved"),
    ("rejected-on", "Rejected"),
    ("sent-on", "Sent"),
];

/// What Why this text, State and Waits show of draft record `d` (task rust-draft-state-edit), as the older page read it:
/// `waits`, the steps it waits on still open; `None`, not read.
fn stated(d: &Value, by_claude: bool, waits: Option<&[String]>) -> Map<String, Value> {
    let text = |k: &str| d[k].as_str().unwrap_or_default().trim().to_string();
    let why: Vec<&str> = match &d["context"] {
        Value::Array(all) => all.iter().filter_map(Value::as_str).collect(),
        one => one.as_str().into_iter().collect(),
    };
    let why: Vec<&str> = why
        .into_iter()
        .map(str::trim)
        .filter(|w| !w.is_empty())
        .collect();
    let dates: Vec<Value> = (DATED.iter())
        .filter_map(|(k, label)| Some(json!({"label": label, "value": d[*k].as_str()?})))
        .collect();
    let waits_unread = waits.is_none();
    let waits: Vec<Value> = (waits.unwrap_or_default().iter())
        .map(|w| json!({"label": w, "value": w, "href": step_href(w)}))
        .collect();
    let meanwhile = d["waits-on"]["in-flight"].as_str().unwrap_or_default();
    let props = json!({
        "stated": true, "id": text("id"), "copy": !by_claude, "why": why, "where": text("where"),
        "bold": bold(d, by_claude), "dates": dates, "send_on": text("send-on"), "waits": waits, "meanwhile": meanwhile,
        "waits_unread": waits_unread, "keep_bold": d["channel"] == "form" && d["form-bold"] == true,
    });
    props.as_object().cloned().unwrap_or_default()
}

/// A step's page, as the older app's `Steps.href/1`: a draft, a lead, a task; else none.
fn step_href(step: &str) -> Option<String> {
    let (table, id) = step.split_once(':')?;
    match table {
        "drafts" => Some(moved(id)),
        "leads" => Some(format!("/o/{}", dash_comments::segment(id))),
        "tasks" => Some(surface::task_href(id, None)),
        _ => None,
    }
}

/// Whether his bold reaches the reader, per channel, the older page's words: a form says so once checked on the form itself
/// (@nha, 2026-10-20, on /d/mistral-defense-form: « I said it before, IF this is allowed => bold »). Empty: not said.
fn bold(d: &Value, by_claude: bool) -> &'static str {
    match (d["channel"].as_str(), d["form-bold"].as_bool()) {
        (Some("linkedin"), _) => "Kept: LinkedIn gets Unicode bold at the send",
        (Some("form"), Some(true)) => {
            "Kept: this form shows formatting, the **bold** goes as written"
        }
        (Some("form"), Some(false)) => {
            "None: this form's fields take plain text only, so the message carries no bold"
        }
        (Some("form"), None) => {
            "Not checked: plain text until the form is checked (its :form-bold)"
        }
        _ if by_claude => "Kept: bold in the mail's HTML part",
        _ => "",
    }
}

/// The statuses whose day out he may still set, and whose text he may still change: `Schema.open_draft_status/0` and
/// `Write.editable?/1`.
const OPEN: [&str; 3] = ["pending", "approved-pending-changes", "approved"];
const EDITABLE: [&str; 2] = ["pending", "approved-pending-changes"];

/// `POST /drafts/<id>/set`: State's Save on draft `id`'s page.
pub fn set_posted(line: &str) -> Option<String> {
    let mut words = line.split_whitespace();
    (words.next() == Some("POST")).then_some(())?;
    let id = words
        .next()?
        .split('?')
        .next()?
        .strip_prefix("/drafts/")?
        .strip_suffix("/set")?;
    let id = crate::pages::segment_decoded(id);
    (!id.is_empty() && !id.contains('/') && !id.starts_with('-')).then_some(id)
}

/// Draft `id` goes out on `day` (an ISO date; empty: with the next outbox), written through the older app's `dash draft set
/// <id> --send-on <day>`, which refuses a draft already closed. Err: the status to answer and why.
pub fn send_on(id: &str, day: &str) -> Result<(), (&'static str, String)> {
    let day = day.trim();
    if !day.is_empty() && !(day.len() == 10 && dash_core::days(day).is_some()) {
        let why = format!("{day}: a day is written 2026-11-15, or left empty");
        return Err(("400 Bad Request", why));
    }
    let args = ["draft", "set", id, "--send-on", day];
    dash_process::run(&crate::tasks::older_app().bin, &args, None, OLDER_CALL)
        .map(drop)
        .map_err(|e| ("502 Bad Gateway", e))
}

/// What a draft answers, as its template's props, and each store read that failed, named: the ad from its archive, its
/// Application form and its lead's last events, its search note among them (task rust-draft-posting).
type Answers = (Map<String, Value>, Vec<String>);

/// `draft`'s ad, form and lead's events from `postings` and `leads` (`dash store files postings|opportunities`), `None` where
/// not read. A read that failed leaves its own parts out alone: the ad shows with no lead, the lead with no ad (/code-review).
fn answers(
    draft: &Value,
    postings: Option<Result<Value, String>>,
    leads: Option<Result<Value, String>>,
) -> Answers {
    let mut unread = vec![];
    let mut got = |read: Option<Result<Value, String>>, what: &str| match read? {
        Ok(v) => Some(v),
        Err(e) => {
            unread.push(format!("{what}: {e}"));
            None
        }
    };
    let (postings, leads) = (got(postings, "postings"), got(leads, "leads"));
    let lead =
        (leads.as_ref()).and_then(|l| drafts::posting::lead(l, draft["opportunity"].as_str()?));
    let mut props = Map::new();
    let ad = postings
        .as_ref()
        .and_then(|p| drafts::posting::archive_of(draft, p));
    if let Some(p) = ad.map(drafts::posting::parse) {
        let mut facts: Vec<Value> = (p.facts.iter())
            .map(|(label, value)| json!({"label": label, "value": value}))
            .collect();
        facts.extend(
            p.url
                .map(|u| json!({"label": "Source", "value": u, "href": u})),
        );
        props.insert("posting".into(), facts.into());
        props.insert("ad".into(), p.text.into());
    }
    props.insert(
        "apply".into(),
        drafts::posting::apply(draft, lead.as_ref()).into(),
    );
    let recent = (lead.as_ref()).map_or(vec![], |l| drafts::posting::recent(l, &today(), 3));
    let recent: Vec<Value> = (recent.into_iter())
        .map(|(day, words, source)| json!({"label": day, "value": words, "href": source}))
        .collect();
    props.insert("recent".into(), recent.into());
    (props, unread)
}

/// Claude sends it, as the older app's `Write.by_claude?/1` says: its « I sent it » is refused (second /code-review).
fn by_claude(d: &Value) -> bool {
    match d["channel"].as_str() {
        Some("linkedin") => true,
        Some("email") => matches!(d["from"].as_str(), None | Some("nicolas@klar.im")),
        Some("form") => {
            (d["where"].as_str()).is_some_and(|w| w.to_lowercase().contains("free-work"))
        }
        _ => false,
    }
}

/// What /drafts/<id> draws: the draft, its page's note, and whether it is the older app's now (only then are its decisions drawn:
/// a tap on the table's copy could act on a status already changed; /code-review), and whether Claude sends it.
struct Shown {
    draft: Draft,
    note: String,
    fresh: bool,
    by_claude: bool,
    answers: Map<String, Value>,
}

/// /drafts/<id>'s draft: the older app's now, read on this request's own thread (never the list's five-minute read, so the page
/// his tap lands on shows it), with each version the table kept before; kept again, so the text he decides on stays
/// readable once it is rewritten. With no read, the table's last kept, named as such. `None`: neither holds it.
fn current(dir: &Path, id: &str) -> Result<Option<Shown>, String> {
    let mut log = log(dir)?;
    let before = kept(&log)?;
    let last = before.drafts.get(id);
    let fallback = |why: String| {
        last.map(|d| Shown {
            draft: d.clone(),
            note: format!("{why}: this is the table's last kept."),
            fresh: false,
            by_claude: false,
            answers: Map::new(),
        })
    };
    match older_draft(id) {
        Ok(Some((now, by_claude, (answers, unread)))) => {
            let draft = drafts::with_kept_versions(last, now);
            let kept = drafts::save(&mut log, &before, std::slice::from_ref(&draft)).err();
            // a store that does not read leaves the ad out and says so, never the draft unshown
            let unread = (!unread.is_empty()).then(|| unread.join(", "));
            let note = [
                kept.map(|e| format!("The table did not keep it: {e}")),
                unread.map(|e| {
                    format!("What it answers did not read ({e}): those parts are left out.")
                }),
            ];
            Ok(Some(Shown {
                draft,
                note: note.into_iter().flatten().collect::<Vec<_>>().join(" "),
                fresh: true,
                by_claude,
                answers,
            }))
        }
        Ok(None) => Ok(fallback("The older app no longer holds it".into())),
        Err(e) => fallback(format!("The older app's read failed ({e})"))
            .map(Some)
            .ok_or(e),
    }
}

/// The moves a status offers beside Approve and Request corrections, as the older app's `Write.moves/2` lists them: « I sent
/// it » only on a draft he sends himself, and every other status back to review (second /code-review).
fn moves(status: &str, by_claude: bool) -> Vec<Value> {
    let offered: &[(&str, &str)] = match (status, by_claude) {
        ("pending", _) => &[("rejected", "Reject"), ("moot", "Handled another way")],
        ("approved", false) => &[("sent", "I sent it"), ("pending", "Back to review")],
        _ => &[("pending", "Back to review")],
    };
    (offered.iter())
        .map(|(status, label)| json!({"status": status, "label": label}))
        .collect()
}

/// A draft's own page as a UI tree (task rust-draft-approve; rfc-rust-plugins-ui section 4, pick A, with today's Approve): the
/// message draft's template whole, its goal, sender and receiver above its current text (its last version), its decisions
/// while `shown` is fresh, `open_comments` holding Approve back, then each earlier version.
fn draft_tree(shown: &Shown, open_comments: usize, editing: bool) -> String {
    let d = &shown.draft;
    let (body, before) = d
        .versions
        .split_last()
        .map_or(("", &[][..]), |(v, rest)| (v.text.as_str(), rest));
    // no number: a text set back to an earlier one moves to last, so a position is no order of writing (second /code-review)
    let earlier: Vec<Value> = (before.iter())
        .filter(|v| !v.text.trim().is_empty())
        .map(|v| json!({"title": format!("Earlier: on {} by {}", v.on, v.by), "text": v.text}))
        .collect();
    // the older app's draft write, POST /d/<id>/status: a write, not the page /d/<id> was
    let decide = shown.fresh.then(|| format!("/d/{}", d.id));
    // his edit and the day it goes out, on a draft read now that is still open to them (task rust-draft-state-edit)
    let open = |statuses: &[&str]| shown.fresh && statuses.contains(&d.status.as_str());
    let change = open(&EDITABLE).then(|| moved(&d.id));
    let schedule = open(&OPEN).then(|| format!("{}/set", moved(&d.id)));
    let mut props = json!({
        "kind": "message-draft", "receiver": d.receiver, "sender": d.sender, "channel": d.channel, "status": d.status,
        "goal": d.goal, "lead": d.lead.as_deref().unwrap_or_default(), "scores": d.jev_line(), "text": body,
        "decide": decide, "earlier": earlier, "open_comments": open_comments, "moves": moves(&d.status, shown.by_claude),
        "change": change, "editing": editing && change.is_some(), "schedule": schedule,
    });
    props
        .as_object_mut()
        .expect("a map")
        .extend(shown.answers.clone());
    let doc = json!({"type": "document", "props": props});
    let mut children = vec!["draft"];
    let mut elements = Map::from_iter([("draft".to_string(), doc)]);
    if !shown.note.is_empty() {
        let note = json!({"type": "text", "props": {"text": shown.note}});
        elements.insert("note".into(), note);
        children.insert(0, "note");
    }
    elements.insert(
        "page".into(),
        json!({"type": "stack", "children": children}),
    );
    json!({"root": "page", "elements": elements}).to_string()
}

/// A draft drawn whole: `None` when no draft of that id is held. His open comments on it, from the comment queue's table; unread, the
/// draft is drawn with no decision and says why, as the table's copy is (second /code-review).
fn draft_page(dir: &Path, id: &str, editing: bool) -> Result<Option<surface::Drawing>, String> {
    let Some(mut shown) = current(dir, id)? else {
        return Ok(None);
    };
    let open = crate::comments::open_on(dir, &moved(id)).unwrap_or_else(|e| {
        shown.fresh = false;
        let said = format!(
            "{} His comments did not read ({e}): no decision until they do.",
            shown.note
        );
        shown.note = said.trim_start().to_string();
        0
    });
    Ok(Some(surface::Drawing::Tree {
        title: format!("Draft {id}"),
        tree: draft_tree(&shown, open, editing),
    }))
}

static READ: Mutex<Read<Result<Listed, String>>> = Mutex::new(Read::new());

/// The page from the last refresh, held in memory (a view reads no SQLite on the routing thread), and whether one has
/// ended: one starts past five minutes on its own thread, as /tools reads, so a view never waits on the older app. A
/// refresh with nothing to list is the page's error.
pub fn page(dir: &Path, show: Option<&str>) -> Result<(surface::Drawing, bool), String> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH);
    let now = now.map_err(|e| e.to_string())?.as_secs();
    let at = dir.to_path_buf();
    let read = fresh::last(&READ, now, "drafts", move || refreshed(&at))?;
    let (note, listed) = match &read {
        None => ("Reading the older app's drafts.".to_string(), None),
        Some((Err(e), _)) => return Err(format!("drafts: {e}")),
        Some((Ok(l), at)) => (fresh::ago(now, *at) + &l.said, Some(l.drafts.as_slice())),
    };
    if show == Some("to-send") {
        let ids = read
            .as_ref()
            .and_then(|r| r.0.as_ref().ok()?.to_send.as_deref());
        let note = match (&read, ids) {
            (Some(_), None) => format!(
                "{note} Which drafts go now is not known: the older app's export was not read."
            ),
            _ => note,
        };
        let tree = to_send_tree(listed.zip(ids), &note);
        let title = "Drafts to send".into();
        return Ok((surface::Drawing::Tree { title, tree }, read.is_some()));
    }
    let title = "Drafts".into();
    let tree = tree(listed, &note);
    Ok((surface::Drawing::Tree { title, tree }, read.is_some()))
}

// The drafts waiting (task rust-drafts-list): no read ended is no list yet, never « no draft waiting ».
inventory::submit! {
    crate::pages::Page { path: "/drafts", step: "route_drafts", reads: crate::pages::Reads::Board, redraw: false, serve: |a| {
        match a.board::<crate::tasks::Board>().map(|b| page(b.data(), a.param("show").as_deref())) {
            Some(Ok((_, false))) if !a.html => (
                "202 Accepted",
                crate::pages::TEXT,
                "reading the older app's drafts: ask again in a few seconds".into(),
            ),
            read => a.form(read.map(|r| r.map(|(drawing, _)| drawing))),
        }
    } }
}
// A draft opens on the drafts' page as the Gmail view of the one document (task rust-one-doc-component, done-when 3; @nha,
// 2026-11-09: « Aim to simplify and unify not create one-offs every time. »): the draft, then every draft by stage, read
// on this request's thread so his tap shows.
inventory::submit! {
    crate::pages::Page { path: "/drafts/*", step: "route_draft", reads: crate::pages::Reads::Nothing, redraw: false, serve: |a| {
        let editing = a.param("edit").is_some();
        match crate::pages::serve_from_dir().and_then(|dir| opened(dir, &a.id(), editing)) {
            Ok(Some(page)) => a.form(Some(Ok(page))),
            Ok(None) => ("404 Not Found", crate::pages::TEXT, "no draft of that id".into()),
            Err(e) => a.form(Some(Err(e))),
        }
    } }
}
// /d/<id>, a draft's own page once, is gone: a link to a draft this side holds (a comment's, a notification's) lands on its
// view, its query kept; one it does not hold stays the older app's to answer (/code-review).
inventory::submit! {
    crate::pages::Page { path: "/d/*", step: "route_draft_moved", reads: crate::pages::Reads::Nothing, redraw: false, serve: |a| {
        match crate::pages::serve_from_dir().and_then(|dir| current(dir, &a.id())) {
            Ok(Some(_)) => ("308 Permanent Redirect", crate::pages::TEXT, moved_to(&a.id(), a.query)),
            Ok(None) => ("404 Not Found", crate::pages::TEXT, "no draft of that id".into()),
            // never a page drawn here, an error's included: /d/<id> draws nothing of its own (done-when 3)
            Err(e) => a.failed(e),
        }
    } }
}

/// Where draft `id` opens: /drafts/<id>, encoded as the comment queue keys it (dash_comments::page_of).
pub(crate) fn moved(id: &str) -> String {
    format!("/drafts/{}", dash_comments::segment(id))
}

/// Where /d/<id>?<query> now opens, its query kept: ?view=data still answers JSON.
fn moved_to(id: &str, query: &str) -> String {
    match query {
        "" => moved(id),
        q => format!("{}?{q}", moved(id)),
    }
}

/// /drafts/<id>: the draft drawn whole, then the drafts' list as /drafts draws it; `None` when no draft of that id is held.
fn opened(dir: &Path, id: &str, editing: bool) -> Result<Option<surface::Drawing>, String> {
    let Some(surface::Drawing::Tree { title, tree: draft }) = draft_page(dir, id, editing)? else {
        return Ok(None);
    };
    // a list that did not read says so under the draft, never an empty list (/code-review)
    let said = |t: String| {
        json!({"root": "page", "elements": {"page": {"type": "text", "props": {"text": t}}}})
            .to_string()
    };
    let list = match page(dir, None) {
        Ok((surface::Drawing::Tree { tree, .. }, _)) => Some(tree),
        Ok(_) => None,
        Err(e) => Some(said(format!("The drafts list did not read: {e}"))),
    };
    Ok(Some(surface::Drawing::Tree {
        title,
        tree: joined(&draft, list.as_deref())?,
    }))
}

/// Two trees as one page, the second's ids prefixed so none clashes with the first's.
fn joined(first: &str, second: Option<&str>) -> Result<String, String> {
    let read = |t: &str| serde_json::from_str::<Value>(t).map_err(|e| e.to_string());
    let mut page = read(first)?;
    let Some(second) = second else {
        return Ok(first.to_string());
    };
    let second = read(second)?;
    let renamed = |id: &Value| json!(format!("list-{}", id.as_str().unwrap_or_default()));
    let root = page["root"].clone();
    let elements = page["elements"]
        .as_object_mut()
        .ok_or("a tree holds its elements")?;
    for (id, el) in second["elements"].as_object().into_iter().flatten() {
        let mut el = el.clone();
        if let Some(children) = el.get_mut("children").and_then(Value::as_array_mut) {
            children.iter_mut().for_each(|c| *c = renamed(c));
        }
        elements.insert(format!("list-{id}"), el);
    }
    let both = json!({"type": "stack", "children": [root, renamed(&second["root"])]});
    elements.insert("opened".into(), both);
    page["root"] = json!("opened");
    Ok(page.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::memory::TestBudget;

    fn draft(id: &str, status: &str, receiver: &str, goal: &str, on: &str) -> Draft {
        let version = drafts::Version {
            text: "t".into(),
            by: "claude".into(),
            on: on.into(),
            jev: [("too_long".to_string(), 0.4)].into(),
            ..Default::default()
        };
        Draft {
            id: id.into(),
            goal: goal.into(),
            sender: "Nicolas Ha".into(),
            receiver: receiver.into(),
            channel: "email".into(),
            lead: None,
            status: status.into(),
            approved: None,
            from_blocks: None,
            sent_on: None,
            created: String::new(),
            versions: vec![version],
            reply: None,
        }
    }

    /// Done-when 2 and 3 of rust-drafts-list: one tree, both forms; each open draft its receiver, goal line and Jev's answers.
    #[test]
    fn the_drafts_page_draws_both_forms_from_one_tree() {
        let _budget = TestBudget::new();
        let all = [
            draft(
                "late",
                "pending",
                "Bo (Bee)",
                "ask for the call",
                "2026-11-07",
            ),
            draft("gone", "sent", "Sam", "sent already", "2026-10-01"),
            draft("early", "pending", "Ana (a@x.io)", "", "2026-10-24"),
            draft(
                "fix",
                "approved-pending-changes",
                " ",
                "thank them",
                "2026-11-08",
            ),
        ];
        let tree = tree(Some(&all), "Read 0 min ago.");
        let text = surface::text(&tree).expect("text");
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(
            lines,
            [
                "Drafts",
                "Read 0 min ago.",
                "Review in series, the simplest first /review",
                "",
                "Corrections in progress",
                "Receiver\tChannel\tGoal\tScores\tLink",
                "no receiver\temail\tthank them\ttoo_long 0.4\t/drafts/fix",
                "",
                "To review",
                "Receiver\tChannel\tGoal\tScores\tLink",
                "Ana (a@x.io)\temail\tno goal\ttoo_long 0.4\t/drafts/early",
                "Bo (Bee)\temail\task for the call\ttoo_long 0.4\t/drafts/late",
            ],
            "{text}"
        );
        let html = surface::screen::page("Drafts", &tree).expect("html");
        // a table, a row a draft opening its page, no card (task rust-drafts-table; @nha, 2026-11-09: « propose a different
        // UI not a sea of cards. »)
        assert!(
            html.contains(r#"class="table""#) && html.contains(r#"href="/drafts/early""#),
            "{html}"
        );
        assert!(
            !html.contains(r#"class="cards""#) && !tree.contains(r#""type":"cards""#),
            "{html}"
        );
        assert!(
            html.contains(r#"id="stage-pending""#) && !html.contains("Sam"),
            "{html}"
        );
        let none = tree_text(&[draft("gone", "sent", "Sam", "", "2026-10-01")]);
        assert!(none.ends_with("No draft waiting.\n"), "{none}");
        // no read ended is no list, never « no draft waiting » (/code-review of rust-drafts-list)
        let reading =
            surface::text(&super::tree(None, "Reading the older app's drafts.")).expect("text");
        assert_eq!(reading, "Drafts\nReading the older app's drafts.\n");
    }

    /// Done-when 2 of rust-drafts-list: a card's title is its receiver, whole; a blank one says « no receiver ».
    #[test]
    fn a_card_s_title_is_its_receiver_and_a_blank_one_says_no_receiver() {
        let _budget = TestBudget::new();
        let title = |receiver: &str| {
            let text = tree_text(&[draft("a", "pending", receiver, "g", "2026-11-01")]);
            let row = text
                .lines()
                .find(|l| l.ends_with("\t/drafts/a"))
                .map(str::to_string);
            row.and_then(|l| l.split('\t').next().map(str::to_string))
        };
        let whole = "Clyde PATIENT (Business Manager, REACTIS Group), c@reactis.fr";
        assert_eq!(title(whole).as_deref(), Some(whole));
        assert_eq!(title("  \n ").as_deref(), Some("no receiver"));
        // and whole in the HTML he reads, not only in the text
        let tree = tree(
            Some(&[draft("a", "pending", whole, "g", "2026-11-01")]),
            "n",
        );
        let html = surface::screen::page("Drafts", &tree).expect("html");
        assert!(
            html.contains(&format!(">{whole}<")) && html.contains(r#"href="/drafts/a""#),
            "{html}"
        );
    }

    /// Done-when 2 of rust-daily-today: « N to send » opens the outbox's drafts that go now, in its order, and only those.
    #[test]
    fn to_send_lists_the_outbox_drafts_in_its_order() {
        let _budget = TestBudget::new();
        let all = [
            draft(
                "pending-free",
                "pending",
                "Pre (p@x.io)",
                "approved blocks",
                "2026-11-01",
            ),
            draft("waits", "pending", "Wai", "new words", "2026-11-02"),
            draft("go", "approved", "Go (g@x.io)", "reply", "2026-11-03"),
            draft("gone", "sent", "Sam", "sent already", "2026-10-01"),
        ];
        let ids = ["go".to_string(), "pending-free".to_string()];
        let text =
            surface::text(&to_send_tree(Some((&all, &ids)), "Read 0 min ago.")).expect("text");
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(
            lines,
            [
                "Drafts to send",
                "Read 0 min ago.",
                "All drafts, by stage /drafts",
                "",
                "To send",
                "Receiver\tChannel\tGoal\tScores\tLink",
                "Go (g@x.io)\temail\treply\ttoo_long 0.4\t/drafts/go",
                "Pre (p@x.io)\temail\tapproved blocks\ttoo_long 0.4\t/drafts/pending-free",
            ],
            "{text}"
        );
        let tree = to_send_tree(Some((&all, &ids)), "n");
        let html = surface::screen::page("Drafts to send", &tree).expect("html");
        assert!(
            html.contains(r#"href="/drafts/go">Go (g@x.io)<"#) && !html.contains("Wai"),
            "{html}"
        );
        let none = surface::text(&to_send_tree(Some((&all, &[])), "n")).expect("text");
        assert!(none.ends_with("Nothing to send.\n"), "{none}");
        // no export read: no list, never « nothing to send »
        let unknown = surface::text(&to_send_tree(None, "n")).expect("text");
        assert!(!unknown.contains("Nothing"), "{unknown}");
    }

    /// Done-when 1 and 2 of rust-draft-approve: /d/<id> says its goal, sender and receiver above its text; pending, Approve
    /// posts « approved » to the older app's draft write; an earlier version stays readable; approved, no decision is left.
    #[test]
    fn a_draft_s_page_shows_its_three_things_above_its_text_then_approve() {
        let _budget = TestBudget::new();
        let mut d = draft(
            "grant-reply",
            "pending",
            "k.b@hostman.com",
            "answer their six questions",
            "2026-11-07",
        );
        let corrected = drafts::Version {
            text: "Hello,\nthe answers.".into(),
            by: "claude".into(),
            on: "2026-11-08".into(),
            ..Default::default()
        };
        d.versions.push(corrected);
        let shown = |d: &Draft, fresh: bool| Shown {
            draft: d.clone(),
            note: String::new(),
            fresh,
            by_claude: false,
            answers: Map::new(),
        };
        let tree = draft_tree(&shown(&d, true), 0, false);
        let text = surface::text(&tree).expect("text");
        assert_eq!(
            text,
            "k.b@hostman.com\npending email\nContext\nGoal\nanswer their six questions\nFrom\nNicolas Ha\nTo\n\
             k.b@hostman.com\nScores\ntoo_long 0.4 (an earlier version: to rescore)\nMessage\nHello,\nthe answers.\nChange\n\
             Decision\nApprove\nRequest corrections\nReject\nHandled another way\nEarlier: on 2026-11-07 by claude\nt\n"
        );
        // its decisions post to the older app's draft write, /d/<id>/status (the template's test holds their forms)
        assert!(tree.contains(r#""decide":"/d/grant-reply""#), "{tree}");
        // an open comment of his holds Approve back (/code-review)
        let held = surface::text(&draft_tree(&shown(&d, true), 1, false)).expect("text");
        assert!(
            held.contains("Not ready to approve: 1 open comment to apply.\n"),
            "{held}"
        );
        // the table's copy, the older app unread: no decision drawn on it (/code-review)
        let stale = surface::text(&draft_tree(&shown(&d, false), 0, false)).expect("text");
        assert!(!stale.contains("Decision"), "{stale}");
        d.status = "approved".into();
        let after = surface::text(&draft_tree(&shown(&d, true), 0, false)).expect("text");
        assert!(
            after.contains("Decision\nI sent it\nBack to review\n"),
            "{after}"
        );
        assert!(
            !after.contains("Approve") && !after.contains("Request corrections"),
            "{after}"
        );
        // one Claude sends: no « I sent it », the older app refuses it (second /code-review)
        let ours = Shown {
            by_claude: true,
            ..shown(&d, true)
        };
        let ours = surface::text(&draft_tree(&ours, 0, false)).expect("text");
        assert!(ours.contains("Decision\nBack to review\nEarlier"), "{ours}");
        // rejected, sent or moot: back to review, as the older app moves them (second /code-review)
        for status in ["rejected", "sent", "moot"] {
            d.status = status.into();
            let closed = surface::text(&draft_tree(&shown(&d, true), 0, false)).expect("text");
            assert!(
                closed.contains("Decision\nBack to review\nEarlier"),
                "{status}: {closed}"
            );
        }
    }

    /// Done-when 1 and 2 of rust-draft-posting, wired: /d/xpertdirect-ml-rag-intro's form draft, its lead's archive and its
    /// lead as the store gives them, drawn with the ad's facts and text, the Application form, the lead's page and the search
    /// note; a store that does not read is said, never the draft unshown.
    #[test]
    fn a_form_draft_on_a_lead_draws_its_ad_its_form_and_its_search_note() {
        let _budget = TestBudget::new();
        let ad = "https://www.linkedin.com/jobs/view/4487144354/";
        let draft = json!({"id": "xpertdirect-ml-rag-intro", "channel": "form", "opportunity": "xpertdirect-ml-engineer-llm-rag",
            "where": format!("Easy Apply LinkedIn, {ad}, champ message")});
        let archive = format!(
            "# x\n\n**Source:** {ad}\n\n---\n\n# Senior ML Engineer | XpertDirect | European Union | 32 applicants\n\
            {ad}\n{{\"Employment type\": \"Contract\"}}\n\nRemote (EU)\n"
        );
        let postings = json!({"xpertdirect-ml-engineer-llm-rag.md": archive, "other.md": "# y"});
        let lead = json!({"id": "~:xpertdirect-ml-engineer-llm-rag", "urls": {"posting": ad, "apply": ad},
            "timeline": [{"date": "2026-11-10", "event": "Recherche LinkedIn du 2026-11-10", "ref": ad}]});
        let leads =
            json!({"2026-11-x.edn": lead.to_string(), "2026-11-y.edn": "{\"id\": \"~:y\"}"});
        let (props, unread) = answers(&draft, Some(Ok(postings.clone())), Some(Ok(leads)));
        assert!(unread.is_empty(), "{unread:?}");
        let d = drafts::from_store(&json!([draft]), &json!({}), &b""[..])
            .expect("draft")
            .remove(0);
        let shown = Shown {
            draft: d,
            note: String::new(),
            fresh: false,
            by_claude: false,
            answers: props,
        };
        let text = surface::text(&draft_tree(&shown, 0, false)).expect("text");
        let want = format!(
            "The ad\nTitle\nSenior ML Engineer\nCompany\nXpertDirect\nLocation\nEuropean Union\nContract\nContract\nSource\n{ad}\n\
             The ad's text\nSenior ML Engineer | XpertDirect | European Union | 32 applicants\n"
        );
        assert!(text.contains(&want), "{text}");
        for line in [
            "Lead\nxpertdirect-ml-engineer-llm-rag\t/o/xpertdirect-ml-engineer-llm-rag\n"
                .to_string(),
            format!("Application form\n{ad}\n"),
            format!("Recent\n2026-11-10\nRecherche LinkedIn du 2026-11-10\t{ad}\n"),
        ] {
            assert!(text.contains(&line), "{line}: {text}");
        }
        // the leads unread: the ad still shows, the failure named (/code-review)
        let (props, unread) = answers(&draft, Some(Ok(postings)), Some(Err("down".into())));
        assert_eq!(unread, ["leads: down"]);
        assert!(
            props["posting"].as_array().is_some_and(|f| f.len() == 5),
            "{props:?}"
        );
        assert_eq!(props["recent"], json!([]));
    }

    fn tree_text(all: &[Draft]) -> String {
        surface::text(&tree(Some(all), "n")).expect("text")
    }

    /// Done-when 3 of rust-one-doc-component: a draft opens on /drafts/<id>, the draft first, then the list by stage, their
    /// ids kept apart; an id that is no path segment is encoded.
    #[test]
    fn a_draft_opens_above_the_list_on_one_page() {
        let _budget = TestBudget::new();
        assert_eq!(moved("a b/c"), "/drafts/a%20b%2Fc");
        assert_eq!(moved_to("a b", "view=data"), "/drafts/a%20b?view=data");
        assert_eq!(
            moved("a b"),
            dash_comments::page_of("/d/a b"),
            "the comment queue keys it the same"
        );
        let draft = r#"{"root":"page","elements":{"page":{"type":"stack","children":["draft"]},"draft":{"type":"text","props":{"text":"the draft"}}}}"#;
        let list = r#"{"root":"page","elements":{"page":{"type":"stack","children":["title"]},"title":{"type":"heading","props":{"text":"Drafts"}}}}"#;
        let both = joined(draft, Some(list)).expect("two trees");
        assert_eq!(surface::text(&both).expect("text"), "the draft\nDrafts\n");
        assert_eq!(joined(draft, None).expect("one tree"), draft);
    }
}
