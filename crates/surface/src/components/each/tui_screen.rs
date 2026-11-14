//! TuiScreen: a ratatui Buffer as HTML, a cell a character, its styles as classes: how /design shows a component's terminal
//! form beside its web form, drawn in-process (rust/CLAUDE.md rule 4, task rust-tui-tracer).
use crate::component::{Component, Terminal};
use dioxus::prelude::*;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Paragraph, Widget};

inventory::submit! {
    Component {
        slug: "tui-screen",
        classes: &["tui-screen", "tui-bold", "tui-dim", "tui-rev", "tui-at"],
        tags: &[],
        name: "Terminal form",
        used: "A component as `dash tui` draws it: ratatui's buffer, a cell a character, beside its web form on /design. Each \
               row a component drew carries its anchor, so the web compass points at it as `dash tui`'s does.",
        example: Some(|_| {
            let keys = ratatui::text::Line::from(vec![super::kbd::key("Enter"), " open".into()]);
            rsx! { TuiScreen { id: "c-tui", buffer: buffer(&keys.into(), 24) } }
        }),
        library: Some("https://docs.rs/ratatui/0.30.2/ratatui/buffer/struct.Buffer.html"),
        css: ".tui-screen { box-sizing: border-box; margin: 0; padding: var(--spacing-s2); overflow-x: auto; white-space: pre; border-radius: var(--radius-box); background: var(--color-neutral); color: var(--color-neutral-content); font-family: ui-monospace, Menlo, monospace; font-size: 12px; line-height: 1.4; }\n.tui-bold { font-weight: 700; }\n.tui-dim { opacity: 0.6; }\n.tui-rev { background: var(--color-neutral-content); color: var(--color-neutral); }\n.tui-at span { pointer-events: none; }\n",
        terminal: Some(crate::component::Terminal::Words),
        ..Component::BARE
    }
}

/// A component's terminal form, `text` its web form's text form, already read: `Words` draws it, a component's own drawing
/// its lines given the page's query. None for BARE's, which dash-gate refuses.
pub fn form_of(c: &Component, text: &str, query: &str) -> Option<Text<'static>> {
    Some(match c.terminal? {
        Terminal::Words => words(text),
        Terminal::Drawn(draw) => draw(query),
    })
}

/// A terminal form as wide as its widest line, drawn into a buffer: never cut, the screen scrolls (/code-review).
pub fn drawn(form: &Text<'_>) -> Buffer {
    buffer(form, form.width().clamp(8, u16::MAX as usize) as u16)
}

/// A text form's lines, a record's tabs as two spaces: a terminal draws no tab stop.
pub fn words(text: &str) -> Text<'static> {
    let lines: Vec<Line<'static>> = (text.trim_end_matches('\n').lines())
        .map(|l| Line::from(l.replace('\t', "  ")))
        .collect();
    Text::from(lines)
}

/// `text` drawn by ratatui into a buffer `width` cells wide and as tall as its lines, wrapped at nothing.
pub fn buffer(text: &Text<'_>, width: u16) -> Buffer {
    let height = text.lines.len().max(1) as u16;
    let mut buf = Buffer::empty(Rect::new(0, 0, width, height));
    Paragraph::new(text.clone()).render(buf.area, &mut buf);
    buf
}

/// Each row's runs of one style, its trailing blanks dropped: a wide character's trailing cell is skipped, as a terminal
/// shows it.
pub fn runs(buf: &Buffer) -> Vec<Vec<(String, Modifier)>> {
    let area = buf.area;
    (area.top()..area.bottom())
        .map(|y| {
            let mut row: Vec<(String, Modifier)> = vec![];
            let mut skip = 0;
            for x in area.left()..area.right() {
                if skip > 0 {
                    skip -= 1;
                    continue;
                }
                let cell = &buf[(x, y)];
                skip = Span::raw(cell.symbol()).width().saturating_sub(1);
                match row.last_mut() {
                    Some((text, m)) if *m == cell.modifier => text.push_str(cell.symbol()),
                    _ => row.push((cell.symbol().to_string(), cell.modifier)),
                }
            }
            // a styled blank stays (a keycap's padding); a plain one at the end goes
            while let Some((text, m)) = row.last_mut() {
                if !m.is_empty() {
                    break;
                }
                let kept = text.trim_end().len();
                text.truncate(kept);
                if !text.is_empty() {
                    break;
                }
                row.pop();
            }
            row
        })
        .collect()
}

/// A run's classes: its modifiers that the look draws; ponytail: colours are not drawn, none of our terminal forms sets one.
fn look(m: Modifier) -> String {
    let mut classes = vec![];
    for (bit, class) in [
        (Modifier::BOLD, "tui-bold"),
        (Modifier::DIM, "tui-dim"),
        (Modifier::REVERSED, "tui-rev"),
    ] {
        if m.contains(bit) {
            classes.push(class);
        }
    }
    classes.join(" ")
}

/// A row's runs, each a span carrying its classes when it has any.
fn row_of(row: Vec<(String, Modifier)>) -> Element {
    rsx! {
        for (text, m) in row {
            if look(m).is_empty() { "{text}" } else { span { class: look(m), "{text}" } }
        }
    }
}

/// A buffer as HTML: its rows in one block, each run of a style a span carrying its classes. A row inside one of `anchors`
/// (the cells a component drew, and its anchor: `Tui::spots`) is one span carrying `data-anchor`, its words alone (task
/// rust-tui-compass); its runs take no pointer (`.tui-at span`), so the web compass's tap lands on that span and quotes the
/// row, as `dash tui`'s pointing does (/code-review). ponytail: an anchor holds whole rows, as `dash tui` draws a component
/// a row each; a cell-exact one waits for components drawn side by side.
#[component]
pub fn TuiScreen(
    #[props(into)] id: String,
    buffer: Buffer,
    #[props(default)] anchors: Vec<(Rect, String)>,
) -> Element {
    let top = buffer.area.top();
    let rows = runs(&buffer);
    let last = rows.len().saturating_sub(1);
    let anchor = |i: usize| {
        let y = top + i as u16;
        (anchors.iter())
            .find(|(r, _)| r.top() <= y && y < r.bottom())
            .map(|(_, a)| a.clone())
    };
    rsx! {
        pre { id, class: "tui-screen",
            for (i, row) in rows.into_iter().enumerate() {
                if let Some(a) = anchor(i) {
                    span { class: "tui-at", "data-anchor": a, {row_of(row)} }
                } else {
                    {row_of(row)}
                }
                if i < last { "\n" }
            }
        }
    }
}
