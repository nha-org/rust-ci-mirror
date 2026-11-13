use crate::component::{Component, Section};
use dioxus::prelude::*;
use std::collections::BTreeMap;

/// One box of a Treemap: its name, its size (its area), what it uses, each a box's path (its names from the top joined by
/// `/`), and the boxes nested in it. A box with children is as big as they are, its own `size` unread.
#[derive(Clone, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct Nest {
    pub name: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub uses: Vec<String>,
    #[serde(default)]
    pub children: Vec<Nest>,
}

impl Nest {
    pub fn area(&self) -> u64 {
        match self.children.is_empty() {
            true => self.size,
            false => self.children.iter().map(Nest::area).sum(),
        }
    }
}

/// x, y, width, height on the treemap's canvas.
type Rect = (f64, f64, f64, f64);

const CANVAS: (f64, f64) = (1200.0, 700.0);

/// Squarified treemap (Bruls, Huizing and van Wijk, 2000): `sizes`, largest first, laid in `r` a row at a time along its
/// shorter side, a row growing while its worst aspect ratio improves.
fn squarify(sizes: &[f64], r: Rect) -> Vec<Rect> {
    let (mut x, mut y, mut w, mut h) = r;
    let scale = w * h / sizes.iter().sum::<f64>();
    let worst = |row: &[f64], side: f64| {
        let s = row.iter().sum::<f64>() * scale;
        let (big, small) = (row[0] * scale, row[row.len() - 1] * scale);
        (side * side * big / (s * s)).max(s * s / (side * side * small))
    };
    let (mut out, mut i) = (Vec::new(), 0);
    while i < sizes.len() {
        let side = w.min(h);
        let mut j = i + 1;
        while j < sizes.len() && worst(&sizes[i..=j], side) <= worst(&sizes[i..j], side) {
            j += 1;
        }
        let thick = sizes[i..j].iter().sum::<f64>() * scale / side;
        let mut along = 0.0;
        for v in &sizes[i..j] {
            let len = v * scale / thick;
            out.push(match w >= h {
                true => (x, y + along, thick, len),
                false => (x + along, y, len, thick),
            });
            along += len;
        }
        if w >= h {
            (x, w) = (x + thick, w - thick);
        } else {
            (y, h) = (y + thick, h - thick);
        }
        i = j;
    }
    out
}

/// A box as drawn: its path, name, size, depth, place and uses.
struct Placed<'a> {
    path: String,
    nest: &'a Nest,
    depth: usize,
    rect: Rect,
}

/// Each box with lines, parents before their children; a box too small to hold its children draws none of them. A parent's
/// title line and frame take room from its children, so its weight adds its frame's area, laid out again until that holds: a
/// file's area is its lines' share wherever it sits (/code-review: a small module's title line took 45% of its files' room).
fn placed(boxes: &[Nest]) -> Vec<Placed<'_>> {
    fn weight(b: &Nest, path: &str, per_line: f64, frames: &BTreeMap<String, f64>) -> f64 {
        if b.children.is_empty() {
            return b.size as f64 * per_line;
        }
        let inner = b
            .children
            .iter()
            .map(|c| weight(c, &format!("{path}/{}", c.name), per_line, frames));
        inner.sum::<f64>() + frames.get(path).copied().unwrap_or(0.0)
    }
    type Frames = BTreeMap<String, f64>;
    fn lay<'a>(
        boxes: &'a [Nest],
        r: Rect,
        at: &str,
        depth: usize,
        w: (f64, &Frames),
        out: &mut Vec<Placed<'a>>,
    ) {
        let path = |b: &Nest| {
            if at.is_empty() {
                b.name.clone()
            } else {
                format!("{at}/{}", b.name)
            }
        };
        let mut sized: Vec<(f64, &Nest)> = (boxes.iter().filter(|b| b.area() > 0))
            .map(|b| (weight(b, &path(b), w.0, w.1), b))
            .collect();
        sized.sort_by(|a, b| b.0.total_cmp(&a.0));
        let weights: Vec<f64> = sized.iter().map(|(a, _)| *a).collect();
        if weights.is_empty() {
            return;
        }
        for ((_, b), rect) in sized.into_iter().zip(squarify(&weights, r)) {
            out.push(Placed {
                path: path(b),
                nest: b,
                depth,
                rect,
            });
            // a name's line on top, 2 px of frame around
            let (x, y, wide, high) = rect;
            if !b.children.is_empty() && wide > 12.0 && high > 30.0 {
                lay(
                    &b.children,
                    (x + 2.0, y + 16.0, wide - 4.0, high - 18.0),
                    &path(b),
                    depth + 1,
                    w,
                    out,
                );
            }
        }
    }
    let (canvas, lines) = (
        CANVAS.0 * CANVAS.1,
        boxes.iter().map(Nest::area).sum::<u64>() as f64,
    );
    let mut frames = Frames::new();
    let mut out = Vec::new();
    // ponytail: a fixed 8 rounds; a frame's area moves less each round, under 1% of a box by the fourth on /deps
    for _ in 0..8 {
        out.clear();
        let per_line = (canvas - frames.values().sum::<f64>()).max(0.0) / lines.max(1.0);
        lay(
            boxes,
            (0.0, 0.0, CANVAS.0, CANVAS.1),
            "",
            0,
            (per_line, &frames),
            &mut out,
        );
        let drawn: BTreeMap<&str, Rect> = out.iter().map(|p| (p.path.as_str(), p.rect)).collect();
        let parent = |p: &Placed| {
            !p.nest.children.is_empty()
                && drawn
                    .keys()
                    .any(|k| k.strip_prefix(&p.path).is_some_and(|r| r.starts_with('/')))
        };
        frames = (out.iter().filter(|p| parent(p)))
            .map(|p| {
                (
                    p.path.clone(),
                    p.rect.2 * p.rect.3 - (p.rect.2 - 4.0) * (p.rect.3 - 18.0),
                )
            })
            .collect();
    }
    out
}

/// Thousands with commas: 14800 is 14,800.
pub fn grouped(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// What a hover shows: for each drawn box, a rule that outlines the boxes it uses (solid) and those using it (dashed), only
/// while it is under the pointer. CSS alone, no script: `scope` holds only [A-Za-z0-9_-], each box's key its index.
fn hover_css(scope: &str, drawn: &[Placed]) -> String {
    let at: BTreeMap<&str, usize> = drawn
        .iter()
        .enumerate()
        .map(|(k, p)| (p.path.as_str(), k))
        .collect();
    let mut users: Vec<Vec<usize>> = vec![Vec::new(); drawn.len()];
    let uses: Vec<Vec<usize>> = (drawn.iter().enumerate())
        .map(|(k, p)| {
            let mut to: Vec<usize> = p
                .nest
                .uses
                .iter()
                .filter_map(|u| at.get(u.as_str()).copied())
                .filter(|t| *t != k)
                .collect();
            to.sort_unstable();
            to.dedup();
            to.iter().for_each(|t| users[*t].push(k));
            to
        })
        .collect();
    let is = |ks: &[usize]| {
        ks.iter()
            .map(|k| format!("[data-k=\"{k}\"]"))
            .collect::<Vec<_>>()
            .join(",")
    };
    // a box both used and using is double, never one rule overriding the other (/code-review)
    let mut css = String::new();
    for k in 0..drawn.len() {
        let on = format!("[data-map=\"{scope}\"]:has([data-k=\"{k}\"]:hover)");
        let both: Vec<usize> = uses[k]
            .iter()
            .filter(|t| users[k].contains(t))
            .copied()
            .collect();
        let only = |ks: &[usize]| {
            ks.iter()
                .filter(|t| !both.contains(t))
                .copied()
                .collect::<Vec<_>>()
        };
        for (ks, look) in [
            (only(&uses[k]), "solid var(--color-primary)"),
            (only(&users[k]), "dashed var(--color-accent)"),
            (both.clone(), "double var(--color-primary)"),
        ] {
            if !ks.is_empty() {
                css += &format!(
                    "{on} :is({}){{outline:3px {look};outline-offset:-3px}}\n",
                    is(&ks)
                );
            }
        }
    }
    css
}

inventory::submit! {
    Component {
        slug: "treemap",
        classes: &["treemap", "treemap-box"],
        tags: &[],
        name: "Treemap",
        used: "/deps' code map: layers, crates, modules and files nested, a box's area its lines of code; hovering a box outlines what it uses and what uses it.",
        sections: &[Section {
            family: "charts",
            id: "treemap",
            title: "Treemap",
            after: "share",
            rows: &[("/deps' code map", |_| {
                let leaf = |name: &str, size: u64, uses: &[&str]| Nest {
                    name: name.into(),
                    size,
                    uses: uses.iter().map(|u| u.to_string()).collect(),
                    children: vec![],
                };
                let boxes = vec![
                    Nest {
                        name: "Rings".into(),
                        size: 0,
                        uses: vec!["Core".into()],
                        children: vec![
                            leaf("store", 900, &["Core/core"]),
                            leaf("surface", 1400, &["Core/core"]),
                        ],
                    },
                    Nest {
                        name: "Core".into(),
                        size: 0,
                        uses: vec![],
                        children: vec![leaf("core", 700, &[])],
                    },
                ];
                rsx! { Treemap { id: "c-treemap", boxes } }
            })],
        }],
        ..Component::BARE
    }
}

/// Nested boxes whose area is their size, as windirstat draws a disk (task rust-deps-files-hover; @nha, 2026-11-09: « Something
/// similar to the matroshka / windirstat but where we can see nested files. Hoovering on a file / box shows direct links to
/// others. »): laid out here on the server, a box a positioned div, hovering one outlines what it uses and what uses it, by
/// CSS alone. Its text form (`treemap_text`) is the same tree indented, held in the HTML too (`.sr-only`, the drawn boxes
/// `aria-hidden`), so the page's text and the tree's say the same (task design-agent-forms).
#[component]
pub fn Treemap(#[props(into)] id: String, boxes: Vec<Nest>) -> Element {
    let drawn = placed(&boxes);
    let scope: String = id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        .collect();
    let css = hover_css(&scope, &drawn);
    let pct = |v: f64, of: f64| format!("{:.3}%", v * 100.0 / of);
    let text = treemap_text(&boxes).join("\n");
    rsx! {
        p { class: "treemap-key", "Area is lines of code. Hover a box: a solid outline on what it uses, dashed on what uses it, double on both." }
        // its rules inside the element its id names, so a live patch by id swaps them with the boxes (/code-review)
        div { id, class: "treemap", "data-map": "{scope}", "aria-hidden": "true",
            style { dangerous_inner_html: css }
            for (k, p) in drawn.iter().enumerate() {
                div {
                    class: "treemap-box",
                    "data-k": "{k}",
                    "data-depth": "{p.depth.min(3)}",
                    "data-path": "{p.path}",
                    title: "{p.path}: {grouped(p.nest.area())} lines",
                    style: "left:{pct(p.rect.0, CANVAS.0)};top:{pct(p.rect.1, CANVAS.1)};width:{pct(p.rect.2, CANVAS.0)};height:{pct(p.rect.3, CANVAS.1)}",
                    "{p.nest.name}"
                }
            }
        }
        pre { class: "sr-only", "{text}" }
    }
}

/// A Treemap's text form: a box with an area a line, indented under its parent, its lines and what it uses, each named from
/// its nearest shared ancestor; a box using what its parent uses says it once, on the parent.
pub fn treemap_text(boxes: &[Nest]) -> Vec<String> {
    fn walk(boxes: &[Nest], above: &mut Vec<String>, theirs: &[String], out: &mut Vec<String>) {
        for b in boxes.iter().filter(|b| b.area() > 0) {
            let rel = |u: &String| {
                let near = above
                    .iter()
                    .rev()
                    .find_map(|a| u.strip_prefix(&format!("{a}/")));
                near.unwrap_or(u).to_string()
            };
            let uses = match b.uses.is_empty() || b.uses == theirs {
                true => String::new(),
                false => format!(
                    "; uses {}",
                    b.uses.iter().map(rel).collect::<Vec<_>>().join(", ")
                ),
            };
            let (n, s) = (b.area(), if b.area() == 1 { "" } else { "s" });
            let depth = "  ".repeat(above.len());
            out.push(format!("{depth}{}: {} line{s}{uses}", b.name, grouped(n)));
            let path = above
                .last()
                .map_or(b.name.clone(), |a| format!("{a}/{}", b.name));
            above.push(path);
            walk(&b.children, above, &b.uses, out);
            above.pop();
        }
    }
    let mut out = Vec::new();
    walk(boxes, &mut Vec::new(), &[], &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;

    /// Done-when 1 of rust-deps-files-hover: every file's area is its share of the lines, inside a parent or not, and a child
    /// sits inside its parent; a box of no lines is not drawn.
    #[dash_test]
    fn a_treemap_box_has_its_share_of_the_area_inside_its_parent() {
        let leaf = |name: &str, size| Nest {
            name: name.into(),
            size,
            uses: vec![],
            children: vec![],
        };
        let boxes = vec![
            Nest {
                name: "a".into(),
                size: 0,
                uses: vec![],
                children: vec![leaf("x", 300), leaf("y", 100), leaf("t", 0)],
            },
            leaf("b", 600),
            leaf("c", 100),
        ];
        let drawn = placed(&boxes);
        let paths: Vec<&str> = drawn.iter().map(|p| p.path.as_str()).collect();
        assert_eq!(paths, ["b", "a", "a/x", "a/y", "c"]);
        // every leaf the same area per line, the files in « a » too, despite its title line
        let per_line: Vec<f64> = [("b", 600.0), ("a/x", 300.0), ("a/y", 100.0), ("c", 100.0)]
            .iter()
            .map(|(path, lines)| {
                let r = drawn.iter().find(|p| p.path == *path).expect(path).rect;
                r.2 * r.3 / lines
            })
            .collect();
        assert!(
            per_line
                .iter()
                .all(|v| (v / per_line[0] - 1.0).abs() < 0.01),
            "{per_line:?}"
        );
        let (a, x) = (drawn[1].rect, drawn[2].rect);
        assert!(
            x.0 >= a.0
                && x.1 >= a.1
                && x.0 + x.2 <= a.0 + a.2 + 1e-9
                && x.1 + x.3 <= a.1 + a.3 + 1e-9,
            "{a:?} {x:?}"
        );
    }
}
