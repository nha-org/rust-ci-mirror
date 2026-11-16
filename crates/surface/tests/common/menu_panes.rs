// The menu's panes as the binary registers them beside its pages (dash_core::pane!), for the design system's tests, which
// link no binary: Design is the design system's own. notion_browser.rs includes it, forms.rs's tests hold the same list; the binary's
// pages.rs test holds both to the real panes.
macro_rules! panes {
    ($(($origin:literal, $name:literal, $href:literal, $order:literal, $icon:literal, [$($path:literal),*], $under:expr)),* $(,)?) => {
        $(inventory::submit! { dash_core::switches::Pane { origin: $origin, name: $name, href: $href, order: $order, icon: $icon,
            paths: &[$($path),*], under: $under } })*
    };
}
panes![
    ("dash::tasks", "Inbox", "/inbox", 10, "inbox", [], None),
    ("dash::today", "Today", "/today", 20, "calendar", [], None),
    ("dash::mail", "Mail", "/mail", 30, "mail", [], None),
    (
        "dash::tasks",
        "Tasks",
        "/tasks",
        40,
        "check",
        ["/graph"],
        None
    ),
    (
        "dash::fallback",
        "Projects",
        "/projects",
        50,
        "board",
        [],
        None
    ),
    ("dash::tasks", "Plan", "/plan", 60, "list", [], None),
    (
        "dash::doc_index",
        "Documents",
        "/documents",
        70,
        "doc",
        ["/v/private/docs"],
        None
    ),
    (
        "dash::tasks",
        "Insights",
        "/insights",
        80,
        "chart",
        [],
        None
    ),
    ("dash::agents", "Agents", "/agents", 90, "agent", [], None),
    (
        "dash::comments",
        "Comments",
        "/comments",
        0,
        "",
        [],
        Some("/agents")
    ),
    ("dash::access", "Tools", "/tools", 100, "tool", [], None),
    (
        "dash::comments",
        "Watchers",
        "/watchers",
        0,
        "",
        [],
        Some("/tools")
    ),
    (
        "dash::fallback",
        "Status",
        "/status",
        110,
        "signal",
        [],
        None
    ),
    ("dash::codemap", "Code", "/deps", 0, "", [], Some("/design")),
    (
        "dash::settings",
        "Settings",
        "/settings",
        130,
        "sliders",
        [],
        None
    ),
];
