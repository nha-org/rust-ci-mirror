use super::fold::Fold;
use crate::component::Component;
use dioxus::prelude::*;
use std::collections::BTreeMap;

inventory::submit! {
    Component {
        slug: "diagram",
        classes: &["diagram"],
        tags: &[],
        name: "Diagram",
        used: "/deps' crate graph: its Mermaid source is the text, Mermaid with ELK's layout draws it, a box a layer.",
        example: Some(|_| {
            rsx! {
                Diagram { id: "c-diagram", source: "flowchart LR\n  subgraph l0[\"Core\"]\n    c_core[\"core\"]\n  end\n  c_store[\"store\"] --> c_core\n" }
            }
        }),
        formed: true,
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
    use dash_meter::memory::TestBudget;

    /// The crate graph's diagram: each box's name drawn, a label is text and never markup, and a source the renderer refuses
    /// is an error, never a page without its diagram unsaid.
    #[test]
    fn a_diagram_is_svg_drawn_here_its_labels_text_and_a_bad_source_an_error() {
        let _budget = TestBudget::new();
        let svg = diagram_svg("flowchart LR\n  a[\"surface\"] --> b[\"<script>x</script>\"]\n")
            .expect("drawn");
        assert!(
            svg.starts_with("<svg") && svg.contains(">surface<"),
            "{svg}"
        );
        assert!(!svg.contains("<script"), "{svg}");
        assert!(diagram_svg("not a diagram").is_err());
    }
}
