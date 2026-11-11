//! /today, the daily's page (task rust-daily-today, rfc-rust-plugins-ui section 2): one line per plugin, each a count that opens
//! its list. It reads the « today » slot's lines from the registry, never a plugin by name, so a plugin removed takes its line
//! with it.
use dash_core::registry::Registry;
use dash_surface as surface;
use serde_json::{Map, json};

/// The page's tree, and whether every line has a count: an agent must never read a line still reading as none. His comments
/// on /today sit on the words they quote.
pub fn page(registry: &Registry) -> (surface::Drawing, bool) {
    let anchors = crate::comments::threads_on("/today")
        .into_iter()
        .map(|t| surface::documents::Anchor {
            quote: t.quote,
            thread: t.id,
        })
        .collect();
    let (tree, read) = tree(registry, anchors);
    let title = "Today".into();
    (surface::Drawing::Tree { title, tree }, read)
}

/// /today is a document of the one model, drawn by the one component in its Notion view (task rust-one-doc-component; @nha,
/// 2026-11-09: « this should be a notion-like doc. which allows comments naturally. »): its title, then an item a line, its
/// count opening its list and what it counts under it. A line still reading is the design system's loading state under the
/// id its count fills (@nha, 2026-11-09: « This is a card loading state. same element. »).
fn tree(registry: &Registry, anchors: Vec<surface::documents::Anchor>) -> (String, bool) {
    let slot = registry.slot_lines("today");
    let mut read = true;
    let said: Vec<(String, String, bool)> = (slot.iter().enumerate())
        .map(|(i, (origin, line))| {
            let (says, reading) = match &line.says {
                Ok(Some(says)) => (says.clone(), false),
                Ok(None) => (String::new(), true),
                Err(e) => (format!("{origin}: {e}"), false),
            };
            read &= !reading;
            (format!("line-{i}"), says, reading)
        })
        .collect();
    let lines: Vec<surface::documents::Line> = (said.iter().zip(&slot))
        .map(
            |((id, says, reading), (_, line))| surface::documents::Line {
                id,
                says,
                what: &line.what,
                href: Some(line.href.as_str()).filter(|h| !h.is_empty()),
                reading: *reading,
            },
        )
        .collect();
    let mut elements = Map::new();
    let root = if lines.is_empty() {
        let none = "No plugin has a line for today.";
        elements.insert(
            "title".into(),
            json!({"type": "heading", "props": {"text": "Today"}}),
        );
        elements.insert(
            "none".into(),
            json!({"type": "text", "props": {"text": none}}),
        );
        json!({"type": "stack", "children": ["title", "none"]})
    } else {
        let doc = surface::documents::outline("Today", &lines);
        elements.insert(
            "today".into(),
            json!({"type": "doc-view", "props": {"doc": doc, "anchors": anchors}}),
        );
        json!({"type": "stack", "children": ["today"]})
    };
    elements.insert("page".into(), root);
    (
        json!({"root": "page", "elements": elements}).to_string(),
        read,
    )
}

// the daily's page reads the registry's « today » slot, so it routes on the serial thread; each line reads its plugin's last
// count, never a source, so that thread never waits on one
inventory::submit! {
    crate::pages::Page { path: "/today", step: "route_today", reads: crate::pages::Reads::Registry, redraw: false, serve: |a| {
        let Some(registry) = a.registry else {
            return ("404 Not Found", crate::pages::TEXT, "no registry in this process".into());
        };
        let (drawing, read) = page(registry);
        if !read && !a.html {
            let why = "reading a line of today: ask again in a few seconds";
            return ("202 Accepted", crate::pages::TEXT, why.into());
        }
        a.form(Some(Ok(drawing)))
    } }
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
        let (tree, read) = tree(registry, vec![]);
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
                "Today\n3 to send\t/drafts?show=to-send\ndrafts approved, not yet sent\n2 replies waiting\t/mail\nunread answers in his inbox\n".into(),
                true
            )
        );
        let html = surface::screen::page("Today", &tree(&both, vec![]).0).expect("html");
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
        assert_eq!(
            said,
            "Today\nbeing read\ndrafts approved, not yet sent\nmail: no password\t/mail\nunread answers in his inbox\n"
        );
    }

    /// The element drawn under `id` in the page's HTML: its opening tag's name and attributes, its brackets left out (the dash
    /// crate holds no HTML, dash-gate's the_dash_crate_holds_no_html).
    fn tag<'a>(html: &'a str, id: &str) -> &'a str {
        let at = html.find(&format!(r#"id="{id}""#)).expect(id);
        let start = html[..at].rfind('<').expect("a tag") + 1;
        &html[start..at + html[at..].find('>').expect("its end")]
    }

    /// Done-when 5 of rust-one-doc-component: /today is the one document drawn by the one component, and his comment sits on
    /// the words it quotes, tied to its thread on the rail; a quote no line holds any more marks nothing.
    #[test]
    fn today_is_a_view_of_the_document_its_comments_on_its_words() {
        let _budget = TestBudget::new();
        let mut r = Registry::default();
        r.add("drafts", drafts(Some(3))).unwrap();
        let anchors = vec![
            surface::documents::Anchor {
                quote: "3 to send".into(),
                thread: "c-a1".into(),
            },
            surface::documents::Anchor {
                quote: "9 to send".into(),
                thread: "c-gone".into(),
            },
        ];
        let (tree, _) = tree(&r, anchors);
        assert!(tree.contains(r#""type":"doc-view""#), "{tree}");
        let html = surface::screen::page("Today", &tree).expect("html");
        // the markup is the component's, pinned by dash-surface's an_outline_draws_links_loading_lines_and_comments_on_its_words
        assert!(html.contains(r#"class="dv notion""#), "{html}");
        assert!(
            html.contains(r#"href="/drafts?show=to-send""#)
                && html.contains(r#"class="dv-anchor" data-thread="c-a1">3 to send"#),
            "{html}"
        );
        assert!(!html.contains("c-gone"), "{html}");
    }

    /// Done-when 1 of rust-today-doc: a line still reading is the design system's card in its loading state, under the id its
    /// count fills, and the page carries the refill loop that swaps one for the other; never a line of text.
    #[test]
    fn a_reading_line_is_the_card_in_its_loading_state() {
        let _budget = TestBudget::new();
        let page = |n: Option<usize>| {
            let mut r = Registry::default();
            r.add("drafts", drafts(n)).unwrap();
            surface::screen::page("Today", &tree(&r, vec![]).0).expect("html")
        };
        let (reading, counted) = (page(None), page(Some(3)));
        assert_eq!(
            tag(&reading, "line-0"),
            r#"div id="line-0" class="dv-item skeleton" aria-busy="true" role="status" aria-label="Being read""#
        );
        assert!(
            reading.contains("drafts approved, not yet sent") && reading.contains("dashLive"),
            "what it counts shows while it reads, and the page refills: {reading}"
        );
        assert!(
            !reading.contains("reading."),
            "never a line of text: {reading}"
        );
        assert!(
            tag(&counted, "line-0") == r#"div class="dv-item" id="line-0""#,
            "{counted}"
        );
        assert!(!counted.contains("dashLive"), "nothing left to refill");
    }

    /// Done-when 2 of rust-today-doc: from a cold start, each line ends its read inside 20 s, a count or its error: a read that
    /// never answers is its error `fresh::SLOW` seconds in, never « reading » for good.
    #[test]
    fn each_line_ends_its_read_within_20_s_of_a_cold_start() {
        use crate::fresh::{self, Read};
        use std::sync::Mutex;
        use std::sync::atomic::{AtomicU64, Ordering};
        use std::time::Duration;
        let _budget = TestBudget::new();
        const { assert!(fresh::SLOW < 20) };
        static CLOCK: AtomicU64 = AtomicU64::new(1000);
        static DRAFTS: Mutex<Read<usize>> = Mutex::new(Read::new());
        static MAIL: Mutex<Read<u64>> = Mutex::new(Read::new());
        let now = || CLOCK.load(Ordering::SeqCst);
        let to_send = move || Ok(fresh::bounded(&DRAFTS, now(), "drafts", || 3)?.map(|r| r.0));
        let hung = || {
            std::thread::sleep(Duration::from_secs(600));
            1
        };
        let replies = move || Ok(fresh::bounded(&MAIL, now(), "mail", hung)?.map(|r| r.0));
        let mut r = Registry::default();
        r.add(
            "drafts",
            Contribution {
                slots: vec![(
                    crate::drafts::SLOT.into(),
                    Box::new(dash_drafts::outbox::ToSend(to_send)) as Box<dyn Slot>,
                )],
                ..Default::default()
            },
        )
        .unwrap();
        r.add(
            "mail",
            Contribution {
                slots: vec![(
                    crate::mail::SLOT.into(),
                    Box::new(dash_mail::RepliesWaiting(replies)) as Box<dyn Slot>,
                )],
                ..Default::default()
            },
        )
        .unwrap();

        let (cold, read) = text(&r);
        assert!(
            !read
                && cold
                    == "Today\nbeing read\ndrafts approved, not yet sent\nbeing read\nunread answers in his inbox\n",
            "{cold}"
        );
        fresh::waited(&DRAFTS, Duration::from_secs(5)).expect("the drafts read");
        CLOCK.store(1000 + fresh::SLOW, Ordering::SeqCst);
        let (ended, read) = text(&r);
        assert!(read, "no line reads past {} s: {ended}", fresh::SLOW);
        assert_eq!(
            ended,
            format!(
                "Today\n3 to send\t/drafts?show=to-send\ndrafts approved, not yet sent\nmail: no count {} s after the first read\t/mail\nunread answers in his inbox\n",
                fresh::SLOW
            )
        );
    }
}
