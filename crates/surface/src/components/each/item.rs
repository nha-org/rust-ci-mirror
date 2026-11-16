use super::badge::Badge;
use super::parts::joined;
use crate::component::Component;
use dioxus::prelude::*;

inventory::submit! {
    Component {
        slug: "list",
        classes: &[],
        tags: &[],
        name: "List",
        used: "The default list: home's suggestions, /inbox, /today, /agents, /tools, a row each with its title, its line and its link; a tree's « list » draws its cards this way.",
        example: Some(|_| {
            rsx! {
                div { id: "c-list", class: "list",
                    Item { id: "c-list-a", look: "issue-row", href: "/inbox".to_string(),
                        span { class: "title", "Inbox" }
                        span { class: "says", "2 wait on you" }
                    }
                    Item { id: "c-list-b", look: "issue-row",
                        span { class: "title", "Plan" }
                        span { class: "says", "now, next, later" }
                    }
                }
            }
        }),
        terminal: Some(crate::component::Terminal::Drawn(|_| {
            let row = |title: &str, says: &str| ratatui::text::Line::from(vec![title.to_string().into(), format!("  {says}").into()]);
            list_terminal(vec![row("Inbox", "2 wait on you"), row("Plan", "now, next, later")], Some(0))
        })),
        ..Component::BARE
    }
}

/// List's terminal form, ratatui's List (task rust-tui-app): an item a line, each marked by the theme's primary colour,
/// the one `picked` drawn reversed with its arrow, as `dash tui`'s page menu shows the page it opens.
pub fn list_terminal(
    items: Vec<ratatui::text::Line<'static>>,
    picked: Option<usize>,
) -> ratatui::text::Text<'static> {
    use ratatui::style::Style;
    use ratatui::widgets::{ListItem, ListState, StatefulWidget};
    const MARK: &str = "▸ ";
    let primary = Style::new().fg(crate::theme::tone("primary"));
    let width = items.iter().map(|l| l.width()).max().unwrap_or(0) + MARK.chars().count();
    let area = ratatui::layout::Rect::new(0, 0, width.max(1) as u16, items.len().max(1) as u16);
    let mut buf = ratatui::buffer::Buffer::empty(area);
    let list = ratatui::widgets::List::new(items.into_iter().map(ListItem::new))
        .highlight_symbol(ratatui::text::Span::styled(MARK, primary.bold()))
        .highlight_style(Style::new().reversed())
        .highlight_spacing(ratatui::widgets::HighlightSpacing::Always);
    StatefulWidget::render(
        list,
        area,
        &mut buf,
        &mut ListState::default().with_selected(picked),
    );
    super::tui_screen::text_of(&buf)
}

inventory::submit! {
    Component {
        slug: "item",
        classes: &["issue-row", "item", "access"],
        tags: &[],
        name: "Item",
        used: "The tools page's accesses, a row each; the tasks list's rows, look issue-row, each its task's link.",
        example: Some(|_| {
            rsx! {
                Access { id: "c-item", status: "works", name: "fly"
                }
                Item { id: "c-row", look: "issue-row", href: "/tasks/a".to_string(), task: "a", b { "a" } " Port the board" }
            }
        }),
        terminal: Some(crate::component::Terminal::Drawn(|_| {
            let access = ratatui::text::Line::from(vec![super::badge::badge("chip", "works"), " fly".into()]);
            ratatui::text::Text::from(vec![access, issue_line("a", "Port the board", "")])
        })),
        ..Component::BARE
    }
}

/// One row of a list: /tools' accesses, /tasks' rows (a link when it has `href`). `status` is `data-status`, which a look's
/// colours read (components.css); `task` is `data-task`, as Card's.
#[component]
pub fn Item(
    #[props(into)] id: String,
    #[props(default)] look: &'static str,
    #[props(default, into)] status: Option<String>,
    #[props(default)] href: Option<String>,
    #[props(default, into)] task: Option<String>,
    children: Element,
) -> Element {
    let class = joined("item", look);
    match href {
        Some(href) => {
            rsx! { a { id, class, "data-status": status, "data-task": task, href, {children} } }
        }
        None => rsx! { div { id, class, "data-status": status, "data-task": task, {children} } },
    }
}

/// One access on /tools: its state's pill, then its name with room between them (task design-access-pill; @nha,
/// 2026-11-09: « "works" pill eats the space next to fly and overlaps »), then what it used, why and where from. The design
/// page draws this same component, never a copy of its markup.
#[component]
pub fn Access(
    #[props(into)] id: String,
    #[props(into)] status: String,
    #[props(into)] name: String,
    #[props(default, into)] usage: String,
    #[props(default, into)] note: String,
    #[props(default, into)] place: String,
    #[props(default, into)] source: String,
) -> Element {
    rsx! {
        Item { id, look: "access", status: status.clone(),
            p { class: "head", Badge { look: "chip", "{status}" } b { class: "name", "{name}" } }
            p { class: "usage", "{usage}" }
            p { class: "note", "{note}" }
            p { class: "place", span { class: "source", "{source}" } " {place}" }
        }
    }
}

/// A task's row in the terminal: its id bold, its title, then what it waits on dimmed (`dash tui`'s /tasks, task
/// rust-tui-tracer).
pub fn issue_line(task: &str, title: &str, says: &str) -> ratatui::text::Line<'static> {
    use ratatui::style::Stylize;
    let mut spans = vec![task.to_string().bold(), format!(" {title}").into()];
    if !says.is_empty() {
        spans.push(format!("  {says}").dim());
    }
    spans.into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;

    /// Done-when 4 of rust-tui-app: List's terminal form is ratatui's List, an item a line behind the arrow's room, the one
    /// picked reversed under the theme's primary arrow; none picked, no arrow.
    #[dash_test]
    fn a_list_draws_ratatui_s_list_the_one_picked_under_the_theme_arrow() {
        let items = || {
            vec![
                ratatui::text::Line::from("Inbox"),
                ratatui::text::Line::from("Plan"),
            ]
        };
        let buf = crate::components::drawn(&list_terminal(items(), Some(1)));
        // the picked row reversed to the list's width
        assert_eq!(crate::tui::lines(&buf), ["  Inbox", "▸ Plan "]);
        assert_eq!(buf[(0, 1)].fg, crate::theme::tone("primary"));
        assert!(
            buf[(2, 1)]
                .modifier
                .contains(ratatui::style::Modifier::REVERSED)
        );
        assert!(
            !buf[(2, 0)]
                .modifier
                .contains(ratatui::style::Modifier::REVERSED)
        );
        let none = crate::components::drawn(&list_terminal(items(), None));
        assert_eq!(crate::tui::lines(&none), ["  Inbox", "  Plan"]);
    }
}
