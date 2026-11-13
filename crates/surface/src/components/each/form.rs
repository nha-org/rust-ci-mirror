use super::button::Button;
use super::dropzone::{FileUpload, drop_words};
use super::input::Input;
use crate::component::Component;
use dioxus::prelude::*;

inventory::submit! {
    Component {
        slug: "form",
        classes: &["form"],
        tags: &["form"],
        name: "Form",
        used: "Fields and one button that send: the search bar, a mail's reply, a draft's approval, /settings' backup read back (a file, an « upload » in a tree) and its confirmation (an « action »).",
        example: Some(|_| {
            rsx! {
                Form { id: "c-form-search", role: "search", method: "get", action: "/search",
                    Input { r#type: "search", name: "q", placeholder: "Search tasks" }
                    Button { "Search" }
                }
                Form { id: "c-form-upload", action: "/settings/restore", file: true,
                    FileUpload { name: "file", accept: ".zip", label: drop_words(".zip"), required: true,
                        Button { "Read a backup" }
                    }
                }
            }
        }),
        menu: true,
        terminal: Some(crate::component::Terminal::Words),
        ..Component::BARE
    }
}

/// Fields and one button that send to `action`: the search bar (`get`), a reply, an upload (`file`: its picker, sent as
/// multipart), a step he confirms. Its own tag, so no page writes one by hand (task dash-settings-backup's forms were).
#[component]
pub fn Form(
    #[props(into)] id: Option<String>,
    #[props(into)] action: String,
    #[props(default = "post")] method: &'static str,
    #[props(default)] file: bool,
    role: Option<&'static str>,
    /// Kept from view: a PropertyMenu's choice its filter leaves out.
    #[props(default)]
    hidden: bool,
    children: Element,
) -> Element {
    let (enctype, hidden) = (
        file.then_some("multipart/form-data"),
        hidden.then_some(true),
    );
    rsx! { form { id, class: "form", role, method, action, enctype, hidden, {children} } }
}

/// The search field: it GETs /search, `value` the words shown back in it, `hidden` the fields sent with them (/inbox's
/// `in=inbox`). One component for a page's field and the menu's (task rust-search-port).
#[component]
pub fn SearchBar(
    #[props(into)] id: Option<String>,
    #[props(into)] placeholder: String,
    #[props(default, into)] value: String,
    #[props(default)] hidden: Vec<(String, String)>,
) -> Element {
    let hidden = hidden.iter().map(|(name, value)| {
        rsx! { Input { r#type: "hidden", name: "{name}", value: "{value}" } }
    });
    let value = (!value.is_empty()).then_some(value);
    rsx! {
        Form { id, role: "search", method: "get", action: "/search",
            {hidden}
            Input { r#type: "search", name: "q", placeholder: "{placeholder}", value }
            Button { "Search" }
        }
    }
}
