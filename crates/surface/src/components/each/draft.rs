//! A reply draft as the design system draws it (task drafts-genuinely-good, done-when 4): its card, what it is above its
//! words and actions, and the list of drafts waiting, a row a draft. The drafts plugin builds their trees; it draws nothing.
use super::badge::Badge;
use super::table::{Table, TableRow};
use crate::component::Component;
use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

inventory::submit! {
    Component {
        slug: "draft_card",
        classes: &["draft-card"],
        tags: &[],
        name: "Reply draft",
        used: "A message being written, under the conversation it answers: its goal, who it is from and to, and whether \
               every check of its channel passes, then its words and what can be done with them (Send, Undo, Request \
               corrections) as its children.",
        example: Some(|_| rsx! {
            DraftCard { id: "c-draft-card", goal: "Book a call about the platform role", from: "nicolas@klar.im",
                to: "ada@example.com", state: "every check passes",
                p { "Hi Ada, thanks for the note. Would Tuesday at 10 suit you?" }
            }
        }),
        css: include_str!("draft.css"),
        terminal: Some(crate::component::Terminal::Words),
        ..Component::BARE
    }
}

inventory::submit! {
    Component {
        slug: "draft_list",
        classes: &[],
        tags: &[],
        name: "Drafts list",
        used: "The drafts waiting at one stage, a row a draft opening its page: its receiver, channel, goal and Jev's scores, \
               and its send's state once one began. A table, so it sorts and filters as every table does.",
        example: Some(|_| rsx! {
            DraftList { id: "c-draft-list", rows: vec![
                DraftRow { receiver: "Ada (Acme)".into(), channel: "email".into(), goal: "Book a call".into(),
                    scores: "too_long 0.2".into(), send: "waiting".into(), href: "/drafts/ada-intro".into() },
                DraftRow { receiver: "Bo".into(), channel: "linkedin".into(), goal: "Follow up".into(),
                    scores: "not scored".into(), send: String::new(), href: "/drafts/bo-relance".into() },
            ] }
        }),
        terminal: Some(crate::component::Terminal::Words),
        ..Component::BARE
    }
}

/// One draft of a list: who it goes to, its channel, its goal in one line, Jev's scores, its send's state (empty with none)
/// and its page.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DraftRow {
    pub receiver: String,
    pub channel: String,
    pub goal: String,
    pub scores: String,
    #[serde(default)]
    pub send: String,
    pub href: String,
}

/// The columns and rows a list draws: its Send column only once a row has a send.
pub fn draft_table(rows: &[DraftRow]) -> (Vec<String>, Vec<TableRow>) {
    let sent = rows.iter().any(|r| !r.send.is_empty());
    let mut columns: Vec<String> = ["Receiver", "Channel", "Goal", "Scores"]
        .map(String::from)
        .into();
    if sent {
        columns.push("Send".into());
    }
    let rows = rows.iter().map(|r| {
        let mut cells = vec![
            r.receiver.clone(),
            r.channel.clone(),
            r.goal.clone(),
            r.scores.clone(),
        ];
        if sent {
            cells.push(r.send.clone());
        }
        TableRow {
            cells,
            href: Some(r.href.clone()),
        }
    });
    (columns, rows.collect())
}

/// The drafts at one stage, a table a row a draft.
#[component]
pub fn DraftList(#[props(into)] id: String, rows: Vec<DraftRow>) -> Element {
    let (columns, rows) = draft_table(&rows);
    rsx! { Table { id, columns, rows } }
}

/// A reply draft's card: its goal, from and to, its checks' state, then its words and actions.
#[component]
pub fn DraftCard(
    #[props(into)] id: String,
    #[props(into)] goal: String,
    #[props(into)] from: String,
    #[props(into)] to: String,
    #[props(into)] state: String,
    children: Element,
) -> Element {
    rsx! {
        article { id, class: "draft-card",
            header {
                div { class: "goal", "Goal: {goal}" }
                div { class: "who", "From {from} · To {to} " Badge { "{state}" } }
            }
            {children}
        }
    }
}
