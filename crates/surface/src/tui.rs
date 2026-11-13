//! The terminal form of a page, drawn by ratatui from the same UI tree as its HTML and its text (rust/CLAUDE.md rule 4, task
//! rust-tui-tracer; @nha, 2026-11-13: « better get this in early to get it right »). Each element draws through its
//! component's terminal form (crate::component::Terminal): a task's row its Item's, a count its Badge's, a key its Kbd's; an
//! element whose words are the whole of it, its text form. `dash tui` runs `Tui` on crossterm; a test draws it into a Buffer.
use super::{Element as Kind, Tree, check};
use crate::components::{badge, issue_line, key, words};
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};

/// One line of a page in the terminal; `task`: the task it opens.
struct Row {
    line: Line<'static>,
    task: Option<String>,
}

/// A tree's rows, from its root: what `dash tui` lists.
fn rows(tree: &str) -> Result<Vec<Row>, String> {
    let tree = check(tree)?;
    let mut out = vec![];
    walk(&tree, &tree.root, &mut out);
    Ok(out)
}

fn walk(tree: &Tree, id: &str, out: &mut Vec<Row>) {
    let Some(n) = tree.elements.get(id) else {
        return;
    };
    let plain = |line: Line<'static>| Row { line, task: None };
    match &n.element {
        Kind::IssueRow {
            id: task,
            title,
            says,
            ..
        } => out.push(Row {
            line: issue_line(task, title, says),
            task: Some(task.clone()),
        }),
        Kind::IssueGroup {
            status,
            title,
            more,
        } => {
            let count = n.children.len() + more.as_ref().map_or(0, |m| m.count);
            let name = title.as_deref().unwrap_or(status.name()).to_string();
            out.push(plain(Line::from(vec![
                name.bold(),
                " ".into(),
                badge("count", &count.to_string()),
            ])));
            for child in &n.children {
                walk(tree, child, out);
            }
        }
        // for agents alone: nothing of it drawn for him, as on the web page
        Kind::Section { agents: true, .. } => {}
        Kind::Section { title, .. } => {
            out.push(plain(Line::from(title.clone().bold())));
            for child in &n.children {
                walk(tree, child, out);
            }
        }
        Kind::Heading { text } => out.push(plain(Line::from(text.clone().bold()))),
        Kind::Stack | Kind::List | Kind::Cards | Kind::Bar | Kind::Board { .. } => {
            for child in &n.children {
                walk(tree, child, out);
            }
        }
        // the rest: its words, read off the very HTML the web page draws of it
        _ => out.extend(
            words(&crate::words::text_of(&crate::screen::node_html(tree, id)))
                .lines
                .into_iter()
                .map(plain),
        ),
    }
}

/// A key `dash tui` reads, as crossterm names it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Key {
    Up,
    Down,
    Enter,
    Esc,
    Backspace,
    Char(char),
}

/// What a key asks `dash tui` to do: open a task's page, post a comment on a task through the web's handler, or end.
#[derive(Debug, PartialEq)]
pub enum Act {
    Open(String),
    Comment { task: String, text: String },
    Quit,
}

/// /tasks in the terminal: its list, the task selected, a task opened, a comment being typed.
pub struct Tui {
    list: Vec<Row>,
    /// The row selected: always a task's while the list has one.
    at: usize,
    /// The task opened: its id and its page's rows, scrolled by `scroll`.
    open: Option<(String, Vec<Row>)>,
    scroll: usize,
    typing: Option<String>,
    note: String,
}

impl Tui {
    /// The list page's tree: /tasks as `Board::tasks` draws it.
    pub fn of(tree: &str) -> Result<Tui, String> {
        let list = rows(tree)?;
        let at = list.iter().position(|r| r.task.is_some()).unwrap_or(0);
        Ok(Tui {
            list,
            at,
            open: None,
            scroll: 0,
            typing: None,
            note: String::new(),
        })
    }

    /// A task's page opened: its tree, as /tasks/<id> draws it, then `rail`, the text form of its comment rail, which the web
    /// draws beside the tree (forms.rs's sidebar), never in it.
    pub fn opened(&mut self, task: &str, tree: &str, rail: &str) -> Result<(), String> {
        let mut page = rows(tree)?;
        page.extend(
            words(rail)
                .lines
                .into_iter()
                .map(|line| Row { line, task: None }),
        );
        self.open = Some((task.to_string(), page));
        self.scroll = 0;
        Ok(())
    }

    /// The task open, when one is.
    pub fn open_task(&self) -> Option<&str> {
        self.open.as_ref().map(|(task, _)| task.as_str())
    }

    /// His comment landed: the box closed, `note` said under the page.
    pub fn posted(&mut self, note: impl Into<String>) {
        self.typing = None;
        self.say(note);
    }

    /// Text pasted into the comment box, its line breaks as spaces: a paste never presses Enter or a key (/code-review).
    /// Outside the box, a paste does nothing.
    pub fn paste(&mut self, text: &str) {
        if let Some(typing) = &mut self.typing {
            typing.extend(text.chars().map(|c| if c.is_control() { ' ' } else { c }));
        }
    }

    /// A line under the page: what the last act did.
    pub fn say(&mut self, note: impl Into<String>) {
        self.note = note.into();
    }

    /// The task a comment goes to: the one open, else the one selected.
    fn task(&self) -> Option<String> {
        match &self.open {
            Some((task, _)) => Some(task.clone()),
            None => self.list.get(self.at).and_then(|r| r.task.clone()),
        }
    }

    /// The next task's row up or down from the one selected; none past the ends.
    fn step(&mut self, down: bool) {
        let tasks = |i: &usize| self.list[*i].task.is_some();
        let next = match down {
            true => (self.at + 1..self.list.len()).find(tasks),
            false => (0..self.at).rev().find(tasks),
        };
        self.at = next.unwrap_or(self.at);
    }

    pub fn key(&mut self, key: Key) -> Option<Act> {
        if let Some(text) = &mut self.typing {
            match key {
                Key::Char(c) => text.push(c),
                Key::Backspace => {
                    text.pop();
                }
                Key::Esc => self.typing = None,
                // kept until `posted`: a post refused leaves his words in the box (/code-review)
                Key::Enter => {
                    let text = text.clone();
                    let task = self.task()?;
                    return (!text.trim().is_empty()).then_some(Act::Comment { task, text });
                }
                _ => {}
            }
            return None;
        }
        match (key, &self.open) {
            // Esc goes back, never out: q alone quits, as the keys say (/code-review)
            (Key::Char('q'), _) => return Some(Act::Quit),
            (Key::Char('c'), _) if self.task().is_some() => self.typing = Some(String::new()),
            (Key::Esc, Some(_)) => self.open = None,
            (Key::Up, Some(_)) => self.scroll = self.scroll.saturating_sub(1),
            (Key::Down, Some((_, rows))) => {
                self.scroll = (self.scroll + 1).min(rows.len().saturating_sub(1))
            }
            (Key::Up, None) => self.step(false),
            (Key::Down, None) => self.step(true),
            (Key::Enter, None) => return self.task().map(Act::Open),
            _ => {}
        }
        None
    }

    /// The keys this state takes, each its Kbd's terminal form then what it does.
    fn keys(&self) -> Line<'static> {
        let pairs: &[(&str, &str)] = match (&self.typing, &self.open) {
            (Some(_), _) => &[("Enter", "send"), ("Esc", "cancel")],
            (None, Some(_)) => &[
                ("↑↓", "scroll"),
                ("c", "comment"),
                ("Esc", "back"),
                ("q", "quit"),
            ],
            (None, None) => &[
                ("↑↓", "select"),
                ("Enter", "open"),
                ("c", "comment"),
                ("q", "quit"),
            ],
        };
        let mut spans = vec![];
        for (k, does) in pairs {
            spans.extend([key(k), format!(" {does}  ").into()]);
        }
        Line::from(spans)
    }

    /// The whole screen into `buf`'s `area`: its title, the list or the task, the line typed or the last note, then the keys.
    pub fn draw(&self, area: Rect, buf: &mut Buffer) {
        let [head, body, note, keys] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .areas(area);
        let (title, rows, at, top) = match &self.open {
            Some((task, rows)) => (format!("Task {task}"), rows, None, self.scroll),
            None => {
                // the selected row kept in sight
                let top = self
                    .at
                    .saturating_sub(body.height.saturating_sub(1) as usize);
                ("Tasks".to_string(), &self.list, Some(self.at), top)
            }
        };
        Line::from(title.bold()).render(head, buf);
        let lines: Vec<Line<'static>> = (rows.iter().enumerate().skip(top))
            // a row selected is a task's: with none in the list, nothing is drawn selected (/code-review)
            .map(|(i, r)| match Some(i) == at && r.task.is_some() {
                true => r.line.clone().patch_style(Modifier::REVERSED),
                false => r.line.clone(),
            })
            .collect();
        Paragraph::new(lines).render(body, buf);
        let said = match (&self.typing, self.task()) {
            (Some(text), Some(task)) => Line::from(vec![
                Span::raw(format!("Comment on {task}: ")).bold(),
                Span::raw(format!("{text}▏")),
            ]),
            _ => Line::from(self.note.clone().dim()),
        };
        said.render(note, buf);
        self.keys().render(keys, buf);
    }
}

/// A buffer's rows as plain lines, their trailing blanks dropped: what a test reads of a screen.
pub fn lines(buf: &Buffer) -> Vec<String> {
    crate::components::runs(buf)
        .into_iter()
        .map(|row| row.into_iter().map(|(t, _)| t).collect::<String>())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;
    use serde_json::json;

    fn board() -> String {
        json!({"root": "page", "elements": {
            "page": {"type": "stack", "children": ["title", "open"]},
            "title": {"type": "heading", "props": {"text": "Tasks"}},
            "open": {"type": "issue-group", "props": {"status": "todo"}, "children": ["a", "b"]},
            "a": {"type": "issue-row", "props": {"id": "a", "title": "Port the board", "status": "todo", "says": "", "comments": 0}},
            "b": {"type": "issue-row", "props": {"id": "b", "title": "Draw it in the terminal", "status": "todo", "says": "waits on a", "comments": 0}},
        }})
        .to_string()
    }

    fn screen(tui: &Tui) -> Buffer {
        let mut buf = Buffer::empty(Rect::new(0, 0, 60, 8));
        tui.draw(buf.area, &mut buf);
        buf
    }

    /// Done-when 3's surface half: the list drawn from the tree, a task selected and opened, the keys shown at the foot.
    #[dash_test]
    fn the_list_selects_and_opens_a_task_its_keys_at_the_foot() {
        let mut tui = Tui::of(&board()).expect("a tree");
        let shown = lines(&screen(&tui));
        assert_eq!(shown[0], "Tasks");
        assert!(shown.contains(&"a Port the board".to_string()), "{shown:?}");
        assert_eq!(shown[7], " ↑↓  select   Enter  open   c  comment   q  quit");
        // the selection is drawn reversed, on the first task's row
        let buf = screen(&tui);
        let row = (0..8)
            .find(|y| lines(&buf)[*y as usize].starts_with("a Port"))
            .expect("a's row");
        assert!(buf[(0, row)].modifier.contains(Modifier::REVERSED));
        assert_eq!(tui.key(Key::Down), None);
        assert_eq!(tui.key(Key::Enter), Some(Act::Open("b".into())));
        let page = json!({"root": "p", "elements": {"p": {"type": "text", "props": {"text": "Its brief"}}}});
        tui.opened("b", &page.to_string(), "").expect("a tree");
        let shown = lines(&screen(&tui));
        assert_eq!(shown[0], "Task b");
        assert_eq!(shown[1], "Its brief");
        assert_eq!(shown[7], " ↑↓  scroll   c  comment   Esc  back   q  quit");
        assert_eq!(tui.key(Key::Esc), None);
        assert_eq!(lines(&screen(&tui))[0], "Tasks");
        assert_eq!(tui.key(Key::Char('q')), Some(Act::Quit));
    }

    /// Done-when 4's surface half: c, words, Enter asks for the comment on the task selected; Esc drops it.
    #[dash_test]
    fn a_comment_is_typed_then_asked_for_its_task() {
        let mut tui = Tui::of(&board()).expect("a tree");
        tui.key(Key::Char('c'));
        for c in "Looks right".chars() {
            tui.key(Key::Char(c));
        }
        assert_eq!(lines(&screen(&tui))[6], "Comment on a: Looks right▏");
        assert_eq!(
            tui.key(Key::Enter),
            Some(Act::Comment {
                task: "a".into(),
                text: "Looks right".into()
            })
        );
        // refused, his words stay in the box; posted, it closes
        assert_eq!(lines(&screen(&tui))[6], "Comment on a: Looks right▏");
        tui.paste("\nand\tthis");
        tui.posted("Comment c-1 posted on a");
        assert_eq!(lines(&screen(&tui))[6], "Comment c-1 posted on a");
        tui.key(Key::Char('c'));
        tui.paste("x\ny");
        assert_eq!(lines(&screen(&tui))[6], "Comment on a: x y▏");
        assert_eq!(tui.key(Key::Esc), None);
        assert_eq!(tui.key(Key::Esc), None, "Esc on the list never quits");
        assert_eq!(tui.key(Key::Enter), Some(Act::Open("a".into())));
    }
}
