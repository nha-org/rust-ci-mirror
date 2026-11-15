//! The comment queue's board on /comments and its agent's line, drawn as a page tree from the queue's rows. Here, not in
//! the binary, which only shrinks (rust-binary-ratchet; task words-capture moved it): dash hands each filed task's place in
//! the roadmap.
use crate::Row;
use std::collections::BTreeMap;

/// What the queue's agent is on, one line.
pub fn agent_line(rows: &BTreeMap<String, Row>) -> String {
    let waiting = crate::waiting(rows);
    match crate::working(rows) {
        Some(r) => format!(
            "The comment agent works comment {} on {}: « {} » ({waiting} open).",
            r.id,
            r.page,
            crate::clipped(&r.text)
        ),
        None if waiting > 0 => format!("The comment agent starts on the oldest of {waiting} open."),
        None => "The comment agent is idle: no comment waits.".into(),
    }
}

/// The board: the queue, the answered and the filed, each a card in its column. `place`: a filed task's closed and its
/// roadmap bucket.
pub fn queue_tree(
    rows: &BTreeMap<String, Row>,
    place: &dyn Fn(&str) -> (bool, Option<String>),
) -> String {
    use serde_json::json;
    let text = |t: &str| json!({"type": "text", "props": {"text": t}});
    let since = |at: &str| at.get(..16).unwrap_or(at).replace('T', " ");
    let mut elements = serde_json::Map::new();
    elements.insert("agent".into(), text(&agent_line(rows)));
    // (column key, its status for the icon, its name, its cards)
    let mut columns: [(&str, &str, &str, Vec<String>); 4] = [
        ("backlog", "backlog", "Backlog", vec![]),
        ("answered", "in-review", "Answered", vec![]),
        ("created", "todo", "Task created", vec![]),
        ("done", "done", "Done", vec![]),
    ];
    let mut card = |column: usize, name: String, row: serde_json::Value| {
        elements.insert(name.clone(), row);
        columns[column].3.push(name);
    };
    let queue = crate::queue(rows);
    for (i, r) in queue.iter().enumerate() {
        let (state, href) = match (r.from.as_str(), &r.worker) {
            (crate::COMPASS, Some(_)) => ("the agent works it", format!("{}#c-{}", r.page, r.id)),
            (crate::COMPASS, None) => ("waits for the agent", format!("{}#c-{}", r.page, r.id)),
            (crate::CHAT, _) => ("his question in chat, the orchestrator's", r.link.clone()),
            (crate::PANE, _) => ("a task's question, the question pass's", r.link.clone()),
            (crate::words::WORDS, _) => ("a point of his message, to land", r.link.clone()),
            _ => ("the orchestrator's, from the old app", r.link.clone()),
        };
        let row = json!({"type": "issue-row", "props": {
            "id": r.page, "title": crate::clipped(&r.text), "status": "backlog",
            "says": format!("{state}, {}", since(&r.at)), "comments": 0, "href": href,
        }});
        card(0, format!("queued-{i}"), row);
    }
    for (i, r) in crate::answered_newest(rows, crate::COMPASS, usize::MAX)
        .iter()
        .enumerate()
    {
        let says = format!("on {}, {}", r.page, since(&r.at));
        let (column, row) = match &r.task {
            // its task's card: the task's key, his words, the task's issue page (the comment agent files in the older app), its
            // roadmap bucket the card's label (@nha, 2026-11-09: « task created (+ pill now/next/later + link »)
            Some(id) => {
                let (closed, horizon) = place(id);
                let status = if closed { "done" } else { "todo" };
                let row = json!({"type": "issue-row", "props": {
                    "id": id, "title": crate::clipped(&r.text), "status": status, "says": says, "comments": 0,
                    "project": "site_perso", "labels": horizon.into_iter().collect::<Vec<_>>(),
                }});
                (if closed { 3 } else { 2 }, row)
            }
            // the agent's answer under his words, clipped as they are: the page's thread holds it whole (task
            // comments-answered-shows-answer; @nha, 2026-11-10: « I have no way to see the answers. »)
            None => {
                let row = json!({"type": "issue-row", "props": {
                    "id": r.page, "title": crate::clipped(&r.text), "status": "in-review", "says": says,
                    "comments": 0, "href": format!("{}#c-{}", r.page, r.id), "answer": crate::clipped(r.answer.as_deref().unwrap_or_default()),
                }});
                (1, row)
            }
        };
        card(column, format!("answered-{i}"), row);
    }
    let mut groups = vec![];
    for (key, status, title, names) in columns {
        let group = json!({"type": "issue-group", "props": {"status": status, "title": title}, "children": names});
        elements.insert(key.into(), group);
        groups.push(key);
    }
    elements.insert(
        "board".into(),
        json!({"type": "board", "props": {}, "children": groups}),
    );
    // no heading: the menu's Comments entry names the page (@nha, 2026-11-09: « remove that we have a tab now »)
    let mut children = vec!["agent"];
    // the agent's line counts the compass's rows; the old app's are the orchestrator's, said apart so the two agree
    let older = queue.iter().filter(|r| r.from == crate::OLDER).count();
    if older > 0 {
        let said = format!("{older} more from the old app, the orchestrator's.");
        elements.insert("older".into(), text(&said));
        children.push("older");
    }
    children.push("board");
    let page = json!({"type": "stack", "children": children});
    elements.insert("page".into(), page);
    json!({"root": "page", "elements": elements}).to_string()
}
