//! Each /design section as rows: a state's label, then the same parts on design.json's light and dark tokens, side by side.
//! A forced state is a class beside its real pseudo-class (design.css), and script.rs checks the two draw the same.
use blitz_dom::BaseDocument;
use dioxus::prelude::*;

/// (left, top, width, height) in CSS px of the first node `sel` matches.
pub fn rect(doc: &BaseDocument, sel: &str) -> Option<(f64, f64, f64, f64)> {
    let n = doc.get_node(doc.query_selector(sel).ok()??)?;
    let p = n.absolute_position(0.0, 0.0);
    let s = n.final_layout().size;
    Some((p.x as f64, p.y as f64, s.width as f64, s.height as f64))
}

use crate::components::Fold;
pub use crate::components::{Avatar, Badge};
use crate::components::{Table, TableRow};

/// A section's head on every /design tab: its name, then the Light and Dark columns its rows draw.
pub(crate) fn head(name: &str) -> Element {
    rsx! {
        div { class: "ds-head",
            span { class: "ds-name", "{name}" }
            span { class: "ds-side", "Light" }
            span { class: "ds-side", "Dark" }
        }
    }
}

/// A component or document type laid out as a family's section: its head, one row drawing it light and dark, then its two
/// forms folded under it, never a column of raw HTML beside it (task rust-design-components-fit; @nha, 2026-11-09: « these
/// are all fairly ugly and don't fit other design tabs »). Both cells are its one render: the dark copy keeps no id (an id is
/// the page's once) and is inert (no tab stop the screen reader is told is absent).
pub(crate) fn specimen(
    id: &str,
    name: &'static str,
    used: &'static str,
    (html, text): (String, String),
    library: Option<String>,
) -> Element {
    let dark = no_ids(&html);
    rsx! {
        section { id: "s-{id}", key: "{id}", class: "ds-section",
            {head(name)}
            div { class: "ds-row",
                div { class: "ds-label", "{used}" }
                div { class: "app light ds-cell", dangerous_inner_html: html.clone() }
                div { class: "app dark ds-cell", "aria-hidden": "true", inert: true, dangerous_inner_html: dark }
            }
            Fold { id: "h-{id}", open: false, summary: rsx! { "HTML and text" },
                if let Some(href) = library {
                    a { class: "ds-lib", href, "Its page in the library" }
                }
                div { class: "ds-side", "HTML" }
                pre { class: "ds-code", "{html}" }
                div { class: "ds-side", "Text" }
                pre { class: "ds-code", "{text}" }
            }
        }
    }
}

/// Our own render with its ` id="…"` attributes dropped: a text's quote is escaped, so the pattern is only ever an attribute.
fn no_ids(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(at) = rest.find(" id=\"") {
        out.push_str(&rest[..at]);
        let value = &rest[at + 5..];
        rest = value.find('"').map_or("", |end| &value[end + 1..]);
    }
    out + rest
}

/// What `specimen` draws, for the tabs' tests: the page escapes a text as dioxus does.
#[cfg(test)]
pub(crate) fn specimen_html(
    id: &str,
    name: &str,
    used: &str,
    (html, text): (&str, &str),
    library: Option<&str>,
) -> String {
    let esc = |s: &str| {
        s.replace('&', "&#38;")
            .replace('<', "&#60;")
            .replace('>', "&#62;")
            .replace('"', "&#34;")
            .replace('\'', "&#39;")
    };
    let lib = library.map_or(String::new(), |l| {
        format!(
            r#"<a class="ds-lib" href="{}">Its page in the library</a>"#,
            esc(l)
        )
    });
    format!(
        r#"<section class="ds-section" id="s-{id}"><div class="ds-head"><span class="ds-name">{name}</span><span class="ds-side">Light</span><span class="ds-side">Dark</span></div><div class="ds-row"><div class="ds-label">{used}</div><div class="app light ds-cell">{html}</div><div class="app dark ds-cell" aria-hidden="true" inert=true>{dark}</div></div><details class="fold" id="h-{id}"><summary>HTML and text</summary>{lib}<div class="ds-side">HTML</div><pre class="ds-code">{shown}</pre><div class="ds-side">Text</div><pre class="ds-code">{words}</pre></details></section>"#,
        used = esc(used),
        dark = no_ids(html),
        shown = esc(html),
        words = esc(text),
    )
}

#[cfg(test)]
#[test]
fn no_ids_drops_each_id_and_keeps_the_rest() {
    let _budget = dash_meter::memory::TestBudget::new();
    assert_eq!(
        no_ids(r#"<a id="c-card" class="card"><b id="x">a id=&#34;y&#34;</b></a>"#),
        r#"<a class="card"><b>a id=&#34;y&#34;</b></a>"#
    );
}

/// Two calls, not a cloned Element: one VNode cannot mount twice.
fn row(label: &'static str, parts: impl Fn() -> Element) -> Element {
    rsx! {
        div { class: "ds-row",
            div { class: "ds-label", "{label}" }
            div { class: "app light ds-cell", {parts()} }
            // The same words again: the text form (crate::forms::text_of) reads them once.
            div { class: "app dark ds-cell", "aria-hidden": "true", inert: true, {parts()} }
        }
    }
}

/// Lucide-style strokes in `currentColor`: the one way an SVG follows the theme (a CSS stroke does not reach it in Blitz).
pub(crate) const CHECK: &str = "M4 12l5 5L20 6";
pub(crate) const BELL: &str = "M6 16V11a6 6 0 0112 0v5l2 2H4zM10 21h4";
pub(crate) const LOCK: &str = "M6 11h12v9H6zM8 11V8a4 4 0 018 0v3";
pub(crate) const SEARCH: &str = "M11 4a7 7 0 100 14 7 7 0 000-14zM16 16l5 5";
pub(crate) const PLUS: &str = "M12 5v14M5 12h14";
pub(crate) const INBOX: &str = "M4 13l3-8h10l3 8v6H4zM4 13h5l1 2h4l1-2h5";
pub(crate) const DOC: &str = "M7 3h7l4 4v14H7zM14 3v4h4";
pub(crate) const LINK: &str =
    "M10 14a4 4 0 006 0l3-3a4 4 0 00-6-6l-1 1M14 10a4 4 0 00-6 0l-3 3a4 4 0 006 6l1-1";
pub(crate) const DOTS: &str = "M5 12h.01M12 12h.01M19 12h.01";
pub(crate) const PERSON: &str = "M12 4a4 4 0 100 8 4 4 0 000-8zM5 20a7 7 0 0114 0";
pub(crate) const CHAT: &str = "M4 5h16v11H9l-5 4z";
pub(crate) const SEND: &str = "M12 19V6M6 12l6-6 6 6";
pub(crate) const CROSS: &str = "M6 6l12 12M18 6L6 18";
// the menu's entries (task rust-menu-icons; @nha, 2026-11-09: « logo for each tab please. like the elixir / linear had »)
pub(crate) const LIST: &str = "M4 6h16M4 12h10M4 18h6";
/// Three columns of a board, the tasks' board view.
pub(crate) const BOARD: &str = "M4 5h4v14H4zM10 5h4v10h-4zM16 5h4v7h-4z";
/// Two nodes leading to a third, the dependency graph view.
pub(crate) const GRAPH: &str = "M6 6h.01M18 6h.01M12 18h.01M6 6l6 12M18 6l-6 12";
pub(crate) const CHART: &str = "M5 20v-8M12 20V5M19 20v-11";
pub(crate) const CALENDAR: &str = "M4 6h16v14H4zM4 10h16M8 3v4M16 3v4";
pub(crate) const MAIL: &str = "M4 6h16v12H4zM4 7l8 6 8-6";
pub(crate) const AGENT: &str = "M7 8h10v10H7zM12 4v4M10 13h.01M14 13h.01";
pub(crate) const TOOL: &str = "M14 6a4 4 0 00-5 5l-5 5 3 3 5-5a4 4 0 005-5l-2 2-3-3z";
pub(crate) const PEN: &str = "M4 20l4-1 11-11-3-3L5 16z";
pub(crate) const SLIDERS: &str =
    "M4 6h9M17 6h3M4 12h3M11 12h9M4 18h11M19 18h1M15 4v4M9 10v4M17 16v4";
const STAR: &str = "M12 4l2.5 5.5 6 .7-4.4 4 1.2 5.8L12 17l-5.3 3 1.2-5.8-4.4-4 6-.7z";
const CLOSED: &str = "M9 6l6 6-6 6";
const OPEN: &str = "M6 9l6 6 6-6";

pub(crate) fn icon(d: &'static str) -> Element {
    rsx! {
        svg { class: "ic", width: "16", height: "16", view_box: "0 0 24 24",
            path { d, fill: "none", stroke: "currentColor", stroke_width: "2", stroke_linecap: "round", stroke_linejoin: "round" }
        }
    }
}

pub(crate) fn status(kind: &'static str) -> Element {
    let tone = match kind {
        "started" => "tx-warning",
        "review" | "done" => "tx-primary",
        "canceled" => "tx-muted",
        _ => "tx-muted",
    };
    let fill = match kind {
        "started" => "M8 3a5 5 0 010 10z",
        "review" => "M8 3a5 5 0 11-4.3 7.5L8 8z",
        "done" => "M8 3a5 5 0 110 10A5 5 0 018 3z",
        _ => "",
    };
    rsx! {
        svg { class: "ic {tone}", width: "16", height: "16", view_box: "0 0 16 16",
            // Backlog's circle is dashed, as Linear's: it never reads as Todo's (/code-review of rust-board-linear)
            circle { cx: "8", cy: "8", r: "6.5", fill: "none", stroke: "currentColor", stroke_width: "1.5",
                stroke_dasharray: if kind == "backlog" { "2 2" } }
            path { d: fill, fill: "currentColor" }
        }
    }
}

const BUTTONS: [(&str, &str); 5] = [
    ("button", "Primary"),
    ("button accept", "Approve"),
    ("button reject", "Reject"),
    ("button warn", "Lock"),
    ("button quiet", "Cancel"),
];

fn buttons(state: &'static str) -> Element {
    rsx! {
        div { class: "ds-flow",
            for (class, name) in BUTTONS {
                button { key: "{class}", class: "{class} {state}", "{name}" }
            }
        }
    }
}

fn rules(items: &'static [(&'static str, &'static str)]) -> Element {
    rsx! {
        div { class: "ds-rules",
            for (lead, rest) in items.iter().copied() {
                div { key: "{lead}", class: "ds-rule", b { "{lead} " } span { class: "ds-rest", "{rest}" } }
            }
        }
    }
}

fn swatches(names: &'static [(&'static str, &'static str)]) -> Element {
    rsx! {
        div { class: "ds-flow",
            for (var, name) in names.iter().copied() {
                div { key: "{var}", class: "ds-swatch",
                    div { class: "ds-chip", style: "background: var(--{var})" }
                    span { class: "meta", "{name}" }
                }
            }
        }
    }
}

fn ramp(name: &'static str) -> Element {
    rsx! {
        div { class: "ds-ramp",
            for step in ["50", "100", "200", "300", "400", "500", "600", "700", "800", "900", "950"] {
                div { key: "{step}", style: "background: var(--color-{name}-{step})" }
            }
        }
    }
}

fn field(class: &'static str, value: &'static str, hint: Element) -> Element {
    rsx! {
        div { class: "ds-field",
            div { class: "ds-field-label", "Daily rate, EUR" }
            input { class: "input {class}", value, disabled: class.contains("disabled"), readonly: class.contains("readonly") }
            {hint}
        }
    }
}

fn saying(class: &'static str, words: &'static str) -> Element {
    rsx! { div { class: "ds-say {class}", "{words}" } }
}

fn card(title: &'static str, actions: Element) -> Element {
    rsx! {
        div { class: "ds-card",
            div { class: "ds-card-title", "{title}" }
            div { class: "meta", "A line that says what it is about." }
            div { class: "ds-card-foot", {actions} }
        }
    }
}

fn comment(who: &'static str, state: &'static str, words: &'static str, foot: Element) -> Element {
    rsx! {
        div { class: "note-card {state}",
            div { class: "note-head", Avatar { initials: who } span { class: "meta", "2 h" } }
            div { class: "ds-quote", "each component with its states" }
            div { "{words}" }
            {foot}
        }
    }
}

fn mail_row(class: &'static str, star: bool) -> Element {
    rsx! {
        div { class: "line ds-line {class}",
            if class == "ticked" {
                div { class: "box on", {icon(CHECK)} }
            } else {
                div { class: "box" }
            }
            div { class: if star { "star on" } else { "star" }, {icon(STAR)} }
            span { class: "sender", "Ada" }
            span { class: "subject", "Contract ", span { class: "snippet", "two clauses to check" } }
            span { class: "date", "12:40" }
            div { class: "actions", span { class: "action", "Archive" } span { class: "action", "Delete" } }
        }
    }
}

fn table_rows(
    id: &'static str,
    rows: &'static [(&'static str, &'static str, &'static str)],
) -> Element {
    let columns = ["Company", "Stage", "Rate"].map(String::from).to_vec();
    let rows: Vec<TableRow> = (rows.iter())
        .map(|&(a, b, c)| TableRow {
            cells: [a, b, c].map(String::from).to_vec(),
            href: None,
        })
        .collect();
    let empty = rows.is_empty();
    rsx! {
        Table { id, columns, rows }
        if empty { div { class: "ds-empty", "No lead matches this filter." } }
    }
}

const WEEKS: [u32; 8] = [6, 8, 4, 11, 7, 9, 9, 5];

fn week_bars() -> Element {
    rsx! {
        div { class: "ds-bars",
            for (i, n) in WEEKS.into_iter().enumerate() {
                div { key: "{i}", class: "ds-column", style: "height: {n * 6}px" }
            }
        }
    }
}

/// A line in `currentColor`, from the data.
fn spark(points: &'static [u32]) -> Element {
    let d: String = points
        .iter()
        .enumerate()
        .map(|(i, p)| {
            format!(
                "{}{},{} ",
                if i == 0 { "M" } else { "L" },
                i * 20,
                40 - p * 4
            )
        })
        .collect();
    rsx! {
        svg { class: "tx-primary ds-spark", width: "160", height: "44", view_box: "0 0 160 44",
            path { d, fill: "none", stroke: "currentColor", stroke_width: "2" }
        }
    }
}

fn step(words: &'static str) -> Element {
    rsx! {
        div { class: "ds-step",
            div { class: "box" }
            div { class: "ds-grow", b { "{words}" } div { class: "meta", "The question list, then paste me the answers." } }
            button { class: "button accept", "Approve" }
        }
    }
}

fn lane(tone: &'static str, name: &'static str) -> Element {
    rsx! { Badge { look: tone, "{name}" } }
}

fn issue_row(kind: &'static str, class: &'static str, title: &'static str) -> Element {
    rsx! {
        div { class: "ds-issue {class}",
            span { class: "meta", "AAA-12" }
            {status(kind)}
            span { class: "ds-grow", "{title}" }
            Badge { look: "tone-outline", "rust" }
            Avatar { initials: "CL" }
        }
    }
}

fn work_card(state: &'static str, tone: &'static str, title: &'static str) -> Element {
    rsx! {
        div { class: "ds-card",
            div { class: "ds-card-title", "{title}" }
            div { class: "ds-flow", Badge { look: tone, "{state}" } span { class: "meta", "since 20/10, due 30/10" } }
            div { class: "ds-card-foot", button { class: "button quiet", "Drop" } button { class: "button accept", "Done" } }
        }
    }
}

const STAGES: [&str; 4] = ["Draft", "Review", "Accepted", "Sent"];

/// Stage as Notion's select property: its name, then the current value as a pill; a click opens the values, a pick sets one and
/// closes them (task design-stage-notion-toggle; @nha, 2026-11-09: « No, this is a notion-style toggle instead. »). The window
/// picks through these handlers; the web, which runs none, through DESIGN_SCRIPT (design/design.js). Blitz opens the <details>
/// itself, out of Dioxus's sight, so each pick draws a fresh one, closed: its key is the count of picks.
#[component]
fn Stage(on: usize, open: bool) -> Element {
    let mut now = use_signal(|| on);
    let mut picks = use_signal(|| 0);
    rsx! {
        div { class: "ds-prop",
            span { class: "ds-prop-name", "Stage" }
            // One item, keyed: a new key is a new, closed <details> (see above), never a list.
            for n in std::iter::once(picks()) {
                details { key: "{n}", class: "ds-toggle", open: open && n == 0,
                    summary { class: "ds-pill on", "{STAGES[now()]}" }
                    div { class: "ds-pop",
                        for (i, name) in STAGES.into_iter().enumerate() {
                            button { key: "{name}", r#type: "button", class: if i == now() { "ds-pop-line on" } else { "ds-pop-line" },
                                onclick: move |_| { now.set(i); picks += 1; }, "{name}"
                            }
                        }
                    }
                }
            }
        }
    }
}

const LEADS: &[(&str, &str, &str)] = &[
    ("Northwind", "Applied", "600"),
    ("Globex", "Screening", "700"),
    ("Umbrella Labs", "Interviewing", "650"),
];
const SPARK: &[u32] = &[3, 5, 2, 7, 4, 6, 8];
const SURFACES: &[(&str, &str)] = &[
    ("color-base-100", "base-100"),
    ("color-base-200", "base-200"),
    ("color-base-300", "base-300"),
    ("color-base-content", "content"),
];
const ROLES: &[(&str, &str)] = &[
    ("color-primary", "primary"),
    ("color-secondary", "secondary"),
    ("color-accent", "accent"),
    ("color-neutral", "neutral"),
];
const SIGNALS: &[(&str, &str)] = &[
    ("color-info", "info"),
    ("color-success", "success"),
    ("color-warning", "warning"),
    ("color-error", "error"),
];

/// A task's page, Linear's issue view as the older app's issue page lays it out (task rust-task-page-linear; @nha, 2026-11-09:
/// « the tasks page need to match the linear design »): the way back and its key, then its title and parts, its properties
/// beside them. `kind`: a status `status` draws, `name` its words; `side`: what the properties hold past the status.
pub(crate) fn issue(
    key: &str,
    title: &str,
    kind: &'static str,
    name: &str,
    says: &str,
    main: Element,
    side: Element,
) -> Element {
    rsx! {
        div { class: "li-issue",
            nav { class: "li-crumbs", a { href: "/tasks", "Tasks" } span { class: "meta", "›" } span { "{key}" } }
            div { class: "li-issue-main",
                h1 { class: "li-issue-title", "{title}" }
                {main}
            }
            aside { class: "li-issue-side",
                div { class: "li-side-head", "Properties" }
                div { class: "li-prop", {status(kind)} span { "{name}" } }
                if !says.is_empty() {
                    div { class: "li-prop meta", "{says}" }
                }
                {side}
            }
        }
    }
}

/// A titled part of an issue: Done when, Activity.
pub(crate) fn issue_part(title: &str, children: Element) -> Element {
    rsx! {
        section { class: "li-part",
            h2 { class: "li-part-head", "{title}" }
            {children}
        }
    }
}

/// One check of a done-when: met a done circle, not met a cross, not judged an empty circle. `word`: its verdict in words.
pub(crate) fn issue_check(met: Option<bool>, word: &str, text: &str) -> Element {
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
pub(crate) fn issue_event(what: &str, when: &str) -> Element {
    rsx! { div { class: "li-event", span { class: "li-dot" } b { "{what}" } time { class: "meta", "{when}" } } }
}

/// An issue's pull requests among its properties: each its number, the link GitHub's.
pub(crate) fn issue_prs(links: &[(&str, &str)]) -> Element {
    rsx! {
        if !links.is_empty() {
            div { class: "li-side-head", "Pull requests" }
        }
        for (url, number) in links.iter().copied() {
            a { class: "li-tag li-pr", href: "{url}", {icon(LINK)} "#{number}" }
        }
    }
}

/// A section's rows.
pub(crate) fn body(id: &str) -> Element {
    match id {
        "guidelines" => rsx! {
            {row("Banned", || rules(&[("An eyebrow above a heading.", "Say it in the heading."), ("A card in a card.", "One frame per thing."), ("An emoji as an icon.", "Icons are strokes, in the text's colour.")]))}
            {row("Directing", || rules(&[("One primary action per view.", "The rest are neutral."), ("Show, then count.", "A quantity is a visual first.")]))}
            {row("Spacing", || rsx! {
                for s in 0..10 {
                    div { key: "{s}", class: "ds-space", style: "width: var(--spacing-s{s})" }
                }
            })}
            {row("Type", || rsx! {
                div { style: "font-size: var(--text-caption); line-height: var(--text-caption--line-height)", "caption, 12 px" }
                div { style: "font-size: var(--text-ui); line-height: var(--text-ui--line-height)", "ui, 14 px" }
                div { style: "font-size: var(--text-body); line-height: var(--text-body--line-height)", "body, 15 px" }
                div { style: "font-size: var(--text-title); line-height: var(--text-title--line-height)", "title, 18 px" }
                div { style: "font-size: var(--text-heading); line-height: var(--text-heading--line-height)", "heading, 24 px" }
                div { style: "font-size: var(--text-display); line-height: var(--text-display--line-height)", "display, 32 px" }
            })}
            {row("Faces", || rsx! {
                div { span { class: "ds-face ds-sans", "The same words" } " sans" }
                div { span { class: "ds-face ds-note", "The same words" } " hand" }
                div { code { class: "ds-face ds-narrow", "iiiiiiii" } " " code { class: "ds-face ds-wide", "WWWWWWWW" } " mono: one width" }
                div { code { "cargo run -p dioxus-page -- --design" } }
            })}
            {row("Blur", || rsx! { div { class: "ds-blur", "Behind frosted glass" } })}
            {row("Radius", || rsx! {
                div { class: "ds-flow",
                    for r in ["selector", "field", "box"] {
                        div { key: "{r}", class: "ds-radius", style: "border-radius: var(--radius-{r})", "{r}" }
                    }
                }
            })}
        },
        "heroui" => rsx! {
            {row("Against ours", || table_rows("ds-ours", &[("button", "restyled", "Petal"), ("chip", "restyled", "badge"), ("alert", "ours", "banner")]))}
        },
        "shell" => rsx! {
            {row("Menu, current", || rsx! {
                div { class: "ds-menu",
                    div { class: "nav-item", {icon(INBOX)} " Inbox" }
                    div { class: "nav-item on", {icon(DOC)} " Design" }
                    div { class: "nav-item is-hover", {icon(PERSON)} " Job search, hovered" }
                }
            })}
            {row("Inbox waits on him", || rsx! {
                div { class: "ds-menu", div { class: "nav-item", {icon(INBOX)} " Inbox " span { class: "ds-count", "3" } } }
            })}
            {row("Search field", || rsx! {
                div { class: "ds-search", {icon(SEARCH)} span { class: "meta", "Pages, documents, leads" } }
            })}
            {row("Logo, then none", || rsx! {
                div { class: "ds-flow", Avatar { initials: "NH" } div { class: "ds-logo", "G" } div { class: "ds-logo none", "U" } }
            })}
        },
        "bell" => rsx! {
            {row("Closed, two unread", || rsx! {
                div { class: "ds-flow", div { class: "ds-bell", {icon(BELL)} span { class: "ds-dot", "2" } } }
            })}
            {row("Open", || rsx! {
                div { class: "ds-pop",
                    div { class: "ds-pop-line unread", "Claude answered your comment" span { class: "meta", "5 min" } }
                    div { class: "ds-pop-line unread", "Waiting on you: a reply" span { class: "meta", "3 h" } }
                    div { class: "ds-pop-line", "A task is flagged" span { class: "meta", "2 d" } }
                }
            })}
            {row("Nothing pushed", || rsx! { div { class: "ds-pop", div { class: "ds-empty", "Nothing pushed yet." } } })}
        },
        "shared" => rsx! {
            {row("Approve", || rsx! {
                div { class: "ds-flow",
                    button { class: "button accept", {icon(CHECK)} " Approve" }
                    button { class: "button accept is-busy", span { class: "ds-spin" } " Approve" }
                }
            })}
            {row("Dates", || rsx! {
                div { class: "ds-flow", span { class: "meta", "20 s" } span { class: "meta", "5 min" } span { class: "meta", "2 h" } span { class: "meta", "3 d" } span { class: "meta", "in 2 d" } }
            })}
            {row("A task's status", || rsx! {
                div { class: "ds-flow", {lane("tone-warn", "Needs you")} {lane("tone-info", "Working")} {lane("tone-primary", "Ready")} {lane("tone-secondary", "Todo")} {lane("tone-outline", "Done")} }
            })}
            {row("Points", || rules(&[("1. An audit lists every page:", "proof, the table."), ("2. Dates read as a moment:", "proof, a test.")]))}
        },
        "cards" => rsx! {
            {row("With its actions", || card("One thing", rsx! { button { class: "button quiet", "Later" } button { class: "button", "Do it" } }))}
            {row("Hovered", || rsx! { div { class: "ds-card is-hover", div { class: "ds-card-title", "A raised surface" } div { class: "meta", "Its shadow rises on hover." } } })}
            {row("Locked", || card("The same card, done", rsx! { {icon(LOCK)} span { class: "meta", "Locked on 18/10" } }))}
            {row("Nothing to do", || rsx! { div { class: "ds-empty", "Nothing to do yet." } })}
        },
        "colors" => rsx! {
            {row("Surfaces", || swatches(SURFACES))}
            {row("Roles", || swatches(ROLES))}
            {row("Signals", || swatches(SIGNALS))}
            {row("Ramps", || rsx! {
                {ramp("primary")} {ramp("secondary")} {ramp("info")} {ramp("success")} {ramp("warning")} {ramp("danger")}
            })}
        },
        "data" => rsx! {
            {row("In this order", || rules(&[("A trend over time:", "columns per period."), ("Stages that narrow:", "the funnel."), ("A figure, last:", "with its change beside it.")]))}
            {row("Never", || rules(&[("A number alone,", "above all on the home page."), ("A pie", "for more than three parts.")]))}
        },
        "table" => rsx! {
            {row("Leads", || table_rows("ds-leads", LEADS))}
            {row("Filter bar", || rsx! {
                div { class: "ds-flow", input { class: "input ds-short", value: "Glob" } span { class: "ds-pill on", "Stage: Applied" } span { class: "ds-pill", "Rate" } }
            })}
            {row("Empty", || table_rows("ds-no-lead", &[]))}
        },
        "charts" => rsx! {
            {row("Funnel", || rsx! {
                for (name, w) in [("Opened", 100), ("Applied", 62), ("Replied", 30), ("Signed", 8)] {
                    div { key: "{name}", class: "ds-funnel", span { class: "meta ds-funnel-name", "{name}" } div { class: "ds-track", div { class: "ds-fill", style: "width: {w}%" } } }
                }
            })}
            {row("Per week", week_bars)}
            {row("A line", || spark(SPARK))}
            {row("No value yet", || rsx! { div { class: "ds-empty", "No week has a value yet." } })}
        },
        "stats" => rsx! {
            {row("Share bar", || rsx! {
                div { class: "ds-share", div { style: "width: 67%" } }
                div { class: "meta", "8 of 12 done, 4 left" }
            })}
            {row("A figure", || rsx! {
                div { class: "ds-figure", "12" } div { class: "meta", "up 3 on last week" }
            })}
            {row("Sparkline", || rsx! { div { class: "ds-flow", span { "Replies" } {spark(SPARK)} } })}
        },
        "timeline" => rsx! {
            {row("Three events", || rsx! {
                div { class: "ds-time",
                    div { class: "ds-event", b { "Contract draft received" } div { class: "meta", "16/10, two clauses to check" } }
                    div { class: "ds-event", b { "Call with the CTO" } div { class: "meta", "14/10, rate agreed" } }
                    div { class: "ds-event dim", b { "Intro sent" } div { class: "meta", "21/09, greyed by the point of view" } }
                }
            })}
            {row("No event", || rsx! { div { class: "ds-empty", "No event yet." } })}
        },
        "lists" => rsx! {
            {row("A document", || card("Rendering plan", rsx! { Badge { look: "tone-outline", "review" } }))}
            {row("A folded group", || rsx! {
                div { class: "ds-fold", {icon(CLOSED)} " Closed leads " span { class: "meta", "4" } }
                div { class: "ds-fold", {icon(OPEN)} " Open leads " span { class: "meta", "2" } }
                div { class: "ds-issue", "Northwind" }
            })}
        },
        "records" => rsx! {
            {row("Record chips", || rsx! {
                div { class: "ds-flow",
                    span { class: "ds-record", Avatar { initials: "AL" } "Ada" }
                    span { class: "ds-record", div { class: "ds-logo small", "G" } "Globex" }
                    span { class: "ds-record", div { class: "ds-logo small none", "?" } "None" }
                }
            })}
            {row("A record board", || rsx! {
                div { class: "ds-flow ds-top",
                    div { class: "ds-mini", div { class: "meta", "Screening" } div { class: "ds-card", b { "Backend lead" } div { class: "meta", "Globex, remote" } } }
                    div { class: "ds-mini", div { class: "meta", "Applied" } div { class: "ds-card", b { "Staff engineer" } div { class: "meta", "Umbrella Labs" } } }
                    div { class: "ds-fold", {icon(CLOSED)} " Dropped 1" }
                }
            })}
        },
        "knowledge" => rsx! {
            {row("With its source", || rsx! { div { class: "ds-card", b { "Payments" } div { "They settle in T+2." } a { class: "ds-link", "the call of 14/10" } } })}
            {row("No source yet", || rsx! { div { class: "ds-card", b { "Payments" } div { class: "meta", "No source yet." } } })}
        },
        "buttons" => rsx! {
            {row("Rest", || buttons(""))}
            {row("Hover", || buttons("is-hover"))}
            {row("Pressed", || buttons("is-active"))}
            {row("Focus", || buttons("is-focus"))}
            {row("Disabled", || rsx! {
                div { class: "ds-flow",
                    for (class, name) in BUTTONS {
                        button { key: "{class}", class, disabled: true, "{name}" }
                    }
                }
            })}
            {row("Busy", || rsx! {
                div { class: "ds-flow", button { class: "button is-busy", span { class: "ds-spin" } " Sending" } }
            })}
            {row("Done", || rsx! { div { class: "ds-flow", button { class: "button warn is-done", {icon(LOCK)} " Locked on 18/10" } } })}
            {row("Try it", || rsx! {
                div { class: "ds-flow try",
                    for (class, name) in BUTTONS {
                        button { key: "{class}", class, "{name}" }
                    }
                }
            })}
        },
        "group" => rsx! {
            {row("One of several", || rsx! { Toggle { multi: false } })}
            {row("Each on or off", || rsx! { Toggle { multi: true } })}
            {row("Forced states", || rsx! {
                div { class: "ds-group",
                    span { class: "ds-seg", "Rest" } span { class: "ds-seg is-hover", "Hover" } span { class: "ds-seg on", "On" } span { class: "ds-seg off", "Disabled" }
                }
            })}
            {row("Chip bar", || rsx! {
                div { class: "ds-flow", span { class: "ds-pill on", "Project" } span { class: "ds-pill", "Lane" } span { class: "ds-pill is-hover", "Team, hovered" } }
            })}
        },
        "field" => rsx! {
            {row("Rest, its placeholder", || rsx! { Field {} })}
            {row("Hover", || field("is-hover", "650", rsx! {}))}
            {row("Focus", || field("is-focus", "650", rsx! {}))}
            {row("Saving", || field("", "650", saying("", "Saving...")))}
            {row("Saved", || field("", "650", saying("ok", "Saved.")))}
            {row("Error", || field("is-error", "six fifty", saying("bad", "A rate is a whole number of euros.")))}
            {row("Disabled", || field("disabled", "650", rsx! {}))}
            {row("Read only", || field("readonly", "650", rsx! {}))}
            {row("Try it", || rsx! { div { class: "try-field", input { class: "input", value: "650" } } })}
            {row("Checkbox", || rsx! {
                div { class: "ds-flow",
                    span { class: "ds-tick", div { class: "box" } "Off" }
                    span { class: "ds-tick", div { class: "box on", {icon(CHECK)} } "On" }
                    span { class: "ds-tick", div { class: "box is-hover" } "Hover" }
                    span { class: "ds-tick", div { class: "box is-focus" } "Focus" }
                    span { class: "ds-tick off", div { class: "box" } "Disabled" }
                }
            })}
            {row("Radio and switch", || rsx! {
                div { class: "ds-flow",
                    span { class: "ds-tick", div { class: "box round" } "Off" }
                    span { class: "ds-tick", div { class: "box round dot" } "On" }
                    div { class: "ds-switch", div {} }
                    div { class: "ds-switch on", div {} }
                }
            })}
            {row("Select, open", || rsx! {
                div { class: "input ds-select", "Applied " {icon(OPEN)} }
                div { class: "ds-pop",
                    div { class: "ds-pop-line is-hover", "Applied" }
                    div { class: "ds-pop-line", "Screening" }
                }
            })}
        },
        "text" => rsx! {
            {row("Read, with Edit", || rsx! { div { class: "ds-letter", "Dear Ada, the payment comes once the contract is signed." } button { class: "button quiet", "Edit" } })}
            {row("Open", || rsx! { textarea { class: "input ds-area", "Dear Ada, the payment comes once the contract is signed." } button { class: "button", "Save" } })}
            {row("Saved", || rsx! { div { class: "ds-letter", "Dear Ada, the payment comes." } {saying("ok", "Saved.")} })}
            {row("Error, kept open", || rsx! { textarea { class: "input ds-area is-error", "Dear Ada," } {saying("bad", "This edit drops the passage comment k1 is on.")} })}
            {row("Decided, read only", || rsx! { div { class: "ds-letter", "Dear Ada, the payment comes." } })}
        },
        "stage" => rsx! {
            {row("Closed", || rsx! { Stage { on: 1, open: false } })}
            {row("Open, picking", || rsx! { Stage { on: 1, open: true } })}
        },
        "wait" => rsx! {
            {row("Actionable", || lane("tone-primary", "Ready"))}
            {row("Waits on one", || rsx! { Badge { look: "tone-warn", "Waits on: contract signed" } })}
            {row("Waits on two", || rsx! { div { class: "ds-flow", Badge { look: "tone-warn", "contract signed" } Badge { look: "tone-warn", "payment provider" } } })}
        },
        "chips" => rsx! {
            {row("Badges, each colour", || rsx! {
                div { class: "ds-flow",
                    {lane("tone-primary", "primary")} {lane("tone-secondary", "secondary")} {lane("tone-info", "info")} {lane("tone-warn", "warning")} {lane("tone-destructive", "danger")} {lane("tone-outline", "outline")}
                }
            })}
            {row("Chips", || rsx! { div { class: "ds-flow", span { class: "ds-pill", "a label" } a { class: "ds-pill ds-link", "a link" } span { class: "ds-pill is-hover", "hovered" } } })}
        },
        "reading" => rsx! {
            {row("Chances", || rsx! { div { class: "ds-share", div { style: "width: 72%" } } div { class: "meta", "72, good: remote, rate in range" } })}
            {row("What the ad asks", || rules(&[("Clojure:", "seven years."), ("Payments:", "the Griffin work.")]))}
            {row("Not judged yet", || rsx! { div { class: "ds-empty", "Not judged yet." } })}
        },
        "comments" => rsx! {
            {row("Open, on a passage", || comment("NH", "", "Can the table keep its filter?", rsx! { div { class: "note-actions", button { class: "button quiet", "Reply" } button { class: "button quiet", "Resolve" } } }))}
            {row("Answered", || comment("CL", "", "Done, the URL holds it.", rsx! { a { class: "ds-link", "task table-filter" } }))}
            {row("Replying", || comment("NH", "", "Keep the sort too?", rsx! { input { class: "input is-focus", value: "Yes" } }))}
            {row("Posting", || comment("NH", "", "Keep the sort too?", saying("", "Posting...")))}
            {row("Not sent", || comment("NH", "is-error", "Keep the sort too?", saying("bad", "Not sent. Retry.")))}
            {row("Its passage gone", || rsx! { div { class: "note-card", div { class: "ds-quote gone", "passage no longer in the page" } div { "It is kept here." } } })}
            {row("Resolved", || rsx! { div { class: "note-card off", div { class: "note-head", {icon(CHECK)} "Resolved" } } })}
        },
        "comments-principles" => rsx! {
            {row("Three of ten", || rules(&[("Intent over look.", "A button names its intent."), ("Shown when needed.", "Actions only on an open card."), ("Themed by tokens.", "A theme change reaches each part.")]))}
        },
        "handoff" => rsx! {
            {row("A step", || step("Call Northwind on 22/10"))}
            {row("His words, the answer", || rules(&[("@nha:", "already done?"), ("Done:", "sent again.")]))}
            {row("A diff", || rsx! {
                div { class: "ds-diff del", "- the old line" }
                div { class: "ds-diff add", "+ the new line" }
            })}
        },
        "mail" => rsx! {
            {row("Read", || mail_row("", false))}
            {row("Unread, starred", || mail_row("unread", true))}
            {row("Hover", || mail_row("is-hover", false))}
            {row("Ticked", || mail_row("ticked", false))}
            {row("Compose", || rsx! {
                input { class: "input", value: "ada@example.com" }
                textarea { class: "input ds-area", "Dear Ada," }
                div { class: "dialog-actions", button { class: "button quiet", "Discard" } button { class: "button", "Send" } }
            })}
            {row("Folded", || rsx! { div { class: "ds-fold", {icon(CLOSED)} " Ada, 12:40 " span { class: "meta", "two clauses to check" } } })}
            {row("Nothing listed", || rsx! { div { class: "ds-empty", "Nothing listed yet." } })}
        },
        "board" => rsx! {
            {row("Lanes", || rsx! { div { class: "ds-flow", {lane("tone-warn", "Needs you 2")} {lane("tone-info", "Working 3")} {lane("tone-primary", "Ready 1")} } })}
            {row("Rows", || rsx! {
                {issue_row("started", "", "Phone pages")}
                {issue_row("review", "on", "The CLI, selected")}
            })}
        },
        "issues" => rsx! {
            {row("Status icons", || rsx! {
                div { class: "ds-flow", {status("backlog")} {status("todo")} {status("started")} {status("review")} {status("done")} }
            })}
            {row("The list", || rsx! {
                {issue_row("todo", "", "Sign the app")}
                {issue_row("todo", "is-hover", "A host change waits, hovered")}
                {issue_row("done", "off", "The old CRM retired")}
            })}
            {row("Keys", || rsx! { div { class: "ds-flow", kbd { class: "ds-kbd", "Cmd" } kbd { class: "ds-kbd", "K" } kbd { class: "ds-kbd", "?" } } })}
        },
        // /tasks/<id>'s parts, the page itself drawn from these (task rust-task-page-linear)
        "tasks" => rsx! {
            {row("Issue page", || issue("AAA-12", "Port the queue page", "started", "In Progress", "running, try 2",
                rsx! {
                    p { "Why it matters, in his words." }
                    {issue_part("Done when: 1 of 3 met", rsx! {
                        {issue_check(Some(true), "met", "The page lists the queue: a screenshot")}
                        {issue_check(Some(false), "not met", "A test holds it")}
                        {issue_check(None, "not judged", "The old link opens it")}
                    })}
                    {issue_part("Activity", rsx! {
                        {issue_event("Started, try 1", "2026-11-08 09:12 UTC")}
                        {issue_event("Finished: failed", "2026-11-08 10:40 UTC")}
                    })}
                },
                issue_prs(&[("https://github.com/o/r/pull/96", "96")])))}
            {row("Verdicts", || rsx! { div { class: "ds-flow", {lane("tone-destructive", "not met")} {lane("tone-primary", "met")} span { class: "meta", "nudged once, 52 turns" } } })}
            {row("A PR", || rsx! { div { class: "ds-flow", a { class: "ds-link", "#96" } {lane("tone-warn", "changes requested")} } })}
            {row("No question", || rsx! { div { class: "ds-empty", "No question open." } })}
        },
        "work-card" => rsx! {
            {row("A step open", || work_card("needs you", "tone-warn", "A step only you can take"))}
            {row("Closed by its check", || work_card("done", "tone-primary", "Sign in again"))}
        },
        "agents" => rsx! {
            {row("The orchestrator", || rsx! { div { class: "ds-agent", div { class: "ds-live" } b { "Orchestrator" } span { class: "meta", "login runs out in 2 h" } } })}
            {row("Down", || rsx! { div { class: "ds-agent", div { class: "ds-live down" } b { "Orchestrator" } span { class: "meta", "no reading" } } })}
            {row("Its reviewer under it", || rsx! {
                div { class: "ds-agent", div { class: "ds-live" } b { "rust-ui" } span { class: "meta", "working" } }
                div { class: "ds-agent child", div { class: "ds-live" } b { "review-rust-ui" } span { class: "meta", "reading" } }
            })}
        },
        _ => rsx! {},
    }
}

/// Live, as /design's « Try it ».
#[component]
fn Toggle(multi: bool) -> Element {
    let mut picked = use_signal(|| vec![1usize]);
    rsx! {
        div { class: "ds-group",
            for (i, name) in ["Month", "Week", "Day"].into_iter().enumerate() {
                button { key: "{name}", class: if picked.read().contains(&i) { "ds-seg on" } else { "ds-seg" },
                    onclick: move |_| {
                        let mut p = picked.write();
                        if !multi {
                            p.clear();
                            p.push(i);
                        } else if let Some(at) = p.iter().position(|x| *x == i) {
                            p.remove(at);
                        } else {
                            p.push(i);
                        }
                    },
                    "{name}"
                }
            }
        }
    }
}

/// An input's placeholder is drawn by a span over it, as Blitz draws none: its text goes as the value does.
#[component]
pub(crate) fn Field(
    #[props(default)] id: &'static str,
    #[props(default)] class: &'static str,
    #[props(default = "650")] hint: &'static str,
    #[props(default)] oninput: EventHandler<String>,
    #[props(default)] value: Option<String>,
) -> Element {
    let mut own = use_signal(String::new);
    let now = value.clone().unwrap_or_else(|| own.cloned());
    rsx! {
        div { class: "ds-holder",
            input { id, class: "input {class}", value: "{now}",
                oninput: move |e| {
                    own.set(e.value());
                    oninput.call(e.value());
                },
            }
            if now.is_empty() {
                span { class: "ds-placeholder", "{hint}" }
            }
        }
    }
}
