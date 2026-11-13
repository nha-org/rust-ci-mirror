use crate::component::Component;
use dioxus::prelude::*;

inventory::submit! {
    Component {
        slug: "splitter",
        classes: &["split", "splitter", "split-drag"],
        tags: &[],
        name: "Splitter",
        used: "Columns side by side, each but the last sized by the bar on its right: dragged with the mouse, or arrows on the \
               focused bar. On a phone (700px or less) the columns stack and the bars go.",
        css: include_str!("splitter.css"),
        ..Component::BARE
    }
}

/// A column's narrowest and widest, in pixels.
pub const NARROWEST: f64 = 140.0;
pub const WIDEST: f64 = 520.0;
/// An arrow key's step.
const STEP: f64 = 16.0;

/// A column's width as a bar sets it: within NARROWEST and WIDEST.
pub fn sized(width: f64) -> f64 {
    width.clamp(NARROWEST, WIDEST)
}

/// What a Split shares with its bars: each column's width, and the bar held (its column, where the press was, the width then).
#[derive(Clone, Copy)]
struct Sizes {
    widths: Signal<Vec<f64>>,
    held: Signal<Option<(usize, f64, f64)>>,
}

/// Columns side by side, each but the last as wide as `widths` says until a `Splitter` resizes it (task
/// design-doc-tree-menu; @nha, 2026-11-12: « perhaps vertical splitters »). Its children are the columns, a `Splitter`
/// between each two. While a bar is held, a layer over the page takes the moves, so only a drag sends them (liveview sends
/// each over its socket).
#[component]
pub fn Split(
    #[props(into)] id: String,
    label: &'static str,
    widths: Vec<f64>,
    children: Element,
) -> Element {
    let mut widths = use_signal(|| widths);
    let mut held = use_signal(|| None::<(usize, f64, f64)>);
    use_context_provider(|| Sizes { widths, held });
    let columns: String = widths().iter().map(|w| format!("{w}px 6px ")).collect();
    rsx! {
        div { id, class: "split", role: "group", "aria-label": label, style: "--split:{columns}minmax(0, 1fr)",
            {children}
            if let Some((at, from, width)) = held() {
                div { class: "split-drag",
                    onmousemove: move |e: MouseEvent| {
                        let x = e.client_coordinates().x;
                        if let Some(w) = widths.write().get_mut(at) {
                            *w = sized(width + x - from);
                        }
                    },
                    onmouseup: move |_| held.set(None),
                }
            }
        }
    }
}

/// The bar right of column `at` of its Split: pressed and dragged, or arrows once focused, it resizes that column.
#[component]
pub fn Splitter(at: usize, label: &'static str) -> Element {
    let Sizes {
        mut widths,
        mut held,
    } = use_context::<Sizes>();
    let width = widths.read().get(at).copied().unwrap_or(NARROWEST);
    rsx! {
        div { class: "splitter", role: "separator", tabindex: "0", "aria-orientation": "vertical", "aria-label": label,
            "aria-valuenow": "{width}", "aria-valuemin": "{NARROWEST}", "aria-valuemax": "{WIDEST}",
            onmousedown: move |e: MouseEvent| {
                e.prevent_default();
                held.set(Some((at, e.client_coordinates().x, width)));
            },
            onkeydown: move |e: KeyboardEvent| {
                // by name, as the compass reads Escape: the same in the window and through liveview
                let by = match e.key().to_string().as_str() {
                    "ArrowLeft" => -STEP,
                    "ArrowRight" => STEP,
                    _ => return,
                };
                // the width now, not the drawn one: keys repeated before a redraw each count
                if let Some(w) = widths.write().get_mut(at) {
                    *w = sized(*w + by);
                }
            },
        }
    }
}
