//! /drafts and /drafts/<id>, the plugin's pages (task drafts-genuinely-good, part 3): each reads the drafts view through its
//! host (a draft's record and its send's state, the plugin's own file's changesets), never the older app, and of its file
//! only the pass's read time (`crate::latest::passed_at`); the
//! trees are list.rs's, which the design system draws as HTML, text and dash tui. /d/<id>, a draft's page once, sends there.
use crate::{Draft, list};
use dash_sdk::{Answer, Asked, Drawing, Host, Page, Reads, TEXT};
use std::collections::BTreeMap;

/// The drafts view's rows: each draft whose record reads, read as of the pass's last read when it still holds it, and each
/// one's send's state once one began (a check that sent it back is no send: /code-review).
fn viewed(host: &dyn Host) -> Result<(Vec<Draft>, BTreeMap<String, String>), String> {
    let (mut drafts, mut sends) = (vec![], BTreeMap::new());
    for (_, id, record, send) in host.view()?.rows("drafts")? {
        drafts.extend(
            serde_json::from_str(&record)
                .ok()
                .and_then(|j| crate::from_json(&j)),
        );
        if !send.is_empty() && !matches!(send.as_str(), "returned" | "corrected") {
            sends.insert(id, send);
        }
    }
    // no pass read yet, or its row unread: each draft keeps its own read time, the older one
    let passed = crate::db(host).and_then(|db| crate::latest::passed_at(&db));
    if let Ok(Some(at)) = passed {
        for d in drafts.iter_mut().filter(|d| d.facts["gone"] != true) {
            let own = d.facts["read_ms"].as_i64();
            if own.is_some_and(|own| own < at) {
                d.facts["read_ms"] = at.into();
            }
        }
    }
    Ok((drafts, sends))
}

/// The drafts that go now, in the outbox's order (`facts.to_send`, its rules the older app's), none whose send waits,
/// goes or went, none the older app no longer holds and none closed since the pass placed it (rejected through its page:
/// /code-review); `None` while no import read the outbox, so no count is known and none is « nothing to send ».
pub fn to_send(drafts: &[Draft], sends: &BTreeMap<String, String>) -> Option<Vec<String>> {
    (drafts.iter().any(|d| !d.facts["to_send"].is_null())).then_some(())?;
    let mut go: Vec<(u64, &str)> = (drafts.iter())
        .filter(|d| d.facts["gone"] != true && crate::OPEN.contains(&d.status.as_str()))
        .filter(|d| {
            !matches!(
                sends.get(&d.id).map(String::as_str),
                Some("sent" | "sending" | "waiting")
            )
        })
        .filter_map(|d| Some((d.facts["to_send"].as_u64()?, d.id.as_str())))
        .collect();
    go.sort();
    Some(go.into_iter().map(|(_, id)| id.to_string()).collect())
}

/// The drafts the list shows, those the older app still holds, and what its note says of their read: its age, from the
/// newest import; `None` while none was imported, so no list and never « no draft waiting ».
fn listed(drafts: &[Draft], now_ms: i64) -> (Option<Vec<Draft>>, String) {
    let held: Vec<Draft> = drafts
        .iter()
        .filter(|d| d.facts["gone"] != true)
        .cloned()
        .collect();
    match held
        .iter()
        .filter_map(|d| d.facts["read_ms"].as_i64())
        .max()
    {
        None => (
            None,
            "Not read from the older app yet: the latest pass imports its drafts.".into(),
        ),
        Some(at) => (
            Some(held),
            format!("Read {} min ago.", (now_ms - at).max(0) / 60_000),
        ),
    }
}

/// `viewed`, holding draft `id` when the older app does: one it wrote since the last pass is read in once, by the binary's
/// key (/code-review of PR 1662: a 404 for up to five minutes).
fn viewed_holding(
    host: &dyn Host,
    id: &str,
) -> Result<(Vec<Draft>, BTreeMap<String, String>), String> {
    let read = viewed(host)?;
    if read.0.iter().any(|d| d.id == id) {
        return Ok(read);
    }
    match dash_sdk::context(host, "drafts/read-in", id) {
        Ok(_) => viewed(host),
        Err(_) => Ok(read),
    }
}

fn list_page(a: &Asked) -> Answer {
    let drawn = viewed(a.host).map(|(drafts, sends)| {
        let (held, note) = listed(&drafts, a.host.now_ms());
        match a.param("show").as_deref() {
            Some("to-send") => {
                let ids = held.as_deref().and_then(|h| to_send(h, &sends));
                let note = match ids {
                    None => {
                        format!("{note} Which drafts go now is not known: the outbox was not read.")
                    }
                    Some(_) => note,
                };
                let tree = list::to_send_tree(held.as_deref().zip(ids.as_deref()), &note);
                Drawing::Tree {
                    title: "Drafts to send".into(),
                    tree,
                }
            }
            _ => Drawing::Tree {
                title: "Drafts".into(),
                tree: list::tree(held.as_deref(), &sends, &note),
            },
        }
    });
    a.form(Some(drawn))
}

/// His open comments on `page`, through the comments plugin's key.
fn open_comments(host: &dyn Host, page: &str) -> Result<usize, String> {
    let lines = dash_sdk::context(host, "comments/open-on", page)?;
    let n = lines.first().ok_or("no count")?;
    n.parse().map_err(|e| format!("{n}: {e}"))
}

/// A draft opens on the drafts' page as the Gmail view of the one document (task rust-one-doc-component): the draft, then
/// every draft by stage.
fn draft_page(a: &Asked) -> Answer {
    let (id, editing) = (a.id(), a.param("edit").is_some());
    let read = viewed_holding(a.host, &id).and_then(|(drafts, sends)| {
        let Some(d) = drafts.iter().find(|d| d.id == id) else {
            return Ok(None);
        };
        let open = open_comments(a.host, &list::href(&id));
        let draft = list::draft_tree(d, open, editing, a.host.now_ms());
        let (held, _) = listed(&drafts, a.host.now_ms());
        let tree = list::joined(&draft, &list::tree(held.as_deref(), &sends, ""))?;
        Ok(Some(Drawing::Tree {
            title: format!("Draft {id}"),
            tree,
        }))
    });
    match read {
        Ok(None) => ("404 Not Found", TEXT, "no draft of that id".into()).into(),
        read => a.form(read.transpose()),
    }
}

/// /d/<id> lands on /drafts/<id>, its query kept: ?view=data still answers JSON. One this side does not hold stays the
/// older app's to answer (/code-review), never a page drawn here.
fn moved(a: &Asked) -> Answer {
    match viewed_holding(a.host, &a.id()) {
        Ok((drafts, _)) if drafts.iter().any(|d| d.id == a.id()) => {}
        Ok(_) => return ("404 Not Found", TEXT, "no draft of that id".into()).into(),
        Err(e) => return a.failed(e),
    }
    let to = match a.query {
        "" => list::href(&a.id()),
        q => format!("{}?{q}", list::href(&a.id())),
    };
    ("308 Permanent Redirect", TEXT, to).into()
}

// /drafts follows the drafts view: each diff (a send recorded, a draft kept) draws its list again (task
// drafts-genuinely-good, done-when 2)
inventory::submit! { dash_sdk::Follows { page: "/drafts", stream: "/drafts?view=patches", views: &["drafts"], live: list::LIVE } }
dash_sdk::submit! { Page { path: "/drafts", step: "route_drafts", reads: Reads::View, redraw: true, serve: list_page } }
dash_sdk::submit! { Page { path: "/drafts/*", step: "route_draft", reads: Reads::View, redraw: false, serve: draft_page } }
dash_sdk::submit! { Page { path: "/d/*", step: "route_draft_moved", reads: Reads::View, redraw: false, serve: moved } }

#[cfg(test)]
mod tests {
    use super::*;
    use dash_sdk::{dash_meter, dash_test};
    use serde_json::json;

    fn draft(id: &str, status: &str, facts: serde_json::Value) -> Draft {
        let j = json!({"id": id, "status": status, "versions": [], "facts": facts});
        crate::from_json(&j).expect("a draft")
    }

    /// /today's « N to send » and /drafts?show=to-send: a draft the pass placed is off once rejected, gone or going.
    #[dash_test]
    fn to_send_leaves_out_closed_gone_and_going_drafts() {
        let all = [
            draft("b", "approved", json!({"to_send": 1})),
            draft("a", "approved", json!({"to_send": 0})),
            draft("rejected", "rejected", json!({"to_send": 2})),
            draft("gone", "approved", json!({"to_send": 3, "gone": true})),
            draft("going", "approved", json!({"to_send": 4})),
            draft("later", "approved", json!({"to_send": false})),
        ];
        let sends = BTreeMap::from([("going".to_string(), "waiting".to_string())]);
        assert_eq!(to_send(&all, &sends), Some(vec!["a".into(), "b".into()]));
        assert_eq!(to_send(&all[5..], &sends), Some(vec![]));
        assert_eq!(to_send(&[draft("x", "approved", json!({}))], &sends), None);
    }
}
