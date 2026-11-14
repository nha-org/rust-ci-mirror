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
        terminal: Some(crate::component::Terminal::Words),
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

    /// /code-review of PR 1592: two drawings on one page each point at their own marker, an arrow's own marker stays, and
    /// drawing a drawn (saved) SVG again gives the same markup.
    #[dash_test]
    fn each_drawing_keeps_its_own_marker_ids() {
        let svg = |colour: &str| {
            format!(
                r##"<svg><defs><marker id="arrow"><path fill="{colour}"/></marker></defs><path class="arrow" d="M0 0"/><path class="arrow" marker-end="url(#other)"/><use href="#arrow"/></svg>"##
            )
        };
        let (a, b) = (scoped(&svg("red")), scoped(&svg("blue")));
        let id = |s: &str| {
            s.split(" id=\"")
                .nth(1)
                .and_then(|r| r.split('"').next())
                .map(str::to_string)
                .expect("an id")
        };
        let (ia, ib) = (id(&a), id(&b));
        assert!(
            ia.ends_with("-arrow") && ib.ends_with("-arrow") && ia != ib,
            "{ia} {ib}"
        );
        assert!(
            a.contains(&format!(r#"class="arrow" marker-end="url(#{ia})""#)),
            "{a}"
        );
        assert!(
            a.contains(r#"marker-end="url(#other)""#) && a.matches("marker-end").count() == 2,
            "{a}"
        );
        assert!(a.contains(&format!(r##"href="#{ia}""##)), "{a}");
        assert_eq!(scoped(&a), a, "a drawn SVG draws the same");
        assert_eq!(
            scoped("<svg><text>x</text></svg>"),
            "<svg><text>x</text></svg>"
        );
    }

    /// A rule's declarations, `selector { a: b; ... }` opening a line of `css`, by name: its block up to the first `}`.
    fn declared(css: &str, selector: &str) -> BTreeMap<String, String> {
        let opens = format!("{selector} {{");
        let at = (css.match_indices(&opens))
            .find(|&(i, _)| i == 0 || css.as_bytes()[i - 1] == b'\n')
            .unwrap_or_else(|| panic!("no rule {selector}"))
            .0;
        let rest = &css[at + opens.len()..];
        let inside = &rest[..rest.find('}').expect("a closed rule")];
        (inside.split(';'))
            .filter_map(|d| d.split_once(':'))
            .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
            .collect()
    }

    /// WCAG's contrast of two `#rgb` or `#rrggbb` colours.
    fn contrast(a: &str, b: &str) -> f64 {
        let lum = |hex: &str| {
            let h = hex.trim_start_matches('#');
            let h: String = match h.len() {
                3 => h.chars().flat_map(|c| [c, c]).collect(),
                _ => h.to_string(),
            };
            let channel = |i: usize| {
                let c =
                    f64::from(u8::from_str_radix(&h[i..i + 2], 16).expect("a hex colour")) / 255.0;
                if c <= 0.03928 {
                    c / 12.92
                } else {
                    ((c + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * channel(0) + 0.7152 * channel(2) + 0.0722 * channel(4)
        };
        let (x, y) = (lum(a), lum(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    /// Task rust-doc-view-parity, done-when 2 (@nha, 2026-11-14: « diagrams are unreadable. Maybe dark mode messes things. »):
    /// a document's inline SVG, drawn in currentColor with the rfc skill's boxes, reads in light and dark. Under each theme's
    /// tokens (theme.css) the diagram's ground and ink resolve to its own pair, the same in both, never a theme token whose
    /// dark value would put light text on its light ground; its words and arrows take that ink, its boxes draw no fill (an
    /// SVG rect unstyled fills black), and its accent box keeps the ink readable.
    #[dash_test]
    fn a_diagram_reads_on_its_own_surface_in_both_themes() {
        let css = crate::component::css();
        let theme = include_str!("../../theme.css");
        let diagram = declared(css, ".diagram");
        for (k, v) in [
            ("background", "var(--diagram-surface)"),
            ("color", "var(--diagram-ink)"),
        ] {
            assert_eq!(
                diagram.get(k).map(String::as_str),
                Some(v),
                "{k}: {diagram:?}"
            );
        }
        let mut drawn = vec![];
        for look in ["light", "dark"] {
            // the theme's tokens, then the diagram's own, which a var inside it reads first
            let mut vars = declared(theme, &format!(".app.{look}"));
            vars.extend(
                diagram
                    .iter()
                    .filter(|(k, _)| k.starts_with("--"))
                    .map(|(k, v)| (k.clone(), v.clone())),
            );
            let resolved = |v: &str| {
                let name = v
                    .strip_prefix("var(")
                    .and_then(|r| r.strip_suffix(')'))
                    .unwrap_or(v);
                vars.get(name).cloned().unwrap_or_else(|| v.to_string())
            };
            let (ground, ink) = (
                resolved(&diagram["background"]),
                resolved(&diagram["color"]),
            );
            let accent = resolved(&diagram["--diagram-accent-ground"]);
            let (on_ground, on_accent) = (contrast(&ground, &ink), contrast(&accent, &ink));
            assert!(
                on_ground >= 7.0 && on_accent >= 7.0,
                "{look}: ink {ink} on {ground} {on_ground:.1}, on {accent} {on_accent:.1}"
            );
            // the dark theme's own text on this ground is what made them unreadable: it is never the ink there
            if look == "dark" {
                assert!(
                    contrast(&ground, &vars["--color-base-content"]) < 1.5,
                    "the dark theme's text would vanish here"
                );
                assert_ne!(
                    ink, vars["--color-base-content"],
                    "{look}: theme text on the diagram's ground"
                );
            }
            drawn.push((ground, ink));
        }
        assert_eq!(drawn[0], drawn[1], "one ground and ink in both themes");
        assert_eq!(declared(css, ".diagram svg")["color"], "var(--diagram-ink)");
        assert_eq!(
            declared(css, ".dv .diagram svg text:not([fill])")["fill"],
            "currentColor"
        );
        assert_eq!(
            declared(css, ".dv .diagram svg .box:not([fill])")["fill"],
            "none"
        );
        // a rule beats an SVG attribute: none overrides a fill, size or marker the drawing sets (/code-review of PR 1592)
        for (selector, attr) in [
            ("text", "fill"),
            ("text", "font-size"),
            (".box", "fill"),
            (".arrow", "marker-end"),
        ] {
            assert!(
                !declared(css, &format!(".dv .diagram svg {selector}")).contains_key(attr),
                "{selector} {attr} set over the drawing's own"
            );
        }
        assert_eq!(
            declared(css, ".dv .diagram svg .arrow")["stroke"],
            "currentColor"
        );
        assert!(
            !css.contains(".dark .diagram"),
            "no theme redraws a diagram's ground"
        );
    }

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
            let svg = scoped(&svg);
            rsx! { div { id, class: "diagram", role: "img", "aria-label": "{words}", dangerous_inner_html: svg } }
        }
        false => rsx! { img { id, class: "diagram", src: svg_uri(&svg), alt: "{words}" } },
    }
}

/// An inline SVG's ids prefixed by its own print, and each reference to them (`url(#…)`, `href="#…"`), so two drawings on one
/// page each keep their own marker: an id is page-wide, and the rfc skill's drawings all name theirs `arrow` (/code-review of
/// PR 1592). An `.arrow` with no marker of its own points at its drawing's `arrow`. A prefix an earlier draw wrote (a save's
/// export) is taken off first, so drawing it again gives the same markup.
fn scoped(svg: &str) -> String {
    let plain = unscoped(svg);
    let ids: Vec<&str> = (plain.split(" id=\"").skip(1))
        .filter_map(|r| r.split('"').next())
        .filter(|i| !i.is_empty())
        .collect();
    if ids.is_empty() {
        return plain;
    }
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    plain.hash(&mut h);
    let p = format!("s{:08x}", h.finish() as u32);
    let mut out = plain.clone();
    for id in &ids {
        for lead in LEADS {
            let end = if lead == "url(#" { ")" } else { "\"" };
            out = out.replace(&format!("{lead}{id}{end}"), &format!("{lead}{p}-{id}{end}"));
        }
    }
    if !ids.contains(&"arrow") {
        return out;
    }
    // each `class="arrow"` tag with no marker-end of its own
    let mut drawn = String::with_capacity(out.len());
    let mut rest = out.as_str();
    while let Some(at) = rest.find("class=\"arrow\"") {
        let end = at + "class=\"arrow\"".len();
        let open = rest[..at].rfind('<').unwrap_or(0);
        let close = rest[end..].find('>').map_or(rest.len(), |c| end + c);
        drawn.push_str(&rest[..end]);
        if !rest[open..close].contains("marker-end") {
            drawn.push_str(&format!(" marker-end=\"url(#{p}-arrow)\""));
        }
        rest = &rest[end..];
    }
    drawn.push_str(rest);
    drawn
}

const LEADS: [&str; 3] = [" id=\"", "url(#", "href=\"#"];

/// `scoped`'s markup as it was before: each print prefix and each marker it gave an `.arrow` taken off. A save's export
/// writes this, so the file keeps the drawing's own ids and reads back to the same block.
pub(crate) fn unscoped(svg: &str) -> String {
    let injected = "class=\"arrow\" marker-end=\"url(#";
    let mut parts = svg.split(injected);
    let mut plain = parts.next().unwrap_or_default().to_string();
    for part in parts {
        let ours =
            unprefixed(&format!("#{part}"), "#").starts_with("#arrow)\"") && part.len() >= 17;
        plain.push_str(if ours { "class=\"arrow\"" } else { injected });
        plain.push_str(if ours { &part[17..] } else { part });
    }
    for lead in LEADS {
        plain = unprefixed(&plain, lead);
    }
    plain
}

/// `svg` with each `lead` followed by a print prefix (`s` and 8 hex digits, then `-`) given back its bare id.
fn unprefixed(svg: &str, lead: &str) -> String {
    let mut parts = svg.split(lead);
    let mut out = parts.next().unwrap_or_default().to_string();
    for part in parts {
        let b = part.as_bytes();
        let prefixed = b.len() > 10
            && b[0] == b's'
            && b[1..9].iter().all(u8::is_ascii_hexdigit)
            && b[9] == b'-';
        out.push_str(lead);
        out.push_str(if prefixed { &part[10..] } else { part });
    }
    out
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
