use super::parts::{CALENDAR, DOTS, INBOX, MAIL, icon};
use crate::component::Component;
use dioxus::prelude::*;

inventory::submit! {
    Component {
        slug: "bottom_nav",
        classes: &["bottom-nav", "bottom-nav-grid", "bottom-nav-item", "bottom-nav-label"],
        tags: &[],
        name: "BottomNav",
        used: "Every page's menu on a phone, under 700 px wide: its first entries at the screen's foot, the rest behind More, which opens the sidebar. Built on rust-ui's BottomNav, which dioxus-components lacks.",
        example: Some(|_| {
            rsx! {
                BottomNav { id: "c-bottom-nav", label: "Pages",
                    BottomNavItem { href: "/inbox", current: true, icon: icon(INBOX), "Inbox" }
                    BottomNavItem { href: "/today", current: false, icon: icon(CALENDAR), "Today" }
                    BottomNavItem { href: "/mail", current: false, icon: icon(MAIL), "Mail" }
                    BottomNavMore { id: "c-bottom-nav-more", r#for: "c-bottom-nav-fold", controls: "c-bottom-nav-rest", current: false, icon: icon(DOTS), "More" }
                }
            }
        }),
        library: Some("https://rust-ui.com/docs/components/bottom-nav"),
        menu: true,
        terminal: Some(crate::component::Terminal::Words),
        ..Component::BARE
    }
}

/// A phone's menu at the foot of the screen: rust-ui's BottomNav (notice in NOTICE-rust-ui), which dioxus-components lacks,
/// on this system's tokens and in dioxus-components' way, its parts plain markup and `aria-current` the state (task
/// rust-ui-bottom-nav; @nha, 2026-11-10: « mobile - https://rust-ui.com/docs/components/bottom-nav »). Kept from rust-ui:
/// one grid row of equal columns, each an icon over its label, the current one in the primary tone, a press that shrinks it,
/// no tap flash or callout, and room for the phone's home bar. Where it sits is its page's: it draws in place.
#[component]
pub fn BottomNav(#[props(into)] id: String, label: &'static str, children: Element) -> Element {
    rsx! { nav { id, class: "bottom-nav", "aria-label": label, div { class: "bottom-nav-grid", {children} } } }
}

/// One page of a `BottomNav`: its icon over its name, `aria-current` on the shown one.
#[component]
pub fn BottomNavItem(
    #[props(into)] href: String,
    current: bool,
    icon: Element,
    children: Element,
) -> Element {
    let on = current.then_some("page");
    rsx! { a { href, class: "bottom-nav-item", "aria-current": on, {icon} span { class: "bottom-nav-label", {children} } } }
}

/// A `BottomNav`'s last slot, what it leaves out: the label of the checkbox `r#for` that shows it, so it needs no script;
/// marked current (`aria-current="true"`, a button is no page) when the page is one of those, `controls` the part it shows.
/// A key reaches it, and its `aria-expanded` follows, through its page's script, as the checkbox is hidden.
#[component]
pub fn BottomNavMore(
    #[props(into)] id: String,
    #[props(into)] r#for: String,
    #[props(into)] controls: String,
    current: bool,
    icon: Element,
    children: Element,
) -> Element {
    let on = current.then_some("true");
    rsx! {
        label { id, r#for, class: "bottom-nav-item", role: "button", tabindex: "0", "aria-controls": controls, "aria-current": on,
            {icon} span { class: "bottom-nav-label", {children} }
        }
    }
}
