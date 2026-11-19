//! The components every page draws with, each one of dioxuslabs.com/components ported to design.json's tokens (@nha,
//! 2026-11-09: « our design page should have https://dioxuslabs.com/components or equivalents either use or port what we
//! need », « We want all components to be dual-use »). Dual-use: each is a Dioxus element, so the window draws it and the
//! web's server render is its HTML, whose text form (crate::forms::text_of) is what an agent reads. /design?tab=components
//! (design/catalog.rs) shows each one's two forms side by side. `look` picks a component's look: its CSS class, never written by a page.
//! Each is a module of components/each/, declared in each.rs, that registers its /design entry, its classes, its tags and its
//! /design sections beside its code (crate::component); helpers several share are in each/parts.rs. A module names another
//! as `super::<module>::`, never through this root, so the modules hold no loop. The gate (dash-gate's `page_local_markup`)
//! refuses a registered class or tag anywhere but in a component's module, and `shared_component_lists` a list naming
//! components outside theirs.

pub mod each;
pub use each::*;
