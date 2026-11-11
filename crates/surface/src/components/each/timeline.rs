use super::parts::{joined, today};
use crate::component::{Component, Section};
use dioxus::prelude::*;
use std::collections::BTreeMap;

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
        classes: &["timeline", "strip", "when", "gantt", "gantt-svg", "gantt-grid", "gantt-tick", "gantt-row", "gantt-label", "gantt-bar", "gantt-milestone", "gantt-arrow", "gantt-today", "gantt-group"],
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
                ("A Gantt: a row an item, start to target, today, what waits on what, a milestone (Linear's projects timeline)", |_| {
                    let today = today();
                    let day = |n: i64| Some(dash_core::date(today + n));
                    let row = |id: &str, name: &str, start: Option<String>, target: i64, after: &[&str], mark: Mark| GanttRow {
                        id: id.into(),
                        name: name.into(),
                        start,
                        target: dash_core::date(today + target),
                        after: after.iter().map(|a| a.to_string()).collect(),
                        mark,
                        group: Some(if target < 15 { "Now" } else { "Next" }.into()),
                        href: None,
                    };
                    let rows = vec![
                        row("gantt", "Gantt component", day(-12), -3, &[], Mark::Done),
                        row("model", "Plan data model", day(-2), 8, &["gantt"], Mark::Next),
                        row("sync", "Linear sync", day(-9), 10, &[], Mark::Failed),
                        row("page", "/plan page", day(9), 20, &["model", "sync"], Mark::Next),
                        row("ships", "/plan ships", None, 21, &["page"], Mark::Next),
                    ];
                    rsx! { Gantt { id: "c-gantt", today, rows } }
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

/// One row of a `Gantt`: an item from its `start` to its `target` day, or a milestone, its target alone (no `start`).
/// Both days are held, a target day drawn whole, as Linear's. `after`: the rows it waits on, by `id`, each a dotted arrow from
/// their end to its start. `mark`: done (filled), next (not done yet, pale, the default: a row fed with no mark is never shown
/// finished) or failed (late, the error colour), as a `TimelineEvent`'s dot. `group`: the name its rows sit under, drawn once
/// above the first row of a run of them, as Linear groups its timeline (/plan's Urgent, Now, Next, Later; task
/// linear-projects-view). `href`: what its name opens, checked as a card's by the tree (/plan's row opens its panel).
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct GanttRow {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub start: Option<String>,
    pub target: String,
    #[serde(default)]
    pub after: Vec<String>,
    #[serde(default = "next")]
    pub mark: Mark,
    #[serde(default)]
    pub group: Option<String>,
    #[serde(default)]
    pub href: Option<String>,
}

fn next() -> Mark {
    Mark::Next
}

impl GanttRow {
    /// Its first and last day, a milestone's both its target; `None` when a day does not read (2026-11-02) or it starts after
    /// its target, shown as unread rather than drawn swapped (/code-review).
    fn span(&self) -> Option<(i64, i64)> {
        let to = dash_core::days(&self.target)?;
        let from = self.start.as_deref().map_or(Some(to), dash_core::days)?;
        (from <= to).then_some((from, to))
    }

    /// Where it sits on the axis, in days: a bar from its first day to the end of its last, a milestone at the end of its day.
    fn edges(&self) -> Option<(i64, i64)> {
        let (from, to) = self.span()?;
        Some(match self.start {
            None => (to + 1, to + 1),
            Some(_) => (from, to + 1),
        })
    }
}

/// Items on a time axis, Linear's projects timeline the model (task rust-gantt-component; @nha, 2026-11-11: « /plan should
/// have a view like linear projects … Start with the design system first. Gantt diagram. »): a row an item, its name then its
/// bar from start to target, a milestone a diamond, the today line, a dotted arrow from each row it waits on (in the error
/// colour when that row ends after this one starts). The Timeline's rows laid on days. Drawn here as SVG, as the charts are;
/// its text form is a Mermaid gantt of the same rows (`gantt_mermaid`), never written twice.
#[component]
pub fn Gantt(#[props(into)] id: String, rows: Vec<GanttRow>, today: i64) -> Element {
    const W: f64 = 720.0;
    const LEFT: f64 = 168.0;
    const RIGHT: f64 = 16.0;
    const HEAD: f64 = 34.0;
    const ROW: f64 = 32.0;
    let spans: Vec<Option<(i64, i64)>> = rows.iter().map(GanttRow::edges).collect();
    let days = || {
        spans
            .iter()
            .flatten()
            .flat_map(|&(a, b)| [a, b])
            .chain([today])
    };
    let (lo, hi) = (
        days().min().unwrap_or(today) - 3,
        days().max().unwrap_or(today) + 3,
    );
    let day_w = (W - LEFT - RIGHT) / (hi - lo) as f64;
    let x = |d: i64| LEFT + (d - lo) as f64 * day_w;
    // a group's name takes a row of its own above the first row of its run (Linear's grouped timeline): `slots`, each row's
    // place counting those; `heads`, each name's place
    let (mut slots, mut heads) = (vec![], vec![]);
    for (i, r) in rows.iter().enumerate() {
        if let Some(g) = r.group.as_deref()
            && (i == 0 || rows[i - 1].group.as_deref() != Some(g))
        {
            heads.push((i + heads.len(), g.to_string()));
        }
        slots.push(i + heads.len());
    }
    let place = |s: usize| HEAD + ROW * s as f64 + ROW / 2.0;
    let mid = |i: usize| place(slots[i]);
    let h = HEAD + ROW * (rows.len() + heads.len()) as f64 + 8.0;
    // Mondays up to three months, else each month's first day; at most twelve labels, every other one past that
    let weekly = hi - lo <= 90;
    let ticks: Vec<(f64, String)> = (lo..=hi)
        .filter_map(|d| {
            let s = dash_core::date(d);
            let month = dash_core::MONTHS[s.get(5..7)?.parse::<usize>().ok()?.checked_sub(1)?];
            let day = s.get(8..10)?;
            match weekly {
                true => ((d + 3).rem_euclid(7) == 0)
                    .then(|| format!("{month} {}", day.trim_start_matches('0'))),
                false => (day == "01").then(|| month.to_string()),
            }
            .map(|label| (x(d), label))
        })
        .collect();
    let every = ticks.len().div_ceil(12).max(1);
    // a row's own place is its own; an id two rows share points at the last of them
    let at: BTreeMap<&str, (usize, (i64, i64))> = rows
        .iter()
        .zip(&spans)
        .enumerate()
        .filter_map(|(i, (r, s))| Some((r.id.as_str(), (i, (*s)?))))
        .collect();
    let mut arrows = vec![];
    for (i, (r, span)) in rows.iter().zip(&spans).enumerate() {
        let Some((start, _)) = *span else {
            continue;
        };
        for a in &r.after {
            // ponytail: a row named in `after` that is not drawn draws no arrow; the Mermaid text still names it
            let Some(&(j, (_, end))) = at.get(a.as_str()) else {
                continue;
            };
            let (x1, y1, x2, y2) = (x(end), mid(j), x(start), mid(i));
            let dx = ((x2 - x1).abs() / 2.0).max(24.0);
            let d = format!(
                "M{x1:.1},{y1:.1} C{:.1},{y1:.1} {:.1},{y2:.1} {x2:.1},{y2:.1}",
                x1 + dx,
                x2 - dx
            );
            arrows.push((d, end > start));
        }
    }
    let label = |name: &str| match name.chars().count() > 20 {
        // ponytail: 20 letters fit the label column at 14 px; a text measure if names run long
        true => format!("{}…", name.chars().take(19).collect::<String>()),
        false => name.to_string(),
    };
    let view = format!("0 0 {W} {h}");
    let (arrow, late) = (format!("{id}-arrow"), format!("{id}-late"));
    let tx = x(today);
    let text = gantt_mermaid(&rows, today);
    rsx! {
        div { id, class: "gantt",
            svg { class: "gantt-svg", view_box: view, "aria-hidden": "true",
                defs {
                    for (mid_id, class) in [(&arrow, "gantt-arrow"), (&late, "gantt-arrow late")] {
                        marker { id: "{mid_id}", class, "color": late_colour(class.ends_with("late")), view_box: "0 0 8 8", "refX": "7", "refY": "4", "markerWidth": "7", "markerHeight": "7", "orient": "auto",
                            path { d: "M0,0 L8,4 L0,8 Z", fill: "currentColor" }
                        }
                    }
                }
                line { class: "gantt-grid", stroke: "currentColor", stroke_opacity: "0.2", x1: "0", x2: "{W}", y1: "{HEAD - 4.0}", y2: "{HEAD - 4.0}" }
                for (tick_x, tick) in ticks.iter().step_by(every) {
                    line { class: "gantt-grid", stroke: "currentColor", stroke_opacity: "0.2", x1: "{tick_x:.1}", x2: "{tick_x:.1}", y1: "{HEAD - 4.0}", y2: "{h}" }
                    // a label the today line would cross is left out, its line kept; one near the right edge ends at its line
                    if (tick_x - tx).abs() > 28.0 {
                        if *tick_x > W - RIGHT - 44.0 {
                            text { class: "gantt-tick", fill: "currentColor", font_size: "12", x: "{tick_x - 3.0:.1}", y: "{HEAD - 10.0}", text_anchor: "end", "{tick}" }
                        } else {
                            text { class: "gantt-tick", fill: "currentColor", font_size: "12", x: "{tick_x + 3.0:.1}", y: "{HEAD - 10.0}", "{tick}" }
                        }
                    }
                }
                for (slot, name) in &heads {
                    text { class: "gantt-group", fill: "currentColor", font_size: "12", font_weight: "600", x: "8", y: "{place(*slot) + 5.0:.1}", "{name}" }
                }
                for (i, (r, span)) in rows.iter().zip(&spans).enumerate() {
                    g { class: joined("gantt-row", r.mark.class().unwrap_or_default()),
                        title { "{r.name}: " {r.start.as_deref().map_or(String::new(), |s| format!("{s} to "))} "{r.target}" }
                        if let Some(href) = &r.href {
                            a { href: "{href}",
                                text { class: "gantt-label", fill: "currentColor", font_size: "14", x: "8", y: "{mid(i) + 5.0:.1}", {label(&r.name)} }
                            }
                        } else {
                            text { class: "gantt-label", fill: "currentColor", font_size: "14", x: "8", y: "{mid(i) + 5.0:.1}", {label(&r.name)} }
                        }
                        match (span, &r.start) {
                            (Some((d, _)), None) => {
                                let (cx, cy) = (x(*d), mid(i));
                                rsx! { path { class: joined("gantt-milestone", r.mark.class().unwrap_or_default()), "color": late_colour(r.mark == Mark::Failed), fill: "currentColor", d: "M{cx:.1},{cy - 7.0:.1} L{cx + 7.0:.1},{cy:.1} L{cx:.1},{cy + 7.0:.1} L{cx - 7.0:.1},{cy:.1} Z" } }
                            }
                            (Some((from, to)), Some(_)) => rsx! {
                                rect {
                                    class: joined("gantt-bar", r.mark.class().unwrap_or_default()),
                                    "color": late_colour(r.mark == Mark::Failed),
                                    fill: "currentColor",
                                    fill_opacity: if r.mark == Mark::Next { "0.25" } else { "1" },
                                    stroke: "currentColor",
                                    x: "{x(*from):.1}", y: "{mid(i) - 9.0:.1}", width: "{(x(*to) - x(*from)).max(4.0):.1}", height: "18", rx: "4",
                                }
                            },
                            (None, _) => rsx! {},
                        }
                    }
                }
                for (d, is_late) in arrows {
                    path {
                        class: if is_late { "gantt-arrow late" } else { "gantt-arrow" },
                        "color": late_colour(is_late),
                        d, fill: "none", stroke: "currentColor", stroke_width: "1.5", stroke_dasharray: "3 3",
                        "marker-end": if is_late { "url(#{late})" } else { "url(#{arrow})" },
                    }
                }
                g { class: "gantt-today",
                    line { stroke: "currentColor", stroke_width: "1.5", x1: "{tx:.1}", x2: "{tx:.1}", y1: "14", y2: "{h}" }
                    text { fill: "currentColor", font_size: "11", x: "{tx:.1}", y: "10", text_anchor: "middle", "Today" }
                }
            }
            pre { class: "sr-only", "{text}" }
        }
    }
}

/// A failed row's or a late arrow's colour as an attribute, the light theme's error: Blitz draws an inline svg as one image no
/// CSS reaches, so there late would look on time; a browser's CSS (`.gantt-bar.failed`) wins over it (/code-review).
fn late_colour(late: bool) -> Option<&'static str> {
    late.then_some("#a8321e")
}

/// A Gantt's text form: a Mermaid gantt of the same rows, in their order (rust/CLAUDE.md rule 4: « diagrams etc. need to have a
/// text form. say mermaid for example. »). Each row keeps its own days; what it waits on is the Mermaid comment line above it
/// (Mermaid's `after` would move its start), today a comment too. A milestone is `milestone`, a done row `done`, a failed
/// one `crit`; a row whose day does not read is a comment, so no row goes missing.
pub fn gantt_mermaid(rows: &[GanttRow], today: i64) -> String {
    // Mermaid splits a task on « : » and « , » and reads « # » and « ; » as its own; a name or an id keeps none of them. A
    // line opening on one of its words (« Section rework ») is that statement, so such a name, or none, opens on « · »; an id
    // that is one of its tags (« done »), or none, opens on « id_ » (/code-review).
    const WORDS: [&str; 16] = [
        "gantt",
        "section",
        "title",
        "dateformat",
        "axisformat",
        "tickinterval",
        "excludes",
        "includes",
        "todaymarker",
        "weekday",
        "weekend",
        "inclusiveenddates",
        "topaxis",
        "displaymode",
        "acctitle",
        "accdescr",
    ];
    let name = |s: &str| {
        let s = s
            .replace([':', ';', '#', ',', '\n', '\r'], " ")
            .trim()
            .to_string();
        let lower = s.to_lowercase();
        match s.is_empty() || s.starts_with("%%") || WORDS.iter().any(|w| lower.starts_with(w)) {
            true => format!("· {s}").trim_end().to_string(),
            false => s,
        }
    };
    let token = |s: &str| {
        let t = s.replace(|c: char| !c.is_alphanumeric() && c != '_' && c != '-', "_");
        match t.is_empty()
            || ["done", "active", "crit", "milestone", "after", "until"]
                .contains(&t.to_lowercase().as_str())
        {
            true => format!("id_{t}"),
            false => t,
        }
    };
    let mut out = format!(
        "gantt\n  dateFormat YYYY-MM-DD\n  inclusiveEndDates\n  %% today {}\n",
        dash_core::date(today)
    );
    for (i, r) in rows.iter().enumerate() {
        // a group a Mermaid section, as the drawing names it once above its run
        if let Some(g) = r.group.as_deref()
            && (i == 0 || rows[i - 1].group.as_deref() != Some(g))
        {
            out += &format!("  section {}\n", name(g));
        }
        if !r.after.is_empty() {
            let after: Vec<String> = r.after.iter().map(|a| token(a)).collect();
            out += &format!("  %% {} after {}\n", token(&r.id), after.join(" "));
        }
        // what its name opens, for an agent as for him (/plan's panel, task linear-projects-view)
        if let Some(href) = &r.href {
            out += &format!(
                "  %% {} opens {}\n",
                token(&r.id),
                href.replace(['\n', '\r'], " ")
            );
        }
        let Some((from, to)) = r.span() else {
            let start = r
                .start
                .as_deref()
                .map_or(String::new(), |s| format!("{} to ", name(s)));
            out += &format!(
                "  %% {}: no day ({start}{})\n",
                name(&r.name),
                name(&r.target)
            );
            continue;
        };
        let mut tags: Vec<String> = match r.mark {
            Mark::Done => vec!["done".into()],
            Mark::Failed => vec!["crit".into()],
            Mark::Next => vec![],
        };
        let days = match r.start {
            None => {
                tags.push("milestone".into());
                format!("{}, 0d", dash_core::date(to))
            }
            Some(_) => format!("{}, {}", dash_core::date(from), dash_core::date(to)),
        };
        tags.push(token(&r.id));
        out += &format!("  {} :{}, {days}\n", name(&r.name), tags.join(", "));
    }
    out
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

    fn rows() -> Vec<GanttRow> {
        let row = |id: &str,
                   name: &str,
                   start: Option<&str>,
                   target: &str,
                   after: &[&str],
                   mark: Mark| GanttRow {
            id: id.into(),
            name: name.into(),
            start: start.map(String::from),
            target: target.into(),
            after: after.iter().map(|a| a.to_string()).collect(),
            mark,
            group: None,
            href: None,
        };
        vec![
            row(
                "gantt",
                "The Gantt",
                Some("2026-11-01"),
                "2026-11-08",
                &[],
                Mark::Done,
            ),
            row(
                "model",
                "Plan: data model",
                Some("2026-11-09"),
                "2026-11-20",
                &["gantt"],
                Mark::Next,
            ),
            row(
                "sync",
                "Linear sync",
                Some("2026-11-02"),
                "2026-11-25",
                &[],
                Mark::Failed,
            ),
            row(
                "ships",
                "/plan ships",
                None,
                "2026-11-21",
                &["model", "sync"],
                Mark::Next,
            ),
            row("odd", "No day yet", None, "soon", &[], Mark::Next),
        ]
    }

    /// Done-when 2 of rust-gantt-component: a Gantt's text form is a Mermaid gantt of its rows, in order, each with its own days
    /// and mark, a milestone, what it waits on and today as comments; Mermaid reads it (the renderer draws it), and the page's
    /// text form is that source, the drawing left out.
    #[test]
    fn a_gantt_s_text_form_is_a_mermaid_gantt_of_the_same_rows() {
        let _budget = TestBudget::new();
        let today = dash_core::days("2026-11-11").expect("a day");
        let mermaid = gantt_mermaid(&rows(), today);
        assert_eq!(
            mermaid,
            "gantt\n  dateFormat YYYY-MM-DD\n  inclusiveEndDates\n  %% today 2026-11-11\n  The Gantt :done, gantt, 2026-11-01, 2026-11-08\n  %% model after gantt\n  Plan  data model :model, 2026-11-09, 2026-11-20\n  Linear sync :crit, sync, 2026-11-02, 2026-11-25\n  %% ships after model sync\n  /plan ships :milestone, ships, 2026-11-21, 0d\n  %% No day yet: no day (soon)\n"
        );
        // mermaid-rs-renderer reads any line starting « gantt » as the header, so the test's names start otherwise
        let svg = super::super::diagram::diagram_svg(&mermaid).expect("Mermaid reads it");
        let drawn: Vec<bool> = [
            "The Gantt",
            "Plan  data model",
            "Linear sync",
            "/plan ships",
        ]
        .iter()
        .map(|n| svg.contains(&format!(">{n}</tspan>")))
        .collect();
        assert_eq!(drawn, [true; 4], "{svg}");
        let html = dioxus_ssr::render_element(rsx! { Gantt { id: "g", rows: rows(), today } });
        assert_eq!(crate::forms::text_of(&html), mermaid);
    }

    /// Done-when 1 of rust-gantt-component, the drawing: a row an item, its bar from start to target on one scale, the today
    /// line between them, a dotted arrow from each row it waits on (late when that row ends after it starts), a milestone a
    /// diamond, a row with no day its name alone.
    #[test]
    fn a_gantt_draws_a_bar_a_row_today_its_arrows_and_a_milestone() {
        let _budget = TestBudget::new();
        let today = dash_core::days("2026-11-11").expect("a day");
        let html = dioxus_ssr::render_element(rsx! { Gantt { id: "g", rows: rows(), today } });
        // the axis runs 3 days either side: 2026-10-29 to 2026-11-29 (the end of 11-25 and 3 more), 31 days on 536 units from x 168
        let x = |d: &str| {
            168.0
                + (dash_core::days(d).expect("a day")
                    - dash_core::days("2026-10-29").expect("a day")) as f64
                    * 536.0
                    / 31.0
        };
        let at = |d: &str| format!("{:.1}", x(d));
        let w = |a: &str, b: &str| format!("{:.1}", x(b) - x(a));
        let names: Vec<_> = [
            "The Gantt",
            "Plan: data model",
            "Linear sync",
            "/plan ships",
            "No day yet",
        ]
        .iter()
        .map(|n| html.matches(&format!(">{n}</text>")).count())
        .collect();
        assert_eq!(names, [1; 5], "a row an item: {html}");
        assert_eq!(html.matches("<rect").count(), 3, "three bars: {html}");
        // Dioxus writes an element's fixed attributes first: each bar is read attribute by attribute
        let bars: Vec<&str> = html
            .split("<rect")
            .skip(1)
            .map(|r| &r[..r.find('>').expect("a tag")])
            .collect();
        for (bar, (mark, a, b, y)) in bars.iter().zip([
            // each bar runs to the end of its target day, the next day's line
            ("", "2026-11-01", "2026-11-09", "41.0"),
            (" next", "2026-11-09", "2026-11-21", "73.0"),
            (" failed", "2026-11-02", "2026-11-26", "105.0"),
        ]) {
            let opacity = if mark == " next" { "0.25" } else { "1" };
            for want in [
                format!(r#"class="gantt-bar{mark}""#),
                format!(r#"fill-opacity="{opacity}""#),
                format!(r#"x="{}""#, at(a)),
                format!(r#"y="{y}""#),
                format!(r#"width="{}""#, w(a, b)),
            ] {
                assert!(bar.contains(&want), "{want} in {bar}");
            }
        }
        let t = at("2026-11-11");
        assert!(html.contains(&format!(r#"<g class="gantt-today"><line stroke="currentColor" stroke-width="1.5" y1="14" x1="{t}" x2="{t}""#)), "{html}");
        // gantt ends 11-08, model starts 11-09: on time; model ends 11-20, ships 11-21: on time; sync ends 11-25 after: late
        assert_eq!(
            html.matches(r#" class="gantt-arrow"#).count(),
            2 + 3,
            "two markers, three arrows: {html}"
        );
        assert_eq!(
            html.matches(r#"stroke-dasharray="3 3""#).count(),
            3,
            "dotted: {html}"
        );
        assert_eq!(
            html.matches(r#"marker-end="url(#g-late)""#).count(),
            1,
            "{html}"
        );
        assert_eq!(
            html.matches(r#"marker-end="url(#g-arrow)""#).count(),
            2,
            "{html}"
        );
        // a milestone at the end of its day, 2026-11-21
        let m = at("2026-11-22");
        assert!(
            html.contains(&format!(
                r#"<path fill="currentColor" class="gantt-milestone next" d="M{m},139.0 L"#
            )),
            "{html}"
        );
        let svg = &html[..html.find("<pre").expect("the text")];
        assert!(
            !svg.contains(">No day yet</text><path") && !svg.contains(">No day yet</text><rect"),
            "no day, no mark: {html}"
        );
    }

    /// /code-review of rust-gantt-component: a row fed with no mark is not done; a row starting on the day its predecessor ends
    /// is late; a row starting after its target, or an id two rows share, never draws a wrong bar or arrow; a name or an id
    /// that is a Mermaid word, or a target holding a new line, stays one row of the text.
    #[test]
    fn a_gantt_reads_its_rows_strictly() {
        let _budget = TestBudget::new();
        let today = dash_core::days("2026-11-11").expect("a day");
        let fed: GanttRow =
            serde_json::from_str(r#"{"id": "a", "name": "A", "target": "2026-11-12"}"#)
                .expect("a row");
        assert_eq!(fed.mark, Mark::Next);
        let row = |id: &str, name: &str, start: &str, target: &str, after: &[&str]| GanttRow {
            id: id.into(),
            name: name.into(),
            start: Some(start.into()),
            target: target.into(),
            after: after.iter().map(|a| a.to_string()).collect(),
            mark: Mark::Next,
            group: None,
            href: None,
        };
        let rows = vec![
            row("a", "A", "2026-11-02", "2026-11-09", &[]),
            row("b", "B", "2026-11-09", "2026-11-12", &["a"]),
            row("b", "Section rework", "2026-11-20", "2026-11-10", &["a"]),
            row("done", "", "2026-11-13", "2026-11-14", &["b"]),
            row("b", "B again", "2026-11-15", "2026-11-16", &[]),
        ];
        let html =
            dioxus_ssr::render_element(rsx! { Gantt { id: "g", rows: rows.clone(), today } });
        // b starts on a's last day: late, read from b's own start, never « B again »'s, which shares its id; the swapped row
        // draws nothing; « done » waits on the last drawn b, « B again », which ends after it starts: late too
        assert_eq!(html.matches("<rect").count(), 4, "{html}");
        assert_eq!(
            html.matches(r#"marker-end="url(#g-late)""#).count(),
            2,
            "{html}"
        );
        assert_eq!(
            html.matches(r#"marker-end="url(#g-arrow)""#).count(),
            0,
            "{html}"
        );
        let mut odd = rows[2].clone();
        odd.target = "soon\nsection Hacked".into();
        let mermaid = gantt_mermaid(&[rows[2].clone(), rows[3].clone(), odd], today);
        assert_eq!(
            mermaid,
            "gantt\n  dateFormat YYYY-MM-DD\n  inclusiveEndDates\n  %% today 2026-11-11\n  %% b after a\n  %% · Section rework: no day (2026-11-20 to 2026-11-10)\n  %% id_done after b\n  · :id_done, 2026-11-13, 2026-11-14\n  %% b after a\n  %% · Section rework: no day (2026-11-20 to soon section Hacked)\n"
        );
    }
}
