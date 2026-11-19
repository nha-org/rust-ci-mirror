use super::avatar::Avatar;
use super::badge::Badge;
use super::parts::{CROSS, DOWN, LINK, UP, icon, joined, status};
use crate::component::Component;
use dioxus::prelude::*;

inventory::submit! {
    Component {
        slug: "issue",
        classes: &["li-issue", "li-crumbs", "li-crumb-title", "li-crumb-key", "li-position", "li-step", "li-tools", "li-issue-main", "li-issue-title", "li-issue-side", "li-side-head", "li-prop", "li-part", "li-part-head", "li-check", "li-event", "li-dot", "li-pr"],
        tags: &[],
        name: "Issue",
        used: "A task's page, Linear's issue view (/tasks/<id>, a task brief): the way back and its key, its title and parts \
               (IssuePart: Done when, its IssueCheck lines; Activity, its IssueEvent lines), its properties beside them \
               (IssueProp a line, IssueHead a heading, IssuePriority, IssuePrs), its key and steps in its bar (IssueKey, IssueStep).",
        example: Some(|_| specimen()),
        terminal: Some(crate::component::Terminal::Words),
        ..Component::BARE
    }
}

/// The component's example, /design's « Issue page » row too (design::families).
pub(crate) fn specimen() -> Element {
    rsx! {
        Issue { title: "Port the queue page", says: "running, try 2",
            bar: rsx! { IssueKey { id: "AAA-12", place: "4 / 12".to_string() } },
            state: rsx! { IssueProp { {status("started")} span { "In Progress" } } },
            side: rsx! { IssuePrs { links: vec![("https://github.com/o/r/pull/96".to_string(), "96".to_string())] } },
            p { "Why it matters, in his words." }
            IssuePart { title: "Done when: 1 of 3 met",
                IssueCheck { met: Some(true), word: "met", text: "The page lists the queue: a screenshot" }
                IssueCheck { met: Some(false), word: "not met", text: "A test holds it" }
                IssueCheck { met: None, word: "not judged", text: "The old link opens it" }
            }
            IssuePart { title: "Activity",
                IssueEvent { what: "Started, try 1", when: "2026-11-08 09:12 UTC" }
                IssueEvent { what: "Finished: failed", when: "2026-11-08 10:40 UTC" }
            }
        }
    }
}

/// A task's page, Linear's issue view as the older app's issue page lays it out (task rust-task-page-linear; @nha, 2026-11-09:
/// « the tasks page need to match the linear design »): the way back and its key, then its title and parts (its children),
/// its properties beside them. `says`: its state in words; `side`: what the properties hold past the status.
/// `bar`: what sits at the right end of its way back (its key, its place among the board's issues); `tools`: Linear's round copy
/// buttons under it; `state`: its status, a menu where it is his to set (task rust-board-buttons-new-issue).
#[component]
pub fn Issue(
    #[props(into)] title: String,
    #[props(into, default)] says: String,
    bar: Element,
    #[props(default = rsx! {})] tools: Element,
    state: Element,
    side: Element,
    children: Element,
) -> Element {
    rsx! {
        div { class: "li-issue",
            nav { class: "li-crumbs", a { href: "/tasks", "Tasks" } span { class: "meta", "›" } span { class: "li-crumb-title", "{title}" }
                {bar}
            }
            div { class: "li-tools", {tools} }
            div { class: "li-issue-main",
                h1 { class: "li-issue-title", "{title}" }
                {children}
            }
            aside { class: "li-issue-side",
                IssueHead { "Properties" }
                {state}
                if !says.is_empty() {
                    IssueProp { look: "meta", "{says}" }
                }
                {side}
            }
        }
    }
}

/// An issue as a line of a list on /design's board tabs: its key, status, title, label and assignee. `look`: a state the
/// page forces to show it (« on » selected, « off » dimmed, « is-hover »). A task's row on /tasks is Item's `issue-row`.
#[component]
pub fn IssueLine(
    kind: &'static str,
    #[props(default)] look: &'static str,
    title: &'static str,
) -> Element {
    rsx! {
        div { class: "ds-issue {look}",
            span { class: "meta", "AAA-12" }
            {status(kind)}
            span { class: "ds-grow", "{title}" }
            Badge { look: "tone-outline", "rust" }
            Avatar { initials: "CL" }
        }
    }
}

/// An issue's key at the right end of its way back, then its place among the board's issues, as Linear's « 42 / 2193 ».
#[component]
pub fn IssueKey(#[props(into)] id: String, #[props(default)] place: Option<String>) -> Element {
    rsx! {
        span { class: "meta li-crumb-key", "{id}" }
        if let Some(place) = place {
            span { class: "meta li-position", "{place}" }
        }
    }
}

/// A step to the next or the previous issue, after an IssueKey's place. Always in place: with nowhere to go it is greyed
/// out and disabled, so a move to the first or last card shifts nothing (task rust-task-page-edits; @nha, 2026-11-11:
/// « changing from todo -> backlog causes a dissappear of the up arrow which moves the content. I suggest greying it out
/// instead »).
#[component]
pub fn IssueStep(to: Option<String>, next: bool) -> Element {
    let (label, path) = match next {
        true => ("Next issue", DOWN),
        false => ("Previous issue", UP),
    };
    // with no href an `a` is no link: its role keeps it one, so a screen reader says « Previous issue, dimmed » (/code-review)
    let (disabled, role, said) = match &to {
        Some(href) => (None, None, href.clone()),
        None => (Some("true"), Some("link"), "none".to_string()),
    };
    rsx! { a { class: "li-step", href: to, role, title: label, "aria-label": label, "aria-disabled": disabled,
    span { class: "text-only", "{label}: {said}" } {icon(path)} } }
}

/// A line among an issue's properties: an icon and its words. `title`: what it is, on hover; `look`: « meta » for a note.
#[component]
pub fn IssueProp(
    #[props(default)] title: Option<String>,
    #[props(default)] look: &'static str,
    children: Element,
) -> Element {
    rsx! { div { class: joined("li-prop", look), title, {children} } }
}

/// A heading among an issue's properties: Properties, Labels, Project.
#[component]
pub fn IssueHead(children: Element) -> Element {
    rsx! { h3 { class: "li-side-head", {children} } }
}

/// An issue's priority as Linear's sidebar shows it, the inside of its property: Urgent its « ! » box, High to Low three
/// bars filled to its level, then its name (board_live.ex's issue page; task rust-board-buttons-new-issue). A level past 4 reads as none set.
#[component]
pub fn IssuePriority(level: u8) -> Element {
    let name = priority_name(level);
    // High fills three bars, Medium two, Low one, none set none (/code-review of PR 1395: 0 drew High's three)
    let filled = match level {
        2 => 3,
        3 => 2,
        4 => 1,
        _ => 0,
    };
    let mark = match level {
        1 => rsx! { span { class: "li-urgent", "aria-hidden": "true", "!" } },
        _ => rsx! {
            svg { class: "ic", width: "16", height: "16", view_box: "0 0 16 16", "aria-hidden": "true",
                for (i, (x, h)) in [(2, 6), (7, 9), (12, 12)].into_iter().enumerate() {
                    rect { x: "{x}", y: "{14 - h}", width: "3", height: "{h}", rx: "1", fill: "currentColor",
                        opacity: if i < filled { "1" } else { "0.3" } }
                }
            }
        },
    };
    rsx! { {mark} span { "{name}" } }
}

/// A priority's name, 1 Urgent to 4 Low; any other none set.
pub fn priority_name(level: u8) -> &'static str {
    match level {
        1 => "Urgent",
        2 => "High",
        3 => "Medium",
        4 => "Low",
        _ => "No priority",
    }
}

/// A titled part of an issue: Done when, Activity.
#[component]
pub fn IssuePart(#[props(into)] title: String, children: Element) -> Element {
    rsx! {
        section { class: "li-part",
            h2 { class: "li-part-head", "{title}" }
            {children}
        }
    }
}

/// One check of a done-when: met a done circle, not met a cross, not judged an empty circle. `word`: its verdict in words.
#[component]
pub fn IssueCheck(
    met: Option<bool>,
    #[props(into)] word: String,
    #[props(into)] text: String,
) -> Element {
    let mark = match met {
        Some(true) => status("done"),
        Some(false) => rsx! { span { class: "tx-error", {icon(CROSS)} } },
        None => status("todo"),
    };
    let class = match met {
        Some(true) => "li-check met",
        Some(false) => "li-check unmet",
        None => "li-check",
    };
    // its verdict in words for the text form alone: the mark says it on screen (components.css .text-only)
    rsx! { div { class, title: "{word}", {mark} span { class: "text-only", "{word}:" } span { "{text}" } } }
}

/// A line of an issue's Activity: a dot, what happened, when; its text `what\twhen`, the time an item (forms::ITEMS).
#[component]
pub fn IssueEvent(#[props(into)] what: String, #[props(into)] when: String) -> Element {
    rsx! { div { class: "li-event", span { class: "li-dot" } b { "{what}" } time { class: "meta", "{when}" } } }
}

/// An issue's pull requests among its properties: each (its URL, its number), the link GitHub's.
#[component]
pub fn IssuePrs(links: Vec<(String, String)>) -> Element {
    rsx! {
        if !links.is_empty() {
            IssueHead { "Pull requests" }
        }
        for (url, number) in links {
            a { class: "li-tag li-pr", href: "{url}", {icon(LINK)} "#{number}" }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;

    /// Task untangle-one-editor, done-when 3: the issue page and its parts are components of the design system, each
    /// drawing what `families::issue*` drew, and registered, so dash-gate refuses a page that writes their classes itself.
    #[dash_test]
    fn the_issue_page_and_its_parts_are_components() {
        let html = dioxus_ssr::render_element(specimen());
        for part in [
            r#"<div class="li-issue"><nav class="li-crumbs"><a href="/tasks">Tasks</a><span class="meta">›</span><span class="li-crumb-title">Port the queue page</span>"#,
            r#"<div class="li-tools"></div><div class="li-issue-main"><h1 class="li-issue-title">Port the queue page</h1><p>Why it matters, in his words.</p><section class="li-part"><h2 class="li-part-head">Done when: 1 of 3 met</h2><div class="li-check met" title="met">"#,
            r#"<div class="li-check unmet" title="not met"><span class="tx-error">"#,
            r#"<span class="text-only">not judged:</span><span>The old link opens it</span></div></section>"#,
            r#"<div class="li-event"><span class="li-dot"></span><b>Started, try 1</b><time class="meta">2026-11-08 09:12 UTC</time></div>"#,
            r#"<aside class="li-issue-side"><h3 class="li-side-head">Properties</h3><div class="li-prop">"#,
            r#"<div class="li-prop meta">running, try 2</div><h3 class="li-side-head">Pull requests</h3><a class="li-tag li-pr" href="https://github.com/o/r/pull/96">"#,
        ] {
            assert!(html.contains(part), "{part}\nnot in\n{html}");
        }
        let priority = |level| dioxus_ssr::render_element(rsx! { IssuePriority { level } });
        assert!(priority(1).ends_with(
            r#"<span class="li-urgent" aria-hidden="true">!</span><span>Urgent</span>"#
        ));
        assert!(priority(0).ends_with("<span>No priority</span>"));
        let registered = crate::component::registered()
            .iter()
            .find(|c| c.slug == "issue")
            .expect("registered");
        assert!(registered.classes.contains(&"li-issue") && registered.example.is_some());
        // the functions are gone from the design page's module, whatever their visibility: it draws the components
        let families = include_str!("../../design/families.rs");
        let back: Vec<&str> = (families.lines())
            .filter(|l| l.contains("fn issue"))
            .collect();
        let line = dioxus_ssr::render_element(
            rsx! { IssueLine { kind: "review", look: "on", title: "The CLI, selected" } },
        );
        assert!(
            line.starts_with(r#"<div class="ds-issue on"><span class="meta">AAA-12</span>"#),
            "{line}"
        );
        assert!(back.is_empty(), "families::issue* is back: {back:?}");
    }
}
