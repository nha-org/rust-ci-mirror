//! /design, the design system's catalog (task rust-design-page): drawn by dash-surface, registered here as each page is from
//! its own module (pages.rs, task rust-registry-no-hubs).
use crate::pages::{Page, Reads};
use dash_surface as surface;

inventory::submit! {
    Page { path: "/design", step: "route_design", reads: Reads::Nothing, redraw: false, serve: |a| {
        let tab = a.param("tab").unwrap_or_default();
        let theme = a.param("theme");
        a.form(Some(Ok(surface::Drawing::Page(surface::design::page(&tab, theme.as_deref())))))
    } }
}
