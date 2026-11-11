//! Gmail-like lines over the spike's 100,000 rows, windowed by hand: dioxus-components' VirtualList reads the offset
//! through document::eval, which needs JS.
use crate::{Avatar, Badge, Button, Dialog, Input};
use dioxus::prelude::*;
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU64, Ordering};
use ui_spike_common::{ROW_H, ROWS, Tone, row};

/// Lines drawn at once: an 800 px window plus overscan, as blitz-list.
const WINDOW_ROWS: usize = 18;

/// The list's offset, for the meter outside the component.
static SCROLL_TOP: AtomicU64 = AtomicU64::new(0);
/// The offset the lines were last built for: a frame painted before they catch up shows old lines and must not count.
static RENDERED_TOP: AtomicU64 = AtomicU64::new(0);

pub fn scroll_top() -> f64 {
    f64::from_bits(SCROLL_TOP.load(Ordering::Relaxed))
}

pub fn rows_caught_up() -> bool {
    RENDERED_TOP.load(Ordering::Relaxed) == SCROLL_TOP.load(Ordering::Relaxed)
}

#[component]
pub(crate) fn Mail() -> Element {
    let mut top = use_signal(|| 0.0f64);
    let mut query = use_signal(String::new);
    let mut starred = use_signal(BTreeSet::<usize>::new);
    let mut ticked = use_signal(BTreeSet::<usize>::new);
    let mut composing = use_signal(|| false);
    let mut draft = use_signal(String::new);
    let mut sent = use_signal(|| 0);
    // ponytail: scans every row on each key, a search index when the real mailbox lands (rust-ui-board).
    let matches = use_memo(move || {
        let q = query().to_lowercase();
        (!q.is_empty()).then(|| {
            (0..ROWS)
                .filter(|i| row(*i).title.to_lowercase().contains(&q))
                .collect::<Vec<_>>()
        })
    });
    RENDERED_TOP.store(top.read().to_bits(), Ordering::Relaxed);
    let count = matches.read().as_ref().map_or(ROWS, Vec::len);
    let first = (*top.read() / ROW_H) as usize;
    let last = (first + WINDOW_ROWS).min(count);
    let offset = first as f64 * ROW_H;
    let total = count as f64 * ROW_H;
    let shown: Vec<usize> = (first.min(last)..last)
        .map(|n| matches.read().as_ref().map_or(n, |m| m[n]))
        .collect();
    rsx! {
        div { class: "screen",
            div { class: "bar",
                span { class: "bar-title", "Mail" }
                Input { id: "mail-search", class: "input search", placeholder: "Search mail", value: query(),
                    oninput: move |v| {
                        query.set(v);
                        // The list starts over from its top: it remounts by its key, as a shorter list has no such offset.
                        SCROLL_TOP.store(0f64.to_bits(), Ordering::Relaxed);
                        top.set(0.0);
                    }
                }
                span { id: "mail-count", class: "meta", "{count} lines" }
                span { id: "mail-state", class: "meta", "{starred.read().len()} starred, {ticked.read().len()} ticked, {sent} sent" }
                span { class: "spring" }
                Button { id: "compose", onclick: move |_| composing.set(true), "Compose" }
            }
            div { id: "list", key: "{query}", class: "scroller",
                onscroll: move |e| {
                    let y = e.data().scroll_top();
                    SCROLL_TOP.store(y.to_bits(), Ordering::Relaxed);
                    top.set(y);
                },
                div { class: "spacer", style: "height:{total}px",
                    div { class: "window", style: "top:{offset}px",
                        for i in shown {
                            // No key: the same 18 rows take new props. A key remounts each row as it scrolls, and that one word
                            // took the peak from 85.8 to 155.7 MB over 7,429 frames, never returned (section 9, point p-window-budget).
                            Line { i,
                                star: starred.read().contains(&i),
                                tick: ticked.read().contains(&i),
                                onstar: move |_| { starred.write().toggle_member(i); },
                                ontick: move |_| { ticked.write().toggle_member(i); },
                            }
                        }
                    }
                }
            }
        }
        if composing() {
            Dialog { title: "New message", onclose: move |_| composing.set(false),
                div { class: "field",
                    Input { id: "draft-input", placeholder: "Message", value: draft(), oninput: move |v| draft.set(v) }
                }
                div { id: "draft-echo", class: "meta", "{draft.read().chars().count()} characters" }
                div { class: "dialog-actions",
                    Button { id: "send", onclick: move |_| { sent += 1; draft.set(String::new()); composing.set(false); }, "Send" }
                }
            }
        }
    }
}

trait Toggle {
    fn toggle_member(&mut self, i: usize);
}

impl Toggle for BTreeSet<usize> {
    fn toggle_member(&mut self, i: usize) {
        if !self.remove(&i) {
            self.insert(i);
        }
    }
}

#[component]
fn Line(
    i: usize,
    star: bool,
    tick: bool,
    onstar: EventHandler<()>,
    ontick: EventHandler<()>,
) -> Element {
    let r = row(i);
    let tone = match r.tone {
        Tone::Primary => "tone-primary",
        Tone::Secondary => "tone-secondary",
        Tone::Outline => "tone-outline",
        Tone::Destructive => "tone-destructive",
    };
    let sender = r.subtitle.split(',').next().unwrap_or_default();
    let class = match (tick, i % 3 == 0) {
        (true, _) => "line ticked",
        (_, true) => "line unread",
        _ => "line",
    };
    rsx! {
        div { id: "line-{i}", class,
            div { id: "tick-{i}", class: if tick { "box on" } else { "box" }, onclick: move |_| ontick.call(()) }
            div { id: "star-{i}", class: if star { "star on" } else { "star" }, onclick: move |_| onstar.call(()), if star { "★" } else { "☆" } }
            Avatar { initials: r.initials }
            div { class: "sender", "{sender}" }
            div { class: "subject", "{r.title} ", span { class: "snippet", "{r.subtitle}" } }
            Badge { class: tone, "{r.badge}" }
            div { class: "date", "Nov 6" }
            div { class: "actions", div { class: "action", "Archive" } div { class: "action", "Delete" } }
        }
    }
}
