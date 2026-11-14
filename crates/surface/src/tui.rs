//! The terminal form of a page, drawn by ratatui from the same UI tree as its HTML and its text (rust/CLAUDE.md rule 4, task
//! rust-tui-tracer; @nha, 2026-11-13: « better get this in early to get it right »). Each element draws through its
//! component's terminal form (crate::component::Terminal): a task's row its Item's, a count its Badge's, a key its Kbd's; an
//! element whose words are the whole of it, its text form. `dash tui` runs `Tui` on crossterm; a test draws it into a Buffer.
use super::{Element as Kind, Tree, check};
use crate::components::{Node, badge, issue_line, key, quote, words};
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};
use std::cell::Cell;

/// One line of a page in the terminal; `task`: the task it opens; `anchor`: the id of the tree's element that drew it, the
/// `id` its HTML carries on the web page (task rust-tui-compass), none for the rail's lines.
struct Row {
    line: Line<'static>,
    task: Option<String>,
    anchor: Option<String>,
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
    let anchor = Some(id.to_string());
    let plain = |line: Line<'static>| Row {
        line,
        task: None,
        anchor: anchor.clone(),
    };
    match &n.element {
        Kind::IssueRow {
            id: task,
            title,
            says,
            ..
        } => out.push(Row {
            line: issue_line(task, title, says),
            task: Some(task.clone()),
            anchor: anchor.clone(),
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

/// What a key asks `dash tui` to do: open a task's page, post a comment on a page through the web's handler (its quote the
/// words pointed at, "" the whole page, as the web compass sends it), or end.
#[derive(Debug, PartialEq)]
pub enum Act {
    Open(String),
    Comment {
        page: String,
        quote: String,
        text: String,
    },
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
    /// The compass on: the row pointed at, an anchored one of the rows shown.
    pointing: Option<usize>,
    /// What the box comments on once pointed: its anchor and its quote.
    pointed: Option<(String, String)>,
    /// The area the last draw filled: where each row's cells are, for a click.
    drawn: Cell<Rect>,
}

/// The screen's parts, top to bottom: the title, the page, the line typed or said, the keys.
fn parts(area: Rect) -> [Rect; 4] {
    Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(area)
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
            pointing: None,
            pointed: None,
            drawn: Cell::new(Rect::default()),
        })
    }

    /// A task's page opened: its tree, as /tasks/<id> draws it, then `rail`, the text form of its comment rail, which the web
    /// draws beside the tree (forms.rs's sidebar), never in it.
    pub fn opened(&mut self, task: &str, tree: &str, rail: &str) -> Result<(), String> {
        let mut page = rows(tree)?;
        page.extend(words(rail).lines.into_iter().map(|line| Row {
            line,
            task: None,
            anchor: None,
        }));
        self.open = Some((task.to_string(), page));
        self.scroll = 0;
        self.pointing = None;
        Ok(())
    }

    /// The task open, when one is.
    pub fn open_task(&self) -> Option<&str> {
        self.open.as_ref().map(|(task, _)| task.as_str())
    }

    /// His comment landed: the box closed, `note` said under the page.
    pub fn posted(&mut self, note: impl Into<String>) {
        self.typing = None;
        self.pointed = None;
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

    /// The page shown, as the web serves it: what a comment pointed on it is posted on.
    fn page(&self) -> String {
        match &self.open {
            Some((task, _)) => format!("/tasks/{task}"),
            None => "/tasks".into(),
        }
    }

    /// The rows shown.
    fn rows(&self) -> &[Row] {
        match &self.open {
            Some((_, rows)) => rows,
            None => &self.list,
        }
    }

    /// The first row in sight in a page `height` rows tall: the one pointed at, else the list's selected one, kept in sight.
    fn top(&self, height: u16) -> usize {
        let last = height.saturating_sub(1) as usize;
        let start = match &self.open {
            Some(_) => self.scroll,
            None => self.at.saturating_sub(last),
        };
        match self.pointing {
            Some(p) => start.min(p).max(p.saturating_sub(last)),
            None => start,
        }
    }

    /// Each anchored row in sight as the last draw put it: its cells and its anchor (task rust-tui-compass). A click names
    /// what is under it from these, and /design's TuiScreen carries them, so the web compass points at the same rows.
    pub fn spots(&self) -> Vec<(Rect, String)> {
        (self.cells().into_iter())
            .map(|(area, _, anchor)| (area, anchor))
            .collect()
    }

    /// Row `i`'s words as the web compass quotes a tap on them (compass's `quote`): one thread whichever form he pointed from.
    /// One row, never its element's every row: the web quotes the innermost element tapped, a paragraph, about a line of the
    /// text form, and a thread anchors on a passage inside one block (documents' `anchored`), never a whole page's words.
    fn quote_at(&self, i: usize) -> String {
        let text = self.rows()[i]
            .line
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        quote(&[Node {
            tag: "span".into(),
            text,
            ..Node::default()
        }])
    }

    /// Each anchored row in sight: its cells and its place among the rows. An element of many lines draws many rows of one
    /// anchor, so a click keeps the row it hit, never the first of its anchor.
    fn cells(&self) -> Vec<(Rect, usize, String)> {
        let [_, body, _, _] = parts(self.drawn.get());
        let top = self.top(body.height);
        (self
            .rows()
            .iter()
            .enumerate()
            .skip(top)
            .take(body.height as usize))
        .zip(body.y..)
        .filter_map(|((i, r), y)| {
            let width = (r.line.width() as u16).min(body.width);
            let anchor = r.anchor.clone().filter(|_| width > 0)?;
            Some((Rect::new(body.x, y, width, 1), i, anchor))
        })
        .collect()
    }

    /// The anchor pointed at, while the compass is on.
    pub fn pointed_at(&self) -> Option<&str> {
        self.rows().get(self.pointing?)?.anchor.as_deref()
    }

    /// The compass's tap in the terminal: the anchored row pointed at gets the comment box, quoting it as the web does.
    fn point(&mut self, i: usize) {
        let Some(anchor) = self.rows()[i].anchor.clone() else {
            return;
        };
        self.pointed = Some((anchor, self.quote_at(i)));
        self.unpoint();
        self.typing = Some(String::new());
    }

    /// The compass off, a task's page left scrolled where pointing took it: the row pointed at stays in sight (/code-review).
    /// ponytail: the list's view follows its selected task, so it may move back; a view of its own when that bites.
    fn unpoint(&mut self) {
        let [_, body, _, _] = parts(self.drawn.get());
        if self.open.is_some() {
            self.scroll = self.top(body.height);
        }
        self.pointing = None;
    }

    /// The compass on: what `dash tui` captures the mouse for, a click being its tap.
    pub fn pointing(&self) -> bool {
        self.pointing.is_some()
    }

    /// A click at a cell, as the terminal reports it: while the compass is on, the row drawn there is pointed at. The anchor
    /// it names, none off every row.
    pub fn click(&mut self, x: u16, y: u16) -> Option<String> {
        self.pointing?;
        let at = ratatui::layout::Position::new(x, y);
        let (_, i, anchor) = self.cells().into_iter().find(|(r, ..)| r.contains(at))?;
        self.point(i);
        Some(anchor)
    }

    /// The next anchored row up or down from the one pointed at; none past the ends.
    fn aim(&mut self, down: bool) {
        let Some(at) = self.pointing else { return };
        let rows = self.rows();
        let anchored = |i: &usize| rows[*i].anchor.is_some();
        let next = match down {
            true => (at + 1..rows.len()).find(anchored),
            false => (0..at).rev().find(anchored),
        };
        self.pointing = Some(next.unwrap_or(at));
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
                Key::Esc => {
                    self.typing = None;
                    self.pointed = None;
                }
                // kept until `posted`: a post refused leaves his words in the box (/code-review)
                Key::Enter => {
                    let text = text.clone();
                    let (page, quote) = match &self.pointed {
                        Some((_, quote)) => (self.page(), quote.clone()),
                        None => (format!("/tasks/{}", self.task()?), String::new()),
                    };
                    return (!text.trim().is_empty()).then_some(Act::Comment { page, quote, text });
                }
                _ => {}
            }
            return None;
        }
        if self.pointing.is_some() {
            match key {
                Key::Up => self.aim(false),
                Key::Down => self.aim(true),
                Key::Enter => self.point(self.pointing?),
                Key::Esc => self.unpoint(),
                _ => {}
            }
            return None;
        }
        match (key, &self.open) {
            // Esc goes back, never out: q alone quits, as the keys say (/code-review)
            (Key::Char('q'), _) => return Some(Act::Quit),
            (Key::Char('c'), _) if self.task().is_some() => self.typing = Some(String::new()),
            // the compass: from the row selected, or the first in sight, to the first anchored one
            (Key::Char('p'), _) => {
                let from = match &self.open {
                    Some(_) => self.scroll,
                    None => self.at,
                };
                let anchored = |i: &usize| self.rows()[*i].anchor.is_some();
                self.pointing = (from..self.rows().len())
                    .find(anchored)
                    .or_else(|| (0..from).rev().find(anchored));
                if self.pointing.is_none() {
                    self.say("Nothing here to point at");
                }
            }
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
            _ if self.pointing.is_some() => &[
                ("↑↓", "point"),
                ("Enter", "comment on it"),
                ("Esc", "cancel"),
            ],
            (None, Some(_)) => &[
                ("↑↓", "scroll"),
                ("c", "comment"),
                ("p", "point"),
                ("Esc", "back"),
                ("q", "quit"),
            ],
            (None, None) => &[
                ("↑↓", "select"),
                ("Enter", "open"),
                ("c", "comment"),
                ("p", "point"),
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
    /// The area is kept: `spots` reads where each row went from it.
    pub fn draw(&self, area: Rect, buf: &mut Buffer) {
        self.drawn.set(area);
        let [head, body, note, keys] = parts(area);
        let (title, at) = match &self.open {
            Some((task, _)) => (format!("Task {task}"), None),
            None => ("Tasks".to_string(), Some(self.at)),
        };
        Line::from(title.bold()).render(head, buf);
        let lines: Vec<Line<'static>> =
            (self.rows().iter().enumerate().skip(self.top(body.height)))
                .map(
                    |(i, r)| match (Some(i) == self.pointing, Some(i) == at && r.task.is_some()) {
                        // the row pointed at underlined, as the web compass outlines it
                        (true, _) => r.line.clone().patch_style(Modifier::UNDERLINED),
                        // a row selected is a task's: with none in the list, nothing is drawn selected (/code-review)
                        (_, true) => r.line.clone().patch_style(Modifier::REVERSED),
                        _ => r.line.clone(),
                    },
                )
                .collect();
        Paragraph::new(lines).render(body, buf);
        let target = match (&self.pointed, self.task()) {
            (Some((anchor, _)), _) => Some(anchor.clone()),
            (None, task) => task,
        };
        let said = match (&self.typing, target, self.pointed_at()) {
            (Some(text), Some(target), _) => Line::from(vec![
                Span::raw(format!("Comment on {target}: ")).bold(),
                Span::raw(format!("{text}▏")),
            ]),
            // the compass names the anchor pointed at
            (None, _, Some(anchor)) => Line::from(format!("Pointing at {anchor}").bold()),
            _ => Line::from(self.note.clone().dim()),
        };
        said.render(note, buf);
        self.keys().render(keys, buf);
    }

    /// The screen drawn into a buffer `width` by `height`, and its anchored rows' cells: /design's TuiScreen of it.
    pub fn screen(&self, width: u16, height: u16) -> (Buffer, Vec<(Rect, String)>) {
        let mut buf = Buffer::empty(Rect::new(0, 0, width, height));
        self.draw(buf.area, &mut buf);
        (buf, self.spots())
    }
}

/// A /tasks tree of two tasks: /design's picture of `dash tui` (the Board tab's Terminal section) and the tests'.
pub fn sample() -> String {
    serde_json::json!({"root": "page", "elements": {
        "page": {"type": "stack", "children": ["title", "open"]},
        "title": {"type": "heading", "props": {"text": "Tasks"}},
        "open": {"type": "issue-group", "props": {"status": "todo"}, "children": ["a", "b"]},
        "a": {"type": "issue-row", "props": {"id": "a", "title": "Port the board", "status": "todo", "says": "", "comments": 0}},
        "b": {"type": "issue-row", "props": {"id": "b", "title": "Draw it in the terminal", "status": "todo", "says": "waits on a", "comments": 0}},
    }})
    .to_string()
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

    fn screen(tui: &Tui) -> Buffer {
        let mut buf = Buffer::empty(Rect::new(0, 0, 60, 8));
        tui.draw(buf.area, &mut buf);
        buf
    }

    /// Done-when 3's surface half: the list drawn from the tree, a task selected and opened, the keys shown at the foot.
    #[dash_test]
    fn the_list_selects_and_opens_a_task_its_keys_at_the_foot() {
        let mut tui = Tui::of(&sample()).expect("a tree");
        let shown = lines(&screen(&tui));
        assert_eq!(shown[0], "Tasks");
        assert!(shown.contains(&"a Port the board".to_string()), "{shown:?}");
        assert_eq!(
            shown[7],
            " ↑↓  select   Enter  open   c  comment   p  point   q  quit"
        );
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
        assert_eq!(
            shown[7],
            " ↑↓  scroll   c  comment   p  point   Esc  back   q  quit"
        );
        assert_eq!(tui.key(Key::Esc), None);
        assert_eq!(lines(&screen(&tui))[0], "Tasks");
        assert_eq!(tui.key(Key::Char('q')), Some(Act::Quit));
    }

    /// Done-when 4's surface half: c, words, Enter asks for the comment on the task selected; Esc drops it.
    #[dash_test]
    fn a_comment_is_typed_then_asked_for_its_task() {
        let mut tui = Tui::of(&sample()).expect("a tree");
        tui.key(Key::Char('c'));
        for c in "Looks right".chars() {
            tui.key(Key::Char(c));
        }
        assert_eq!(lines(&screen(&tui))[6], "Comment on a: Looks right▏");
        assert_eq!(
            tui.key(Key::Enter),
            Some(Act::Comment {
                page: "/tasks/a".into(),
                quote: String::new(),
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

    /// Done-when 1 of rust-tui-compass: the screen keeps each anchored row's cells and its anchor, the id of the tree's
    /// element that drew it (the `id` its HTML carries on the web); pointing mode names the anchor, by keys (p, arrows, Enter)
    /// and by a click on a cell, and the comment box quotes it as the web compass quotes a tap.
    #[dash_test]
    fn pointing_by_keys_or_a_click_names_the_anchor_drawn_in_its_cells() {
        let mut tui = Tui::of(&sample()).expect("a tree");
        let shown = lines(&screen(&tui));
        let spots = tui.spots();
        let at = |anchor: &str| {
            spots
                .iter()
                .find(|(_, a)| a == anchor)
                .map(|(r, _)| *r)
                .unwrap_or_else(|| panic!("{anchor}: {spots:?}"))
        };
        // each row in its cells: the heading, the group, then the tasks, each as wide as its words
        assert_eq!(at("title"), Rect::new(0, 1, 5, 1));
        assert_eq!(
            shown[at("b").y as usize],
            "b Draw it in the terminal  waits on a"
        );
        assert_eq!(
            at("b").width as usize,
            shown[at("b").y as usize].chars().count()
        );
        assert_eq!(
            spots.iter().map(|(_, a)| a.as_str()).collect::<Vec<_>>(),
            ["title", "open", "a", "b"]
        );
        // keys: p points from the row selected, an arrow moves, the anchor named under the page
        assert_eq!(tui.key(Key::Char('p')), None);
        assert_eq!(tui.pointed_at(), Some("a"));
        tui.key(Key::Down);
        assert_eq!(tui.pointed_at(), Some("b"));
        let shown = lines(&screen(&tui));
        assert_eq!(shown[6], "Pointing at b");
        assert_eq!(shown[7], " ↑↓  point   Enter  comment on it   Esc  cancel");
        let buf = screen(&tui);
        assert!(buf[(0, at("b").y)].modifier.contains(Modifier::UNDERLINED));
        tui.key(Key::Enter);
        tui.paste("Too long");
        assert_eq!(lines(&screen(&tui))[6], "Comment on b: Too long▏");
        assert_eq!(
            tui.key(Key::Enter),
            Some(Act::Comment {
                page: "/tasks".into(),
                quote: "b Draw it in the terminal waits on a".into(),
                text: "Too long".into()
            })
        );
        tui.posted("posted");
        // a click: off the compass it names nothing; on, the anchor drawn in that cell
        assert_eq!(tui.click(2, at("title").y), None);
        tui.key(Key::Char('p'));
        assert_eq!(tui.click(40, at("title").y), None, "past its words");
        assert_eq!(tui.click(2, at("title").y), Some("title".into()));
        tui.paste("Rename it");
        assert_eq!(
            tui.key(Key::Enter),
            Some(Act::Comment {
                page: "/tasks".into(),
                quote: "Tasks".into(),
                text: "Rename it".into()
            })
        );
        // Esc closes the box and forgets what it pointed at: c then comments on the task again
        tui.key(Key::Esc);
        tui.key(Key::Char('c'));
        tui.paste("x");
        assert_eq!(lines(&screen(&tui))[6], "Comment on a: x▏");
        tui.key(Key::Esc);
        // a page longer than the screen: the row pointed at far down stays in sight once the box opens (/code-review)
        let mut elements = serde_json::Map::new();
        let names: Vec<String> = (0..9).map(|i| format!("t{i}")).collect();
        elements.insert("p".into(), json!({"type": "stack", "children": names}));
        for n in &names {
            elements.insert(
                n.clone(),
                json!({"type": "text", "props": {"text": format!("Line {n}")}}),
            );
        }
        let long = json!({"root": "p", "elements": elements});
        tui.opened("a", &long.to_string(), "").expect("a tree");
        let _ = screen(&tui);
        tui.key(Key::Char('p'));
        for _ in 0..8 {
            tui.key(Key::Down);
        }
        assert_eq!(tui.pointed_at(), Some("t8"));
        tui.key(Key::Enter);
        assert!(lines(&screen(&tui)).contains(&"Line t8".to_string()));
    }

    /// Done-when 3 of rust-tui-compass: /design's picture of `dash tui` (the Board tab's Terminal section) carries each anchored row's
    /// anchor, its words alone inside, so the web compass's tap there quotes what `dash tui`'s pointing quotes: one thread.
    #[dash_test]
    fn design_s_tui_screen_carries_each_row_s_anchor_for_the_web_compass() {
        /// TuiScreen's HTML: rsx in a module of its own, as dioxus's prelude names a Key of its own
        mod drawn {
            use dioxus::prelude::*;
            use ratatui::{buffer::Buffer, layout::Rect};
            pub fn html(buffer: Buffer, anchors: Vec<(Rect, String)>) -> String {
                dioxus_ssr::render_element(
                    rsx! { crate::components::TuiScreen { id: "t", buffer, anchors } },
                )
            }
        }
        /// The anchored span's whole HTML, its inner spans counted to find its own end.
        fn span_of<'a>(html: &'a str, anchor: &str) -> &'a str {
            let open = format!(r#"<span class="tui-at" data-anchor="{anchor}">"#);
            let at = html.find(&open).unwrap_or_else(|| panic!("{open}: {html}"));
            let (mut depth, mut i) = (0, at);
            loop {
                let (o, c) = (html[i..].find("<span"), html[i..].find("</span>"));
                match (o, c) {
                    (Some(o), Some(c)) if o < c => (depth, i) = (depth + 1, i + o + 5),
                    (_, Some(c)) if depth == 1 => return &html[at..i + c + 7],
                    (_, Some(c)) => (depth, i) = (depth - 1, i + c + 7),
                    _ => panic!("{anchor}: no end"),
                }
            }
        }
        /// What the web compass quotes of a tap on that span: its runs take no pointer, so the span itself is tapped.
        fn tapped(html: &str) -> String {
            quote(&[Node {
                tag: "span".into(),
                text: crate::forms::text_of(html),
                ..Node::default()
            }])
        }
        /// What dash tui quotes, pointed at the row `anchor` drew.
        fn pointed(tui: &mut Tui, anchor: &str) -> String {
            tui.key(Key::Char('p'));
            let spots = tui.spots();
            let (spot, _) = (spots.iter().find(|(_, a)| a == anchor)).expect("its spot");
            assert_eq!(tui.click(spot.x, spot.y), Some(anchor.to_string()));
            tui.paste("same");
            let Some(Act::Comment { quote, .. }) = tui.key(Key::Enter) else {
                panic!("no comment on {anchor}");
            };
            tui.key(Key::Esc);
            quote
        }
        let page = crate::markup(&crate::design::page("board", None, "tab=board"));
        assert!(
            crate::component::css().contains(".tui-at span { pointer-events: none; }"),
            "a tap on a run lands on its anchored span"
        );
        let mut tui = Tui::of(&sample()).expect("a tree");
        let _ = screen(&tui);
        for anchor in ["title", "open", "a", "b"] {
            let html = span_of(&page, anchor);
            assert_eq!(tapped(html), pointed(&mut tui, anchor), "{anchor}: {html}");
        }
        // an element of two rows (a shell's two lines): a span a row, each quoting its own row in both forms
        let two = json!({"root": "p", "elements": {"p": {"type": "terminal", "props": {"text": "$ echo hi\nhi", "action": "/terminal/t/send"}}}});
        tui.opened("b", &two.to_string(), "").expect("a tree");
        let (buffer, anchors) = tui.screen(60, 8);
        let rows: Vec<&Rect> = (anchors.iter())
            .filter(|(_, a)| a == "p")
            .map(|(r, _)| r)
            .collect();
        assert!(rows.len() >= 2, "{:?}", lines(&buffer));
        let html = drawn::html(buffer, anchors.clone());
        let spans = html
            .matches(r#"<span class="tui-at" data-anchor="p">"#)
            .count();
        assert_eq!(spans, rows.len(), "a span a row: {html}");
        for (n, at) in html
            .match_indices(r#"<span class="tui-at" data-anchor="p">"#)
            .map(|(i, _)| i)
            .enumerate()
        {
            let words = tapped(span_of(&html[at..], "p"));
            tui.key(Key::Char('p'));
            assert_eq!(tui.click(rows[n].x, rows[n].y), Some("p".into()));
            tui.paste("same");
            let Some(Act::Comment { quote, .. }) = tui.key(Key::Enter) else {
                panic!("no comment on row {n}");
            };
            tui.key(Key::Esc);
            assert_eq!(words, quote, "row {n}: {html}");
            assert!(n > 0 || words.contains("echo hi"), "{words}");
        }
        assert!(!page.contains("<script"), "no script of ours");
    }
}
