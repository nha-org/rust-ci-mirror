use crate::component::Component;
use dioxus::prelude::*;

inventory::submit! {
    Component {
        slug: "tabs",
        classes: &["tabs"],
        tags: &[],
        name: "Tabs",
        used: "The tasks board's views; a tree's « links »: /mail's accounts and pager, /settings' download.",
        example: Some(|_| {
            rsx! {
                Tabs { id: "c-tabs", label: "Views",
                    Tab { href: "/tasks", current: true, "List" }
                    Tab { href: "/tasks?layout=board", current: false, "Board" }
                }
            }
        }),
        terminal: Some(crate::component::Terminal::Drawn(|_| {
            tabs_terminal(&["List".into(), "Board".into()], Some(0))
        })),
        ..Component::BARE
    }
}

/// One view of several, each a link: the server render runs no handler, so a tab changes the URL.
#[component]
pub fn Tabs(#[props(into)] id: String, label: &'static str, children: Element) -> Element {
    rsx! { nav { id, class: "tabs", "aria-label": label, {children} } }
}

#[component]
pub fn Tab(#[props(into)] href: String, current: bool, children: Element) -> Element {
    let on = current.then_some("page");
    rsx! { a { href, "aria-current": on, {children} } }
}

/// Tabs' terminal form, ratatui's Tabs (task rust-tui-app): each tab's name, a bar between, the one shown bold and
/// underlined in the theme's primary colour, the rest the muted one.
pub fn tabs_terminal(names: &[String], shown: Option<usize>) -> ratatui::text::Text<'static> {
    use ratatui::style::Style;
    use ratatui::widgets::Widget;
    // ratatui pads each name a cell each side and puts a divider between
    let width = names
        .iter()
        .map(|n| n.chars().count() as u16 + 2)
        .sum::<u16>()
        + names.len().saturating_sub(1) as u16;
    let area = ratatui::layout::Rect::new(0, 0, width.max(1), 1);
    let mut buf = ratatui::buffer::Buffer::empty(area);
    ratatui::widgets::Tabs::new(names.to_vec())
        .select(shown)
        .style(Style::new().fg(crate::theme::tone("neutral-content")).dim())
        .highlight_style(
            Style::new()
                .fg(crate::theme::tone("primary"))
                .not_dim()
                .bold()
                .underlined(),
        )
        .render(area, &mut buf);
    super::tui_screen::text_of(&buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;

    /// Done-when 4 of rust-tui-app: Tabs' terminal form is ratatui's Tabs, the names a bar apart, the one shown bold and
    /// underlined in the theme's primary colour, the others muted.
    #[dash_test]
    fn tabs_draw_ratatui_s_tabs_the_one_shown_in_the_theme_colour() {
        let names = ["List", "Board", "Graph"].map(String::from);
        let buf = crate::components::drawn(&tabs_terminal(&names, Some(1)));
        let shown = crate::tui::lines(&buf);
        // its last blank kept, dimmed as the rest
        assert_eq!(shown, [" List │ Board │ Graph "]);
        let board = shown[0].chars().position(|c| c == 'B').expect("Board") as u16;
        assert_eq!(buf[(board, 0)].fg, crate::theme::tone("primary"));
        let m = buf[(board, 0)].modifier;
        assert!(
            m.contains(ratatui::style::Modifier::BOLD)
                && m.contains(ratatui::style::Modifier::UNDERLINED)
        );
        assert_eq!(buf[(1, 0)].fg, crate::theme::tone("neutral-content"));
        assert!(buf[(1, 0)].modifier.contains(ratatui::style::Modifier::DIM));
    }
}
