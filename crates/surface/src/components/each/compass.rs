use super::button::Button;
use super::textarea::Textarea;
use crate::component::Component;
use dioxus::html::{Modifiers, NamedKey};
use dioxus::prelude::*;
use std::{future::Future, pin::Pin, rc::Rc};

inventory::submit! {
    Component {
        slug: "compass",
        classes: &["compass", "pick-bar", "pick-layer"],
        tags: &[],
        name: "Compass",
        used: "Every page's comment on any element: the menu's compass, then a tap on the element, its words quoted in a box at the \
               foot, Send writing it to the comment queue. Its handlers are Rust, run in the window and through liveview on the web.",
        example: Some(|_| rsx! {
            Compass { id: "ds-idle" }
            Compass { id: "ds-picking", picking: true }
            Compass { id: "ds-box", opened: "Rust tasks" }
        }),
        menu: true,
        library: Some("https://github.com/DioxusLabs/dioxus/tree/main/packages/liveview"),
        terminal: Some(crate::component::Terminal::Words),
        ..Component::BARE
    }
}

/// The comment compass's button, as the older app's top bar carried it (`DashboardWeb.CommentThread.comment_page_button/1`; @nha,
/// 2026-11-09: « the menu is getting nice. It needs the compass still. »): its drawing is `DashboardWeb.Library.compass/1`'s. A
/// label of the live `Compass`'s switch (`<id>-compass-on`), so it sits in the menu's head, which the page draws, while the
/// compass runs apart from the menu: its pick bar and box show over the page, the phone's drawer closed or not. `id`: the
/// menu's; none on /design's Page comments. Its name is also its text form's (`.text-only`): an icon says nothing to an agent.
pub fn compass(id: Option<&'static str>) -> Element {
    rsx! {
        // a button to a screen reader and to dash parity, as the older app's `<button id="comment-page">` (task
        // rust-shell-theme-bell): a label alone reads as a field's name
        label { class: "button icon", r#for: "dash-compass-on", id, role: "button",
            title: "Comment on an element: tap it next", aria_label: "Comment on an element",
            span { class: "text-only", "Comment on an element" }
            svg { view_box: "0 0 24 24", width: "16", height: "16", fill: "none", stroke: "currentColor", stroke_width: "2",
                stroke_linecap: "round", stroke_linejoin: "round", "aria-hidden": "true",
                path { d: "M12 22a10 10 0 1 0 0-20 10 10 0 0 0 0 20z" }
                path { d: "M16 8l-2 6-6 2 2-6z" }
            }
        }
    }
}

/// One element under the compass's tap, as its renderer reads it: the window's document, or the browser's through liveview.
#[derive(Clone, Debug, Default, PartialEq, serde::Deserialize)]
#[serde(default)]
pub struct Node {
    pub tag: String,
    pub id: String,
    pub role: String,
    /// Its words: a list's chosen option, a text box's typed value, else its text.
    pub text: String,
    /// What names it: aria-label, title, alt, placeholder, a text field's value.
    pub named: String,
}

/// What a look of the page found: its path, and the elements under the tap, innermost first; none, the whole page.
#[derive(Clone, Debug, Default, PartialEq, serde::Deserialize)]
#[serde(default)]
pub struct Pick {
    pub page: String,
    pub chain: Vec<Node>,
}

/// What the compass asks its renderer. Each look clears the element marked before it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Look {
    /// Mark the element at this point of the screen while picking (the outline).
    Mark(f64, f64),
    /// The elements at this point.
    Take(f64, f64),
    /// The whole page.
    Whole,
    /// Only the clearing: the pick stopped.
    Unmark,
}

/// A look of the page, answered when its renderer has read it.
pub type Looks = Rc<dyn Fn(Look) -> Pin<Box<dyn Future<Output = Pick>>>>;
/// A comment written: (page, quote, words) to its id, or why not.
pub type Sends = Rc<dyn Fn(&str, &str, &str) -> Result<String, String>>;

/// What runs the compass gives it: the window (dash_ui) or a liveview page (dash's live.rs). `look` reads the page under it,
/// `send` writes a comment of (page, quote, words) and returns its id.
#[derive(Clone)]
pub struct CompassHost {
    pub look: Looks,
    pub send: Sends,
}

/// The compass a renderer runs, `host` its own: the root of the web page's liveview and the window's `Beside` (dash's live.rs)
/// call it, so only the design system draws the compass. `host` is read on the first render; a later one is the same.
pub fn compass_hosted(host: CompassHost) -> Element {
    use_context_provider(move || host);
    rsx! { Compass {} }
}

/// Words as a quote keeps them: trimmed, each run of spaces one, 500 characters at most.
fn words(text: &str) -> String {
    let one: Vec<&str> = text.split_whitespace().collect();
    one.join(" ").chars().take(500).collect()
}

/// The quote of a tap: the element's words, else what names it, else those of an ancestor up to its own control (a link,
/// button or label) or, outside one, its parent, never a whole menu's name; else its tag. None, the whole page: "".
pub fn quote(chain: &[Node]) -> String {
    let Some(first) = chain.first() else {
        return String::new();
    };
    let control =
        |n: &Node| matches!(n.tag.as_str(), "a" | "button" | "label") || n.role == "button";
    let top = chain
        .iter()
        .position(control)
        .unwrap_or(1)
        .min(chain.len() - 1);
    chain[..=top]
        .iter()
        .map(|n| match words(&n.text) {
            w if w.is_empty() => words(&n.named),
            w => w,
        })
        .find(|w| !w.is_empty())
        .unwrap_or_else(|| first.tag.clone())
}

/// The comment compass, every page's, its behaviour all Rust (rfc-rust-unified#ui-lib; @nha, 2026-11-11: « I thought we did
/// not use JS / TS at all ? »): its switch (the menu's `compass` button is its label) starts a pick; a layer over the page
/// takes the next tap, outlines what is under the pointer, and asks the renderer what is there (`Look`); the box at the foot
/// quotes its words (`quote`), and Send writes the comment through the host, where the queue's agent answers it. Any element
/// takes it, the menu included (task rust-compass-any-element); a tap on the compass again stops the pick, as do the pick
/// bar's Cancel and Escape in an empty box. With no host (/design's picture) it draws and does nothing. `picking` and `opened`
/// start it picking or with its box open on that quote, as /design shows it. `id` prefixes its ids: the page's is `dash`.
#[component]
pub fn Compass(
    #[props(default = "dash")] id: &'static str,
    #[props(default)] picking: bool,
    #[props(default, into)] opened: Option<String>,
) -> Element {
    let host = try_consume_context::<CompassHost>();
    // a fixed layer over the whole screen: drawn live only, never on /design's picture, where it would take every click
    let live = host.is_some();
    let mut picking = use_signal(|| picking);
    // the page and the quote the box comments on
    let mut picked = use_signal(|| opened.map(|q| (String::new(), q)));
    let mut text = use_signal(String::new);
    let said = use_signal(String::new);
    // a look of the page through the host, spawned: none with no host
    let looks = host.clone().map(|h| h.look);
    let look = move |l: Look| {
        let looks = looks.clone();
        async move {
            match looks {
                Some(look) => Some(look(l).await),
                None => None,
            }
        }
    };
    let (marks, takes, stops) = (look.clone(), look.clone(), look);
    // one mark asked at a time: the web asks the browser each one, so a move while one is out is dropped (/code-review)
    let marking = use_hook(|| Rc::new(std::cell::Cell::new(false)));
    // Fn, not FnMut: each handler holds its own copy, the signals copied in. The pick ends once the look is answered, never
    // before: liveview sends a render's patches ahead of a spawned look's script, so a layer dropped at the tap was gone when
    // the browser looked, and every comment went to the whole page (task compass-selection-anchor; @nha, 2026-11-12:
    // « all my feedback is apparently on "the whole page" because the selection is broken »)
    let take = move |l: Look| {
        let (mut picking, mut picked, mut said) = (picking, picked, said);
        let look = takes.clone();
        spawn(async move {
            let found = look(l).await;
            picking.set(false);
            let Some(found) = found else { return };
            // a tap on the compass itself stops the pick, as it did the script's
            if found.chain.iter().any(|n| n.id == "dash-compass") {
                return;
            }
            picked.set(Some((found.page, quote(&found.chain))));
            said.set(String::new());
        });
    };
    let stop = move || {
        let mut picking = picking;
        picking.set(false);
        let look = stops.clone();
        spawn(async move {
            look(Look::Unmark).await;
        });
    };
    // Send's click and Enter in the box (task comment-box-one-close-enter-sends; @nha, 2026-11-11: « "enter' just sends »)
    // Fn, as `take`: the signals copied in
    let send = move || {
        let (mut text, mut picked, mut said) = (text, picked, said);
        let words = text.peek().trim().to_string();
        let (Some(host), Some((page, quote))) = (host.clone(), picked.peek().clone()) else {
            return;
        };
        if words.is_empty() {
            return;
        }
        match (host.send)(&page, &quote, &words) {
            Ok(_) => {
                said.set("Sent".into());
                text.set(String::new());
                picked.set(None);
            }
            Err(e) => said.set(format!("Not sent: {e}")),
        }
    };
    let quoted = picked().map(|(_, q)| {
        if q.is_empty() {
            "The whole page".to_string()
        } else {
            q
        }
    });
    rsx! {
        div { class: "compass",
            input { r#type: "checkbox", class: "sr-only", id: "{id}-compass-on", aria_label: "Comment on an element",
                checked: picking(),
                // input, not change: Blitz fires only input on a checkbox, a browser both
                oninput: {
                    let stop = stop.clone();
                    move |e: FormEvent| match e.checked() {
                        true => picking.set(true),
                        false => stop(),
                    }
                },
            }
            if picking() && live {
                div { class: "pick-layer", id: "{id}-pick-layer",
                    onmousemove: move |e| {
                        if marking.replace(true) {
                            return;
                        }
                        let at = e.client_coordinates();
                        let (look, marking) = (marks.clone(), marking.clone());
                        spawn(async move {
                            look(Look::Mark(at.x, at.y)).await;
                            marking.set(false);
                        });
                    },
                    onclick: {
                        let take = take.clone();
                        move |e: MouseEvent| {
                            let at = e.client_coordinates();
                            take(Look::Take(at.x, at.y));
                        }
                    },
                }
            }
            if picking() {
                div { id: "{id}-pickbar",
                    div { class: "pick-bar", role: "toolbar", aria_label: "Comment on",
                        span { "Tap the element to comment on" }
                        Button { look: "quiet", r#type: "button", onclick: {
                            let take = take.clone();
                            move |_| take(Look::Whole)
                        }, "Whole page" }
                        Button { look: "quiet", r#type: "button", onclick: {
                            let stop = stop.clone();
                            move |_| stop()
                        }, "Cancel" }
                    }
                }
            }
            if let Some(quoted) = quoted {
                div { id: "{id}-compose",
                    div { class: "doc-compose",
                        blockquote { "{quoted}" }
                        Textarea { rows: "3", placeholder: "Your comment", aria_label: "Your comment", value: "{text}",
                            oninput: move |e: FormEvent| text.set(e.value()),
                            onkeydown: {
                                let send = send.clone();
                                move |e: KeyboardEvent| match e.key() {
                                    Key::Named(NamedKey::Escape) if text.peek().trim().is_empty() => picked.set(None),
                                    // Shift, Alt or Ctrl+Enter is the textarea's own new line; ⌘Enter sends, as in the editor
                                    Key::Named(NamedKey::Enter)
                                        if !e.is_composing()
                                            && !e.modifiers().intersects(Modifiers::SHIFT | Modifiers::ALT | Modifiers::CONTROL) =>
                                    {
                                        e.prevent_default();
                                        send();
                                    }
                                    _ => {}
                                }
                            },
                        }
                        div {
                            Button { r#type: "button", title: "Send (Enter)", onclick: move |_| send(), "Send" }
                            " "
                            Button { look: "quiet", r#type: "button", onclick: move |_| picked.set(None), "Cancel" }
                            " "
                            span { role: "status", "{said}" }
                        }
                    }
                }
            } else if !said().is_empty() {
                span { class: "sr-only", role: "status", "{said}" }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(tag: &str, text: &str, named: &str) -> Node {
        Node {
            tag: tag.into(),
            text: text.into(),
            named: named.into(),
            ..Node::default()
        }
    }

    /// Done-when 1 of rust-window-compass, the quote's half: the compass's quote is Rust's, read off the renderer's chain, as
    /// the script's `quoted` was. The cases the Chrome test of task rust-compass-any-element tapped (@nha, 2026-11-09: « the
    /// comments should work on any element. It doesn't work on anything on the menu »): a menu link, the fold's label, a card,
    /// a heading, an icon in a titled button.
    #[dash_meter::dash_test]
    fn a_tap_quotes_its_words_its_name_or_its_control_s() {
        assert_eq!(quote(&[]), "", "the whole page");
        assert_eq!(quote(&[node("h1", "  Rust \n tasks ", "")]), "Rust tasks");
        // an icon in a button: the button's name
        let icon = [
            node("svg", "", ""),
            node("button", "", "Comment on an element"),
            node("nav", "a whole menu", ""),
        ];
        assert_eq!(quote(&icon), "Comment on an element");
        // outside a control, its parent at most, never the menu around it
        let bare = [
            node("span", "", ""),
            node("div", "", ""),
            node("nav", "a whole menu", ""),
        ];
        assert_eq!(quote(&bare), "span");
        assert_eq!(quote(&[node("p", &"x ".repeat(400), "")]).len(), 500);
        let menu = [
            node("a", "Tasks", ""),
            node("nav", "Inbox Today Mail Tasks", "Pages"),
        ];
        assert_eq!(quote(&menu), "Tasks");
        assert_eq!(
            quote(&[
                node("label", "\u{ab}", "Fold the menu"),
                node("div", "dash \u{ab}", "")
            ]),
            "\u{ab}"
        );
        let card = [
            node("span", "", ""),
            node("a", "klar\n 1 open", ""),
            node("div", "site_perso 3 open klar 1 open", ""),
        ];
        assert_eq!(quote(&card), "klar 1 open");
        let delete = [node("svg", "", ""), node("button", "", "Delete")];
        assert_eq!(quote(&delete), "Delete");
    }

    /// Done-when 1 of rust-window-compass, the page's half: /design's Components tab draws the `Compass` component in its three
    /// states, the switch, the pick bar and the box quoting its tap with Send, and carries no script of ours: its handlers
    /// are the component's Rust ones, which compass_liveview_sends_a_comment and window_compass_sends_a_comment drive.
    #[dash_meter::dash_test]
    fn design_shows_the_compass_component() {
        let page = crate::markup(&crate::design::page("components", None, "tab=components"));
        for want in [
            r#"id="ds-idle-compass-on""#,
            r#"id="ds-picking-pickbar""#,
            "Tap the element to comment on",
            r#"id="ds-box-compose""#,
            "<blockquote>Rust tasks</blockquote>",
        ] {
            assert!(page.contains(want), "{want}: {page}");
        }
        let words = crate::forms::text_of(&page);
        assert!(
            words.contains("Whole page") && words.contains("Send"),
            "{words}"
        );
        // with no host, the picture takes no click: the full-screen layer is drawn live only
        assert!(!page.contains("ds-picking-pick-layer"), "{page}");
        assert!(!page.contains("<script"), "{page}");
    }
}
