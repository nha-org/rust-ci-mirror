//! The Blitz side of the spike. dioxus-components' VirtualList reads the scroll offset through document::eval, which
//! needs JS, and Blitz has none: so this windows the rows by hand on onscroll, the same arithmetic in 10 lines.
use anyrender::ImageRenderer;
use anyrender_vello_cpu::VelloCpuImageRenderer;
use blitz_dom::{Document, DocumentConfig, build_single_font_ctx};
use blitz_paint::paint_scene;
use blitz_traits::events::{
    BlitzPointerEvent, BlitzPointerId, BlitzWheelDelta, BlitzWheelEvent, PointerCoords, UiEvent,
};
use blitz_traits::shell::{ColorScheme, Viewport};
use dioxus::prelude::*;
use dioxus_native_dom::DioxusDocument;
use std::sync::atomic::{AtomicU64, Ordering};
use ui_spike_common::{INTER, ROW_H, ROWS, Tone, row};

const CSS: &str = include_str!("list.css");
/// Rows drawn at once: the tallest window of the picture and measure sizes (800 px), plus overscan like Slint's.
const WINDOW_ROWS: usize = 18;

/// The scroller's offset, for the meter outside the component.
pub static SCROLL_TOP: AtomicU64 = AtomicU64::new(0);

/// The offset the rows were last built for: a frame painted before they catch up shows old rows and must not count.
pub static RENDERED_TOP: AtomicU64 = AtomicU64::new(0);

pub fn scroll_top() -> f64 {
    f64::from_bits(SCROLL_TOP.load(Ordering::Relaxed))
}

pub fn rows_caught_up() -> bool {
    RENDERED_TOP.load(Ordering::Relaxed) == SCROLL_TOP.load(Ordering::Relaxed)
}

#[derive(Props, Clone, PartialEq)]
pub struct AppProps {
    pub dark: bool,
}

#[allow(non_snake_case)]
pub fn App(props: AppProps) -> Element {
    let mut top = use_signal(|| 0.0f64);
    RENDERED_TOP.store(top.read().to_bits(), Ordering::Relaxed);
    let first = (*top.read() / ROW_H) as usize;
    let last = (first + WINDOW_ROWS).min(ROWS);
    let offset = first as f64 * ROW_H;
    let total = ROWS as f64 * ROW_H;
    let theme = if props.dark { "app dark" } else { "app light" };
    rsx! {
        style { {CSS} }
        div { class: theme,
            div { class: "header",
                span { class: "title", "Needs you" }
                span { class: "badge secondary", "100,000" }
            }
            div { class: "scroller",
                onscroll: move |e| {
                    let y = e.data().scroll_top();
                    SCROLL_TOP.store(y.to_bits(), Ordering::Relaxed);
                    top.set(y);
                },
                div { class: "spacer", style: "height:{total}px",
                    div { class: "window", style: "top:{offset}px",
                        for i in first..last {
                            {
                                let r = row(i);
                                let tone = match r.tone {
                                    Tone::Primary => "badge primary",
                                    Tone::Secondary => "badge secondary",
                                    Tone::Outline => "badge outline",
                                    Tone::Destructive => "badge destructive",
                                };
                                rsx! {
                                    div { class: "row",
                                        div { class: "avatar", "{r.initials}" }
                                        div { class: "t", "{r.title}" }
                                        div { class: "s", "{r.subtitle}" }
                                        div { class: tone, "{r.badge}" }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

pub fn vdom(dark: bool) -> VirtualDom {
    VirtualDom::new_with_props(App, AppProps { dark })
}

/// The list with no window, on vello's CPU renderer: what the picture test and --headless share.
pub struct Headless {
    doc: DioxusDocument,
    renderer: VelloCpuImageRenderer,
    buffer: Vec<u8>,
    width: u32,
    height: u32,
    scale: f64,
    coords: PointerCoords,
}

impl Headless {
    pub fn new(dark: bool, width: u32, height: u32, scale: f32) -> Self {
        let scheme = if dark {
            ColorScheme::Dark
        } else {
            ColorScheme::Light
        };
        let mut doc = DioxusDocument::new(
            vdom(dark),
            DocumentConfig {
                font_ctx: Some(build_single_font_ctx(INTER)),
                viewport: Some(Viewport::new(width, height, scale, scheme)),
                ..Default::default()
            },
        );
        doc.initial_build();
        let (x, y) = (100.0, 200.0);
        let coords = PointerCoords {
            page_x: x,
            page_y: y,
            screen_x: x,
            screen_y: y,
            client_x: x,
            client_y: y,
        };
        let renderer = VelloCpuImageRenderer::new(width, height);
        let buffer = vec![0; (width * height * 4) as usize];
        let mut this = Self {
            doc,
            renderer,
            buffer,
            width,
            height,
            scale: scale as f64,
            coords,
        };
        this.settle();
        this.doc
            .handle_ui_event(UiEvent::PointerMove(BlitzPointerEvent {
                id: BlitzPointerId::Mouse,
                is_primary: true,
                coords,
                button: Default::default(),
                buttons: Default::default(),
                mods: Default::default(),
                details: Default::default(),
                element: Default::default(),
                active_pointers: Default::default(),
            }));
        this.settle();
        this
    }

    fn settle(&mut self) {
        for _ in 0..8 {
            if !self.doc.poll(None) {
                break;
            }
        }
        self.doc.inner_mut().resolve(0.0);
    }

    /// The wheel path a user takes: a programmatic scroll fires no onscroll in this Blitz.
    pub fn scroll_by(&mut self, pixels: f64) {
        if pixels > 0.0 {
            self.doc.handle_ui_event(UiEvent::Wheel(BlitzWheelEvent {
                delta: BlitzWheelDelta::Pixels(0.0, -pixels),
                coords: self.coords,
                buttons: Default::default(),
                mods: Default::default(),
                element: Default::default(),
            }));
            self.settle();
        }
    }

    /// RGBA, opaque. The renderer and its buffer stay, as Slint's pixel buffer does: fair frame times and peak.
    pub fn render(&mut self) -> &[u8] {
        let (w, h, scale) = (self.width, self.height, self.scale);
        let inner = &mut self.doc.inner_mut();
        // render() keeps the last frame's scene: unreset, each frame painted over all the ones before and a run never ended.
        self.renderer.reset();
        self.renderer.render(
            |scene| paint_scene(scene, inner, scale, w, h, 0, 0),
            &mut self.buffer,
        );
        &self.buffer
    }
}
