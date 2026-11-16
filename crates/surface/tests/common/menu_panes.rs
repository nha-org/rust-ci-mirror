// The menu's panes as the binary registers them beside its pages (dash_core::pane!), for the design system's tests, which
// link no binary: Design is the design system's own. notion_browser.rs includes it, forms.rs's tests hold the same list; the binary's
// pages.rs test holds both to the real panes.
macro_rules! panes {
    ($(($name:literal, $href:literal, $order:literal, $icon:literal, [$($path:literal),*], $under:expr)),* $(,)?) => {
        $(inventory::submit! { dash_core::switches::Pane { name: $name, href: $href, order: $order, icon: $icon,
            paths: &[$($path),*], under: $under } })*
    };
}
panes![
    ("Inbox", "/inbox", 10, "inbox", [], None),
    ("Today", "/today", 20, "calendar", [], None),
    ("Mail", "/mail", 30, "mail", [], None),
    ("Tasks", "/tasks", 40, "check", ["/graph"], None),
    ("Projects", "/projects", 50, "board", [], None),
    ("Plan", "/plan", 60, "list", [], None),
    (
        "Documents",
        "/documents",
        70,
        "doc",
        ["/v/private/docs"],
        None
    ),
    ("Insights", "/insights", 80, "chart", [], None),
    ("Agents", "/agents", 90, "agent", [], None),
    ("Comments", "/comments", 0, "", [], Some("/agents")),
    ("Tools", "/tools", 100, "tool", [], None),
    ("Watchers", "/watchers", 0, "", [], Some("/tools")),
    ("Status", "/status", 110, "signal", [], None),
    ("Code", "/deps", 0, "", [], Some("/design")),
    ("Settings", "/settings", 130, "sliders", [], None),
];
