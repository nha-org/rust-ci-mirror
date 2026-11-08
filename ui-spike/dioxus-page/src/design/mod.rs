//! The design system page lives in dash-surface now (task rust-design-page), the one source dash serve and dash ui draw;
//! this spike keeps only what drives it in its own window: the fonts and the script.
pub use dash_surface::design::*;
use dioxus::prelude::*;

pub mod script;

pub fn vdom() -> VirtualDom {
    VirtualDom::new_with_props(Design, DesignProps::default())
}

/// design.json's three faces in one collection: sans, then `monospace` and `cursive`.
pub fn font_ctx() -> blitz_dom::FontContext {
    use parley::fontique::{Blob, GenericFamily};
    let mut ctx = blitz_dom::build_single_font_ctx(ui_spike_common::DESIGN_FONT);
    for (face, generic) in [(ui_spike_common::MONO_FONT, GenericFamily::Monospace), (ui_spike_common::HAND_FONT, GenericFamily::Cursive)] {
        let decoded = blitz_dom::decode_font_bytes(face).into_owned();
        let families = ctx.collection.register_fonts(Blob::new(std::sync::Arc::new(decoded) as _), None);
        ctx.collection.set_generic_families(generic, families.iter().map(|(id, _)| *id));
    }
    ctx
}
