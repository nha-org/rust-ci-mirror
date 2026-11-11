use super::parts::{joined, today};
use crate::component::{Component, Section};
use dioxus::prelude::*;

/// How long ago a day was, or how far ahead, from `today`: « today », « yesterday », « 3 days ago », « in 2 weeks ».
/// `None` when `date` starts with no day (2026-11-02).
pub fn ago(date: &str, today: i64) -> Option<String> {
    let n = today - dash_core::days(date)?;
    let (count, unit) = match n.abs() {
        0 => return Some("today".into()),
        1 => return Some(if n > 0 { "yesterday" } else { "tomorrow" }.into()),
        d if d < 14 => (d, "days"),
        d if d < 60 => (d / 7, "weeks"),
        d if d < 730 => (d / 30, "months"),
        d => (d / 365, "years"),
    };
    Some(if n > 0 {
        format!("{count} {unit} ago")
    } else {
        format!("in {count} {unit}")
    })
}

/// A day as he reads it first, « 3 days ago », its full date a tap away; its text form carries both (task
/// rust-timeline-component; @nha, 2026-11-10: « I like momentago style dates (plus the actual date, maybe click to toggle and
/// default to momentago) »). A `details` toggles with no script, in the window and on the web, a key reaching it too.
/// `date` starts with its day, a time after it kept in the full form; one that does not is shown as given.
#[component]
pub fn When(#[props(into)] date: String, today: i64) -> Element {
    match ago(&date, today) {
        Some(ago) => rsx! {
            details { class: "when",
                summary { span { "{ago}" } time { datetime: date.get(..10), "{date}" } }
            }
        },
        None => rsx! { time { class: "when", "{date}" } },
    }
}

inventory::submit! {
    Component {
        slug: "timeline",
        classes: &["timeline", "strip", "when"],
        tags: &[],
        sections: &[Section {
            family: "data",
            id: "timeline",
            title: "Timeline",
            after: "stats",
            rows: &[
                ("Three events", |_| {
                    let today = today();
                    let day = |n: i64| dash_core::date(today - n);
                    rsx! {
                        Timeline {
                            TimelineEvent { date: day(3), today, b { "Contract draft received" } div { class: "meta", "two clauses to check" } }
                            TimelineEvent { date: day(12), today, b { "Call with the CTO" } div { class: "meta", "rate agreed" } }
                            TimelineEvent { date: day(50), today, b { "Intro sent" } }
                        }
                    }
                }),
                ("A strip: a day's runs, then the next (/watchers?tab=automations)", |_| {
                    let today = today();
                    let day = dash_core::date(today);
                    let says = |what: &str, href: Option<&str>, detail: Option<&str>| rsx! {
                        EventSays { what: what.to_string(), href: href.map(str::to_string), detail: detail.map(str::to_string) }
                    };
                    rsx! {
                        Timeline { look: "strip",
                            TimelineEvent { date: format!("{day} 07:07 UTC"), today, {says("filed task morning-run", Some("/projects/site_perso?issue=morning-run"), None)} }
                            TimelineEvent { date: format!("{day} 07:22 UTC"), today, mark: Mark::Failed, {says("failed", None, Some("dash task update: timed out"))} }
                            TimelineEvent { date: "tomorrow at 08:07 local", today, mark: Mark::Next, {says("next run", None, None)} }
                        }
                    }
                }),
                ("A date, its full form a tap away", |_| rsx! { When { date: dash_core::date(today() - 3), today: today() } }),
                ("No event", |_| rsx! { div { class: "ds-empty", "No event yet." } }),
            ],
        }],
        ..Component::BARE
    }
}

/// Events in time, oldest or newest first as the page gives them: each a `TimelineEvent`, its dot on the line that joins them.
/// `look` « strip »: the events side by side on one line, left to right, an automation's runs then its next (task
/// rust-automations-tab; @nha, 2026-11-10: « I suggest making a tab with a visual for both of us, always dual-use »).
#[component]
pub fn Timeline(#[props(default)] look: &'static str, children: Element) -> Element {
    rsx! { ol { class: joined("timeline", look), {children} } }
}

/// One row of a `Timeline`: its date (`When`), then what happened. `mark`: its dot, done unless it failed or is still ahead.
#[component]
pub fn TimelineEvent(
    #[props(into)] date: String,
    today: i64,
    #[props(default)] mark: Mark,
    children: Element,
) -> Element {
    rsx! { li { class: mark.class(), When { date, today } div { {children} } } }
}

/// A `TimelineEvent`'s dot: done (filled), failed (the error colour) or next (hollow, still ahead).
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Mark {
    #[default]
    Done,
    Failed,
    Next,
}

impl Mark {
    /// Its class on the event, none for done, so a plain timeline's HTML is unchanged.
    pub fn class(self) -> Option<&'static str> {
        match self {
            Mark::Done => None,
            Mark::Failed => Some("failed"),
            Mark::Next => Some("next"),
        }
    }

    /// Its word in the text form.
    pub fn word(self) -> &'static str {
        match self {
            Mark::Done => "ok",
            Mark::Failed => "failed",
            Mark::Next => "next",
        }
    }
}

/// One event of a `timeline` tree element: when, what, its dot, and what it opens: a page (`href`, a run's task) or more
/// words unfolded in place (`detail`, a run's error).
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct Event {
    pub at: String,
    pub what: String,
    #[serde(default)]
    pub mark: Mark,
    #[serde(default)]
    pub href: Option<String>,
    #[serde(default)]
    pub detail: Option<String>,
}

/// What a `TimelineEvent` says and opens: its link, its words with the detail a tap away, or its words alone.
#[component]
pub fn EventSays(what: String, href: Option<String>, detail: Option<String>) -> Element {
    match (href, detail) {
        (Some(href), _) => rsx! { a { href, "{what}" } },
        (None, Some(detail)) => rsx! { details { summary { "{what}" } pre { "{detail}" } } },
        (None, None) => rsx! { span { "{what}" } },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::memory::TestBudget;

    /// Done-when 2 and 3 of rust-timeline-component: a day reads « 3 days ago », its full date on a tap (the details opens); the
    /// text form carries both, so an agent reads the exact day.
    #[test]
    fn a_date_reads_its_age_first_and_carries_its_full_date() {
        let _budget = TestBudget::new();
        let today = dash_core::days("2026-11-10").expect("a day");
        let html = dioxus_ssr::render_element(rsx! { When { date: "2026-11-07", today } });
        assert_eq!(
            html,
            r#"<details class="when"><summary><span>3 days ago</span><time datetime="2026-11-07">2026-11-07</time></summary></details>"#
        );
        // the text form carries both, the age then the full date, whichever the screen shows
        assert_eq!(crate::forms::text_of(&html), "3 days ago\t2026-11-07\n");
        // the toggle is the details' `open`: closed shows the age alone, open the full date alone (the browser test clicks it)
        assert!(
            crate::component::css()
                .contains(".when:not([open]) time, .when[open] span { display: none; }"),
            "{}",
            crate::component::css()
        );
        let opened = html.replacen(
            r#"<details class="when">"#,
            r#"<details class="when" open="">"#,
            1,
        );
        assert_eq!(
            crate::forms::text_of(&opened),
            "3 days ago\t2026-11-07\n",
            "open, its text still carries both"
        );
        let odd = dioxus_ssr::render_element(rsx! { When { date: "soon", today } });
        assert_eq!(odd, r#"<time class="when">soon</time>"#);
        let said: Vec<_> = [
            "2026-11-10",
            "2026-11-09",
            "2026-11-11",
            "2026-10-31",
            "2026-10-11",
            "2026-01-10",
            "2023-11-10",
            "2026-11-24 10:00 UTC",
            "11/10",
        ]
        .iter()
        .map(|d| ago(d, today))
        .collect();
        assert_eq!(
            said,
            [
                Some("today"),
                Some("yesterday"),
                Some("tomorrow"),
                Some("10 days ago"),
                Some("4 weeks ago"),
                Some("10 months ago"),
                Some("3 years ago"),
                Some("in 2 weeks"),
                None
            ]
            .map(|s| s.map(String::from))
        );
    }

    /// Done-when 1 and 3 of rust-timeline-component: a row a event, its date then its words; the dot and the line are its style.
    #[test]
    fn a_timeline_is_a_row_an_event_its_date_then_what_happened() {
        let _budget = TestBudget::new();
        let today = dash_core::days("2026-11-10").expect("a day");
        let html = dioxus_ssr::render_element(rsx! {
            Timeline {
                TimelineEvent { date: "2026-11-07", today, "Her reply" }
                TimelineEvent { date: "2026-11-02 09:00 UTC", today, "First message" }
            }
        });
        assert_eq!(
            html,
            r#"<ol class="timeline"><li><details class="when"><summary><span>3 days ago</span><time datetime="2026-11-07">2026-11-07</time></summary></details><div>Her reply</div></li><li><details class="when"><summary><span>8 days ago</span><time datetime="2026-11-02">2026-11-02 09:00 UTC</time></summary></details><div>First message</div></li></ol>"#
        );
        assert_eq!(
            crate::forms::text_of(&html),
            "3 days ago\t2026-11-07\nHer reply\n8 days ago\t2026-11-02 09:00 UTC\nFirst message\n"
        );
    }
}
