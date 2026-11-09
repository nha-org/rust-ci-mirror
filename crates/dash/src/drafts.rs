//! The binary's wiring of the drafts plugin: its table in `drafts.sqlite` on the op log, the older app's export read from a
//! folder holding drafts.json (`dash model drafts`), leads.json (`dash store files opportunities`) and jev.jsonl (Jev's log,
//! private/ats/jev.jsonl), and an evidence/ folder of labelled files (private/evidence/drafts/*.md). `dash draft
//! import|new|show|rates`.
use crate::fresh::{self, Read};
use dash_drafts::{self as drafts, Draft};
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
    let entries = match std::fs::read_dir(&folder) {
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
}

/// The older app's drafts, where drafts are written today: `dash model drafts`, `dash store files opportunities`, Jev's
/// log and the labelled files, as `dash draft import` reads them from an export.
/// ponytail: reads Jev's whole log (43 MB on 2026-11-09) each time, off the routing thread and at most every five minutes;
/// a read from the log's last offset if its time ever shows.
fn exported() -> Result<Vec<Draft>, String> {
    let older = crate::tasks::older_app();
    let read = |args: &[&str]| -> Result<Value, String> {
        let text = dash_process::run(&older.bin, args, None, OLDER_CALL)?;
        serde_json::from_str(&text).map_err(|e| format!("dash {}: {e}", args.join(" ")))
    };
    let repo = crate::tasks::repo();
    let jev = repo.join("private/ats/jev.jsonl");
    let jev = File::open(&jev).map_err(|e| format!("{}: {e}", jev.display()))?;
    let leads = read(&["store", "files", "opportunities"])?;
    let all = drafts::from_store(&read(&["model", "drafts"])?, &leads, BufReader::new(jev))?;
    Ok(with_labels(all, &repo.join("private/evidence/drafts"))?.0)
}

/// A refresh: the export listed and kept in the table of `dir`. The page lists the export, never the table, so a draft
/// the older app no longer holds leaves the list (the table keeps it: data is gold). With no export, the table's last
/// kept, named as such; with neither, or a table holding no open draft, the read's error: never « no draft waiting ».
fn refreshed(dir: &Path) -> Result<Listed, String> {
    match exported() {
        Ok(all) => {
            let saved = log(dir).and_then(|mut log| {
                let before = kept(&log)?;
                drafts::save(&mut log, &before, &all)
            });
            let said = saved.err().map_or(String::new(), |e| {
                format!(" The table did not keep it: {e}")
            });
            Ok(Listed { drafts: all, said })
        }
        Err(e) => {
            let drafts: Vec<Draft> = kept(&log(dir)?)?.drafts.into_values().collect();
            if drafts::open(&drafts).is_empty() {
                return Err(e);
            }
            let said =
                format!(" The older app's read failed, these are the table's last kept: {e}");
            Ok(Listed { drafts, said })
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
        let line = "Everything to review on one page, the simplest first";
        let series = json!({"type": "card", "props": {"title": "Review in series", "line": line, "href": "/review"}});
        elements.insert("series".into(), series);
        elements.insert(
            "tools".into(),
            json!({"type": "cards", "children": ["series"]}),
        );
        children.push("tools".into());
    }
    for (status, title) in drafts::OPEN.iter().zip(titles) {
        let cards: Vec<String> = open
            .iter()
            .filter(|d| d.status == *status)
            .map(|d| {
                let name = format!("draft-{}", d.id);
                let receiver = match d.receiver.trim() {
                    "" => "no receiver",
                    r => r,
                };
                // The message draft's template, listed: its words on its own page (task rust-doc-types).
                let card = json!({"type": "document", "props": {
                    "kind": "message-draft", "receiver": receiver, "sender": d.sender, "channel": d.channel,
                    "status": d.status, "goal": d.goal_line(), "lead": d.lead.as_deref().unwrap_or_default(),
                    "scores": d.jev_line(), "href": format!("/d/{}", d.id),
                }});
                elements.insert(name.clone(), card);
                name
            })
            .collect();
        if cards.is_empty() {
            continue;
        }
        let (section, list) = (format!("stage-{status}"), format!("cards-{status}"));
        elements.insert(list.clone(), json!({"type": "cards", "children": cards}));
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

static READ: Mutex<Read<Result<Listed, String>>> = Mutex::new(Read::new());

/// The page from the last refresh, held in memory (a view reads no SQLite on the routing thread), and whether one has
/// ended: one starts past five minutes on its own thread, as /tools reads, so a view never waits on the older app. A
/// refresh with nothing to list is the page's error.
pub fn page(dir: &Path) -> Result<(surface::Drawing, bool), String> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH);
    let now = now.map_err(|e| e.to_string())?.as_secs();
    let at = dir.to_path_buf();
    let read = fresh::last(&READ, now, "drafts", move || refreshed(&at))?;
    let (note, listed) = match &read {
        None => ("Reading the older app's drafts.".to_string(), None),
        Some((Err(e), _)) => return Err(format!("drafts: {e}")),
        Some((Ok(l), at)) => (fresh::ago(now, *at) + &l.said, Some(l.drafts.as_slice())),
    };
    let title = "Drafts".into();
    let tree = tree(listed, &note);
    Ok((surface::Drawing::Tree { title, tree }, read.is_some()))
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
                "Review in series\tEverything to review on one page, the simplest first\t/review",
                "",
                "Corrections in progress",
                "no receiver\t/d/fix",
                "approved-pending-changes email",
                "thank them · Scores: too_long 0.4",
                "",
                "To review",
                "Ana (a@x.io)\t/d/early",
                "pending email",
                "no goal · Scores: too_long 0.4",
                "Bo (Bee)\t/d/late",
                "pending email",
                "ask for the call · Scores: too_long 0.4",
            ],
            "{text}"
        );
        let html = surface::screen::page("Drafts", &tree).expect("html");
        assert!(
            html.contains(r#"class="doc-title" href="/d/early">Ana (a@x.io)"#)
                && html.contains(r#"class="doc doc-message-draft""#),
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
            let card = text
                .lines()
                .find(|l| l.ends_with("\t/d/a"))
                .map(str::to_string);
            card.and_then(|l| l.split('\t').next().map(str::to_string))
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
        assert!(html.contains(&format!(r#"href="/d/a">{whole}<"#)), "{html}");
    }

    fn tree_text(all: &[Draft]) -> String {
        surface::text(&tree(Some(all), "n")).expect("text")
    }
}
