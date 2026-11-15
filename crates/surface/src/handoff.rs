//! /handoff, what waits on him, drawn from the parts `dash_store::older_handoff::report` reads (task rust-handoff-port):
//! the older app's « For you » card, first on its home, in the catalog's elements. One tree, its HTML for him and its text
//! for agents (rust/CLAUDE.md rule 4); its tabs are links to its parts, each a list of cards.
use crate::Drawing;
use serde_json::{Map, Value, json};

/// The page's parts, each its anchor, its tab's name and the key of its items in the report's parts.
const TABS: [(&str, &str, &str); 4] = [
    ("sandbox", "Sandbox", "sandbox"),
    ("drafts", "Drafts", "drafts"),
    ("tasks", "Tasks and answers", "tasks"),
    ("later", "Later", "later"),
];

fn items(parts: &Value, key: &str) -> Vec<Value> {
    parts[key].as_array().cloned().unwrap_or_default()
}

/// /handoff from `parts`: its count and what to read first, its tabs with their counts, each part's cards, the notes
/// (handoff.md as written, its check lines out) and the rules a step breaks.
pub fn page(parts: &Value) -> Drawing {
    let mut el = Map::new();
    let put = |el: &mut Map<String, Value>, key: String, v: Value| {
        el.insert(key.clone(), v);
        key
    };
    let count = parts["count"].as_u64().unwrap_or_default();
    let mut lead = vec![
        json!({"name": format!("Inbox: everything that needs you · {count}"), "href": "/inbox"}),
    ];
    if let Some(label) = parts["read"]["label"].as_str() {
        let href = parts["read"]["href"].as_str().unwrap_or("/insights#read");
        lead.push(json!({"name": label, "href": href}));
    }
    let lead = put(
        &mut el,
        "lead".into(),
        json!({"type": "links", "props": {"links": lead}}),
    );
    let notes = parts["notes"].as_str().unwrap_or_default();
    let mut tabs: Vec<Value> = TABS
        .iter()
        .map(|(anchor, name, key)| json!({"name": format!("{name} · {}", items(parts, key).len()), "href": format!("#{anchor}")}))
        .collect();
    tabs.push(json!({"name": format!("Notes · {}", usize::from(!notes.trim().is_empty())), "href": "#notes"}));
    let names: Vec<&str> = TABS.iter().map(|(_, n, _)| *n).chain(["Notes"]).collect();
    let tabs = put(
        &mut el,
        "tabs".into(),
        json!({"type": "links", "props": {"links": tabs}}),
    );
    let mut inside = vec![tabs];
    for (anchor, name, key) in TABS {
        let cards: Vec<String> = items(parts, key)
            .iter()
            .enumerate()
            .map(|(i, it)| {
                let card = json!({"type": "card", "props": {
                    "title": it["title"].as_str().unwrap_or_default(),
                    "line": it["line"].as_str().unwrap_or_default(),
                    "href": it["href"]}});
                put(&mut el, format!("{anchor}-{i}"), card)
            })
            .collect();
        let body = match cards.is_empty() {
            true => put(
                &mut el,
                format!("{anchor}-none"),
                json!({"type": "text", "props": {"text": "Nothing waits on you here."}}),
            ),
            false => put(
                &mut el,
                format!("{anchor}-list"),
                json!({"type": "list", "children": cards}),
            ),
        };
        let part = json!({"type": "section", "props": {"title": name}, "children": [body]});
        inside.push(put(&mut el, anchor.into(), part));
    }
    let quote = put(
        &mut el,
        "notes-text".into(),
        json!({"type": "quote", "props": {"text": notes}}),
    );
    inside.push(put(
        &mut el,
        "notes".into(),
        json!({"type": "section", "props": {"title": "Notes"}, "children": [quote]}),
    ));
    let rules: Vec<String> = parts["rules"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .enumerate()
        .map(|(i, r)| {
            put(
                &mut el,
                format!("rule-{i}"),
                json!({"type": "text", "props": {"text": r}}),
            )
        })
        .collect();
    if !rules.is_empty() {
        let part = json!({"type": "section", "props": {"title": "Rules a step breaks"}, "children": rules});
        inside.push(put(&mut el, "rules".into(), part));
    }
    let all = json!({"type": "section", "props": {"title": names.join(", ")}, "children": inside});
    let all = put(&mut el, "parts".into(), all);
    let top = json!({"type": "section", "props": {"title": "For you"}, "children": [lead, all]});
    let top = put(&mut el, "for-you".into(), top);
    let title = put(
        &mut el,
        "title".into(),
        json!({"type": "heading", "props": {"text": "Handoff"}}),
    );
    el.insert(
        "page".into(),
        json!({"type": "stack", "children": [title, top]}),
    );
    Drawing::Tree {
        title: "Handoff".into(),
        tree: json!({"root": "page", "elements": el}).to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;

    /// Task rust-handoff-port, done-when 1's page half: the report's parts drawn as one tree, its HTML and its text each
    /// naming the count, every tab, a draft and its link, the sandbox's approval and the notes.
    #[dash_test]
    fn the_handoff_parts_draw_in_both_forms() {
        let parts = json!({
            "count": 3,
            "read": {"label": "Week 45, 2 November to 8 November.", "href": "/insights#read"},
            "sandbox": [{"title": "Yes, approve the host change", "line": "sandbox.sb.candidate", "href": "/approve"}],
            "drafts": [{"title": "acme-intro to Ana", "line": "new: fresh", "href": "/drafts/acme-intro"}],
            "tasks": [], "later": [{"title": "Crée le certificat", "line": "from 2027-01-04", "href": null}],
            "notes": "# Passation\n- [ ] **Crée le certificat**", "rules": ["ERROR: handoff.md line 2: no :task"],
        });
        let Drawing::Tree { tree, .. } = page(&parts) else {
            panic!("a tree")
        };
        let text = crate::text(&tree).expect("its text");
        let html = crate::screen::page("Handoff", &tree).expect("its HTML");
        for words in [
            "Inbox: everything that needs you · 3",
            "Week 45, 2 November to 8 November.",
            "Sandbox · 1",
            "Drafts · 1",
            "Tasks and answers · 0",
            "Later · 1",
            "Notes · 1",
            "Yes, approve the host change",
            "acme-intro to Ana",
            "Nothing waits on you here.",
            "Rules a step breaks",
        ] {
            assert!(text.contains(words), "text lacks « {words} »:\n{text}");
            assert!(html.contains(words), "HTML lacks « {words} »");
        }
        assert!(
            html.contains("/drafts/acme-intro") && html.contains("/approve"),
            "{html}"
        );
    }
}
