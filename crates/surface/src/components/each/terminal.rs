use super::button::Button;
use super::form::Form;
use super::input::Input;
use dioxus::prelude::*;

/// The id of a terminal's screen inside the `Terminal` drawn under `id`: the one part its patch names.
pub fn terminal_screen(id: &str) -> String {
    format!("{id}-screen")
}

/// A terminal's screen as its program left it, then one line he types into it, posted to `action` (task rust-terminal-page,
/// rfc-rust-unified 20f). The screen alone follows the terminal's view (`data-view`): the page's LIVE_JS swaps it at each
/// change, and the field below keeps what he is typing. The page scrolls, never the screen: a swapped box would start at its
/// top again, hiding the prompt (/code-review).
#[component]
pub fn Terminal(
    #[props(into)] id: String,
    #[props(into)] text: String,
    #[props(into)] action: String,
) -> Element {
    let screen = terminal_screen(&id);
    rsx! {
        div { id, class: "terminal",
            pre { id: screen, class: "terminal-screen", "data-view": "terminal", "{text}" }
            Form { action,
                Input { r#type: "text", name: "text", placeholder: "A line to type; Enter sends it", autocomplete: "off" }
                Button { "Send" }
            }
        }
    }
}
