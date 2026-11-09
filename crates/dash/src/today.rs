//! /today, the daily's page (task rust-daily-today, rfc-rust-plugins-ui section 2): one line per plugin, each a count that opens
//! its list. It reads the « today » slot's lines from the registry, never a plugin by name, so a plugin removed takes its line
//! with it.
use dash_core::registry::Registry;
use dash_surface as surface;
use serde_json::{Map, json};

/// The page's tree, and whether every line has a count: an agent must never read a line still reading as none.
pub fn page(registry: &Registry) -> (surface::Drawing, bool) {
    let (tree, read) = tree(registry);
    let title = "Today".into();
    (surface::Drawing::Tree { title, tree }, read)
}

fn tree(registry: &Registry) -> (String, bool) {
    let mut elements = Map::new();
    let text = |t: &str| json!({"type": "text", "props": {"text": t}});
    elements.insert(
        "title".into(),
        json!({"type": "heading", "props": {"text": "Today"}}),
    );
    let (mut lines, mut said, mut read) = (vec![], vec!["title".to_string()], true);
    for (i, (origin, line)) in registry.slot_lines("today").into_iter().enumerate() {
        let name = format!("line-{i}");
        match line {
            Ok(Some(l)) => {
                let card = json!({"type": "card", "props": {"title": l.says, "line": l.what, "href": l.href}});
                elements.insert(name.clone(), card);
                lines.push(name);
            }
            Ok(None) => {
                read = false;
                elements.insert(name.clone(), text(&format!("{origin}: reading.")));
                said.push(name);
            }
            Err(e) => {
                elements.insert(name.clone(), text(&format!("{origin}: {e}")));
                said.push(name);
            }
        }
    }
    let mut children = said;
    if !lines.is_empty() {
        elements.insert("lines".into(), json!({"type": "cards", "children": lines}));
        children.insert(1, "lines".into());
    }
    if children.len() == 1 {
        elements.insert("none".into(), text("No plugin has a line for today."));
        children.push("none".into());
    }
    elements.insert(
        "page".into(),
        json!({"type": "stack", "children": children}),
    );
    (
        json!({"root": "page", "elements": elements}).to_string(),
        read,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_core::registry::{Contribution, Slot};
    use dash_meter::memory::TestBudget;

    fn drafts(n: Option<usize>) -> Contribution {
        let line: Box<dyn Slot> = Box::new(dash_drafts::outbox::ToSend(move || Ok(n)));
        Contribution {
            slots: vec![(crate::drafts::SLOT.into(), line)],
            ..Default::default()
        }
    }

    fn mail(n: Result<Option<u64>, String>) -> Contribution {
        let line: Box<dyn Slot> = Box::new(dash_mail::RepliesWaiting(move || n.clone()));
        Contribution {
            slots: vec![(crate::mail::SLOT.into(), line)],
            ..Default::default()
        }
    }

    fn text(registry: &Registry) -> (String, bool) {
        let (tree, read) = tree(registry);
        (surface::text(&tree).expect("text"), read)
    }

    /// Done-when 1 and 4 of rust-daily-today: drafts and mail each register their line; /today reads both, and each alone with
    /// the other plugin removed; both forms from one tree.
    #[test]
    fn today_reads_each_plugin_s_line_and_none_of_a_plugin_removed() {
        let _budget = TestBudget::new();
        // mail registers first: drafts to send still come first, by their keys (the task's title: « drafts to send first »)
        let mut both = Registry::default();
        both.add("mail", mail(Ok(Some(2)))).unwrap();
        both.add("drafts", drafts(Some(3))).unwrap();
        assert_eq!(
            text(&both),
            (
                "Today\n3 to send\tdrafts approved, not yet sent\t/drafts?show=to-send\n2 replies waiting\tunread answers in his inbox\t/mail\n".into(),
                true
            )
        );
        let html = surface::screen::page("Today", &tree(&both).0).expect("html");
        assert!(
            html.contains(r#"href="/drafts?show=to-send""#) && html.contains("3 to send"),
            "{html}"
        );

        let mut no_mail = Registry::default();
        no_mail.add("drafts", drafts(Some(1))).unwrap();
        let (only, _) = text(&no_mail);
        assert!(
            only.contains("1 to send") && !only.contains("repl"),
            "{only}"
        );

        let mut no_drafts = Registry::default();
        no_drafts.add("mail", mail(Ok(Some(1)))).unwrap();
        let (only, _) = text(&no_drafts);
        assert!(
            only.contains("1 reply waiting") && !only.contains("to send"),
            "{only}"
        );

        let (none, read) = text(&Registry::default());
        assert_eq!(
            (none.as_str(), read),
            ("Today\nNo plugin has a line for today.\n", true)
        );
    }

    /// A line still reading is said, and the page is not read yet; a failed one names its plugin and leaves the others.
    #[test]
    fn a_line_reading_or_failing_is_said_never_a_zero() {
        let _budget = TestBudget::new();
        let mut r = Registry::default();
        r.add("drafts", drafts(None)).unwrap();
        r.add("mail", mail(Err("no password".into()))).unwrap();
        let (said, read) = text(&r);
        assert!(!read && !said.contains(" 0 "), "{said}");
        assert_eq!(said, "Today\ndrafts: reading.\nmail: no password\n");
    }
}
