use super::page_link::PageLink;
use super::parts::icon;
use crate::component::Component;
use dioxus::prelude::*;

inventory::submit! {
    Component {
        slug: "drag_blocks",
        classes: &["blocks", "block", "block-handle", "block-drop", "block-end"],
        tags: &[],
        name: "Drag blocks",
        used: "A document's blocks, each moved by its handle: dropped on a block's left half it goes before it, on its right half \
               under it, nested, its own nested blocks with it. Mouse events, so the window (Blitz has no HTML5 drag) and the web \
               (through liveview) run the same Rust.",
        example: Some(|_| rsx! { DragBlocks { id: "c-blocks", blocks: sample_blocks() } }),
        css: include_str!("blocks.css"),
        ..Component::BARE
    }
}

/// What /design's examples move: a heading line, two links, one nested.
pub fn sample_blocks() -> Vec<Block> {
    vec![
        Block::text(1, 0, "Rendering plan, its parts"),
        Block::page(
            2,
            0,
            "One tree, two renderers",
            "/design?tab=documents",
            "RFC",
            "draft",
        ),
        Block::page(3, 1, "Fonts", "/tasks/rust-fonts", "Task", "open"),
        Block::page(4, 0, "Selection", "/tasks/rust-selection", "Task", "done"),
    ]
}

/// A link-to-page block's target: where it goes, its type and its stage.
#[derive(Clone, Debug, PartialEq)]
pub struct PageTarget {
    pub href: String,
    pub kind: String,
    pub stage: String,
}

/// One block of a document as it is moved: its id (its key, kept through moves), its depth, its words, and its page when it
/// is a link to one.
#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    pub id: usize,
    pub depth: usize,
    pub text: String,
    pub page: Option<PageTarget>,
}

impl Block {
    pub fn text(id: usize, depth: usize, text: &str) -> Self {
        let text = text.to_string();
        Self {
            id,
            depth,
            text,
            page: None,
        }
    }

    pub fn page(id: usize, depth: usize, title: &str, href: &str, kind: &str, stage: &str) -> Self {
        let page = PageTarget {
            href: href.into(),
            kind: kind.into(),
            stage: stage.into(),
        };
        Self {
            page: Some(page),
            ..Self::text(id, depth, title)
        }
    }
}

/// Where a lifted block lands: before a block, at its depth; into it, its first nested block; or last, at the top.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BlockDrop {
    Before(usize),
    Into(usize),
    End,
}

/// The block at `i` and the deeper ones after it, which move with it.
fn span(blocks: &[Block], i: usize) -> std::ops::Range<usize> {
    let depth = blocks[i].depth;
    let after = blocks[i + 1..].iter().position(|b| b.depth <= depth);
    i..after.map_or(blocks.len(), |n| i + 1 + n)
}

/// `blocks` with the one at `from` and its nested ones moved `to`; a drop on itself or inside what it carries changes nothing.
pub fn dropped(blocks: &[Block], from: usize, to: BlockDrop) -> Vec<Block> {
    let moved = span(blocks, from);
    let (depth, at) = match to {
        BlockDrop::Before(j) | BlockDrop::Into(j) if moved.contains(&j) => return blocks.to_vec(),
        BlockDrop::Before(j) => (blocks[j].depth, j),
        BlockDrop::Into(j) => (blocks[j].depth + 1, j + 1),
        BlockDrop::End => (0, blocks.len()),
    };
    let base = blocks[from].depth;
    let carried = blocks[moved.clone()].iter().map(|b| Block {
        depth: b.depth - base + depth,
        ..b.clone()
    });
    let mut rest: Vec<Block> = [&blocks[..moved.start], &blocks[moved.end..]].concat();
    let at = if at > moved.start {
        at - moved.len()
    } else {
        at
    };
    rest.splice(at..at, carried);
    rest
}

/// Six dots, the handle a block is lifted by.
const GRIP: &str = "M9 5h.01M9 12h.01M9 19h.01M15 5h.01M15 12h.01M15 19h.01";

/// A document's blocks, each moved by its handle (task design-doc-tree-menu; @nha, 2026-11-12: « how drag and drop of
/// notion-like blocks works »). A press on a handle lifts its block; while lifted, each other block's left half drops it
/// before that block and its right half nests it there (`dropped`); the release drops it where the pointer is, off every
/// drop zone nowhere, and leaving the list cancels. Mouse
/// events alone: the window's Blitz has no HTML5 drag, and the web runs these handlers through liveview.
/// ponytail: no keyboard move; Alt+arrows on a focused handle if he asks.
#[component]
pub fn DragBlocks(#[props(into)] id: String, blocks: Vec<Block>) -> Element {
    let mut all = use_signal(|| blocks);
    let mut lifted = use_signal(|| None::<usize>);
    let mut over = use_signal(|| None::<BlockDrop>);
    let shown = all();
    let class = |i: usize| {
        let mut class = String::from("block");
        if lifted() == Some(i) {
            class += " lifted";
        }
        match over() {
            Some(BlockDrop::Before(j)) if j == i => class += " over-before",
            Some(BlockDrop::Into(j)) if j == i => class += " over-into",
            _ => {}
        }
        class
    };
    rsx! {
        div { id: "{id}", class: if lifted().is_some() { "blocks dragging" } else { "blocks" }, role: "list",
            onmouseup: move |_| {
                if let (Some(from), Some(to)) = (lifted(), over()) {
                    let moved = dropped(&all.peek(), from, to);
                    all.set(moved);
                }
                lifted.set(None);
                over.set(None);
            },
            onmouseleave: move |_| {
                lifted.set(None);
                over.set(None);
            },
            for (i, b) in shown.into_iter().enumerate() {
                div { key: "{b.id}", id: "{id}-{b.id}", class: class(i), role: "listitem", style: "--depth:{b.depth}",
                    span { class: "block-handle", title: "Drag to move", "aria-hidden": "true",
                        onmousedown: move |e: MouseEvent| {
                            e.prevent_default();
                            lifted.set(Some(i));
                        },
                        {icon(GRIP)}
                    }
                    match b.page {
                        Some(p) => rsx! { PageLink { href: p.href, title: b.text, kind: p.kind, stage: p.stage } },
                        None => rsx! { p { "{b.text}" } },
                    }
                    if lifted().is_some() {
                        span { class: "block-drop", "data-drop": "before", onmouseenter: move |_| over.set(Some(BlockDrop::Before(i))),
                            onmouseleave: move |_| over.set(None) }
                        span { class: "block-drop", "data-drop": "into", onmouseenter: move |_| over.set(Some(BlockDrop::Into(i))),
                            onmouseleave: move |_| over.set(None) }
                    }
                }
            }
            if lifted().is_some() {
                div { class: if over() == Some(BlockDrop::End) { "block-end over" } else { "block-end" },
                    onmouseenter: move |_| over.set(Some(BlockDrop::End)),
                    onmouseleave: move |_| over.set(None),
                    "Drop here to move it last"
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shape(blocks: &[Block]) -> Vec<(usize, usize)> {
        blocks.iter().map(|b| (b.id, b.depth)).collect()
    }

    /// Done-when 3 of design-doc-tree-menu, the rule: a block dropped before another takes its place and depth, into one
    /// becomes its first nested block, its own nested ones carried along; a drop inside what it carries changes nothing.
    #[dash_meter::dash_test]
    fn a_dropped_block_moves_with_its_nested_ones() {
        let blocks = sample_blocks();
        assert_eq!(shape(&blocks), [(1, 0), (2, 0), (3, 1), (4, 0)]);
        // the last before the first: reordered
        assert_eq!(
            shape(&dropped(&blocks, 3, BlockDrop::Before(0))),
            [(4, 0), (1, 0), (2, 0), (3, 1)]
        );
        // the second, with its nested one, into the last: nested a level down
        assert_eq!(
            shape(&dropped(&blocks, 1, BlockDrop::Into(3))),
            [(1, 0), (4, 0), (2, 1), (3, 2)]
        );
        // a nested one out, last at the top
        assert_eq!(
            shape(&dropped(&blocks, 2, BlockDrop::End)),
            [(1, 0), (2, 0), (4, 0), (3, 0)]
        );
        // into what it carries: no move
        assert_eq!(dropped(&blocks, 1, BlockDrop::Into(2)), blocks);
        assert_eq!(dropped(&blocks, 1, BlockDrop::Before(1)), blocks);
    }
}
