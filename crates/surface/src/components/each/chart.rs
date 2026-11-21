use super::treemap::grouped;
use crate::component::{Component, Section};
use dioxus::prelude::*;

/// One named run of a chart's values, a value a label (rust-ui's `series_names` and `json_values`, notice in NOTICE-rust-ui).
#[derive(Clone, PartialEq, Debug)]
pub struct ChartSeries {
    pub name: String,
    pub values: Vec<f64>,
}

/// How a line or an area runs from one value to the next: rust-ui's ChartCurve.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ChartCurve {
    #[default]
    Smooth,
    Straight,
    Stepline,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Chart {
    Area,
    Bar,
    Line,
}

inventory::submit! {
    Component {
        slug: "area_chart",
        classes: &["chart"],
        tags: &[],
        name: "AreaChart",
        used: "A trend filled to zero, one colour a series: /design's Charts tab. Built on rust-ui's AreaChart, drawn on the server.",
        // on /design's Charts tab, first (task design-charts-tab)
        sections: &[Section {
            family: "charts",
            id: "area",
            title: "Area",
            after: "",
            rows: &[("Two series, a colour each", |_| {
                let (labels, series) = sent_and_replied();
                rsx! { AreaChart { id: "c-area", axis: "Week", labels, series } }
            })],
        }],
        library: Some("https://rust-ui.com/charts/area-chart"),
        terminal: Some(crate::component::Terminal::Drawn(|_| {
            let (labels, series) = sent_and_replied();
            chart_terminal(Chart::Area, &labels, &series)
        })),
        ..Component::BARE
    }
}

/// Series filled down to zero, a gradient unless `gradient` is false: rust-ui's AreaChart (task rust-charts-rust-ui; @nha,
/// 2026-11-10: « https://rust-ui.com/charts/area-chart let's replace our charts with these in our design system »). rust-ui
/// hands its values to ApexCharts in the browser; here the server draws the SVG, so the page runs no script and loads nothing
/// remote, and the text is the values as a table. Its ideas kept: values under labels, curve, gradient fill, a y axis shown
/// unless asked not to, a grid only when asked, negatives in the muted colour; the first series the primary colour.
#[component]
pub fn AreaChart(
    #[props(into)] id: String,
    /// The labels' column name in the table: « Week », « Month ».
    #[props(into)]
    axis: String,
    labels: Vec<String>,
    series: Vec<ChartSeries>,
    #[props(default)] curve: ChartCurve,
    #[props(default = true)] gradient: bool,
    #[props(default = true)] show_yaxis: bool,
    #[props(default)] show_grid: bool,
) -> Element {
    let look = Look {
        kind: Chart::Area,
        curve,
        gradient,
        show_yaxis,
        show_grid,
        spark: false,
    };
    chart(&id, &axis, &labels, &series, look)
}

inventory::submit! {
    Component {
        slug: "bar_chart",
        classes: &[],
        tags: &[],
        name: "BarChart",
        used: "A count a period or a stage, a negative one muted: /design's Charts tab. Built on rust-ui's BarChart, drawn on the server.",
        sections: &[Section {
            family: "charts",
            id: "bars",
            title: "Bars",
            after: "area",
            rows: &[
                ("A month's net, a negative one muted", |_| {
                    let (labels, series) = net();
                    rsx! { BarChart { id: "c-bar", axis: "Month", labels, series } }
                }),
                ("Per week, on a grid", |_| {
                    let values = vec![6.0, 8.0, 4.0, 11.0, 7.0, 9.0, 9.0, 5.0];
                    let labels = (1..=values.len()).map(|w| format!("W{w}")).collect();
                    let series = vec![ChartSeries { name: "Applied".into(), values }];
                    rsx! { BarChart { id: "ds-weeks", axis: "Week", labels, series, show_grid: true } }
                }),
            ],
        }],
        library: Some("https://rust-ui.com/charts/bar-chart"),
        terminal: Some(crate::component::Terminal::Drawn(|_| {
            let (labels, series) = net();
            chart_terminal(Chart::Bar, &labels, &series)
        })),
        ..Component::BARE
    }
}

/// A bar a value from zero, a negative one in the muted colour, its top rounded: rust-ui's BarChart. Several series stand
/// side by side in their label's slot.
#[component]
pub fn BarChart(
    #[props(into)] id: String,
    #[props(into)] axis: String,
    labels: Vec<String>,
    series: Vec<ChartSeries>,
    #[props(default = true)] show_yaxis: bool,
    #[props(default)] show_grid: bool,
) -> Element {
    let look = Look {
        kind: Chart::Bar,
        curve: ChartCurve::Straight,
        gradient: false,
        show_yaxis,
        show_grid,
        spark: false,
    };
    chart(&id, &axis, &labels, &series, look)
}

inventory::submit! {
    Component {
        slug: "line_chart",
        classes: &[],
        tags: &[],
        name: "LineChart",
        used: "A trend through points, a dot each; look spark is the sparkline beside a word: /design's Charts tab. Built on rust-ui's LineChart, drawn on the server.",
        sections: &[Section {
            family: "charts",
            id: "line",
            title: "Line",
            after: "bars",
            rows: &[("Straight, a dot a value", |_| {
                let (labels, series) = replies();
                rsx! { LineChart { id: "c-line", axis: "Day", labels, series, curve: ChartCurve::Straight } }
            })],
        }],
        library: Some("https://rust-ui.com/charts/line-chart"),
        // its Chart, then its look spark, ratatui's Sparkline, beside its word (task rust-tui-app)
        terminal: Some(crate::component::Terminal::Drawn(|_| {
            let (labels, series) = replies();
            let mut text = chart_terminal(Chart::Line, &labels, &series);
            let spark = spark_terminal(&series).lines.into_iter().next().unwrap_or_default();
            let mut beside = ratatui::text::Line::from("Look spark: ");
            beside.spans.extend(spark.spans);
            text.lines.push(beside);
            text
        })),
        ..Component::BARE
    }
}

/// A line through each value, a dot on each: rust-ui's LineChart. Look `spark`: a sparkline, the line alone at a word's
/// height, no axis, grid or dot.
#[component]
pub fn LineChart(
    #[props(into)] id: String,
    #[props(into)] axis: String,
    labels: Vec<String>,
    series: Vec<ChartSeries>,
    #[props(default)] curve: ChartCurve,
    #[props(default = true)] show_yaxis: bool,
    #[props(default)] show_grid: bool,
    #[props(default)] look: &'static str,
) -> Element {
    let spark = look == "spark";
    let look = Look {
        kind: Chart::Line,
        curve,
        gradient: false,
        show_yaxis: show_yaxis && !spark,
        show_grid: show_grid && !spark,
        spark,
    };
    chart(&id, &axis, &labels, &series, look)
}

inventory::submit! {
    Component {
        slug: "progress",
        classes: &["progress", "progress-label", "progress-track", "progress-indicator"],
        tags: &[],
        name: "Progress",
        used: "How far a count has gone of its whole, its words beside it: the Charts tab's share bar, a lead's chances. Built \
               on dioxus-components' Progress; in the terminal ratatui's Gauge.",
        // the Charts tab's share bar, after the sparkline (task rust-tui-app)
        sections: &[Section {
            family: "charts",
            id: "share",
            title: "Share bar",
            after: "sparkline",
            rows: &[("Done of all", |_| {
                rsx! { Progress { id: "ds-done", value: 8.0, max: 12.0, label: "8 of 12 done, 4 left" } }
            })],
        }],
        css: ".progress { display: flex; align-items: center; gap: var(--spacing-s2); max-width: 640px; }\n.progress-label { flex: none; font-size: 14px; }\n.progress-track { flex: 1; position: relative; overflow: hidden; height: 8px; border-radius: 999px; background: var(--color-base-300); }\n.progress-indicator { height: 100%; border-radius: inherit; background: var(--color-primary); }\n",
        terminal: Some(crate::component::Terminal::Drawn(|_| progress_terminal(8.0, 12.0, "8 of 12 done, 4 left"))),
        ..Component::BARE
    }
}

/// `value` of `max`, its track filled that far, `label` beside it: DioxusLabs/dioxus-components' Progress (MIT OR
/// Apache-2.0, notice kept in NOTICE-dioxus-components), its markup kept (a `progressbar` role carrying its values, its
/// indicator inside, `data-state` loading until full); its words shown, so its text form says them. A value past either end
/// is drawn at that end.
#[component]
pub fn Progress(
    #[props(into)] id: String,
    value: f64,
    max: f64,
    #[props(into)] label: String,
) -> Element {
    let ratio = ratio(value, max);
    let state = if ratio >= 1.0 { "complete" } else { "loading" };
    let width = format!("width: {:.1}%", ratio * 100.0);
    rsx! {
        div { id, class: "progress",
            span { class: "progress-label", "{label}" }
            div {
                class: "progress-track",
                role: "progressbar",
                aria_label: "{label}",
                aria_valuemin: "0",
                aria_valuemax: "{max}",
                aria_valuenow: "{value}",
                "data-state": state,
                div { class: "progress-indicator", style: width }
            }
        }
    }
}

/// `value` of `max` as a share from 0 to 1: none for no whole or no number.
fn ratio(value: f64, max: f64) -> f64 {
    match value / max {
        r if r.is_finite() => r.clamp(0.0, 1.0),
        _ => 0.0,
    }
}

/// Progress's terminal form, ratatui's Gauge: its track a line `WIDE` cells long, filled to its share in the theme's
/// primary colour, its words on it (task rust-tui-app).
pub fn progress_terminal(value: f64, max: f64, label: &str) -> ratatui::text::Text<'static> {
    use ratatui::widgets::Widget;
    let area = ratatui::layout::Rect::new(0, 0, WIDE, 1);
    let mut buf = ratatui::buffer::Buffer::empty(area);
    ratatui::widgets::Gauge::default()
        .ratio(ratio(value, max))
        .label(label.to_string())
        .gauge_style(tone(0))
        .use_unicode(true)
        .render(area, &mut buf);
    super::tui_screen::text_of(&buf)
}

#[derive(Clone, Copy)]
struct Look {
    kind: Chart,
    curve: ChartCurve,
    gradient: bool,
    show_yaxis: bool,
    show_grid: bool,
    spark: bool,
}

/// /design's charts, their web example and their terminal form drawn from the same values.
fn sent_and_replied() -> (Vec<String>, Vec<ChartSeries>) {
    let series = |name: &str, values: [f64; 4]| ChartSeries {
        name: name.into(),
        values: values.to_vec(),
    };
    let series = vec![
        series("Sent", [6.0, 8.0, 4.0, 11.0]),
        series("Replied", [2.0, 3.0, 1.0, 5.0]),
    ];
    (chart_labels(&["W1", "W2", "W3", "W4"]), series)
}

fn net() -> (Vec<String>, Vec<ChartSeries>) {
    let series = vec![ChartSeries {
        name: "Net".into(),
        values: vec![2500.0, -230.0, 1800.0],
    }];
    (chart_labels(&["Jan", "Feb", "Mar"]), series)
}

fn replies() -> (Vec<String>, Vec<ChartSeries>) {
    let series = vec![ChartSeries {
        name: "Replies".into(),
        values: vec![3.0, 5.0, 2.0],
    }];
    (chart_labels(&["Mon", "Tue", "Wed"]), series)
}

/// The series' colours, in order, as components.css's `.chart-s<n>` take them: a negative bar the second's.
const TONES: [&str; 3] = ["primary", "secondary", "info"];

/// Series `i`'s colour in the terminal: the theme's (crate::theme::tone).
fn tone(i: usize) -> ratatui::style::Style {
    ratatui::style::Style::new().fg(crate::theme::tone(TONES[i % TONES.len()]))
}

/// A chart's size in the terminal: as wide and as tall as a card of the page.
const WIDE: u16 = 48;
const TALL: u16 = 10;

/// A chart's terminal form, the widget ratatui has for it, each series in its theme colour (task rust-tui-app; @nha,
/// 2026-11-13: « https://ratatui.rs/concepts/widgets/ is a good start for charts in terminal »): a line or an area its Chart,
/// braille dots, a key when it has two series, the first and last labels under it; bars its BarChart, a group a label, each
/// value written on its bar, a negative one's height its size in the muted colour, as the web's. ponytail: an area is drawn as
/// its line, ratatui's Chart fills nothing under one.
pub fn chart_terminal(
    kind: Chart,
    labels: &[String],
    series: &[ChartSeries],
) -> ratatui::text::Text<'static> {
    use ratatui::layout::Rect;
    use ratatui::style::Style;
    use ratatui::symbols::Marker;
    use ratatui::widgets::{Axis, Bar, BarGroup, Dataset, GraphType, Widget};
    let mut buf = ratatui::buffer::Buffer::empty(Rect::new(0, 0, WIDE, TALL));
    let label = |i: usize| labels.get(i).cloned().unwrap_or_default();
    match kind {
        Chart::Bar => {
            let groups = (0..labels.len()).map(|i| {
                let bars: Vec<Bar> = (series.iter().enumerate())
                    .filter_map(|(k, s)| s.values.get(i).map(|v| (k, *v)))
                    .filter(|(_, v)| v.is_finite())
                    .map(|(k, v)| {
                        let tone = tone(if v < 0.0 { 1 } else { k });
                        Bar::new(v.abs().round() as u64)
                            .text_value(chart_number(v))
                            .style(tone)
                            .value_style(tone.reversed())
                    })
                    .collect();
                BarGroup::new(bars).label(label(i))
            });
            let width = (WIDE / labels.len().max(1) as u16).saturating_sub(1).max(1)
                / series.len().max(1) as u16;
            ratatui::widgets::BarChart::grouped(groups.collect::<Vec<_>>())
                .bar_width(width.max(1))
                // a group's bars side by side, a cell between groups: each slot `WIDE / labels` wide
                .bar_gap(0)
                .group_gap(1)
                .render(buf.area, &mut buf);
        }
        Chart::Area | Chart::Line => {
            let points: Vec<Vec<(f64, f64)>> = (series.iter())
                .map(|s| {
                    (s.values.iter().enumerate())
                        .filter(|(_, v)| v.is_finite())
                        .map(|(i, v)| (i as f64, *v))
                        .collect()
                })
                .collect();
            let ys = points.iter().flatten().map(|(_, y)| *y);
            let (low, high) = ys.fold((0.0_f64, 0.0_f64), |(l, h), y| (l.min(y), h.max(y)));
            let last = points
                .iter()
                .map(Vec::len)
                .max()
                .unwrap_or(1)
                .saturating_sub(1);
            let sets = (points.iter().zip(series).enumerate()).map(|(k, (p, s))| {
                Dataset::default()
                    .name(s.name.clone())
                    .marker(Marker::Braille)
                    .graph_type(GraphType::Line)
                    .style(tone(k))
                    .data(p)
            });
            let muted = Style::new().fg(crate::theme::tone("neutral-content")).dim();
            let x = Axis::default()
                .bounds([0.0, last.max(1) as f64])
                .labels([label(0), label(last)])
                .style(muted);
            let y = Axis::default()
                .bounds([low, high.max(low + 1.0)])
                .labels([chart_number(low), chart_number(high)])
                .style(muted);
            // the key shown up to half the chart, as ratatui hides one past a quarter
            let half = ratatui::layout::Constraint::Ratio(1, 2);
            let chart = (ratatui::widgets::Chart::new(sets.collect())
                .x_axis(x)
                .y_axis(y))
            .hidden_legend_constraints((half, half));
            // one series: no key, as the web draws none
            let chart = match series.len() > 1 {
                true => chart,
                false => chart.legend_position(None),
            };
            chart.render(buf.area, &mut buf);
        }
    }
    super::tui_screen::text_of(&buf)
}

/// A spark's terminal form, ratatui's Sparkline: the first series' values a bar of eighths each, a cell each, in its colour;
/// a negative one drawn as none, as a spark shows the trend.
pub fn spark_terminal(series: &[ChartSeries]) -> ratatui::text::Text<'static> {
    use ratatui::widgets::Widget;
    let values: Vec<u64> = (series.first().map(|s| s.values.as_slice()))
        .unwrap_or_default()
        .iter()
        .map(|v| {
            if v.is_finite() {
                v.max(0.0).round() as u64
            } else {
                0
            }
        })
        .collect();
    let area = ratatui::layout::Rect::new(0, 0, values.len().max(1) as u16, 1);
    let mut buf = ratatui::buffer::Buffer::empty(area);
    ratatui::widgets::Sparkline::default()
        .data(&values)
        .style(tone(0))
        .render(area, &mut buf);
    super::tui_screen::text_of(&buf)
}

/// Labels from words: a chart's `labels`.
pub fn chart_labels(words: &[&str]) -> Vec<String> {
    words.iter().map(|w| w.to_string()).collect()
}

/// A value as its table cell and axis label: grouped by thousands, up to two decimals, nothing for no number (NaN).
fn chart_number(v: f64) -> String {
    if !v.is_finite() {
        return String::new();
    }
    let cents = (v.abs() * 100.0).round();
    if cents >= u64::MAX as f64 {
        return format!("{v}");
    }
    let cents = cents as u64;
    let sign = if v < 0.0 && cents > 0 { "-" } else { "" };
    let decimals = match cents % 100 {
        0 => String::new(),
        c => format!(".{c:02}").trim_end_matches('0').to_string(),
    };
    format!("{sign}{}{decimals}", grouped(cents / 100))
}

/// The y axis's ticks: a round step (1, 2 or 5 a power of ten) giving at most five gaps from zero, or from the lowest value
/// to the highest for a sparkline, whose trend is the point (/code-review).
fn chart_ticks(values: impl Iterator<Item = f64>, spark: bool) -> Vec<f64> {
    let finite: Vec<f64> = values.filter(|v| v.is_finite()).collect();
    let from_zero = if spark { f64::NAN } else { 0.0 };
    let lo = finite.iter().copied().fold(from_zero, f64::min);
    let hi = finite.iter().copied().fold(from_zero, f64::max);
    if lo.is_nan() || hi <= lo {
        // no value, or all equal: one step up
        let lo = if lo.is_nan() { 0.0 } else { lo };
        return vec![lo, lo + 1.0];
    }
    if spark {
        return vec![lo, hi];
    }
    let mag = 10f64.powf(((hi - lo) / 5.0).log10().floor());
    let gaps = |s: f64| ((lo / s).floor() as i64, (hi / s).ceil() as i64);
    let step = [1.0, 2.0, 5.0, 10.0, 20.0]
        .into_iter()
        .map(|m| m * mag)
        .find(|&s| gaps(s).1 - gaps(s).0 <= 5)
        .unwrap_or(20.0 * mag);
    let (from, to) = gaps(step);
    (from..=to).map(|k| k as f64 * step).collect()
}

/// A path through the points: straight, in steps, or smooth (Catmull-Rom's curve as Béziers, near ApexCharts' « smooth »).
fn chart_path(points: &[(f64, f64)], curve: ChartCurve) -> String {
    let mut d = String::new();
    for (i, &(x, y)) in points.iter().enumerate() {
        if i == 0 {
            d += &format!("M{x:.1},{y:.1}");
            continue;
        }
        let (px, py) = points[i - 1];
        d += &match curve {
            ChartCurve::Straight => format!(" L{x:.1},{y:.1}"),
            ChartCurve::Stepline => format!(" H{x:.1} V{y:.1}"),
            ChartCurve::Smooth => {
                let (bx, by) = points[i.saturating_sub(2)];
                let (nx, ny) = points[(i + 1).min(points.len() - 1)];
                // each handle held between its segment's ends, so the curve never overshoots a value or the zero line
                let hold = |c: f64| c.clamp(py.min(y), py.max(y));
                let (c1x, c1y) = (px + (x - bx) / 6.0, hold(py + (y - by) / 6.0));
                let (c2x, c2y) = (x - (nx - px) / 6.0, hold(y - (ny - py) / 6.0));
                format!(" C{c1x:.1},{c1y:.1} {c2x:.1},{c2y:.1} {x:.1},{y:.1}")
            }
        };
    }
    d
}

/// A bar from `base` to `y`, its far end rounded as rust-ui's (`borderRadius: 4`, at the end).
fn chart_bar(x: f64, w: f64, base: f64, y: f64) -> String {
    let h = (base - y).abs();
    let r = 4f64.min(w / 2.0).min(h);
    let dir = if y <= base { 1.0 } else { -1.0 };
    let (yr, right) = (y + dir * r, x + w);
    format!(
        "M{x:.1},{base:.1} V{yr:.1} Q{x:.1},{y:.1} {:.1},{y:.1} H{:.1} Q{right:.1},{y:.1} {right:.1},{yr:.1} V{base:.1} Z",
        x + r,
        right - r
    )
}

/// Where each series' key sits: (series, x, y), a new line when the next would pass `end` (/code-review: long names ran
/// past the chart).
fn chart_key(series: &[ChartSeries], left: f64, end: f64) -> Vec<(usize, f64, f64)> {
    let (mut x, mut y) = (left, 6.0);
    let mut out = vec![];
    for (k, s) in series.iter().enumerate() {
        // ponytail: a name's width guessed at 10 units a letter, a text measure if names run long
        let width = 36.0 + 10.0 * s.name.chars().count() as f64;
        if x > left && x + width > end {
            (x, y) = (left, y + 22.0);
        }
        out.push((k, x, y));
        x += width;
    }
    out
}

// Every mark's colour is an attribute in `currentColor` too: Blitz draws an inline svg as one image that no CSS reaches
// (icon()), so there each mark takes the chart's colour; a browser's CSS gives each series its own.
fn chart(id: &str, axis: &str, labels: &[String], series: &[ChartSeries], look: Look) -> Element {
    let (w, h) = if look.spark {
        (160.0, 44.0)
    } else {
        (600.0, 220.0)
    };
    // every value drawn and listed, a label short of one left blank (/code-review: a value past the labels was dropped)
    let n = (series.iter().map(|s| s.values.len()))
        .chain([labels.len()])
        .max()
        .unwrap_or(0);
    let label = |i: usize| labels.get(i).map_or("", String::as_str);
    let left = match (look.spark, look.show_yaxis) {
        (true, _) => 3.0,
        (false, true) => 60.0,
        (false, false) => 16.0,
    };
    let right = if look.spark { 3.0 } else { 16.0 };
    // more than one series: a key over the plot, rust-ui's legend, each name beside its colour
    let key = match series.len() > 1 && !look.spark {
        true => chart_key(series, left, w - right),
        false => vec![],
    };
    let (top, bottom) = match (look.spark, key.last()) {
        (true, _) => (3.0, 3.0),
        (false, Some((_, _, y))) => (y + 28.0, 30.0),
        (false, None) => (12.0, 30.0),
    };
    let (pw, ph) = (w - left - right, (h - top - bottom).max(1.0));
    let ticks = chart_ticks(
        series.iter().flat_map(|s| s.values.iter().copied()),
        look.spark,
    );
    let (lo, hi) = (ticks[0], ticks[ticks.len() - 1]);
    let sy = |v: f64| top + ph * (hi - v.clamp(lo, hi)) / (hi - lo);
    let bar = look.kind == Chart::Bar;
    let sx = |i: usize| match (bar, n) {
        (true, _) => left + pw * (i as f64 + 0.5) / n as f64,
        (false, 0 | 1) => left + pw / 2.0,
        (false, _) => left + pw * i as f64 / (n - 1) as f64,
    };
    let base = sy(lo.max(0.0).min(hi));
    // a label every so often past twelve, as rust-ui's tick amount
    let every = n.div_ceil(12).max(1);
    let ruled: Vec<(f64, String)> = ticks.iter().map(|&t| (sy(t), chart_number(t))).collect();
    let shown: Vec<(f64, &str, &str)> = (0..n)
        .step_by(every)
        .map(|i| {
            let anchor = match (bar, i) {
                (true, _) => "middle",
                (false, 0) if n > 1 => "start",
                (false, _) if i + 1 == n && n > 1 => "end",
                _ => "middle",
            };
            (sx(i), label(i), anchor)
        })
        .collect();
    let slot = if n == 0 { pw } else { pw / n as f64 };
    let bar_w = slot * 0.6 / series.len().max(1) as f64;
    let class = if look.spark { "chart spark" } else { "chart" };
    let view = format!("0 0 {w} {h}");
    rsx! {
        div { id, class,
            svg { class: "chart-svg", view_box: view, "aria-hidden": "true",
                if look.show_grid {
                    for (y, _) in ruled.iter() { line { class: "chart-grid", stroke: "currentColor", stroke_opacity: "0.2", x1: "{left}", x2: "{w - right}", y1: "{y:.1}", y2: "{y:.1}" } }
                }
                if look.show_yaxis {
                    for (y, t) in ruled.iter() { text { class: "chart-tick", fill: "currentColor", x: "{left - 8.0}", y: "{y + 5.0:.1}", text_anchor: "end", "{t}" } }
                }
                for (k, x, y) in key {
                    g { class: "chart-s{k.min(2)}",
                        rect { fill: "currentColor", x: "{x:.1}", y: "{y}", width: "12", height: "12", rx: "2" }
                        text { class: "chart-key", fill: "currentColor", x: "{x + 18.0:.1}", y: "{y + 11.0}", "{series[k].name}" }
                    }
                }
                if !look.spark {
                    line { class: "chart-base", stroke: "currentColor", stroke_opacity: "0.2", x1: "{left}", x2: "{w - right}", y1: "{base:.1}", y2: "{base:.1}" }
                    for (x, label, anchor) in shown { text { class: "chart-tick", fill: "currentColor", x: "{x:.1}", y: "{h - 8.0}", text_anchor: anchor, "{label}" } }
                }
                for (k, s) in series.iter().enumerate() {
                    g { class: "chart-s{k.min(2)}",
                        if bar {
                            for (i, v) in s.values.iter().enumerate().filter(|(_, v)| v.is_finite()) {
                                path {
                                    class: if *v < 0.0 { "chart-bar neg" } else { "chart-bar" },
                                    fill: "currentColor",
                                    d: chart_bar(sx(i) - slot * 0.3 + bar_w * k as f64, bar_w, base, sy(*v)),
                                    title { "{label(i)}: {chart_number(*v)}" }
                                }
                            }
                        } else {
                            {chart_run(id, k, s, labels, &sx, &sy, base, look)}
                        }
                    }
                }
            }
            table { class: "sr-only",
                thead { tr { th { "{axis}" } for s in series { th { "{s.name}" } } } }
                tbody {
                    for i in 0..n {
                        tr {
                            th { "{label(i)}" }
                            for s in series { td { {s.values.get(i).map_or(String::new(), |v| chart_number(*v))} } }
                        }
                    }
                }
            }
        }
    }
}

/// One series of a line or an area chart: its fill under it, then its line, then its dots. A value that is no number (NaN)
/// is left out, the line joining its neighbours.
#[allow(clippy::too_many_arguments)]
fn chart_run(
    id: &str,
    k: usize,
    s: &ChartSeries,
    labels: &[String],
    sx: &dyn Fn(usize) -> f64,
    sy: &dyn Fn(f64) -> f64,
    base: f64,
    look: Look,
) -> Element {
    let drawn: Vec<(usize, f64, f64)> = (s.values.iter().enumerate())
        .filter(|(_, v)| v.is_finite())
        .map(|(i, v)| (i, sx(i), sy(*v)))
        .collect();
    let points: Vec<(f64, f64)> = drawn.iter().map(|&(_, x, y)| (x, y)).collect();
    let line = chart_path(&points, look.curve);
    let fill = match (points.first(), points.last()) {
        (Some((x0, _)), Some((x1, _))) if look.kind == Chart::Area => {
            Some(format!("{line} L{x1:.1},{base:.1} L{x0:.1},{base:.1} Z"))
        }
        _ => None,
    };
    let fade = format!("{id}-fade{k}");
    let label = |i: usize| labels.get(i).map_or("", String::as_str);
    let dots = look.kind == Chart::Line && !look.spark;
    rsx! {
        if let Some(d) = fill {
            if look.gradient {
                linearGradient { id: "{fade}", x1: "0", x2: "0", y1: "0", y2: "1",
                    stop { offset: "0", "stop-color": "currentColor", "stop-opacity": "0.8" }
                    stop { offset: "0.9", "stop-color": "currentColor", "stop-opacity": "0.05" }
                }
                path { class: "chart-area", d, fill: "url(#{fade})" }
            } else {
                path { class: "chart-area", d, fill: "currentColor", fill_opacity: "0.4" }
            }
        }
        path { class: "chart-line", d: line, fill: "none", stroke: "currentColor", stroke_width: "2", stroke_linejoin: "round", stroke_linecap: "round" }
        if dots {
            for (i, x, y) in drawn {
                circle { class: "chart-dot", cx: "{x:.1}", cy: "{y:.1}", r: "3.5", fill: "none", stroke: "currentColor", stroke_width: "2",
                    title { "{label(i)}: {chart_number(s.values[i])}" }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::tone as theme;
    use dash_meter::dash_test;
    use ratatui::buffer::Buffer;

    /// The cells of a terminal form, drawn as /design draws it.
    fn cells(text: &ratatui::text::Text<'static>) -> Buffer {
        crate::components::drawn(text)
    }

    /// The symbols drawn in `colour` (a theme token's), each a cell.
    fn drawn_in(buf: &Buffer, colour: &str) -> Vec<String> {
        (buf.content.iter())
            .filter(|c| c.fg == theme(colour) && !c.symbol().trim().is_empty())
            .map(|c| c.symbol().to_string())
            .collect()
    }

    fn braille(s: &str) -> bool {
        s.chars().all(|c| ('\u{2801}'..='\u{28ff}').contains(&c))
    }

    /// Done-when 2 of rust-tui-app: the charts' terminal forms are ratatui's widgets, read off the buffer they draw. AreaChart
    /// and LineChart draw ratatui's Chart, braille dots in each series' theme colour, a key naming them when there are two, the
    /// first and last labels under it; BarChart its BarChart, a bar a value with the value on it, a negative one in the second
    /// colour, as the web's; a spark ratatui's Sparkline, its tallest value a full block; Progress ratatui's Gauge, filled to
    /// its share in the primary colour, its words on it.
    #[dash_test]
    fn the_charts_draw_ratatui_s_chart_bars_sparkline_and_gauge_in_theme_colours() {
        assert_eq!(
            theme("primary"),
            "#3b9e80".parse().expect("a colour"),
            "theme.css's dark primary"
        );
        // AreaChart: two series, two colours, a key, the labels at the ends
        let (labels, series) = sent_and_replied();
        let area = chart_terminal(Chart::Area, &labels, &series);
        let buf = cells(&area);
        let words = crate::tui::lines(&buf).join("\n");
        for (tone, name) in [("primary", "Sent"), ("secondary", "Replied")] {
            let dots = drawn_in(&buf, tone);
            assert!(
                dots.iter().filter(|d| braille(d)).count() > 5,
                "{tone}: {dots:?}\n{words}"
            );
            assert!(words.contains(name), "the key names {name}:\n{words}");
        }
        assert!(
            words.contains("W1") && words.contains("W4") && words.contains("11"),
            "{words}"
        );
        // LineChart: one series, no key
        let (labels, series) = replies();
        let line = cells(&chart_terminal(Chart::Line, &labels, &series));
        assert!(drawn_in(&line, "primary").iter().any(|d| braille(d)));
        assert!(
            !crate::tui::lines(&line).join("\n").contains("Replies"),
            "one series: no key"
        );
        // BarChart: a bar a value, the value written on it, the negative one in the second colour
        let (labels, series) = net();
        let bars = cells(&chart_terminal(Chart::Bar, &labels, &series));
        let words = crate::tui::lines(&bars).join("\n");
        for said in ["Jan", "Feb", "Mar", "2,500", "-230", "1,800"] {
            assert!(words.contains(said), "{said}:\n{words}");
        }
        assert!(
            drawn_in(&bars, "primary")
                .iter()
                .filter(|s| *s == "█")
                .count()
                > 10,
            "{words}"
        );
        assert!(
            drawn_in(&bars, "secondary")
                .iter()
                .any(|s| s == "█" || s == "▁" || s.chars().all(|c| ('▁'..='█').contains(&c))),
            "the negative bar:\n{words}"
        );
        // the spark: ratatui's Sparkline, a cell a value, the tallest a full block, all in the primary colour
        let spark = cells(&spark_terminal(&series_of(&[3.0, 5.0, 2.0])));
        let marks = drawn_in(&spark, "primary");
        assert_eq!(marks.len(), 3, "{marks:?}");
        assert_eq!(marks[1], "█", "{marks:?}");
        assert!(marks[0] != "█" && marks[2] != "█", "{marks:?}");
        // Progress: ratatui's Gauge, 2 of 5 filling two fifths of its track, its words on it
        let gauge = cells(&progress_terminal(2.0, 5.0, "2 of 5 sent"));
        let filled = (gauge.content.iter())
            .filter(|c| c.fg == theme("primary") && c.symbol() == "█")
            .count();
        let share = (WIDE as usize * 2) / 5;
        assert!((share - 4..=share).contains(&filled), "{filled} of {WIDE}");
        assert!(crate::tui::lines(&gauge)[0].contains("2 of 5 sent"));
        // its web form: the progressbar's values, its indicator that far, its words its text form
        let html = dioxus_ssr::render_element(
            rsx! { Progress { id: "p", value: 2.0, max: 5.0, label: "2 of 5 sent" } },
        );
        assert!(
            html.contains(r#"role="progressbar""#) && html.contains(r#"aria-valuenow="2""#),
            "{html}"
        );
        assert!(
            html.contains("width: 40.0%") && html.contains(r#"data-state="loading""#),
            "{html}"
        );
        assert_eq!(crate::words::text_of(&html).trim(), "2 of 5 sent");
    }

    fn series_of(values: &[f64]) -> Vec<ChartSeries> {
        vec![ChartSeries {
            name: "s".into(),
            values: values.to_vec(),
        }]
    }

    /// rust-charts-rust-ui: the y axis holds zero and every value on round steps, at most five gaps even across zero, a
    /// sparkline its own range; a smooth curve never leaves the band between its two ends; numbers grouped, decimals too, and a
    /// NaN drawn nowhere and listed blank; a value past the labels still drawn and listed; a long key wraps (/code-review).
    #[dash_test]
    fn a_charts_ticks_hold_zero_and_its_curve_never_overshoots() {
        let ticks = |v: &[f64], spark| chart_ticks(v.iter().copied(), spark);
        assert_eq!(ticks(&[6.0, 11.0], false), [0.0, 5.0, 10.0, 15.0]);
        assert_eq!(
            ticks(&[2500.0, -230.0], false),
            [-1000.0, 0.0, 1000.0, 2000.0, 3000.0]
        );
        assert_eq!(ticks(&[-1.0, 9.0], false), [-5.0, 0.0, 5.0, 10.0]);
        assert_eq!(ticks(&[], false), [0.0, 1.0]);
        assert_eq!(ticks(&[f64::NAN], false), [0.0, 1.0]);
        assert_eq!(ticks(&[1000.0, 1010.0, 1005.0], true), [1000.0, 1010.0]);
        assert_eq!(ticks(&[], true), [0.0, 1.0]);
        let fine: Vec<String> = ticks(&[0.25, 0.5], false)
            .into_iter()
            .map(chart_number)
            .collect();
        assert_eq!(fine, ["0", "0.1", "0.2", "0.3", "0.4", "0.5"]);
        let d = chart_path(
            &[(0.0, 10.0), (10.0, 0.0), (20.0, 10.0), (30.0, 10.0)],
            ChartCurve::Smooth,
        );
        let ys: Vec<f64> = (d.split([' ', 'C', 'M']))
            .filter_map(|p| p.split_once(','))
            .map(|(_, y)| y.parse().unwrap())
            .collect();
        assert!(ys.iter().all(|y| (0.0..=10.0).contains(y)), "{d}");
        assert_eq!(
            chart_path(&[(0.0, 4.0), (5.0, 2.0)], ChartCurve::Stepline),
            "M0.0,4.0 H5.0 V2.0"
        );
        let empty = dioxus_ssr::render_element(
            rsx! { LineChart { id: "e", axis: "Day", labels: vec![], series: vec![] } },
        );
        assert_eq!(crate::words::text_of(&empty), "Day\n");
        let numbers: Vec<String> = [1234.5, -12000.0, -0.001, 0.996, f64::NAN]
            .into_iter()
            .map(chart_number)
            .collect();
        assert_eq!(numbers, ["1,234.5", "-12,000", "0", "1", ""]);
        let series = vec![ChartSeries {
            name: "Rate".into(),
            values: vec![1.0, f64::NAN, 3.0],
        }];
        let html = dioxus_ssr::render_element(
            rsx! { LineChart { id: "n", axis: "Day", labels: chart_labels(&["a", "b"]), series } },
        );
        assert!(!html.contains("NaN"), "{html}");
        assert_eq!(html.matches("<circle").count(), 2, "{html}");
        assert_eq!(crate::words::text_of(&html), "Day\tRate\na\t1\nb\n3\n");
        let long = |name: &str| ChartSeries {
            name: name.into(),
            values: vec![],
        };
        let names = [
            "Applications sent",
            "Replies received",
            "Calls booked",
            "Offers signed",
        ]
        .map(long);
        let key = chart_key(&names, 60.0, 584.0);
        assert!(
            key.iter().all(
                |&(k, x, _)| x + 36.0 + 10.0 * names[k].name.len() as f64 <= 584.0 || x == 60.0
            ),
            "{key:?}"
        );
        assert!(key.last().unwrap().2 > 6.0, "wrapped: {key:?}");
    }
}
