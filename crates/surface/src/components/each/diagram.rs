use super::fold::Fold;
use crate::component::{Component, Section};
use dioxus::prelude::*;
use std::collections::BTreeMap;

inventory::submit! {
    Component {
        slug: "diagram",
        classes: &["diagram"],
        tags: &[],
        name: "Diagram",
        used: "/deps' crate graph: its Mermaid source is the text, Mermaid with ELK's layout draws it, a box a layer.",
        sections: &[Section {
            family: "charts",
            id: "diagram",
            title: "Diagram",
            // the treemap follows the share bar too, after this by id: naming it here is a list of two components to
            // dash-gate (shared_component_lists); the_charts_tab_holds_every_chart_the_treemap_and_the_diagram pins the order
            after: "share",
            rows: &[("/deps' crate graph", |_| {
                rsx! {
                    Diagram { id: "c-diagram", source: "flowchart LR\n  subgraph l0[\"Core\"]\n    c_core[\"core\"]\n  end\n  c_store[\"store\"] --> c_core\n" }
                }
            })],
        }],
        ..Component::BARE
    }
}

/// A diagram from its text form, Mermaid (rust/CLAUDE.md rule 4: « diagrams etc. need to have a text form. say mermaid for
/// example. »), drawn to SVG here on the server by mermaid-rs-renderer: no script and no CDN on the page (@nha, 2026-11-09:
/// « Mermaid JS should never load from a CDN. The tool is local first. »). The fold under it holds the source an agent
/// reads; a source the renderer refuses says why, its source still folded under it.
#[component]
pub fn Diagram(#[props(into)] id: String, #[props(into)] source: String) -> Element {
    let drawn = match diagram_svg(&source) {
        Ok(svg) => {
            rsx! { div { id: "{id}", class: "diagram", role: "img", "aria-label": "Diagram", dangerous_inner_html: svg } }
        }
        Err(e) => {
            rsx! { p { id: "{id}", class: "diagram", "This diagram could not be drawn: {e}" } }
        }
    };
    rsx! {
        {drawn}
        Fold { id: "{id}-source", open: false, summary: rsx! { "Mermaid source" }, pre { "{source}" } }
    }
}

/// One look for every diagram: the neutral theme at 11 px (the code map's boxes are the Treemap component). Each source is
/// drawn once per process: the crate graph's routing takes 1.4 s in a release build.
pub fn diagram_svg(source: &str) -> Result<String, String> {
    static DRAWN: std::sync::Mutex<BTreeMap<String, Result<String, String>>> =
        std::sync::Mutex::new(BTreeMap::new());
    // ponytail: grows by one entry per distinct source; a page's diagrams are build constants today
    let mut drawn = DRAWN.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(done) = drawn.get(source) {
        return done.clone();
    }
    let done = draw(source);
    drawn.insert(source.to_string(), done.clone());
    done
}

fn draw(source: &str) -> Result<String, String> {
    let mut theme = mermaid_rs_renderer::Theme::neutral();
    theme.font_size = 11.0;
    let layout = mermaid_rs_renderer::LayoutConfig::default();
    let options = mermaid_rs_renderer::RenderOptions { theme, layout };
    mermaid_rs_renderer::render_with_options(source, options).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;

    /// The crate graph's diagram: each box's name drawn, a label is text and never markup, and a source the renderer refuses
    /// is an error, never a page without its diagram unsaid.
    #[dash_test]
    fn a_diagram_is_svg_drawn_here_its_labels_text_and_a_bad_source_an_error() {
        let svg = diagram_svg("flowchart LR\n  a[\"surface\"] --> b[\"<script>x</script>\"]\n")
            .expect("drawn");
        assert!(
            svg.starts_with("<svg") && svg.contains(">surface<"),
            "{svg}"
        );
        assert!(!svg.contains("<script"), "{svg}");
        assert!(diagram_svg("not a diagram").is_err());
    }

    /// /code-review of fa5aa6443: a drawing is drawn inline only when nothing in it can run, read as a browser reads it:
    /// a scheme hidden by a character reference or a tab, a handler, a style sheet, each makes it an image of itself.
    #[dash_test]
    fn a_drawing_that_could_run_code_is_never_inline() {
        assert!(inert(
            r##"<svg viewBox="0 0 10 10"><text>Nicolas &#8594; agent</text><a href="#g-ask"><text>ask</text></a></svg>"##
        ));
        for unsafe_svg in [
            r#"<svg onload="x()"></svg>"#,
            r#"<svg><rect onclick = "x()"/></svg>"#,
            r#"<svg><a href="javascript&#58;x()"><text>a</text></a></svg>"#,
            r#"<svg><a xlink:href="java&#9;script:x()"><text>a</text></a></svg>"#,
            r#"<svg><a href="JaVaScRiPt&colon;x()"><text>a</text></a></svg>"#,
            r#"<svg><a href="&#x6A;avascript:x()"><text>a</text></a></svg>"#,
            r#"<svg><style>body{display:none}</style></svg>"#,
            r#"<svg><script>x()</script></svg>"#,
            r#"<svg><foreignObject><div>x</div></foreignObject></svg>"#,
        ] {
            assert!(!inert(unsafe_svg), "{unsafe_svg}");
        }
    }
}

/// A drawing kept as its own markup, an SVG a document holds (`dash doc import`, task editor-in-rfcs): drawn as written, so the
/// page's styles reach it, its `words` read by a screen reader. One that could run code (a script, an event handler, a foreign
/// object, a script link) is drawn as an image of itself instead, where nothing runs: an imported web page is never trusted.
#[component]
pub fn SvgDrawing(
    #[props(into)] id: Option<String>,
    #[props(into)] svg: String,
    #[props(into)] words: String,
) -> Element {
    match inert(&svg) {
        true => {
            rsx! { div { id, class: "diagram", role: "img", "aria-label": "{words}", dangerous_inner_html: svg } }
        }
        false => rsx! { img { id, class: "diagram", src: svg_uri(&svg), alt: "{words}" } },
    }
}

/// Whether an SVG's markup runs nothing when drawn inline: no script, foreign object, frame, style sheet, event handler
/// (`onload=`) or script link, read after its character references are decoded and, for a link, its whitespace dropped, as a
/// browser reads `java&#9;script&#58;` (/code-review of fa5aa6443). ponytail: a scan that refuses more than it must (words
/// reading « on top = » are drawn as an image); an allowlist parser of SVG if a page of his ever trips it.
fn inert(svg: &str) -> bool {
    let low = decoded(svg).to_ascii_lowercase();
    let b = low.as_bytes();
    // « on<letters> = » after a space, a slash or a quote: an attribute a browser runs
    let handler = (1..b.len()).any(|i| {
        let after = |mut j: usize, f: fn(&u8) -> bool| {
            while b.get(j).is_some_and(f) {
                j += 1;
            }
            j
        };
        let name = after(i + 2, u8::is_ascii_alphabetic);
        b[i..].starts_with(b"on")
            && (b[i - 1].is_ascii_whitespace() || b"/\"'".contains(&b[i - 1]))
            && name > i + 2
            && b.get(after(name, u8::is_ascii_whitespace)) == Some(&b'=')
    });
    // tags that run, embed or restyle, each looked for after its « < »; a link's scheme with no whitespace or control inside
    let tags = [
        "script",
        "foreignobject",
        "iframe",
        "embed",
        "object",
        "style",
    ];
    let squeezed: String = low
        .chars()
        .filter(|c| !c.is_whitespace() && !c.is_control())
        .collect();
    let schemes = ["javascript:", "vbscript:", "data:text/html"];
    !handler
        && !schemes.iter().any(|s| squeezed.contains(s))
        && !tags.iter().any(|t| low.contains(&format!("<{t}")))
}

/// Character references read as a browser reads them: `&#58;`, `&#x3a;`, and the named ones a scheme can hide in.
fn decoded(svg: &str) -> String {
    let mut out = String::with_capacity(svg.len());
    let mut rest = svg;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let end = rest.find(';').filter(|&e| e <= 12);
        let name = end.map(|e| &rest[1..e]);
        let char = name.and_then(|n| match n.to_ascii_lowercase().as_str() {
            "colon" => Some(':'),
            "tab" => Some('\t'),
            "newline" => Some('\n'),
            n => (n.strip_prefix("#x").map(|h| u32::from_str_radix(h, 16)))
                .or_else(|| n.strip_prefix('#').map(str::parse::<u32>))
                .and_then(Result::ok)
                .and_then(char::from_u32),
        });
        match (char, end) {
            (Some(c), Some(e)) => {
                out.push(c);
                rest = &rest[e + 1..];
            }
            _ => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// An SVG as an image's `data:` URI, every byte outside the plain ones percent-encoded.
fn svg_uri(svg: &str) -> String {
    let body: String = (svg.bytes())
        .map(
            |b| match b.is_ascii_alphanumeric() || b" -_.~/=:;,".contains(&b) {
                true => (b as char).to_string(),
                false => format!("%{b:02X}"),
            },
        )
        .collect();
    format!("data:image/svg+xml,{body}")
}
