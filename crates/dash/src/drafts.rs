//! The binary's wiring of the drafts plugin: its table in `drafts.sqlite` on the op log, the older app's export read from a
//! folder holding drafts.json (`dash model drafts`), leads.json (`dash store files opportunities`) and jev.jsonl (Jev's log,
//! private/ats/jev.jsonl), and an evidence/ folder of labelled files (private/evidence/drafts/*.md). `dash draft
//! import|new|show|rates`.
use dash_drafts::{self as drafts, Draft};
use dash_store::oplog::Replica;
use serde_json::Value;
use std::fs::File;
use std::io::{BufReader, Write};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

fn log(dir: &Path) -> Result<Replica, String> {
    let db = dash_store::connect(&dir.join("drafts.sqlite")).map_err(|e| e.to_string())?;
    db.execute_batch(drafts::SCHEMA)
        .map_err(|e| e.to_string())?;
    Replica::new(db, "dash").map_err(|e| e.to_string())
}

/// Each labelled file of `<from>/evidence`, in name order; none when the folder is not there, a refusal when it cannot be
/// read: a skipped file would drop his labels with no word.
fn evidence(from: &Path) -> Result<Vec<drafts::Labels>, String> {
    let folder = from.join("evidence");
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
    let mut all = store_export(from)?;
    let files = evidence(from)?;
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
