//! Can a Blitz window hold several faces? `build_single_font_ctx` is the page's choice, not Blitz's: this registers a second face
//! in that same context and paints one fixture with each family name, as Chrome would pick them.
use blitz_dom::{Document, DocumentConfig, build_single_font_ctx};
use blitz_traits::shell::{ColorScheme, Viewport};
use dioxus::prelude::*;
use dioxus_native_dom::DioxusDocument;
use parley::fontique::Blob;
use std::sync::Arc;
use ui_spike_common::DESIGN_FONT;
use ui_spike_common::picture::Picture;

const SIZE: (u32, u32) = (320, 80);
const KALAM: &[u8] = include_bytes!("../../fonts/Kalam-latin.ttf");

#[derive(Props, Clone, PartialEq)]
struct Face {
    family: &'static str,
}

#[allow(non_snake_case)]
fn Fixture(p: Face) -> Element {
    rsx! { div { style: "font-family: {p.family}; font-size: 24px", "The same words" } }
}

fn paint(family: &'static str, two_faces: bool) -> Picture {
    use anyrender::ImageRenderer;
    let mut font_ctx = build_single_font_ctx(DESIGN_FONT);
    if two_faces {
        font_ctx.collection.register_fonts(Blob::new(Arc::new(KALAM.to_vec()) as _), None);
    }
    let mut doc = DioxusDocument::new(
        VirtualDom::new_with_props(Fixture, Face { family }),
        DocumentConfig {
            font_ctx: Some(font_ctx),
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
fn a_second_face_draws_by_its_family_name() {
    let sans = paint("'Instrument Sans'", true);
    let hand = paint("Kalam", true);
    let without = paint("Kalam", false);
    assert_ne!(sans.rgba, hand.rgba, "both draw the same face");
    assert_ne!(hand.rgba, without.rgba, "the name found no face it did not have before");
}
