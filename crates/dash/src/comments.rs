//! The binary's wiring of the comments plugin: its table in `comments.sqlite` on the op log, the older app's comments read
//! through its CLI (`dash store text live/comments.edn`, `DASH_CLI` names another), and each wake appended to the
//! orchestrator's wake file, the one `<repo>/.claude/run/owner` names while its supervisor lives (bin/comment-wake's first
//! choice; `DASH_REPO` names another checkout). `dash orch` runs a pass every `POLL_MS` on a thread of its own;
//! `dash comment pass --data <dir>` runs one.
use crate::tasks::{now_ms, older_app, repo};
use dash_comments as comments;
use dash_store::oplog::Replica;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

const READ: Duration = Duration::from_secs(20);

fn log(dir: &Path) -> Result<Replica, String> {
    let db = dash_store::connect(&dir.join("comments.sqlite")).map_err(|e| e.to_string())?;
    db.execute_batch(comments::SCHEMA)
        .map_err(|e| e.to_string())?;
    Replica::new(db, "dash").map_err(|e| e.to_string())
}

/// The orchestrator's wake file: the owner's, while its supervisor lives. None, and the wake waits for the next pass.
/// ponytail: the owner only; `Attention.wake_file`'s fallback to the newest live session if the owner is often unset.
fn wake_file(run: &Path) -> Option<PathBuf> {
    let owner = std::fs::read_to_string(run.join("owner")).ok()?;
    let pid: i32 = owner.trim().parse().ok().filter(|p| *p > 0)?;
    let alive = run.join(format!("{pid}.state")).exists() && unsafe { libc::kill(pid, 0) } == 0;
    alive.then(|| run.join(format!("{pid}.wake")))
}

/// One pass: read his comments, land and close rows, wake on what is due, keep it. A line saying what it did.
pub fn pass(dir: &Path, now_ms: i64) -> Result<String, String> {
    let mut log = log(dir)?;
    let kept = comments::load(&log)?;
    let cli = older_app().bin;
    let text = dash_process::run(&cli, &["store", "text", "live/comments.edn"], None, READ)?;
    let export = serde_json::from_str(&text).map_err(|e| format!("the comments: {e}"))?;
    let rows = comments::step(&kept, &comments::comments(&export)?, now_ms);
    let open = rows.values().filter(|r| r.closed.is_none()).count();
    let due = comments::due(&rows, now_ms);
    let line = (!due.is_empty()).then(|| comments::wake(&due));
    let due: Vec<String> = due.iter().map(|r| r.id.clone()).collect();
    let file = line
        .as_ref()
        .and_then(|_| wake_file(&repo().join(".claude/run")));
    let (Some(line), Some(file)) = (line, file) else {
        let written = comments::save(&mut log, &kept, &rows)?;
        let held = if due.is_empty() {
            String::new()
        } else {
            format!(", {} due and no live owner to wake", due.len())
        };
        return Ok(format!("{open} open, {written} written{held}"));
    };
    // Kept as woken before the wake is written: a save that fails then writes no wake, so a failing store never repeats
    // one every pass. A wake that fails to write puts the rows back unwoken, for the next pass.
    let mut woken = rows.clone();
    comments::told(&mut woken, &due, now_ms);
    let written = comments::save(&mut log, &kept, &woken)?;
    let said = |e: std::io::Error| format!("{}: {e}", file.display());
    let wrote = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&file)
        .and_then(|mut f| writeln!(f, "{line}"));
    if let Err(e) = wrote {
        comments::save(&mut log, &woken, &rows)?;
        return Err(said(e));
    }
    Ok(format!(
        "{open} open, {written} written, woke {} with {}",
        file.display(),
        due.join(" ")
    ))
}

pub fn once(dir: &Path, mut out: File) {
    match pass(dir, now_ms()) {
        Ok(line) => writeln!(out, "{line}").expect("stdout"),
        Err(e) => {
            eprintln!("comment: {e}");
            std::process::exit(1);
        }
    }
}

/// A pass every `POLL_MS`, for as long as the process runs; a line on stderr each time what it says changes.
pub fn watch(dir: PathBuf) {
    std::thread::spawn(move || {
        let mut said = String::new();
        loop {
            // A pass that panics is said and the next one runs: the queue never stops while orch runs.
            let ran = std::panic::catch_unwind(|| pass(&dir, now_ms()));
            let line = match ran {
                Ok(Ok(line)) => line,
                Ok(Err(e)) => format!("comment: {e}"),
                Err(_) => "comment: a pass panicked, the next one runs".to_string(),
            };
            if line != said {
                eprintln!("comment queue: {line}");
                said = line;
            }
            std::thread::sleep(Duration::from_millis(comments::POLL_MS as u64));
        }
    });
}
