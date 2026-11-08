//! Adapter of section 3: the native window (section 9), Dioxus Native as rust/ui-spike picked it (@nha, 2026-11-08: « DIOXUS IS
//! VALIDATED »). It draws whatever component it is given, so the screen stays the surface crate's, the one the web page renders.
use blitz_dom::Document;
use dioxus::prelude::*;
use dioxus_native_dom::DioxusDocument;

/// The document a window would draw, built with no display: what a test reads of the window.
pub fn document(dom: VirtualDom) -> DioxusDocument {
    let mut doc = DioxusDocument::new(dom, Default::default());
    doc.initial_build();
    doc
}

/// The words a window of this component shows, its stylesheet left out.
pub fn shown(dom: VirtualDom) -> String {
    let doc = document(dom);
    let doc = doc.inner();
    let mut words = String::new();
    let mut at = vec![doc.root_element().id];
    while let Some(id) = at.pop() {
        let node = doc.get_node(id).expect("a node of the tree");
        if node
            .data
            .is_element_with_tag_name(&blitz_dom::local_name!("style"))
        {
            continue;
        }
        if let Some(text) = node.text_data() {
            words += &text.content;
            words.push('\n');
        }
        at.extend(node.children.iter().rev());
    }
    words
}

/// The window, until it is closed.
#[cfg(feature = "window")]
pub fn open<P: Clone + 'static, M: 'static>(
    app: impl dioxus::dioxus_core::ComponentFunction<P, M>,
    props: P,
    title: &str,
) {
    use dioxus_native::{LogicalSize, WindowAttributes};
    let window = WindowAttributes::default()
        .with_title(title)
        .with_surface_size(LogicalSize::new(1200.0, 800.0));
    dioxus_native::launch_cfg_with_props(app, props, vec![], vec![Box::new(window)]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::memory::TestBudget;

    #[test]
    fn a_component_s_words_are_what_its_window_shows() {
        let _budget = TestBudget::new();
        fn app() -> Element {
            rsx! { style { "p {{ color: red }}" } h1 { "Rust tasks" } p { "a-task" } }
        }
        assert_eq!(shown(VirtualDom::new(app)), "Rust tasks\na-task\n");
    }
}
