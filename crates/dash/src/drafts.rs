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

/// UTC's day. ponytail: past 23:00 in Paris it is still yesterday; a draft's day 7 moves by an hour or two.
fn today() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    drafts::date((secs / 86_400) as i64)
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
            // each with the versions the table kept, as /d/<id> keeps them: a text he asked corrections on outlives the
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
/// different UI not a sea of cards. »): its receiver, channel, goal and Jev's scores; its words are on /d/<id>.
fn table(drafts: &[&Draft]) -> Value {
    let rows: Vec<Value> = (drafts.iter())
        .map(|d| {
            // a blank receiver says so, as the message draft's template did
            let receiver = match d.receiver.trim() {
                "" => "no receiver".to_string(),
                r => r.to_string(),
            };
            let cells = [receiver, d.channel.clone(), d.goal_line(), d.jev_line()];
            json!({"cells": cells, "href": format!("/d/{}", d.id)})
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

/// Draft `id` as the older app holds it now, its text read alone (no Jev log), and whether Claude sends it: `None` when it
/// holds none of that id.
fn older_draft(id: &str) -> Result<Option<(Draft, bool)>, String> {
    let all = read(&["model", "drafts"])?;
    let Some(d) = all.as_array().into_iter().flatten().find(|d| d["id"] == id) else {
        return Ok(None);
    };
    let draft = drafts::from_store(&json!([d]), &json!({}), &b""[..])?.pop();
    Ok(draft.map(|draft| (draft, by_claude(d))))
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

/// What /d/<id> draws: the draft, its page's note, and whether it is the older app's now (only then are its decisions drawn:
/// a tap on the table's copy could act on a status already changed; /code-review), and whether Claude sends it.
struct Shown {
    draft: Draft,
    note: String,
    fresh: bool,
    by_claude: bool,
}

/// /d/<id>'s draft: the older app's now, read on this request's own thread (never the list's five-minute read, so the page
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
        })
    };
    match older_draft(id) {
        Ok(Some((now, by_claude))) => {
            let draft = drafts::with_kept_versions(last, now);
            let note = drafts::save(&mut log, &before, std::slice::from_ref(&draft))
                .err()
                .map_or(String::new(), |e| format!("The table did not keep it: {e}"));
            Ok(Some(Shown {
                draft,
                note,
                fresh: true,
                by_claude,
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
        ("pending", _) => &[("rejected", "Reject"), ("moot", "Not sending")],
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
fn draft_tree(shown: &Shown, open_comments: usize) -> String {
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
    let decide = shown.fresh.then(|| format!("/d/{}", d.id));
    let doc = json!({"type": "document", "props": {
        "kind": "message-draft", "receiver": d.receiver, "sender": d.sender, "channel": d.channel, "status": d.status,
        "goal": d.goal, "lead": d.lead.as_deref().unwrap_or_default(), "scores": d.jev_line(), "text": body,
        "decide": decide, "earlier": earlier, "open_comments": open_comments, "moves": moves(&d.status, shown.by_claude),
    }});
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

/// /d/<id>: `None` when no draft of that id is held. His open comments on it, from the comment queue's table; unread, the
/// draft is drawn with no decision and says why, as the table's copy is (second /code-review).
pub fn draft_page(dir: &Path, id: &str) -> Result<Option<surface::Drawing>, String> {
    let Some(mut shown) = current(dir, id)? else {
        return Ok(None);
    };
    let open = crate::comments::open_on(dir, &format!("/d/{id}")).unwrap_or_else(|e| {
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
        tree: draft_tree(&shown, open),
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
// A draft's page (task rust-draft-approve), on this request's thread: the older app's read now, so his tap shows.
inventory::submit! {
    crate::pages::Page { path: "/d/*", step: "route_draft", reads: crate::pages::Reads::Nothing, redraw: false, serve: |a| {
        match crate::comments::serve_from_dir().and_then(|dir| draft_page(dir, &a.id())) {
            Ok(Some(page)) => a.form(Some(Ok(page))),
            Ok(None) => ("404 Not Found", crate::pages::TEXT, "no draft of that id".into()),
            Err(e) => a.form(Some(Err(e))),
        }
    } }
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
                "no receiver\temail\tthank them\ttoo_long 0.4\t/d/fix",
                "",
                "To review",
                "Receiver\tChannel\tGoal\tScores\tLink",
                "Ana (a@x.io)\temail\tno goal\ttoo_long 0.4\t/d/early",
                "Bo (Bee)\temail\task for the call\ttoo_long 0.4\t/d/late",
            ],
            "{text}"
        );
        let html = surface::screen::page("Drafts", &tree).expect("html");
        // a table, a row a draft opening its page, no card (task rust-drafts-table; @nha, 2026-11-09: « propose a different
        // UI not a sea of cards. »)
        assert!(
            html.contains(r#"class="table""#) && html.contains(r#"href="/d/early""#),
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
                .find(|l| l.ends_with("\t/d/a"))
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
            html.contains(&format!(">{whole}<")) && html.contains(r#"href="/d/a""#),
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
                "Go (g@x.io)\temail\treply\ttoo_long 0.4\t/d/go",
                "Pre (p@x.io)\temail\tapproved blocks\ttoo_long 0.4\t/d/pending-free",
            ],
            "{text}"
        );
        let tree = to_send_tree(Some((&all, &ids)), "n");
        let html = surface::screen::page("Drafts to send", &tree).expect("html");
        assert!(
            html.contains(r#"href="/d/go">Go (g@x.io)<"#) && !html.contains("Wai"),
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
        };
        let tree = draft_tree(&shown(&d, true), 0);
        let text = surface::text(&tree).expect("text");
        assert_eq!(
            text,
            "k.b@hostman.com\npending email\nContext\nGoal\nanswer their six questions\nFrom\nNicolas Ha\nTo\n\
             k.b@hostman.com\nScores\ntoo_long 0.4 (an earlier version: to rescore)\nMessage\nHello,\nthe answers.\n\
             Decision\nApprove\nRequest corrections\nReject\nNot sending\nEarlier: on 2026-11-07 by claude\nt\n"
        );
        // its decisions post to the older app's draft write, /d/<id>/status (the template's test holds their forms)
        assert!(tree.contains(r#""decide":"/d/grant-reply""#), "{tree}");
        // an open comment of his holds Approve back (/code-review)
        let held = surface::text(&draft_tree(&shown(&d, true), 1)).expect("text");
        assert!(
            held.contains("Not ready to approve: 1 open comment to apply.\n"),
            "{held}"
        );
        // the table's copy, the older app unread: no decision drawn on it (/code-review)
        let stale = surface::text(&draft_tree(&shown(&d, false), 0)).expect("text");
        assert!(!stale.contains("Decision"), "{stale}");
        d.status = "approved".into();
        let after = surface::text(&draft_tree(&shown(&d, true), 0)).expect("text");
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
        let ours = surface::text(&draft_tree(&ours, 0)).expect("text");
        assert!(ours.contains("Decision\nBack to review\nEarlier"), "{ours}");
        // rejected, sent or moot: back to review, as the older app moves them (second /code-review)
        for status in ["rejected", "sent", "moot"] {
            d.status = status.into();
            let closed = surface::text(&draft_tree(&shown(&d, true), 0)).expect("text");
            assert!(
                closed.contains("Decision\nBack to review\nEarlier"),
                "{status}: {closed}"
            );
        }
    }

    fn tree_text(all: &[Draft]) -> String {
        surface::text(&tree(Some(all), "n")).expect("text")
    }
}
