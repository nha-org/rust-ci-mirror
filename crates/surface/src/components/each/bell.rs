use super::parts::icon;
use crate::component::Component;
use dioxus::prelude::*;

inventory::submit! {
    Component {
        slug: "menu-bell",
        classes: &["menu-bell"],
        tags: &[],
        name: "Bell",
        used: "The menu's notifications: the bell and its unread count, opening the notifications page, as the older app's top bar \
               carried it (DashboardWeb.Bell).",
        example: Some(|_| rsx! {
            Bell { href: "/dashboard/push", unread: Some(3) }
            Bell { href: "/dashboard/push", unread: Some(0) }
            Bell { href: "/dashboard/push", unread: None }
        }),
        menu: true,
        ..Component::BARE
    }
}

/// The bell's stroke, the older app's hero-bell.
pub(crate) const BELL: &str = "M6 16V11a6 6 0 0112 0v5l2 2H4zM10 21h4";

/// The notifications' bell (task rust-shell-theme-bell): a link to `href`, named « Notifications, N unread » as the older bell
/// was, its count a Badge on it when any is unread. `unread`: none when the count was not read, the bell then named alone. `id`:
/// the menu's; none on /design.
#[component]
pub fn Bell(
    #[props(into)] href: String,
    unread: Option<u64>,
    #[props(default)] id: Option<&'static str>,
) -> Element {
    let name = match unread {
        Some(n) => format!("Notifications, {n} unread"),
        None => "Notifications".to_string(),
    };
    rsx! {
        a { class: "button icon menu-bell", id, href, title: "{name}", aria_label: "{name}",
            span { class: "text-only", "{name}" }
            {icon(BELL)}
            if let Some(n) = unread.filter(|n| *n > 0) {
                span { class: "badge count", "aria-hidden": "true", if n > 99 { "99+" } else { "{n}" } }
            }
        }
    }
}
