//! Notion-like comments, after private/evidence/notion/2026-11-06: pick a line, its toolbar, a thread beside it, replies
//! folded, a page's own comments, a comment's menu. The text is made up: the evidence shows his job search, and rust/ rides
//! to the CI mirror.
use super::families::{CHAT, DOC, DOTS, Field, INBOX, SEARCH, SEND, icon};
use crate::Avatar;
use dioxus::prelude::*;

const LINES: [(&str, &str, bool); 11] = [
    ("d0", "2026:", false),
    ("d1", "One tree, two renderers", true),
    ("d2", "the window and the web draw the same components", false),
    ("d2", "theme.css generated from design.json, shared by both", false),
    ("d2", "each behaviour is checked against the live document", false),
    ("d1", "Memory", false),
    ("d1", "Frame time", false),
    ("d1", "Fonts", true),
    ("d2", "one face a window", false),
    ("d2", "matches on family name, then falls back", false),
    ("d1", "Selection", false),
];
const FAVOURITES: [&str; 4] = ["Rendering plan", "End of year", "Visas", "Tasks"];
const SKILLS: [&str; 3] = ["Improve writing", "Proofread", "Explain"];
const MENU: [&str; 4] = ["Edit", "Copy link to discussion", "Mute replies", "Delete"];

#[derive(Clone, PartialEq, Default)]
struct Thread {
    line: usize,
    said: Vec<String>,
    /// Older replies folded, as Notion does when the thread loses focus.
    folded: bool,
}

/// One comment, its « ... » menu on hover. Its id says which.
#[component]
fn Said(id: String, words: String, menu: Signal<Option<String>>) -> Element {
    let open = menu() == Some(id.clone());
    let more = format!("{id}-more");
    let key = id.clone();
    rsx! {
        div { id: "{id}", class: if open { "no-comment menu" } else { "no-comment" },
            div { class: "no-who", Avatar { initials: "NH" } "Nicolas Ha" span { class: "meta", "Just now" } }
            div { class: "no-words", "{words}" }
            div { id: "{more}", class: "no-more", onclick: move |_| menu.set(if menu() == Some(key.clone()) { None } else { Some(key.clone()) }), {icon(DOTS)} }
            if open {
                div { id: "no-menu", class: "no-menu",
                    for item in MENU {
                        div { key: "{item}", class: "no-tool-item", "{item}" }
                    }
                }
            }
        }
    }
}

#[component]
pub(crate) fn Notion() -> Element {
    let mut picked = use_signal(|| None::<usize>);
    let mut threads = use_signal(Vec::<Thread>::new);
    let mut composing = use_signal(|| None::<usize>);
    let mut draft = use_signal(String::new);
    let mut page = use_signal(Vec::<String>::new);
    let mut page_open = use_signal(|| false);
    let mut page_draft = use_signal(String::new);
    let menu = use_signal(|| None::<String>);
    let post = move || {
        let Some(line) = composing() else { return };
        let words = draft();
        if words.is_empty() {
            return;
        }
        let mut all = threads.write();
        match all.iter_mut().find(|t| t.line == line) {
            Some(t) => t.said.push(words),
            None => all.push(Thread { line, said: vec![words], folded: false }),
        }
        draft.set(String::new());
    };
    rsx! {
        div { class: "no",
            div { class: "no-side",
                div { class: "no-search", "Search or ask" span { class: "meta", "Cmd K" } }
                div { class: "no-item on", {icon(INBOX)} b { "Home" } }
                div { class: "no-item", {icon(CHAT)} "New chat" }
                div { class: "no-group", "Favorites" }
                for (i, name) in FAVOURITES.into_iter().enumerate() {
                    div { key: "{name}", class: if i == 0 { "no-item on" } else { "no-item" }, {icon(DOC)} "{name}" }
                }
                div { class: "no-group", "Teamspaces" }
                div { class: "no-item", {icon(DOC)} "General" }
            }
            div { class: "no-main",
                div { class: "no-top", "Rendering plan" span { class: "meta", "Private" } span { class: "spring" } span { class: "meta", "Edited Oct 13" } "Share" {icon(SEARCH)} {icon(DOTS)} }
                div { class: "no-work",
                    div { class: "no-doc",
                        div { class: "no-actions",
                            div { id: "no-add", class: "no-add", 
                                onclick: move |_| {
                                    page_open.set(true);
                                    // As Notion, when a thread loses focus.
                                    composing.set(None);
                                    draft.set(String::new());
                                    for t in threads.write().iter_mut() {
                                        t.folded = true;
                                    }
                                },
                                {icon(CHAT)} "Add comment"
                            }
                        }
                        div { class: "no-title", "Rendering plan" }
                        if page_open() || !page.read().is_empty() {
                            div { class: "no-page",
                                for (i, words) in page.read().iter().cloned().enumerate() {
                                    Said { key: "{i}", id: "no-page-{i}", words, menu }
                                }
                                div { class: "no-reply",
                                    Avatar { initials: "NH" }
                                    Field { id: "no-page-input", hint: "Add a comment...", value: page_draft(), oninput: move |v| page_draft.set(v) }
                                    div { id: "no-page-post", class: if page_draft.read().is_empty() { "no-send off" } else { "no-send" },
                                        onclick: move |_| {
                                            if !page_draft.read().is_empty() {
                                                page.write().push(page_draft());
                                                page_draft.set(String::new());
                                            }
                                        },
                                        {icon(SEND)}
                                    }
                                }
                            }
                        }
                        for (l, (depth, words, link)) in LINES.into_iter().enumerate() {
                            div { id: "no-line-{l}", key: "{l}",
                                class: if threads.read().iter().any(|t| t.line == l) { "no-line {depth} marked" } else if picked() == Some(l) { "no-line {depth} selected" } else { "no-line {depth}" },
                                onclick: move |_| picked.set(Some(l)),
                                if depth != "d0" {
                                    span { class: "no-bullet" }
                                }
                                span { class: if link { "no-text no-url" } else { "no-text" }, "{words}" }
                                if picked() == Some(l) {
                                    div { id: "no-tool", class: "no-tool", onclick: move |e| e.stop_propagation(),
                                        div { class: "no-tool-item", "Bulleted list" }
                                        div { class: "no-tool-sep" }
                                        div { class: "no-tool-row",
                                            span { class: "no-icon", "A" } span { class: "no-icon", "B" } span { class: "no-icon", "I" } span { class: "no-icon", "U" } span { class: "no-icon", "S" }
                                        }
                                        div { class: "no-tool-sep" }
                                        div { id: "no-comment", class: "no-tool-item",
                                            onclick: move |_| {
                                                composing.set(Some(l));
                                                draft.set(String::new());
                                                picked.set(None);
                                            },
                                            {icon(CHAT)} "Comment"
                                        }
                                        div { class: "no-tool-sep" }
                                        div { class: "meta no-tool-item", "Skills" }
                                        for name in SKILLS {
                                            div { key: "{name}", class: "no-tool-item", "{name}" }
                                        }
                                    }
                                }
                                for (t, thread) in threads.read().iter().cloned().enumerate().filter(|(_, t)| t.line == l) {
                                    div { id: "no-thread-{t}", key: "{t}", class: "no-thread", onclick: move |e| e.stop_propagation(),
                                        if thread.folded && thread.said.len() > 2 {
                                            Said { id: "no-said-{t}-0", words: thread.said[0].clone(), menu }
                                            div { id: "no-unfold-{t}", class: "no-fold",
                                                onclick: move |_| threads.write()[t].folded = false,
                                                if thread.said.len() == 3 { "Show 1 reply" } else { "Show {thread.said.len() - 2} replies" }
                                            }
                                            Said { id: "no-said-{t}-{thread.said.len() - 1}", words: thread.said[thread.said.len() - 1].clone(), menu }
                                        } else {
                                            for (i, words) in thread.said.iter().cloned().enumerate() {
                                                Said { key: "{i}", id: "no-said-{t}-{i}", words, menu }
                                            }
                                        }
                                        if composing() == Some(l) {
                                            {reply(draft(), move |v| draft.set(v), post, "Reply...")}
                                        }
                                    }
                                }
                                if composing() == Some(l) && !threads.read().iter().any(|t| t.line == l) {
                                    div { id: "no-new", class: "no-thread", onclick: move |e| e.stop_propagation(),
                                        {reply(draft(), move |v| draft.set(v), post, "Add a comment...")}
                                    }
                                }
                            }
                        }
                    }
                    div { class: "no-rail" }
                }
            }
        }
    }
}

fn reply(value: String, on: impl FnMut(String) + 'static, mut send: impl FnMut() + 'static, hint: &'static str) -> Element {
    let empty = value.is_empty();
    rsx! {
        div { class: "no-reply",
            Avatar { initials: "NH" }
            Field { id: "no-input", hint, value, oninput: on }
            div { id: "no-send", class: if empty { "no-send off" } else { "no-send" }, onclick: move |_| send(), {icon(SEND)} }
        }
    }
}
