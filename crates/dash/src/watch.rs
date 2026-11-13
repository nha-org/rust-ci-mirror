//! The comment queue's file, which the watcher's rows share (dash_comments::watch, task rust-watch-port): a leaf, so a block,
//! a lesson or a Datadog push lands from any module (tasks, the front door, alerts) without a module loop through comments.
//! The question pass (`rule_questions`) lives here too: the comment pass lands the panes' questions, the inbox pass rules
//! them, and neither module may reach the other (dash-gate's module-cycle rule).
use dash_comments::watch::Item;
use dash_store::oplog::Replica;
use std::path::Path;

/// `dir`'s comments.sqlite: the comment queue and the watcher's rows.
pub(crate) fn log(dir: &Path) -> Result<Replica, String> {
    let db = dash_store::connect(&dir.join("comments.sqlite")).map_err(|e| e.to_string())?;
    db.execute_batch(dash_comments::SCHEMA)
        .map_err(|e| e.to_string())?;
    Replica::new(db, "dash").map_err(|e| e.to_string())
}

/// Where the transcripts are, as `dash pane pick` reads them.
pub(crate) fn projects() -> std::path::PathBuf {
    std::env::var("HOME")
        .map(|h| std::path::PathBuf::from(h).join(".claude/projects"))
        .unwrap_or_default()
}

/// The question pass (`dash_comments::panes`, task rust-questions-orchestrator-first), run by the inbox pass: each unruled
/// row ruled by `rule` (Sonnet and the one brief), at most `limit`; a reversible one picked in its pane, a one-way one
/// raised on `asks`. The rulings land on the table read again after the calls; a row that fails is told to the orchestrator
/// once, as a block, then tried again every 30 minutes. How many were ruled, and the asks its closed rows settle.
pub(crate) fn rule_questions(
    dir: &Path,
    asks: &mut std::collections::BTreeMap<String, dash_comments::asks::Ask>,
    rule: &mut dyn FnMut(&str) -> Result<String, String>,
    limit: usize,
    now_ms: i64,
) -> Result<(usize, std::collections::BTreeSet<String>), String> {
    use dash_comments::panes;
    let mut log = log(dir)?;
    let mut rows = dash_comments::load(&log)?;
    let tried = panes::pass(&mut rows, asks, rule, &mut picked, limit, now_ms);
    let ids: Vec<String> = tried.iter().map(|(id, _)| id.clone()).collect();
    let kept = dash_comments::load(&log)?;
    let mut fresh = kept.clone();
    panes::onto(&mut fresh, &rows, &ids);
    dash_comments::save(&mut log, &kept, &fresh)?;
    for (id, why) in &tried {
        let (Err(why), Some(row)) = (why, rows.get(id)) else {
            continue;
        };
        let title = format!("the question pass could not answer {}: {why}", row.quote);
        let task = row.task.as_deref();
        let item = Item::new(
            &format!("unruled {id}"),
            dash_comments::watch::Kind::Block,
            task,
            &title,
            &row.text,
            now_ms,
        );
        if let Err(e) = dash_comments::watch::land(&mut log, item) {
            eprintln!("question pass: {id}: the block: {e}");
        }
    }
    let ruled = tried.iter().filter(|(_, r)| r.is_ok()).count();
    Ok((ruled, panes::settled(&fresh, asks)))
}

/// The pass's picks typed in the dialog `key` names, as `dash pane pick` types them: refused, typing nothing, when its pane
/// shows another question by now (/code-review: a pick typed minutes after its ruling may meet a new dialog).
fn picked(key: &str, picks: &[usize]) -> Result<(), String> {
    use dash_terminal::dialog;
    let pane = key.split(' ').nth(1).unwrap_or_default();
    let herdr = dash_terminal::Herdr::from_env();
    let screen = dash_core::tasks::Terminal::read(&herdr, pane)?;
    let shown = dialog::parse(&screen).ok_or("its pane shows no dialog now")?;
    if dash_comments::panes::key(pane, &shown.question) != key {
        return Err("its pane shows another question now".into());
    }
    let picks: Vec<dialog::Pick> = picks.iter().map(|n| dialog::Pick::Number(*n)).collect();
    dialog::pick(
        &herdr,
        pane,
        &picks,
        &projects(),
        std::time::Duration::from_millis(200),
    )
}

/// A row landed in `dir`'s queue: true when new. It wakes on the comment pass that finds it due.
pub(crate) fn land(dir: &Path, item: Item) -> Result<bool, String> {
    dash_comments::watch::land(&mut log(dir)?, item)
}

/// The watcher's rows in `dir`, by key.
#[cfg(test)]
pub(crate) fn watched(dir: &Path) -> Result<std::collections::BTreeMap<String, Item>, String> {
    dash_comments::watch::load(&log(dir)?)
}
