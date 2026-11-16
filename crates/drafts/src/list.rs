//! The /drafts list as a UI tree (task rust-drafts-list, rfc-rust-plugins-ui section 2's drafts waiting), the plugin's `ui`
//! part: data, never markup; the design system draws it as HTML, text and the terminal view.
use crate::Draft;
use dash_sdk::{Tree, json};
use std::collections::BTreeMap;

/// The part of /drafts that follows the drafts view, drawn again whole at each of its diffs (a send recorded, a draft
/// kept), so the page changes with no reload.
pub const LIVE: &str = "drafts";

/// `tree` under `root`, as the host draws a tree.
fn rooted(tree: Tree, root: &str) -> String {
    json!({"root": root, "elements": tree.elements()}).to_string()
}

/// The screen at /drafts: a section a status as `crate::OPEN` orders them, each draft a row (its receiver, its goal in one
/// line, Jev's answers, its send's state from `sends` once one began) opening its page, the whole following the drafts view. A section's name keeps the older page's anchor (`/drafts#stage-approved`). `None`:
/// no read has ended, so no list, and no « no draft waiting » either.
pub fn tree(all: Option<&[Draft]>, sends: &BTreeMap<String, String>, note: &str) -> String {
    let titles = ["Corrections in progress", "To review", "To send"];
    let mut t = Tree::default();
    t.add("title", "heading", json!({"text": "Drafts"}), &[]);
    t.add("note", "text", json!({"text": note}), &[]);
    let mut children = vec!["title".to_string(), "note".to_string()];
    let open = crate::open(all.unwrap_or_default());
    if open.iter().any(|d| d.status == "pending") {
        let series = json!({"name": "Review in series, the simplest first", "href": "/review"});
        t.add("tools", "links", json!({"links": [series]}), &[]);
        children.push("tools".into());
    }
    for (status, title) in crate::OPEN.iter().zip(titles) {
        let stage: Vec<&Draft> = open
            .iter()
            .copied()
            .filter(|d| d.status == *status)
            .collect();
        if stage.is_empty() {
            continue;
        }
        let (section, list) = (format!("stage-{status}"), format!("drafts-{status}"));
        table(&mut t, &list, &stage, sends);
        t.add(&section, "section", json!({"title": title}), &[list]);
        children.push(section);
    }
    if all.is_some() && open.is_empty() {
        t.add("none", "text", json!({"text": "No draft waiting."}), &[]);
        children.push("none".into());
    }
    t.add("page", "stack", json!(null), &children);
    t.add(LIVE, "live", json!({"view": "drafts"}), &["page".into()]);
    rooted(t, LIVE)
}

/// Drafts as the design system's draft list `id`, a row a draft opening its page (task rust-drafts-table; @nha, 2026-11-09:
/// « propose a different UI not a sea of cards. »): its receiver, channel, goal and Jev's scores, its send's state once one
/// began; its words open at /drafts/<id>.
fn table(t: &mut Tree, id: &str, drafts: &[&Draft], sends: &BTreeMap<String, String>) {
    let rows: Vec<_> = (drafts.iter())
        .map(|d| {
            // a blank receiver says so, as the message draft's template did
            let receiver = match d.receiver.trim() {
                "" => "no receiver",
                r => r,
            };
            json!({"receiver": receiver, "channel": d.channel, "goal": d.goal_line(), "scores": d.jev_line(),
                "send": sends.get(&d.id).cloned().unwrap_or_default(), "href": href(&d.id)})
        })
        .collect();
    t.add(id, "draft-list", json!({"rows": rows}), &[]);
}

/// Draft `id`'s page, encoded as the comment queue keys it.
pub fn href(id: &str) -> String {
    format!("/drafts/{}", crate::card::segment(id))
}

/// /drafts?show=to-send, where the daily's « N to send » leads: the outbox's drafts that go now, in its order (oldest
/// approval first). `None`: no read has ended, or the last one did not read the export, so which drafts go is not known: no
/// list, never « nothing to send ».
pub fn to_send_tree(listed: Option<(&[Draft], &[String])>, note: &str) -> String {
    let mut t = Tree::default();
    t.add("title", "heading", json!({"text": "Drafts to send"}), &[]);
    t.add("note", "text", json!({"text": note}), &[]);
    let back = json!({"name": "All drafts, by stage", "href": "/drafts"});
    t.add("tools", "links", json!({"links": [back]}), &[]);
    let mut children = vec!["title".to_string(), "note".to_string(), "tools".to_string()];
    if let Some((all, ids)) = listed {
        let go: Vec<&Draft> = (ids.iter())
            .filter_map(|id| all.iter().find(|d| &d.id == id))
            .collect();
        if go.is_empty() {
            t.add("none", "text", json!({"text": "Nothing to send."}), &[]);
            children.push("none".into());
        } else {
            table(&mut t, "to-send", &go, &BTreeMap::new());
            t.add(
                "stage-to-send",
                "section",
                json!({"title": "To send"}),
                &["to-send".into()],
            );
            children.push("stage-to-send".into());
        }
    }
    t.add("page", "stack", json!(null), &children);
    rooted(t, "page")
}

/// How old a draft's read may be for its page to offer decisions: two passes of the latest pass, so a status changed
/// elsewhere is never acted on from a copy older than that.
const FRESH_MS: i64 = 10 * 60_000;

/// The statuses whose text he may still change: `Write.editable?/1`.
const EDITABLE: [&str; 2] = ["pending", "approved-pending-changes"];

/// A draft's own page as a UI tree (task rust-draft-approve; rfc-rust-plugins-ui section 4): the message draft's template
/// whole, its goal, sender and receiver above its current text (its last version), its decisions while its facts were
/// imported and `open` read (his open comments hold Approve back), then each earlier version. What it answers and its state
/// are its `facts`, the older app's as the latest pass last imported them.
pub fn draft_tree(d: &Draft, open: Result<usize, String>, editing: bool, now_ms: i64) -> String {
    let (body, before) =
        (d.versions.split_last()).map_or(("", &[][..]), |(v, rest)| (v.text.as_str(), rest));
    // no number: a text set back to an earlier one moves to last, so a position is no order of writing (second /code-review)
    let earlier: Vec<_> = (before.iter())
        .filter(|v| !v.text.trim().is_empty())
        .map(|v| json!({"title": format!("Earlier: on {} by {}", v.on, v.by), "text": v.text}))
        .collect();
    let unread = (d.facts["unread"].as_array().into_iter().flatten()).filter_map(|u| u.as_str());
    let mut note: Vec<String> = unread
        .map(|u| format!("What it answers did not read ({u}): those parts are left out."))
        .collect();
    let read = d.facts["read_ms"].as_i64();
    let fresh_read = read.is_some_and(|at| now_ms - at < FRESH_MS);
    match read {
        None => note.push(
            "Not imported from the older app yet: no decision until the latest pass reads it."
                .into(),
        ),
        Some(at) if !fresh_read => note.push(format!(
            "Read {} min ago: no decision until it is read again.",
            (now_ms - at) / 60_000
        )),
        Some(_) => {}
    }
    if d.facts["gone"] == true {
        note.push("The older app no longer holds it: this is the table's last kept.".into());
    }
    let comments = open
        .map_err(|e| {
            note.push(format!(
                "His comments did not read ({e}): no decision until they do."
            ))
        })
        .ok();
    let fresh = fresh_read && d.facts["gone"] != true && comments.is_some();
    let by_claude = d.facts["by_claude"].as_bool().unwrap_or(false);
    // the older app's draft write, POST /d/<id>/status
    let decide = fresh.then(|| format!("/d/{}", crate::card::segment(&d.id)));
    let open_to = |statuses: &[&str]| fresh && statuses.contains(&d.status.as_str());
    let change = open_to(&EDITABLE).then(|| href(&d.id));
    let schedule = open_to(&crate::OPEN).then(|| format!("{}/set", href(&d.id)));
    let mut props = json!({
        "kind": "message-draft", "receiver": d.receiver, "sender": d.sender, "channel": d.channel, "status": d.status,
        "goal": d.goal, "lead": d.lead.as_deref().unwrap_or_default(), "scores": d.jev_line(), "text": body,
        "decide": decide, "earlier": earlier, "open_comments": comments.unwrap_or(0),
        "moves": crate::moves(&d.status, by_claude), "change": change, "editing": editing && change.is_some(),
        "schedule": schedule,
    });
    if let (Some(p), Some(answers)) = (props.as_object_mut(), d.facts["answers"].as_object()) {
        p.extend(answers.clone());
    }
    let mut t = Tree::default();
    t.add("draft", "document", props, &[]);
    let mut children = vec!["draft".to_string()];
    if !note.is_empty() {
        t.add("note", "text", json!({"text": note.join(" ")}), &[]);
        children.insert(0, "note".into());
    }
    t.add("page", "stack", json!(null), &children);
    rooted(t, "page")
}

/// `first`'s tree, then `second`'s under it, its ids renamed `list-<id>`; the list's own part, never its live wrapper: only
/// /drafts follows the drafts view.
pub fn joined(first: &str, second: &str) -> Result<String, String> {
    let parse = |t: &str| serde_json::from_str::<serde_json::Value>(t).map_err(|e| e.to_string());
    let (mut page, second) = (parse(first)?, parse(second)?);
    let renamed =
        |id: &serde_json::Value| json!(format!("list-{}", id.as_str().unwrap_or_default()));
    let root = page["root"].clone();
    let list_root = match &second["elements"][second["root"].as_str().unwrap_or_default()] {
        e if e["type"] == "live" => e["children"][0].clone(),
        _ => second["root"].clone(),
    };
    let elements = page["elements"]
        .as_object_mut()
        .ok_or("a tree holds its elements")?;
    for (id, el) in second["elements"].as_object().into_iter().flatten() {
        let mut el = el.clone();
        if let Some(children) = el.get_mut("children").and_then(|c| c.as_array_mut()) {
            children.iter_mut().for_each(|c| *c = renamed(c));
        }
        elements.insert(format!("list-{id}"), el);
    }
    elements.insert(
        "opened".into(),
        json!({"type": "stack", "children": [root, renamed(&list_root)]}),
    );
    page["root"] = json!("opened");
    Ok(page.to_string())
}
