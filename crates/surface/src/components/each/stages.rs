use crate::component::Component;
use dioxus::prelude::*;

inventory::submit! {
    Component {
        slug: "stages",
        classes: &["doc-stage"],
        tags: &[],
        name: "Stage",
        used: "An RFC's stage in its header line on /v/, the rfc skill's `p.doc-stage`: one label, its name then its value as a \
               pill, as Notion's select property (@nha, 2026-11-14: « draft/review etc. should be a single selector/label. \
               notion-style. »).",
        example: Some(|_| rsx! { Stages { current: "Review" } }),
        css: include_str!("stages.css"),
        terminal: Some(crate::component::Terminal::Words),
        ..Component::BARE
    }
}

/// A document's stage: `current` the one it is at, `label` its name, the doc's own word (« Stage » when empty). Its markup is
/// the rfc skill's `p.doc-stage`, so `dash doc import` reads an export back to the same block.
#[component]
pub fn Stages(
    #[props(into)] id: Option<String>,
    #[props(into)] current: String,
    #[props(into, default)] label: String,
) -> Element {
    let label = if label.is_empty() {
        "Stage".to_string()
    } else {
        label
    };
    // a space between, so its text form reads « Stage Review » (a flex row drops it on the page)
    rsx! { p { id, class: "doc-stage", span { "{label}" } " " b { "{current}" } } }
}

/// The stage a page's own label names (its `p.doc-stage`'s `b`), the one reading of the markup `Stages` writes: the docs' store
/// keeps it beside each page, so the home page's « Docs to review » holds no markup of its own (task rfc-view-doc-type). Its
/// `span` and `b` may carry ids, its `p` other classes; a bold word past the label is not its stage. Empty with no label.
pub fn stage_in(page: &str) -> String {
    label_in(page).unwrap_or_default()
}

fn label_in(page: &str) -> Option<String> {
    let mut rest = page;
    while let Some(at) = rest.find("<p") {
        let open = &rest[at..];
        let end = open.find('>')?;
        let (tag, after) = (&open[..end], &open[end + 1..]);
        rest = after;
        let classes = (tag
            .split_once(" class=\"")
            .and_then(|(_, c)| c.split_once('"')))
        .map_or("", |(c, _)| c);
        if !tag.starts_with("<p ") || !classes.split_whitespace().any(|c| c == "doc-stage") {
            continue;
        }
        let inside = &after[..after.find("</p>")?];
        let b = (inside.match_indices("<b"))
            .find(|(i, _)| matches!(inside.as_bytes().get(i + 2), Some(b'>' | b' ')))?
            .0;
        let words = &inside[b..];
        let words = &words[words.find('>')? + 1..];
        let words = words[..words.find('<').unwrap_or(words.len())].trim();
        return (!words.is_empty()).then(|| words.to_string());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;

    /// What `Stages` writes reads back; ids and other classes are read too, a bold word past the label is not its stage.
    #[dash_test]
    fn a_page_s_stage_is_read_from_its_label_alone() {
        let drawn = dioxus_ssr::render_element(rsx! { Stages { current: "Review" } });
        assert_eq!(stage_in(&drawn), "Review", "{drawn}");
        assert_eq!(
            stage_in(
                r#"<p class="doc-stage x" id="st"><span>Étape</span> <b id="s-now">Relecture</b></p>"#
            ),
            "Relecture"
        );
        assert_eq!(
            stage_in(
                r#"<p class="doc-stage"><span>Stage</span> <br></p><p><b>Review</b> next.</p>"#
            ),
            ""
        );
        assert_eq!(stage_in(r#"<p class="lede"><b>Review</b></p>"#), "");
    }
}
