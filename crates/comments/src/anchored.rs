//! Comments on an element by its anchor, the core's prop every element carries (rfc-rust-plugins-ui sections 3 and 4, task
//! rust-draft-corrections): his words filed on a byte range of the element's text, a row of the queue that wakes the agent,
//! and drawn back on that element by its anchor alone. The plugin never learns whose element it is: a draft's card asks the
//! core for what is placed on its text's anchor, and this answers.
use crate::{COMPASS, Row, load, posted, put};
use dash_core::{Answer, Asked, Host, OpLog, TEXT, Tree, json};
use serde_json::Value as Json;
use std::sync::atomic::{AtomicU32, Ordering};

/// The queue's file in the data folder, the binary's comments.sqlite.
const FILE: &str = "comments.sqlite";
/// Where a form files a comment on an anchor, the core's path: the plugin whose element it is posts to it, never naming this one.
pub const POST: &str = dash_core::COMMENT_ON_ANCHOR;
const TEXT_MOST: usize = 10_000;
const QUOTE_MOST: usize = 2_000;

fn log(host: &dyn Host) -> Result<Box<dyn OpLog>, String> {
    host.log(&host.data(), FILE, &[crate::SCHEMA])
}

/// Why a comment on an anchor is refused as asked (a 400), before anything is read or written; `None`: it may be filed.
fn refused(
    anchor: &str,
    (from, to): (usize, usize),
    quote: &str,
    text: &str,
    page: &str,
) -> Option<String> {
    if anchor.is_empty() || from > to {
        return Some("a comment on an anchor names it and a range".into());
    }
    if !page.starts_with('/') || page.starts_with("//") || page.contains(['\\', '\r', '\n']) {
        return Some("a comment's page is a path of this app".into());
    }
    let text = text.trim();
    (text.is_empty() || text.len() > TEXT_MOST || quote.len() > QUOTE_MOST)
        .then(|| format!("a comment is 1 to {TEXT_MOST} bytes, its quote {QUOTE_MOST} at most"))
}

/// His `text` on the words `quote` of `anchor`'s text, bytes `from..to` of it, the element shown on `page`: one open row of
/// the queue, its id. The id holds the clock, the process and a count, so two processes filing in one millisecond keep both.
pub fn file(
    host: &dyn Host,
    (anchor, from, to): (&str, usize, usize),
    quote: &str,
    text: &str,
    page: &str,
) -> Result<String, String> {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    if let Some(e) = refused(anchor, (from, to), quote, text, page) {
        return Err(e);
    }
    let text = text.trim();
    let now = host.now_ms();
    let (pid, n) = (std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed));
    let id = format!("anchor-{now:x}-{pid}-{n}");
    let row = Row {
        anchor: Some((anchor.into(), from, to)),
        ..posted(
            &id,
            page,
            quote,
            text,
            &dash_core::iso(now.div_euclid(1000)),
            now,
        )
    };
    put(&mut *log(host)?, row)?;
    Ok(id)
}

/// `POST /anchor/comment`: `anchor`, `from`, `to`, `quote`, `text` and `page` as `file` takes them; 200 and the row's id.
fn filed(a: &Asked, form: &str) -> Answer {
    let field = |n| dash_core::param(form, n).unwrap_or_default();
    let at = |n| field(n).parse::<usize>().ok();
    let (Some(from), Some(to)) = (at("from"), at("to")) else {
        return Answer::Body("400 Bad Request", TEXT, "a range is two numbers".into());
    };
    let (anchor, quote, text, page) = (
        field("anchor"),
        field("quote"),
        field("text"),
        field("page"),
    );
    // a refusal is his to fix (400); any other failure is the server's (500), whatever its words
    if let Some(e) = refused(&anchor, (from, to), &quote, &text, &page) {
        return Answer::Body("400 Bad Request", TEXT, e);
    }
    match file(a.host, (&anchor, from, to), &quote, &text, &page) {
        Ok(id) => Answer::Body("200 OK", TEXT, id),
        Err(e) => a.failed(e),
    }
}

/// Every comment on `anchor`, oldest first.
// ponytail: the whole queue read and filtered once per card drawn, as `load` reads it for every pass; a read of one anchor's
// rows once the core has a read port, or the thread's cards drawn from one read, if /mail/t turns slow.
pub fn on(log: &dyn OpLog, anchor: &str) -> Result<Vec<Row>, String> {
    let mut rows: Vec<Row> = load(log)?
        .into_values()
        .filter(|r| r.from == COMPASS && r.anchor.as_ref().is_some_and(|(a, ..)| a == anchor))
        .collect();
    rows.sort_by_key(|r| (r.landed_ms, r.id.clone()));
    Ok(rows)
}

/// The comments on `props`' `anchor`, its element showing `words` now, under `id`: each his words on the words he picked, « on
/// an earlier version » once those words left the text, then the agent's answer. Nothing when none.
fn drawn(host: &dyn Host, props: &Json, id: &str, tree: &mut Tree) -> Result<Vec<String>, String> {
    let anchor = props["anchor"].as_str().ok_or("no anchor named")?;
    let words = props["words"].as_str().unwrap_or_default();
    let rows = on(&*log(host)?, anchor)?;
    if rows.is_empty() {
        return Ok(vec![]);
    }
    let mut children = vec![];
    for (i, r) in rows.iter().enumerate() {
        let gone = !r.quote.trim().is_empty() && !words.contains(r.quote.trim());
        let on = if gone { ", on an earlier version" } else { "" };
        let said = format!("On « {} »{on}: {}", r.quote.trim(), r.text);
        let at = format!("{id}-{i}");
        tree.add(&at, "text", json!({ "text": said }), &[]);
        tree.anchor(&at, &format!("comments/{}", r.id));
        children.push(at.clone());
        if let Some(answer) = &r.answer {
            let at = format!("{at}-answer");
            tree.add(
                &at,
                "text",
                json!({ "text": format!("Answer: {answer}") }),
                &[],
            );
            children.push(at);
        }
    }
    tree.add(id, "section", json!({ "title": "Corrections" }), &children);
    Ok(vec![id.into()])
}

dash_core::submit! { dash_core::Element { key: "anchor/10-comments", draw: drawn } }
dash_core::submit! { dash_core::Post { path: POST, step: "route_comment_on_anchor", serve: filed } }
