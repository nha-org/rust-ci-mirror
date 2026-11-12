//! Notion-like comments, after private/evidence/notion/2026-11-06: drag across words, the toolbar, a thread beside them, replies
//! folded, a page's own comments, a comment's menu. The text is made up: the evidence shows his job search, and rust/ rides
//! to the CI mirror.
use super::families::Avatar;
use super::families::{CHAT, DOTS, Field, SEARCH, SEND, icon};
use crate::components::{
    DropdownMenu, DropdownMenuItem, Toolbar, ToolbarButton, ToolbarGroup, ToolbarSeparator,
};
use blitz_dom::{BaseDocument, NodeId, local_name};
use dioxus::html::input_data::MouseButton;
use dioxus::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

pub(super) const LINES: [(&str, &str, bool); 11] = [
    ("d0", "2026:", false),
    ("d1", "One tree, two renderers", true),
    (
        "d2",
        "the window and the web draw the same components",
        false,
    ),
    (
        "d2",
        "theme.css generated from design.json, shared by both",
        false,
    ),
    (
        "d2",
        "each behaviour is checked against the live document",
        false,
    ),
    ("d1", "Memory", false),
    ("d1", "Frame time", false),
    ("d1", "Fonts", true),
    ("d2", "one face a window", false),
    ("d2", "matches on family name, then falls back", false),
    ("d1", "Selection", false),
];
const TITLE: &str = "Rendering plan";
/// The toggle block's lines, under its summary.
pub(super) const QUESTIONS: [&str; 2] = [
    "Does a slow font load hold the first frame?",
    "One theme file for print too?",
];
const SKILLS: [&str; 3] = ["Improve writing", "Proofread", "Explain"];
const MENU: [&str; 4] = ["Edit", "Copy link to discussion", "Mute replies", "Delete"];
/// The slash menu's blocks: (the class a picked one gives its line, its name).
const BLOCKS: [(&str, &str); 5] = [
    ("text", "Text"),
    ("heading", "Heading"),
    ("todo", "To-do list"),
    ("bullet", "Bulleted list"),
    ("quote", "Quote"),
];
/// The web's handlers: a server render runs none, so this script gives the page its drag, comments and slash menu
/// (@nha, 2026-11-09: « /design?tab=notion => interactions are not functional. »). The toggle is a <details>, which
/// needs none. Its clones come from #no-proto, drawn by the same components as the window's.
/// Blocks its port: no dioxus-liveview runs, so a web page is a dioxus-ssr picture with no handlers
/// (rfc-rust-unified#ui-lib's drift; liveview's own future is open upstream,
/// https://github.com/DioxusLabs/dioxus/issues/2699); a text selection fires no Dioxus event
/// (https://github.com/DioxusLabs/dioxus/issues/5725).
const NOTION_JS: &str = include_str!("notion.js");
/// Export PDF's link: dash answers it with design::document_pdf(), as a file to save. Here, not in design: a use of the
/// parent from here would be a module loop (design draws documents, which reads notion_doc, which reads this).
pub const DOCUMENT_PDF: &str = "/design/rendering-plan.pdf";

/// The document's text form as Markdown, what Export Markdown saves: its title a heading, then each line, a bullet indented
/// by its depth, then the toggle, its summary a bullet and its lines under it.
fn markdown() -> String {
    let mut out = format!("# {TITLE}\n\n");
    for (depth, words, _) in LINES {
        let bullet = match depth {
            "d1" => "- ",
            "d2" => "  - ",
            _ => "",
        };
        out += &format!("{bullet}{words}\n");
    }
    out += "- Open questions\n";
    for q in QUESTIONS {
        out += &format!("  - {q}\n");
    }
    out
}

/// `text` as a `data:` URL of its media type: a link that downloads it with no server and no script.
fn data_url(kind: &str, text: &str) -> String {
    let mut out = format!("data:{kind};charset=utf-8,");
    for b in text.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            _ => out += &format!("%{b:02X}"),
        }
    }
    out
}

/// Whether the slash menu shows this block for what is typed: all of them until a `/`, then those whose name holds the rest.
fn offered(name: &str, typed: &str) -> bool {
    typed
        .strip_prefix('/')
        .is_none_or(|rest| name.to_lowercase().contains(&rest.to_lowercase()))
}

/// Words of one line, as byte offsets into its text.
#[derive(Clone, Copy, PartialEq)]
struct Phrase {
    line: usize,
    start: usize,
    end: usize,
}

impl Phrase {
    fn words(self) -> &'static str {
        &LINES[self.line].1[self.start..self.end]
    }
}

/// What Blitz holds selected in the first line it covers words of. A triple click or a drag past a line's end runs into the
/// next block, one range each, so those past it are left out (@nha, 2026-11-10: « I cannot do this on sentences for some reason
/// only double-click on words. »). Its offsets are into the text it laid out, so a line whose laid-out text differs from its
/// words is passed over rather than giving the wrong words.
fn selected(doc: &BaseDocument) -> Option<Phrase> {
    let (line, start, words) =
        doc.get_text_selection_ranges()
            .into_iter()
            .find_map(|(node, start, end)| {
                let line = line_of(doc, node)?;
                let words = LINES[line].1.get(start..end)?;
                let laid = &doc
                    .get_node(node)?
                    .element_data()?
                    .inline_layout_data
                    .as_ref()?
                    .text;
                (laid.get(start..end) == Some(words) && !words.trim().is_empty())
                    .then_some((line, start, words))
            })?;
    // Without its outer spaces, so an overshoot by one is the same phrase and the same thread.
    let start = start + words.len() - words.trim_start().len();
    let end = start + words.trim().len();
    Some(Phrase { line, start, end })
}

/// Which line's text this node is.
fn line_of(doc: &BaseDocument, node: NodeId) -> Option<usize> {
    doc.get_node(node)?
        .attr(local_name!("id"))?
        .strip_prefix("no-text-")?
        .parse()
        .ok()
}

fn primary(e: &MouseEvent) -> bool {
    e.trigger_button() == Some(MouseButton::Primary)
}

/// A line's words cut wherever a phrase starts or ends: (where it starts, its words, its class).
fn runs(
    line: usize,
    marks: &[Phrase],
    pending: Option<Phrase>,
) -> Vec<(usize, &'static str, &'static str)> {
    let words = LINES[line].1;
    let on_line = marks
        .iter()
        .copied()
        .chain(pending)
        .filter(|p| p.line == line);
    let mut cuts: Vec<usize> = on_line
        .flat_map(|p| [p.start, p.end])
        .chain([0, words.len()])
        .collect();
    cuts.sort_unstable();
    cuts.dedup();
    let covers = |p: &Phrase, at: usize| p.line == line && p.start <= at && at < p.end;
    cuts.windows(2)
        .map(|c| {
            let class = if pending.is_some_and(|p| covers(&p, c[0])) {
                "no-pick"
            } else if marks.iter().any(|p| covers(p, c[0])) {
                "no-mark"
            } else {
                ""
            };
            (c[0], &words[c[0]..c[1]], class)
        })
        .collect()
}

#[derive(Clone, PartialEq)]
struct Thread {
    phrase: Phrase,
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
            div { id: "{more}", class: "no-more",
                onclick: move |e| {
                    e.stop_propagation();
                    menu.set(if menu() == Some(key.clone()) { None } else { Some(key.clone()) });
                },
                {icon(DOTS)}
            }
            if open {
                div { id: "no-menu", class: "no-menu", onclick: move |e| e.stop_propagation(),
                    for item in MENU {
                        div { key: "{item}", class: "no-tool-item", "{item}" }
                    }
                }
            }
        }
    }
}

/// `web`: the server's render, which draws what its script shows and hides (the page's comments, #no-proto) and the script.
#[component]
pub(crate) fn Notion(web: bool) -> Element {
    // The window's and the test's documents provide it: Blitz holds the ranges, and Dioxus's selection event is unimplemented!.
    // None on the web's server render, which runs no handler.
    let document = use_hook(try_consume_context::<Rc<RefCell<BaseDocument>>>);
    let mut picked = use_signal(|| None::<Phrase>);
    let mut toolbar = use_signal(|| false);
    let mut threads = use_signal(Vec::<Thread>::new);
    let mut composing = use_signal(|| None::<Phrase>);
    let mut draft = use_signal(String::new);
    let mut page = use_signal(Vec::<String>::new);
    let mut page_open = use_signal(|| false);
    let mut page_draft = use_signal(String::new);
    let menu = use_signal(|| None::<String>);
    let mut slash = use_signal(String::new);
    let mut kind = use_signal(|| "text");
    let mut page_menu = use_signal(|| false);
    // Once: the document never changes, and a render runs on every drag and key (/code-review).
    let markdown = use_hook(|| data_url("text/markdown", &markdown()));
    let post = move || {
        let Some(phrase) = composing() else { return };
        let words = draft();
        if words.is_empty() {
            return;
        }
        let mut all = threads.write();
        match all.iter_mut().find(|t| t.phrase == phrase) {
            Some(t) => t.said.push(words),
            None => all.push(Thread {
                phrase,
                said: vec![words],
                folded: false,
            }),
        }
        draft.set(String::new());
    };
    let marks: Vec<Phrase> = threads.read().iter().map(|t| t.phrase).collect();
    // What is being picked or written about shows over a mark; a posted thread's phrase is its mark.
    let pending = picked().or(composing().filter(|p| !marks.contains(p)));
    rsx! {
        div { class: "no",
        // A click outside the page's menu closes it; its trigger stops its own.
        onclick: move |_| {
            if page_menu() {
                page_menu.set(false);
            }
        },
        // A press closes the toolbar, so no drag can end on it; the toolbar stops its own press. The pick itself goes at the
        // release: a press that re-cut the line's runs would remove the span just pressed, and Blitz then starts no drag.
        onmousedown: move |e| {
            if primary(&e) && toolbar() {
                toolbar.set(false);
            }
        },
        // A drag fires no click (Blitz's handle_pointerup), so this is where it ends. Our own class draws the pick
        // from here: Blitz's selection colour is a constant, unreadable on a dark theme, so a selection on a line goes,
        // and Cmd+C has nothing to copy there. One elsewhere stays, for Cmd+C.
        // ponytail: a drag released outside .no is never heard: the next press resets Blitz's selection, so that pick is lost, its highlight and any older pick staying until then; a page-level listener if it matters.
        onmouseup: move |e| {
            let Some(document) = document.as_ref() else { return };
            let mut doc = document.borrow_mut();
            // A toolbar still open means the press was on it: the pick stays for its click.
            if primary(&e) && !toolbar() {
                let now = selected(&doc);
                if picked() != now {
                    picked.set(now);
                }
                if now.is_some() {
                    toolbar.set(true);
                }
            }
            if doc.get_text_selection_ranges().iter().any(|r| line_of(&doc, r.0).is_some()) {
                doc.clear_text_selection();
            }
        },
            // No sidebar of its own: the app's menu is the page's one menu, its Search or ask field in that menu's head (@nha,
            // 2026-11-10: « Good but no double menu. the "ask" field is a good idea though it needs to live somewhere else »).
            div { class: "no-main",
                div { class: "no-top", "{TITLE}" span { class: "meta", "Private" } span { class: "spring" } span { class: "meta", "Edited Oct 13" } "Share" {icon(SEARCH)}
                    // The page's « ... »: its exports (@nha, 2026-11-09: « the menu should work, the"..." and propose to export to
                    // PDF. markdown. »). PDF is dash's route, this page printed by headless Chrome (crate::pdf), the document
                    // alone (components.css's [data-print] rule). ponytail: the window's navigate_to drops a data: link and a
                    // download, so both items do nothing there; a save to Downloads in dash-ui if the window needs exports.
                    DropdownMenu { id: "no-page-menu", open: page_menu(), label: "Page menu", trigger: icon(DOTS),
                        toggle: move |e: MouseEvent| {
                            e.stop_propagation();
                            page_menu.toggle();
                        },
                        DropdownMenuItem { id: "no-export-pdf", href: DOCUMENT_PDF, download: "rendering-plan.pdf", "Export PDF" }
                        DropdownMenuItem { id: "no-export-md", href: markdown.clone(), download: "rendering-plan.md", "Export Markdown" }
                    }
                }
                div { class: "no-work",
                    div { class: "no-doc", "data-print": "document",
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
                        div { class: "no-title", "{TITLE}" }
                        if web || page_open() || !page.read().is_empty() {
                            div { id: "no-page", class: "no-page", hidden: web,
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
                        for (l, (depth, _, link)) in LINES.into_iter().enumerate() {
                            div { id: "no-line-{l}", key: "{l}", class: "no-line {depth}",
                                if depth != "d0" {
                                    span { class: "no-bullet" }
                                }
                                // Edited in place on the web, kept in its memory: a reload draws LINES again (@nha, 2026-11-09: « this
                                // doc needs to be able to be edited too. fine not to save it automatically of course. »). Plain text only,
                                // so a paste brings no markup. The window's Blitz edits no text yet.
                                span { id: "no-text-{l}", class: if link { "no-text no-url" } else { "no-text" },
                                    contenteditable: if web { "plaintext-only" } else { "false" },
                                    for (at, words, class) in runs(l, &marks, pending) {
                                        span { key: "{at}", class, "{words}" }
                                    }
                                }
                                if toolbar() && picked().is_some_and(|p| p.line == l) {
                                    {tools(move |_| {
                                        composing.set(picked());
                                        draft.set(String::new());
                                        picked.set(None);
                                        toolbar.set(false);
                                    })}
                                }
                                // Only where it holds something: Blitz's hit test counts an empty box's top row, which ate clicks on the thread above.
                                if marks.iter().any(|p| p.line == l) || composing().is_some_and(|p| p.line == l) {
                                    div { class: "no-threads",
                                        for (t, thread) in threads.read().iter().cloned().enumerate().filter(|(_, t)| t.phrase.line == l) {
                                            div { id: "no-thread-{t}", key: "{t}", class: "no-thread",
                                                // A click reopens it: the same words dragged again would be the only other way. Its controls stop their own click.
                                                onclick: move |_| {
                                                    if composing() != Some(thread.phrase) {
                                                        composing.set(Some(thread.phrase));
                                                        draft.set(String::new());
                                                    }
                                                },
                                                div { id: "no-quote-{t}", class: "no-quote", "{thread.phrase.words()}" }
                                                if thread.folded && thread.said.len() > 2 {
                                                    Said { id: "no-said-{t}-0", words: thread.said[0].clone(), menu }
                                                    div { id: "no-unfold-{t}", class: "no-fold",
                                                        onclick: move |e| {
                                                            e.stop_propagation();
                                                            threads.write()[t].folded = false;
                                                        },
                                                        if thread.said.len() == 3 { "Show 1 reply" } else { "Show {thread.said.len() - 2} replies" }
                                                    }
                                                    Said { id: "no-said-{t}-{thread.said.len() - 1}", words: thread.said[thread.said.len() - 1].clone(), menu }
                                                } else {
                                                    for (i, words) in thread.said.iter().cloned().enumerate() {
                                                        Said { key: "{i}", id: "no-said-{t}-{i}", words, menu }
                                                    }
                                                }
                                                if composing() == Some(thread.phrase) {
                                                    {reply(draft(), move |v| draft.set(v), post, "Reply...")}
                                                }
                                            }
                                        }
                                        if let Some(phrase) = composing().filter(|p| p.line == l && !threads.read().iter().any(|t| t.phrase == *p)) {
                                            div { id: "no-new", class: "no-thread",
                                                div { class: "no-quote", "{phrase.words()}" }
                                                {reply(draft(), move |v| draft.set(v), post, "Add a comment...")}
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        details { id: "no-toggle", class: "no-toggle",
                            summary { id: "no-toggle-head", "Open questions" }
                            for q in QUESTIONS {
                                div { key: "{q}", class: "no-line d1", span { class: "no-bullet" } "{q}" }
                            }
                        }
                        div { id: "no-block", class: "no-line no-block {kind}",
                            Field { id: "no-slash-input", hint: "Type '/' for commands", value: slash(), oninput: move |v| slash.set(v) }
                            div { id: "no-slash", class: "no-tool no-slash", hidden: !slash().starts_with('/'),
                                div { class: "meta no-tool-item", "Basic blocks" }
                                for (class, name) in BLOCKS {
                                    if offered(name, &slash()) {
                                        div { key: "{class}", id: "no-slash-{class}", class: "no-tool-item", "data-kind": class,
                                            onclick: move |_| {
                                                kind.set(class);
                                                slash.set(String::new());
                                            },
                                            "{name}"
                                        }
                                    }
                                }
                            }
                        }
                        if web {
                            div { id: "no-proto", hidden: true, aria_hidden: "true",
                                {tools(|_| {})}
                                div { class: "no-thread",
                                    div { class: "no-quote" }
                                    {reply(String::new(), |_| {}, || {}, "Add a comment...")}
                                }
                                Said { id: "", words: "", menu }
                                div { class: "no-menu",
                                    for item in MENU {
                                        div { key: "{item}", class: "no-tool-item", "{item}" }
                                    }
                                }
                            }
                            script { dangerous_inner_html: NOTION_JS }
                        }
                    }
                    div { class: "no-rail" }
                }
            }
        }
    }
}

fn reply(
    value: String,
    on: impl FnMut(String) + 'static,
    mut send: impl FnMut() + 'static,
    hint: &'static str,
) -> Element {
    let empty = value.is_empty();
    rsx! {
        div { class: "no-reply",
            Avatar { initials: "NH" }
            Field { id: "no-input", hint, value, oninput: on }
            div { id: "no-send", class: if empty { "no-send off" } else { "no-send" }, onclick: move |_| send(), {icon(SEND)} }
        }
    }
}

/// The toolbar over a pick, its Comment item calling `comment`.
fn tools(comment: impl FnMut(MouseEvent) + 'static) -> Element {
    let comment = EventHandler::new(comment);
    rsx! {
        Toolbar { id: "no-tool", label: "Selection", look: "no-tool", vertical: true,
            ToolbarButton { "Bulleted list" }
            ToolbarSeparator {}
            ToolbarGroup { look: "no-tool-row",
                for mark in ["A", "B", "I", "U", "S"] {
                    ToolbarButton { key: "{mark}", look: "no-icon", "{mark}" }
                }
            }
            ToolbarSeparator {}
            ToolbarButton { id: "no-comment", onclick: comment, {icon(CHAT)} "Comment" }
            ToolbarSeparator {}
            ToolbarGroup { title: "Skills",
                for name in SKILLS {
                    ToolbarButton { key: "{name}", "{name}" }
                }
            }
        }
    }
}
