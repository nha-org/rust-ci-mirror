//! Four screens of shadcn-style components on design.json's theme, drawn by Dioxus Native in the Mac window and by a
//! browser from the same Rust (section 10's try): Linear-like cards, an Excalidraw scene, Gmail-like lines over the
//! 100,000 rows, Notion-like comments. Nothing here calls JS: Blitz has none.
use dioxus::prelude::*;

mod board;
mod comments;
pub mod design;
mod diagram;
mod mail;
pub mod script;
pub mod theme;

pub use diagram::scene;
pub use mail::{rows_caught_up, scroll_top};

pub const THEME: &str = include_str!("../../../crates/surface/src/theme.css");
pub const CSS: &str = include_str!("page.css");
pub const WINDOW: (f64, f64) = (1200.0, 800.0);
/// The nav's width and the bar's height: where a screen's content starts in the window's coordinates. Client
/// coordinates less these, because element coordinates mean the event's target on the web and differ by renderer.
pub const NAV_W: f64 = 200.0;
pub const BAR_H: f64 = ui_spike_common::HEADER_H;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Screen {
    Board,
    Diagram,
    Mail,
    Comments,
}

#[component]
pub(crate) fn Button(
    id: String,
    #[props(default = "button")] class: &'static str,
    onclick: EventHandler<MouseEvent>,
    children: Element,
) -> Element {
    rsx! {
        button { id, class, onclick: move |e| onclick.call(e), {children} }
    }
}

#[component]
pub(crate) fn Input(
    id: &'static str,
    value: String,
    #[props(default = "input")] class: &'static str,
    #[props(default)] placeholder: &'static str,
    oninput: EventHandler<String>,
) -> Element {
    rsx! {
        input { id, class, placeholder, value, oninput: move |e| oninput.call(e.value()) }
    }
}

#[component]
pub(crate) fn Badge(class: &'static str, children: Element) -> Element {
    rsx! { span { class: "badge {class}", {children} } }
}

#[component]
pub(crate) fn Avatar(initials: &'static str) -> Element {
    rsx! { div { class: "avatar", "{initials}" } }
}

/// The root's last child, never nested (section 10's « Overlays at the root »). A plain open dialog: showModal needs JS.
#[component]
pub(crate) fn Dialog(title: String, onclose: EventHandler<()>, children: Element) -> Element {
    rsx! {
        div { id: "backdrop", class: "backdrop", onclick: move |_| onclose.call(()),
            dialog { id: "dialog", class: "dialog", open: true, onclick: move |e| e.stop_propagation(),
                div { class: "dialog-title", "{title}" }
                {children}
            }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct AppProps {
    pub start: Screen,
}

#[allow(non_snake_case)]
pub fn App(props: AppProps) -> Element {
    let mut screen = use_signal(|| props.start);
    let mut dark = use_signal(|| false);
    let theme = if dark() { "app dark" } else { "app light" };
    let items = [
        ("nav-board", "Board", Screen::Board),
        ("nav-diagram", "Diagram", Screen::Diagram),
        ("nav-mail", "Mail", Screen::Mail),
        ("nav-comments", "Comments", Screen::Comments),
    ];
    rsx! {
        style { {THEME} }
        style { {CSS} }
        div { class: theme,
            div { class: "nav",
                div { class: "nav-title", "Dash" }
                for (id, name, s) in items {
                    div { id, key: "{id}", class: if screen() == s { "nav-item on" } else { "nav-item" },
                        onclick: move |_| screen.set(s), "{name}"
                    }
                }
                div { class: "nav-foot",
                    Button { id: "theme-toggle", class: "button quiet", onclick: move |_| dark.toggle(), if dark() { "Light" } else { "Dark" } }
                }
            }
            match screen() {
                Screen::Board => rsx! { board::Board {} },
                Screen::Diagram => rsx! { diagram::Diagram {} },
                Screen::Mail => rsx! { mail::Mail {} },
                Screen::Comments => rsx! { comments::Comments {} },
            }
        }
    }
}

pub fn vdom(start: Screen) -> VirtualDom {
    VirtualDom::new_with_props(App, AppProps { start })
}
