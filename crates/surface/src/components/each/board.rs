use super::badge::Badge;
use super::card::Card;
use super::parts::local_path;
use crate::component::Component;
use dioxus::prelude::*;

/// A board's script, once a page: a card dragged to another column posts `/tasks/move` (`id`, its task; `column`, the
/// column's `data-status`) and sits there at once. Once kept, the board view's diff patches both columns, their counts with
/// them (LIVE_JS, task rust-view-patches). A refusal puts the card back where it was and says why on it. The
/// window runs no script: its drag is design/linear.rs's mouse events. ponytail: HTML5 drag events, so a phone's touch does not
/// drag; pointer events if he asks for it there.
/// Blocks its port: no dioxus-liveview runs, so a web page is a dioxus-ssr picture with no handlers
/// (rfc-rust-unified#ui-lib's drift; liveview's own future is open upstream,
/// https://github.com/DioxusLabs/dioxus/issues/2699); drag events on the web are unreliable in Dioxus
/// (https://github.com/DioxusLabs/dioxus/issues/1504).
pub(crate) const BOARD_JS: &str = r#"(()=>{if(window.dashBoard)return;window.dashBoard=1;let held=null;const on=(t,f)=>document.addEventListener(t,f);const col=e=>e.target.closest&&e.target.closest(".board .li-column");on("dragstart",e=>{held=e.target.closest&&e.target.closest(".board [draggable=true][data-task]");if(held)e.dataTransfer.setData("text/plain",held.dataset.task)});on("dragend",()=>{held=null});on("dragover",e=>{if(held&&col(e))e.preventDefault()});on("drop",e=>{const to=held&&col(e);if(!to)return;e.preventDefault();const card=held,from=card.closest(".li-column"),next=card.nextSibling;held=null;if(from===to)return;to.querySelector(".li-column-head").after(card);fetch("/tasks/move",{method:"POST",body:new URLSearchParams({id:card.dataset.task,column:to.dataset.status})}).then(r=>r.text().then(t=>{if(!r.ok)throw t})).catch(t=>{from.insertBefore(card,next);const m=card.querySelector(".meta");if(m)m.textContent="Not moved: "+t})})})()"#;

inventory::submit! {
    Component {
        slug: "board",
        classes: &["board", "li-column", "li-column-head"],
        tags: &[],
        name: "Board, Column, CardLine",
        used: "The tasks board: its columns side by side, a card dragged to another column takes its status (/tasks); a column past its first cards links to them all.",
        example: Some(|_| {
            let more = Some(More {
                count: 41,
                href: "/tasks?layout=list#todo".into(),
            });
            rsx! {
                Board { id: "c-board",
                    Column { id: "c-column", status: "todo", name: "Todo", count: 42, icon: rsx! {}, more,
                        Card { id: "c-board-card", look: "li-card", href: "/tasks/a".to_string(), task: "a", draggable: true,
                            CardLine { says: "next", comments: 1, said: Some("1 comment".to_string()) }
                        }
                    }
                }
            }
        }),
        terminal: Some(crate::component::Terminal::Words),
        ..Component::BARE
    }
}

/// The tasks board: its columns side by side, as Linear's (task rust-board-linear-order), and the script that moves a card.
/// `view`: the views' name its cards follow, its `data-view`: the page's LIVE_JS takes each diff of that view as a patch of
/// the cards and columns it changes, never the whole page (task rust-view-patches).
#[component]
pub fn Board(
    #[props(into)] id: String,
    #[props(default)] view: Option<String>,
    children: Element,
) -> Element {
    rsx! {
        div { id, class: "board", "data-view": view, {children} }
        script { dangerous_inner_html: BOARD_JS }
    }
}

/// A board column's tasks it does not draw: how many, and the page that lists them all (a path of this app). A column of
/// 1205 cards drew 1.4 MB, its render 1.6 s under load (task rust-page-latency, trace 1794314321744293823).
#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
pub struct More {
    pub count: usize,
    pub href: String,
}

/// A board's column: its status icon and name, its count, then its cards. `status`: its `data-status`, the column a card
/// dropped on it moves to. `more`: the cards it does not draw, counted in `count`, and the link to them all, a path of this
/// app (task rust-page-latency).
#[component]
pub fn Column(
    #[props(into)] id: String,
    status: &'static str,
    #[props(into)] name: String,
    count: usize,
    icon: Element,
    #[props(default)] more: Option<More>,
    children: Element,
) -> Element {
    rsx! {
        section { id, class: "li-column", "data-status": status,
            h2 { class: "li-column-head", {icon} "{name}" Badge { look: "meta count", "{count}" } }
            {children}
            MoreLink { more }
        }
    }
}

/// The one link to a group's tasks it does not draw, « N more », a board column's and a list group's alike; none when its
/// href is not a path of this app.
#[component]
pub fn MoreLink(more: Option<More>) -> Element {
    match more.filter(|m| local_path(&m.href)) {
        Some(more) => rsx! { a { class: "li-more", href: more.href, "{more.count} more" } },
        None => rsx! {},
    }
}

impl More {
    /// Its link, when it is a path of this app: what both forms draw.
    pub(crate) fn link(&self) -> Option<&str> {
        local_path(&self.href).then_some(self.href.as_str())
    }
}

/// A board card's last line: what its run says, then its comments when it has any.
#[component]
pub fn CardLine(#[props(into)] says: String, comments: usize, said: Option<String>) -> Element {
    rsx! {
        span { class: "says", "{says}" }
        if let Some(said) = said {
            span { class: "comments", "data-comments": "{comments}", " · {said}" }
        }
    }
}
