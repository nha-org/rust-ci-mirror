//! A Notion-like page: blocks, a comment on a hovered block, a rail of threads, a dialog to resolve one.
use crate::{Avatar, Button, Dialog, Input};
use dioxus::prelude::*;

const BLOCKS: [(&str, &str); 5] = [
    ("h2", "One tree, two renderers"),
    ("", "A screen is written once, as data. The window draws it with Dioxus components and the web draws it from the same tree."),
    ("callout", "Each behaviour is checked against the live document, so a pass means the page really changed."),
    ("h2", "What is left"),
    ("", "The window's memory is traced on the board, and the budget is then met or re-set."),
];

#[derive(Clone, PartialEq)]
struct Thread {
    id: u32,
    block: usize,
    text: String,
}

#[component]
pub(crate) fn Comments() -> Element {
    let mut threads = use_signal(|| vec![Thread { id: 1, block: 2, text: "Is the pixel checked too?".into() }]);
    let mut composing = use_signal(|| None::<usize>);
    let mut draft = use_signal(String::new);
    let mut resolving = use_signal(|| None::<u32>);
    let mut next = use_signal(|| 2);
    let mut resolved = use_signal(|| 0);
    let mut add = move || {
        if let Some(block) = composing() {
            let id = next();
            threads.write().push(Thread { id, block, text: draft() });
            next += 1;
            draft.set(String::new());
            composing.set(None);
        }
    };
    rsx! {
        div { class: "screen",
            div { class: "bar",
                span { class: "bar-title", "Comments" }
                span { id: "thread-count", class: "meta", "{threads.read().len()} open, {resolved} resolved" }
            }
            div { class: "doc-wrap",
                div { class: "doc",
                    div { class: "doc-title", "Rendering plan" }
                    for (b, (kind, text)) in BLOCKS.into_iter().enumerate() {
                        div { id: "block-{b}", key: "{b}",
                            class: if threads.read().iter().any(|t| t.block == b) { "block {kind} marked" } else { "block {kind}" },
                            "{text}"
                            div { id: "add-{b}", class: "add", onclick: move |_| composing.set(Some(b)), "Comment" }
                        }
                    }
                }
                div { class: "rail",
                    if let Some(b) = composing() {
                        div { class: "note-card",
                            div { class: "note-head", Avatar { initials: "NH" } "On block {b}" }
                            Input { id: "comment-input", placeholder: "Add a comment", value: draft(), oninput: move |v| draft.set(v) }
                            div { class: "note-actions",
                                Button { id: "comment-post", onclick: move |_| add(), "Post" }
                                Button { id: "comment-cancel", class: "button quiet", onclick: move |_| composing.set(None), "Cancel" }
                            }
                        }
                    }
                    for t in threads.read().iter().cloned() {
                        div { id: "thread-{t.id}", key: "{t.id}", class: "note-card",
                            div { class: "note-head", Avatar { initials: "CL" } "Block {t.block}" }
                            div { class: "thread-text", "{t.text}" }
                            div { class: "note-actions",
                                Button { id: "resolve-{t.id}", class: "button quiet", onclick: move |_| resolving.set(Some(t.id)), "Resolve" }
                            }
                        }
                    }
                }
            }
        }
        if let Some(id) = resolving() {
            Dialog { title: "Resolve this thread?", onclose: move |_| resolving.set(None),
                div { class: "meta", "It leaves the rail." }
                div { class: "dialog-actions",
                    Button { id: "resolve-no", class: "button quiet", onclick: move |_| resolving.set(None), "Keep" }
                    Button { id: "resolve-yes",
                        onclick: move |_| {
                            threads.write().retain(|t| t.id != id);
                            resolved += 1;
                            resolving.set(None);
                        },
                        "Resolve"
                    }
                }
            }
        }
    }
}
