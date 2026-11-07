//! What CSS Blitz draws, as data: each rule is painted with and without it on one fixture, and a rule that changes no pixel
//! is recorded as not drawn. When a Blitz bump changes one, this fails and the results page's list is stale.
//! `PROBE_DIR=<dir> cargo test --test css_probe` also writes a PNG per rule and `chrome.html`, the same rules for Chrome,
//! to read what was drawn wrong: a changed pixel alone does not say it is right, so those carry `Wrong` by a person's reading.
use blitz_dom::{Document, DocumentConfig, build_single_font_ctx};
use blitz_traits::shell::{ColorScheme, Viewport};
use dioxus::prelude::*;
use dioxus_native_dom::DioxusDocument;
use ui_spike_common::INTER;
use ui_spike_common::picture::{Picture, write_png};

const SIZE: (u32, u32) = (320, 400);
const BASE: &str = "html,body{margin:0}.f{position:relative;width:320px;height:400px;background:#fff;color:#123;font-size:14px;line-height:18px;overflow:hidden}.v{position:absolute;right:4px;top:4px}.t{width:180px;margin:12px;padding:8px;background:#cde}ul{margin:0}table{margin:4px}";
/// The fixture of `Fixture`, for Chrome.
const HTML: &str = r#"<div class="f"><div class="t">Styled box text, long enough to wrap in its own little box here <span class="s">span</span></div><ul class="l"><li>one</li><li>two</li></ul><input class="i" placeholder="Placeholder"/><input class="c" type="checkbox" checked/><div class="u"><b>Lead.</b> Then plain <span>span</span></div><textarea class="a">Area by child</textarea><textarea class="a" value="Area by value"></textarea><button class="b" disabled>Off</button><table class="g"><tr><td>a</td><td>b</td></tr></table><svg class="v" width="24" height="24" viewBox="0 0 24 24"><circle cx="12" cy="12" r="8" fill="none" stroke="currentColor" stroke-width="3"></circle></svg></div>"#;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Seen {
    /// Drawn, and as Chrome draws it.
    Drawn,
    /// No pixel changes.
    Nothing,
    /// Pixels change, not as Chrome draws them: read from the pictures.
    Wrong,
}
use Seen::*;

const PROBES: &[(&str, &str, Seen)] = &[
    // Surfaces
    ("box-shadow", ".t{box-shadow:0 4px 12px rgba(0,0,0,.5)}", Drawn),
    ("box-shadow inset", ".t{box-shadow:inset 0 0 8px red}", Drawn),
    ("outline", ".t{outline:2px solid red}", Drawn),
    ("outline-offset", ".t{outline:2px solid red;outline-offset:6px}", Wrong),
    ("border-radius", ".t{border-radius:16px}", Drawn),
    ("border dashed", ".t{border:2px dashed red}", Drawn),
    ("linear-gradient", ".t{background:linear-gradient(90deg,#f00,#00f)}", Drawn),
    ("radial-gradient", ".t{background:radial-gradient(circle,#f00,#00f)}", Drawn),
    ("conic-gradient", ".t{background:conic-gradient(#f00,#00f)}", Drawn),
    ("opacity", ".t{opacity:.4}", Drawn),
    ("transform rotate", ".t{transform:rotate(6deg)}", Drawn),
    ("filter blur", ".t{filter:blur(2px)}", Nothing),
    ("backdrop-filter", ".s{backdrop-filter:blur(4px)}", Nothing),
    ("mix-blend-mode", ".t{mix-blend-mode:multiply;background:#f00}", Drawn),
    ("clip-path", ".t{clip-path:circle(40px)}", Drawn),
    ("mask-image", ".t{mask-image:linear-gradient(#000,transparent)}", Drawn),
    // Text
    ("text-overflow ellipsis", ".t{white-space:nowrap;overflow:hidden;text-overflow:ellipsis}", Wrong),
    ("line-clamp", ".t{display:-webkit-box;-webkit-box-orient:vertical;-webkit-line-clamp:2;overflow:hidden}", Wrong),
    ("letter-spacing", ".t{letter-spacing:3px}", Drawn),
    ("text-transform", ".t{text-transform:uppercase}", Drawn),
    ("underline", ".t{text-decoration:underline}", Drawn),
    ("wavy underline", ".t{text-decoration:underline wavy red}", Drawn),
    ("underline-offset", ".t{text-decoration:underline;text-underline-offset:6px}", Drawn),
    ("font-weight 650", ".t{font-weight:650}", Drawn),
    ("a span's weight", ".s{font-weight:700}", Drawn),
    ("italic", ".t{font-style:italic}", Wrong),
    ("tabular numbers", ".t{font-variant-numeric:tabular-nums}", Nothing),
    // Layout
    ("grid", ".f{display:grid;grid-template-columns:1fr 1fr;gap:4px}", Drawn),
    ("grid areas", ".f{display:grid;grid-template-areas:'a b' 'c c'}.t{grid-area:c}", Drawn),
    ("flex wrap", ".f{display:flex;flex-wrap:wrap;gap:8px}", Drawn),
    ("aspect-ratio", ".t{aspect-ratio:1}", Drawn),
    ("position sticky", ".f{overflow:auto}.t{position:sticky;top:30px}", Nothing),
    ("clamp()", ".t{width:clamp(50px,20%,90px)}", Drawn),
    ("calc()", ".t{width:calc(100% - 60px)}", Drawn),
    ("table collapse", ".g{border-collapse:collapse}td{border:2px solid red;padding:4px}", Drawn),
    // Pseudo-elements
    ("::before", ".t::before{content:'+++';color:red}", Drawn),
    ("::after, no text", ".t::after{content:'';display:block;height:10px;background:red}", Nothing),
    ("::after, text", ".t::after{content:'+++';color:red}", Drawn),
    ("::placeholder", ".i::placeholder{color:red}", Nothing),
    ("::marker", "li::marker{color:red}", Nothing),
    ("list-style decimal", ".l{list-style:decimal}", Drawn),
    // Selectors
    (":nth-child", "li:nth-child(2){color:red}", Drawn),
    (":has", ".t:has(.s){background:red}", Nothing),
    (":not", ".t:not(.z){background:red}", Drawn),
    (":is", ":is(.t,.z){background:red}", Drawn),
    (":where", ":where(.s){color:red}", Drawn),
    (":checked", ".c:checked{margin-left:40px}", Nothing),
    (":disabled", ".b:disabled{background:red}", Drawn),
    ("[attr]", "[placeholder]{background:red}", Drawn),
    ("nesting", ".t{.s{color:red;background:#ff0}}", Drawn),
    // Conditions
    ("@media width", "@media (max-width:2000px){.t{background:red}}", Drawn),
    ("@media dark", "@media (prefers-color-scheme:light){.t{background:red}}", Drawn),
    ("@supports", "@supports (display:grid){.t{background:red}}", Drawn),
    ("@container", ".f{container-type:inline-size}@container (min-width:100px){.t{background:red}}", Nothing),
    ("@layer", "@layer a{.s{color:red}}", Drawn),
    // Colour
    ("color-mix", ".t{background:color-mix(in oklab,red 50%,white)}", Drawn),
    ("oklch()", ".t{background:oklch(60% .2 30)}", Drawn),
    ("relative colour", ".t{background:rgb(from red r g b / 50%)}", Drawn),
    ("border currentColor", ".t{color:red;border:2px solid currentColor}", Drawn),
    // Controls
    ("accent-color", ".c{accent-color:red}", Nothing),
    // A textarea's text is its child: `value`, as on an input, draws nothing, seen in the fixture's second one.
    ("textarea colour", ".a{color:red}", Drawn),
    ("appearance none", ".c{appearance:none;width:16px;height:16px;background:red}", Wrong),
    ("scrollbar-color", ".f{overflow:scroll;scrollbar-color:red blue}", Nothing),
    // SVG
    ("svg currentColor", ".v{color:red}", Drawn),
    ("svg currentColor, color-mix", ".v{color:color-mix(in oklab,red 50%,white)}", Wrong),
    ("svg stroke from CSS", ".v circle{stroke:red}", Nothing),
    ("svg stroke from var()", ".f{--c:red}.v circle{stroke:var(--c)}", Nothing),
];

#[derive(Props, Clone, PartialEq)]
struct Css {
    css: String,
}

#[allow(non_snake_case)]
fn Fixture(p: Css) -> Element {
    rsx! {
        style { {BASE} }
        style { {p.css} }
        div { class: "f",
            div { class: "t", "Styled box text, long enough to wrap in its own little box here ", span { class: "s", "span" } }
            ul { class: "l", li { "one" } li { "two" } }
            input { class: "i", placeholder: "Placeholder" }
            input { class: "c", r#type: "checkbox", checked: true }
            div { class: "u", b { "Lead." } " Then plain " span { "span" } }
            textarea { class: "a", "Area by child" }
            textarea { class: "a", value: "Area by value" }
            button { class: "b", disabled: true, "Off" }
            table { class: "g", tr { td { "a" } td { "b" } } }
            svg { class: "v", width: "24", height: "24", view_box: "0 0 24 24",
                circle { cx: "12", cy: "12", r: "8", fill: "none", stroke: "currentColor", stroke_width: "3" }
            }
        }
    }
}

fn paint(css: &str) -> Picture {
    use anyrender::ImageRenderer;
    let mut doc = DioxusDocument::new(
        VirtualDom::new_with_props(Fixture, Css { css: css.into() }),
        DocumentConfig {
            font_ctx: Some(build_single_font_ctx(INTER)),
            viewport: Some(Viewport::new(SIZE.0, SIZE.1, 1.0, ColorScheme::Light)),
            ..Default::default()
        },
    );
    doc.initial_build();
    doc.inner_mut().resolve(0.0);
    let mut renderer = anyrender_vello_cpu::VelloCpuImageRenderer::new(SIZE.0, SIZE.1);
    let mut rgba = vec![0; (SIZE.0 * SIZE.1 * 4) as usize];
    let mut inner = doc.inner_mut();
    renderer.render(|scene| blitz_paint::paint_scene(scene, &mut inner, 1.0, SIZE.0, SIZE.1, 0, 0), &mut rgba);
    Picture { width: SIZE.0, height: SIZE.1, rgba }
}

#[test]
fn each_rule_draws_as_recorded() {
    let dir = std::env::var_os("PROBE_DIR").map(std::path::PathBuf::from);
    let plain = paint("");
    let mut chrome = String::from("<!doctype html><meta charset=utf-8><body style='margin:0;display:grid;grid-template-columns:repeat(16,320px);gap:4px'>");
    let mut stale = vec![];
    for (i, (name, css, seen)) in PROBES.iter().enumerate() {
        let picture = paint(css);
        let changed = picture.rgba != plain.rgba;
        if changed != (*seen != Nothing) {
            stale.push(format!("{name}: changed pixels {changed}, recorded {seen:?}"));
        }
        if let Some(dir) = &dir {
            write_png(&dir.join(format!("{i:02}-{}.png", name.replace([' ', '/', ':'], "_"))), &picture);
            let doc = format!("<!doctype html><style>{BASE}{css}</style>{HTML}").replace('&', "&amp;").replace('"', "&quot;");
            chrome += &format!("<iframe srcdoc=\"{doc}\" width=320 height=400 style='border:0'></iframe>");
        }
    }
    if let Some(dir) = &dir {
        std::fs::write(dir.join("chrome.html"), chrome).expect("write chrome.html");
    }
    assert!(stale.is_empty(), "{stale:#?}");
}
