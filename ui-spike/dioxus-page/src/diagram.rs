//! An Excalidraw scene, read from its own file format and drawn as SVG. Blitz draws an inline svg as one image with no
//! inner hit tests, so the canvas tests the pointer against the scene itself, as Excalidraw's own canvas does. Labels are
//! HTML over the drawing, in the app's font: usvg would need a font database, and this window loads one font.
use crate::{BAR_H, Button, Dialog, Input, NAV_W};
use dioxus::prelude::*;

const SCENE: &str = include_str!("diagram.excalidraw");

#[derive(Clone, PartialEq, Debug)]
pub struct Shape {
    pub id: String,
    pub kind: String,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub stroke: String,
    pub fill: String,
    pub label: String,
}

#[derive(Clone, PartialEq, Debug)]
pub struct Arrow {
    pub from: String,
    pub to: String,
}

/// The scene's shapes, each with the text its containerId names, and its arrows by what they bind.
pub fn scene() -> (Vec<Shape>, Vec<Arrow>) {
    let doc: serde_json::Value = serde_json::from_str(SCENE).expect("the scene parses");
    let elements = doc["elements"].as_array().expect("elements");
    let str_of = |v: &serde_json::Value| v.as_str().unwrap_or("").to_string();
    let shapes = elements
        .iter()
        .filter(|e| matches!(e["type"].as_str(), Some("rectangle" | "ellipse" | "diamond")))
        .map(|e| Shape {
            id: str_of(&e["id"]),
            kind: str_of(&e["type"]),
            x: e["x"].as_f64().unwrap_or(0.0),
            y: e["y"].as_f64().unwrap_or(0.0),
            w: e["width"].as_f64().unwrap_or(0.0),
            h: e["height"].as_f64().unwrap_or(0.0),
            stroke: str_of(&e["strokeColor"]),
            fill: str_of(&e["backgroundColor"]),
            label: elements
                .iter()
                .find(|t| t["type"] == "text" && t["containerId"] == e["id"])
                .map_or(String::new(), |t| str_of(&t["text"])),
        })
        .collect();
    let arrows = elements
        .iter()
        .filter(|e| e["type"] == "arrow")
        .map(|e| Arrow { from: str_of(&e["startBinding"]["elementId"]), to: str_of(&e["endBinding"]["elementId"]) })
        .collect();
    (shapes, arrows)
}

/// A small fixed wobble, per point and pass: the sketched look, the same on every frame.
fn wobble(seed: usize, i: usize) -> f64 {
    let n = (seed * 31 + i * 17) % 7;
    n as f64 * 0.5 - 1.5
}

fn outline(s: &Shape) -> Vec<(f64, f64)> {
    let (x, y, w, h) = (s.x, s.y, s.w, s.h);
    match s.kind.as_str() {
        "diamond" => vec![(x + w / 2.0, y), (x + w, y + h / 2.0), (x + w / 2.0, y + h), (x, y + h / 2.0)],
        "ellipse" => (0..24)
            .map(|i| {
                let a = i as f64 / 24.0 * std::f64::consts::TAU;
                (x + w / 2.0 + a.cos() * w / 2.0, y + h / 2.0 + a.sin() * h / 2.0)
            })
            .collect(),
        _ => vec![(x, y), (x + w, y), (x + w, y + h), (x, y + h)],
    }
}

fn sketch(s: &Shape, pass: usize) -> String {
    let seed = s.id.bytes().map(usize::from).sum::<usize>() + pass;
    let points = outline(s);
    let mut d = String::new();
    for (i, p) in points.iter().enumerate() {
        let c = if i == 0 { 'M' } else { 'L' };
        d += &format!("{c}{:.1} {:.1} ", p.0 + wobble(seed, i), p.1 + wobble(seed, i + 3));
    }
    d + "Z"
}

pub fn hit(s: &Shape, x: f64, y: f64) -> bool {
    let (dx, dy) = ((x - s.x - s.w / 2.0) / (s.w / 2.0), (y - s.y - s.h / 2.0) / (s.h / 2.0));
    match s.kind.as_str() {
        "diamond" => dx.abs() + dy.abs() <= 1.0,
        "ellipse" => dx * dx + dy * dy <= 1.0,
        _ => dx.abs() <= 1.0 && dy.abs() <= 1.0,
    }
}

/// Where the line from a shape's middle towards a point leaves its box.
fn edge(s: &Shape, to: (f64, f64)) -> (f64, f64) {
    let (cx, cy) = (s.x + s.w / 2.0, s.y + s.h / 2.0);
    let (dx, dy) = (to.0 - cx, to.1 - cy);
    let t = if s.kind == "rectangle" {
        (s.w / 2.0 / dx.abs().max(1e-6)).min(s.h / 2.0 / dy.abs().max(1e-6))
    } else if s.kind == "diamond" {
        1.0 / (dx.abs() / (s.w / 2.0) + dy.abs() / (s.h / 2.0)).max(1e-6)
    } else {
        1.0 / ((dx / (s.w / 2.0)).powi(2) + (dy / (s.h / 2.0)).powi(2)).sqrt().max(1e-6)
    };
    (cx + dx * t, cy + dy * t)
}

fn arrow_path(a: &Shape, b: &Shape) -> String {
    let middle = |s: &Shape| (s.x + s.w / 2.0, s.y + s.h / 2.0);
    let (p, q) = (edge(a, middle(b)), edge(b, middle(a)));
    let angle = (q.1 - p.1).atan2(q.0 - p.0);
    let wing = |turn: f64| (q.0 - 14.0 * (angle + turn).cos(), q.1 - 14.0 * (angle + turn).sin());
    let (l, r) = (wing(0.45), wing(-0.45));
    format!("M{:.1} {:.1} L{:.1} {:.1} M{:.1} {:.1} L{:.1} {:.1} L{:.1} {:.1}", p.0, p.1, q.0, q.1, l.0, l.1, q.0, q.1, r.0, r.1)
}

#[component]
pub(crate) fn Diagram() -> Element {
    let mut parts = use_signal(scene);
    let mut hovered = use_signal(|| None::<usize>);
    let mut selected = use_signal(|| None::<usize>);
    let mut grabbed = use_signal(|| None::<(usize, f64, f64)>);
    let mut renaming = use_signal(|| None::<(usize, String)>);
    let mut inner_clicks = use_signal(|| 0);
    let top_at = move |x: f64, y: f64| parts.read().0.iter().rposition(|s| hit(s, x, y));
    let status = selected().map_or("selected: none".to_string(), |i| {
        let s = &parts.read().0[i];
        format!("selected: {} at {:.0},{:.0}", s.label, s.x, s.y)
    });
    let (shapes, arrows) = parts();
    let find = |id: &str| shapes.iter().find(|s| s.id == id);
    rsx! {
        div { class: "screen",
            div { class: "bar",
                span { class: "bar-title", "Diagram" }
                span { class: "meta", "diagram.excalidraw" }
                span { class: "spring" }
                Button { id: "rename", class: "button quiet",
                    onclick: move |_| if let Some(i) = selected() { renaming.set(Some((i, parts.read().0[i].label.clone()))) },
                    "Rename"
                }
            }
            div { id: "canvas", class: "canvas",
                onmousemove: move |e: MouseEvent| {
                    let p = e.client_coordinates();
                    let (x, y) = (p.x - NAV_W, p.y - BAR_H);
                    // Released outside the canvas, where no mouseup reaches it.
                    if grabbed.peek().is_some() && e.held_buttons().is_empty() {
                        grabbed.set(None);
                    }
                    if let Some((i, dx, dy)) = grabbed() {
                        let mut all = parts.write();
                        all.0[i].x = x - dx;
                        all.0[i].y = y - dy;
                    } else {
                        let on = top_at(x, y);
                        if *hovered.peek() != on {
                            hovered.set(on);
                        }
                    }
                },
                onmousedown: move |e: MouseEvent| {
                    let p = e.client_coordinates();
                    let (x, y) = (p.x - NAV_W, p.y - BAR_H);
                    let on = top_at(x, y);
                    selected.set(on);
                    if let Some(i) = on {
                        let s = parts.read().0[i].clone();
                        grabbed.set(Some((i, x - s.x, y - s.y)));
                    }
                },
                onmouseup: move |_| grabbed.set(None),
                svg { id: "drawing", class: "canvas-svg", view_box: "0 0 1000 672", width: "1000", height: "672", xmlns: "http://www.w3.org/2000/svg",
                    for a in arrows.iter() {
                        if let (Some(from), Some(to)) = (find(&a.from), find(&a.to)) {
                            path { d: arrow_path(from, to), fill: "none", stroke: "#1e1e1e", stroke_width: "1.6" }
                        }
                    }
                    for (i, s) in shapes.iter().enumerate() {
                        path { id: "shape-{s.id}", d: sketch(s, 0), fill: "{s.fill}", stroke: "{s.stroke}",
                            stroke_width: if hovered() == Some(i) { "3" } else { "1.6" },
                            onclick: move |_| inner_clicks += 1,
                        }
                        path { d: sketch(s, 1), fill: "none", stroke: "{s.stroke}", stroke_width: "1" }
                        if selected() == Some(i) {
                            rect { x: s.x - 6.0, y: s.y - 6.0, width: s.w + 12.0, height: s.h + 12.0, fill: "none", stroke: "#6965db", stroke_width: "1.5" }
                        }
                    }
                }
                for s in shapes.iter() {
                    div { class: "label", key: "{s.id}", style: "left:{s.x + s.w / 2.0}px;top:{s.y + s.h / 2.0}px", "{s.label}" }
                }
            }
            div { class: "status",
                div { id: "diagram-status", class: "meta", "{status}" }
                div { id: "diagram-hover", class: "meta", "under the pointer: {hovered().map_or(\"none\".to_string(), |i| shapes[i].label.clone())}" }
                div { id: "inner-clicks", class: "meta", "a shape's own onclick fired: {inner_clicks}" }
            }
        }
        if let Some((i, label)) = renaming() {
            Dialog { title: "Rename", onclose: move |_| renaming.set(None),
                div { class: "field",
                    Input { id: "label-input", value: label.clone(), oninput: move |v| renaming.set(Some((i, v))) }
                }
                div { class: "dialog-actions",
                    Button { id: "label-save",
                        onclick: move |_| {
                            if let Some((i, label)) = renaming() {
                                parts.write().0[i].label = label;
                            }
                            renaming.set(None);
                        },
                        "Save"
                    }
                }
            }
        }
    }
}
