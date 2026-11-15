//! `dash draft import|new|show|rates`, registered here through the core's `Command` (task rust-move-core-ports' tracer): the
//! host hands each the clock and the plugin's own table, drafts.sqlite, so the binary keeps no arm or helper for them. The
//! older app's export is a folder holding drafts.json (`dash model drafts`), leads.json (`dash store files opportunities`)
//! and jev.jsonl (Jev's log), and an evidence/ folder of labelled files (private/evidence/drafts/*.md).
use crate::{self as drafts, Draft, Kept, Labels};
use dash_sdk::{Command, Host, OpLog};
use serde_json::Value;
use std::fs::File;
use std::io::{BufReader, Write};
use std::path::Path;

pub use crate::{FILE, SCHEMAS as SCHEMA, log};

/// UTC's day, from the host's clock.
fn today(host: &dyn Host) -> String {
    drafts::date(host.now_ms().div_euclid(86_400_000))
}

/// What the table holds: each draft as kept, and a row whose record does not read named on stderr, so the read goes on.
pub fn kept(log: &dyn OpLog) -> Result<Kept, String> {
    let kept = drafts::load(log)?;
    for id in &kept.unread {
        eprintln!(
            "draft {id}: its record does not read, left out; dash draft import writes it again"
        );
    }
    Ok(kept)
}

/// Each labelled file of `folder`, in name order; none when the folder is not there, a refusal when it cannot be read: a
/// skipped file would drop his labels with no word.
fn evidence(folder: &Path) -> Result<Vec<Labels>, String> {
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

fn labelled_file(file: &Path) -> Result<Labels, String> {
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

pub fn with_labels(mut all: Vec<Draft>, evidence_in: &Path) -> Result<(Vec<Draft>, usize), String> {
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

/// Every draft of the export, kept; a line of counts by outcome. The table also keeps a draft the export no longer holds.
fn import(host: &dyn Host, dir: &Path, from: &Path) -> Result<String, String> {
    let (all, labelled) = export(from)?;
    let mut log = log(host, dir)?;
    let before = kept(&*log)?;
    let written = drafts::save(&mut *log, &before, &all)?;
    let gone = before
        .drafts
        .keys()
        .filter(|id| !all.iter().any(|d| &d.id == *id));
    let today = today(host);
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
}

/// A labelled file's draft, kept, then shown: on the export's record of it with `--from`, else on the one kept, else on
/// the file alone.
fn new(host: &dyn Host, dir: &Path, file: &Path, from: Option<&Path>) -> Result<String, String> {
    let l = labelled_file(file)?;
    let mut log = log(host, dir)?;
    let before = kept(&*log)?;
    let base = match from {
        Some(from) => store_export(from)?.into_iter().find(|d| d.id == l.id),
        None => before.drafts.get(&l.id).cloned(),
    };
    let d = drafts::labelled(base, l);
    drafts::save(&mut *log, &before, std::slice::from_ref(&d))?;
    Ok(drafts::show(&d, &today(host)))
}

fn show(host: &dyn Host, dir: &Path, id: &str) -> Result<String, String> {
    let log = log(host, dir)?;
    let kept = kept(&*log)?.drafts;
    let d = kept.get(id).ok_or(format!("{id}: not kept"))?;
    let back = drafts::card::back_to_agent(&*log, id)?;
    let back = back.map_or(String::new(), |why| format!("Back to its agent: {why}\n"));
    Ok(format!("{}{back}", drafts::show(d, &today(host))))
}

/// The agent's rewrite of draft `id` after his correction: `text` its new last version, by claude, `change` what it answers;
/// the earlier versions kept, readable. Its card runs every check of its channel on it at its next draw.
pub fn revise(
    host: &dyn Host,
    dir: &Path,
    id: &str,
    change: &str,
    text: &str,
) -> Result<String, String> {
    if text.trim().is_empty() {
        return Err("a new version has words".into());
    }
    let mut log = log(host, dir)?;
    let before = kept(&*log)?;
    let mut d = before
        .drafts
        .get(id)
        .cloned()
        .ok_or(format!("{id}: not kept"))?;
    drafts::card::revisable(host, &*log, &d)?;
    d.versions.push(drafts::Version {
        text: text.trim().into(),
        by: "claude".into(),
        on: today(host),
        change: change.into(),
        ..drafts::Version::default()
    });
    drafts::save(&mut *log, &before, std::slice::from_ref(&d))?;
    Ok(format!(
        "{id}: version {} kept, its checks run on its card",
        d.versions.len()
    ))
}

fn rates(host: &dyn Host, dir: &Path) -> Result<String, String> {
    let kept: Vec<Draft> = kept(&*log(host, dir)?)?.drafts.into_values().collect();
    Ok(drafts::rates(&kept, &today(host)))
}

const IMPORT: &str = "dash draft import --data <dir> --from <export>";
const NEW: &str = "dash draft new --data <dir> --file <labelled.md> [--from <export>]";
const SHOW: &str = "dash draft show --data <dir> <id>";
const RATES: &str = "dash draft rates --data <dir>";
const REVISE: &str =
    "dash draft revise --data <dir> <id> --change <what it answers> --text <the new words>";

/// A command's line on `out`, or why not on stderr and exit 1; the run armed on its folder once its arguments matched, as the
/// binary's arms were.
fn said(
    host: &dyn Host,
    dir: &str,
    out: &mut dyn Write,
    answer: impl FnOnce(&Path) -> Result<String, String>,
) {
    let dir = Path::new(dir);
    host.arm(dir, "draft");
    match answer(dir) {
        Ok(line) => writeln!(out, "{line}").expect("stdout"),
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}

/// Arguments its usage does not name: the usage, exit 2.
fn refused(usage: &str) -> ! {
    eprintln!("usage: {usage}");
    std::process::exit(2)
}

fn import_run(host: &dyn Host, args: &[&str], out: &mut dyn Write) {
    match args {
        ["--data", dir, "--from", from] => {
            said(host, dir, out, |d| import(host, d, Path::new(from)))
        }
        _ => refused(IMPORT),
    }
}

fn new_run(host: &dyn Host, args: &[&str], out: &mut dyn Write) {
    match args {
        ["--data", dir, "--file", file] => {
            said(host, dir, out, |d| new(host, d, Path::new(file), None))
        }
        ["--data", dir, "--file", file, "--from", from] => said(host, dir, out, |d| {
            new(host, d, Path::new(file), Some(Path::new(from)))
        }),
        _ => refused(NEW),
    }
}

fn show_run(host: &dyn Host, args: &[&str], out: &mut dyn Write) {
    match args {
        ["--data", dir, id] => said(host, dir, out, |d| show(host, d, id)),
        _ => refused(SHOW),
    }
}

fn revise_run(host: &dyn Host, args: &[&str], out: &mut dyn Write) {
    match args {
        ["--data", dir, id, "--change", change, "--text", text] => {
            said(host, dir, out, |d| revise(host, d, id, change, text))
        }
        _ => refused(REVISE),
    }
}

fn rates_run(host: &dyn Host, args: &[&str], out: &mut dyn Write) {
    match args {
        ["--data", dir] => said(host, dir, out, |d| rates(host, d)),
        _ => refused(RATES),
    }
}

inventory::submit! { Command { usage: IMPORT, words: &["draft", "import"], run: Some(import_run) } }
inventory::submit! { Command { usage: NEW, words: &["draft", "new"], run: Some(new_run) } }
inventory::submit! { Command { usage: SHOW, words: &["draft", "show"], run: Some(show_run) } }
inventory::submit! { Command { usage: RATES, words: &["draft", "rates"], run: Some(rates_run) } }
inventory::submit! { Command { usage: REVISE, words: &["draft", "revise"], run: Some(revise_run) } }
