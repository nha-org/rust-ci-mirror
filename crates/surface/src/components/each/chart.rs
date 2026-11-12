use super::treemap::grouped;
use crate::component::Component;
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
enum Chart {
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
        used: "A trend filled to zero, one colour a series: /design's charts. Built on rust-ui's AreaChart, drawn on the server.",
        example: Some(|_| {
            let series = vec![
                ChartSeries {
                    name: "Sent".into(),
                    values: vec![6.0, 8.0, 4.0, 11.0],
                },
                ChartSeries {
                    name: "Replied".into(),
                    values: vec![2.0, 3.0, 1.0, 5.0],
                },
            ];
            rsx! { AreaChart { id: "c-area", axis: "Week", labels: chart_labels(&["W1", "W2", "W3", "W4"]), series } }
        }),
        library: Some("https://rust-ui.com/charts/area-chart"),
        formed: true,
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
        used: "A count a period or a stage, a negative one muted: /design's charts. Built on rust-ui's BarChart, drawn on the server.",
        example: Some(|_| {
            let series = vec![ChartSeries {
                name: "Net".into(),
                values: vec![2500.0, -230.0, 1800.0],
            }];
            rsx! { BarChart { id: "c-bar", axis: "Month", labels: chart_labels(&["Jan", "Feb", "Mar"]), series } }
        }),
        library: Some("https://rust-ui.com/charts/bar-chart"),
        formed: true,
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
        used: "A trend through points, a dot each; look spark is the sparkline beside a figure: /design's charts and stats. Built on rust-ui's LineChart, drawn on the server.",
        example: Some(|_| {
            let series = vec![ChartSeries {
                name: "Replies".into(),
                values: vec![3.0, 5.0, 2.0],
            }];
            rsx! { LineChart { id: "c-line", axis: "Day", labels: chart_labels(&["Mon", "Tue", "Wed"]), series, curve: ChartCurve::Straight } }
        }),
        library: Some("https://rust-ui.com/charts/line-chart"),
        formed: true,
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

#[derive(Clone, Copy)]
struct Look {
    kind: Chart,
    curve: ChartCurve,
    gradient: bool,
    show_yaxis: bool,
    show_grid: bool,
    spark: bool,
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
    use dash_meter::dash_test;

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
