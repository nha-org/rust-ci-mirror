//! The terminal form of a page, drawn by ratatui from the same UI tree as its HTML and its text (rust/CLAUDE.md rule 4, task
//! rust-tui-tracer; @nha, 2026-11-13: « better get this in early to get it right »). Each element draws through its
//! component's terminal form (crate::component::Terminal): a task's row its Item's, a count its Badge's, a key its Kbd's, a
//! table ratatui's Table, a tab bar its Tabs; an element whose words are the whole of it, its text form. `Tui` is the whole
//! app (task rust-tui-app; @nha, 2026-11-13: « I'd like a command to launch the app in terminal mode too. »): any page
//! through its registered handler, as the server answers it, a page menu reaching each, Enter following a row's link, Esc
//! back. `run` drives it on crossterm; a test draws it into a Buffer.
use super::{Element as Kind, Tree, check};
use crate::components::{
    Node, badge, issue_line, key, list_terminal, quote, table_terminal, tabs_terminal, words,
};
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};
use std::cell::Cell;

/// One line of a page in the terminal; `href`: the page Enter opens from it; `task`: the task a comment from it goes to;
/// `anchor`: the id of the tree's element that drew it, the `id` its HTML carries on the web page (task rust-tui-compass),
/// none for the rail's lines.
struct Row {
    line: Line<'static>,
    href: Option<String>,
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

/// Lines with no link, task or anchor: a page's words, its rail, a page that failed.
fn said(text: &str) -> Vec<Row> {
    (words(text).lines.into_iter())
        .map(|line| Row {
            line,
            href: None,
            task: None,
            anchor: None,
        })
        .collect()
}

fn walk(tree: &Tree, id: &str, out: &mut Vec<Row>) {
    let Some(n) = tree.elements.get(id) else {
        return;
    };
    let anchor = Some(id.to_string());
    let row = |line: Line<'static>, href: Option<String>| Row {
        line,
        href,
        task: None,
        anchor: anchor.clone(),
    };
    let plain = |line: Line<'static>| row(line, None);
    // its words, read off the very HTML the web page draws of it, each line opening `href`
    let worded = |out: &mut Vec<Row>, href: Option<&str>| {
        let html = crate::screen::node_html(tree, id);
        let lines = words(&crate::words::text_of(&html)).lines.into_iter();
        out.extend(lines.map(|l| row(l, href.map(String::from))));
    };
    match &n.element {
        Kind::IssueRow {
            id: task,
            title,
            says,
            project,
            href,
            ..
        } => out.push(Row {
            line: issue_line(task, title, says),
            href: Some(
                (href.clone()).unwrap_or_else(|| crate::task_href(task, project.as_deref())),
            ),
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
        // a list's items through ratatui's List, each keeping its link and anchor
        Kind::List => {
            let from = out.len();
            for child in &n.children {
                walk(tree, child, out);
            }
            let items = out[from..].iter().map(|r| r.line.clone()).collect();
            for (r, line) in out[from..].iter_mut().zip(list_terminal(items, None).lines) {
                r.line = line;
            }
        }
        Kind::Stack | Kind::Cards | Kind::Bar | Kind::Board { .. } => {
            for child in &n.children {
                walk(tree, child, out);
            }
        }
        Kind::Card { href, .. } => worded(out, crate::card_href(href)),
        Kind::ThreadRow { href, .. } => worded(out, href.as_deref()),
        // a page's charts through ratatui: /plan's Gantt its Chart, /deps' treemap its BarChart
        Kind::Gantt { rows } => {
            let drawn = crate::components::gantt_terminal(
                &crate::gantt_rows(rows),
                crate::components::today(),
            );
            out.extend(drawn.lines.into_iter().map(plain));
        }
        Kind::Treemap { boxes } => {
            out.extend(
                crate::components::treemap_terminal(boxes)
                    .lines
                    .into_iter()
                    .map(plain),
            );
        }
        Kind::Table {
            columns,
            rows,
            query,
        } => {
            let per_page = crate::components::PER_PAGE;
            let (text, links) = table_terminal(columns, rows, query.as_deref(), per_page);
            out.extend((text.lines.into_iter().zip(links)).map(|(l, href)| row(l, href)));
        }
        // a tab bar after its label: Enter opens the tab after the one shown, as Tab steps along a web tab bar
        Kind::Links { links, label } if !links.is_empty() => {
            let names: Vec<String> = links.iter().map(|l| l.name.clone()).collect();
            let shown = links.iter().position(|l| l.current);
            let next = &links[shown.map_or(0, |s| (s + 1) % links.len())];
            for tabs in tabs_terminal(&names, shown).lines {
                let label = label.as_ref().map(|l| format!("{l}:")).unwrap_or_default();
                let mut line = Line::from(label.bold());
                line.spans.extend(tabs.spans);
                out.push(row(line, Some(next.href.clone())));
            }
        }
        Kind::TaskViews { current, project } => {
            let all = crate::TaskView::ALL;
            let names: Vec<String> = all.iter().map(|v| v.name().to_string()).collect();
            let shown = all.iter().position(|v| v == current).unwrap_or(0);
            let next = all[(shown + 1) % all.len()].href(project.as_deref());
            out.extend(
                tabs_terminal(&names, Some(shown))
                    .lines
                    .into_iter()
                    .map(|l| row(l, Some(next.clone()))),
            );
        }
        // the rest: its words, as its text form
        _ => worded(out, None),
    }
}

/// A key `dash tui` reads, as crossterm names it; `Menu`: Ctrl-K, the web's ⌘ K.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Key {
    Up,
    Down,
    Enter,
    Esc,
    Backspace,
    Menu,
    Char(char),
}

/// What a key asks `dash tui` to do: draw a page (a link followed, the menu's pick, back), post a comment on a page through
/// the web's handler (its quote the words pointed at, "" the whole page, as the web compass sends it), or end.
#[derive(Debug, PartialEq)]
pub enum Act {
    Go(String),
    Comment {
        page: String,
        quote: String,
        text: String,
    },
    Quit,
}

/// The page shown: the URL it was asked at, its title, its rows, whether a part of it is still reading (a skeleton, a 202).
#[derive(Default)]
struct Shown {
    target: String,
    title: String,
    rows: Vec<Row>,
    reading: bool,
}

/// The app in the terminal: the page shown, the pages before it, the row the cursor is on, the page menu, a comment typed.
#[derive(Default)]
pub struct Tui {
    page: Shown,
    /// The pages Esc goes back to, each its URL and the row its cursor was on.
    back: Vec<(String, usize)>,
    /// Going back: the cursor the page had.
    returning: Option<usize>,
    /// The cursor's row.
    at: usize,
    /// The page menu open: its entries, read once as it opens, and the one picked.
    menu: Option<(Vec<(String, String)>, usize)>,
    typing: Option<String>,
    note: String,
    /// The compass on: the row pointed at, an anchored one of the rows shown.
    pointing: Option<usize>,
    /// What the box comments on once pointed: its anchor and its quote.
    pointed: Option<(String, String)>,
    /// The area the last draw filled, and its first row in sight: where each row's cells are, for a click.
    drawn: Cell<Rect>,
    top: Cell<usize>,
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

/// The page menu: the web menu's links (`forms::menu_links`), those a Rust page serves, then every other page registered
/// with a path of its own, so the menu reaches each Rust page; each its name, a sub-entry indented, and its URL.
pub fn menu() -> Vec<(String, String)> {
    let path = |at: &str| at.split('?').next().unwrap_or_default().to_string();
    let rust = |at: &str| dash_core::page(&path(at)).is_some();
    let mut menu: Vec<(String, String)> = (crate::forms::menu_links().into_iter())
        .filter(|(_, _, at)| rust(at))
        .map(|(sub, name, at)| (format!("{}{name}", if sub { "  " } else { "" }), at))
        .collect();
    let mut more: Vec<&str> = (inventory::iter::<dash_core::Page>.into_iter())
        .map(|p| p.path)
        .filter(|p| !p.contains(['*', '<']) && rust(p))
        .filter(|p| !menu.iter().any(|(_, at)| path(at) == *p))
        .collect();
    more.sort_unstable();
    menu.extend(more.into_iter().map(|p| (p.to_string(), p.to_string())));
    menu
}

impl Tui {
    /// `target` as the server answers it, through its registered page (`board` handed to a page that reads it, else
    /// `registry`, as the server hands them), then `rail`, the text form of its comment rail, which the web draws beside the page, never in it. A
    /// page that fails, or that no Rust page serves, is drawn as its words: the app never ends on one.
    pub fn go(
        &mut self,
        host: &dyn dash_core::Host,
        (board, registry): (&dyn std::any::Any, &dyn std::any::Any),
        target: &str,
        rail: &str,
    ) {
        let (path, query) = target.split_once('?').unwrap_or((target, ""));
        let Some(page) = dash_core::page(path) else {
            let why = format!("No Rust page at {path}: the browser opens it");
            return self.shown(target, path, said(&why), rail);
        };
        // his form, as the window's: a page reading still draws its skeleton, where an agent's text waits for the read
        let asked = page.asked((path, query, true), (Some(board), Some(registry)), host);
        let (title, rows, reading) = match (page.serve)(&asked) {
            dash_core::Answer::Drawn(d) => match crate::queried(d, query) {
                crate::Drawing::Tree { title, tree } => {
                    let skeleton = |t: &Tree| {
                        (t.elements.values()).any(|n| matches!(n.element, Kind::Skeleton { .. }))
                    };
                    let reading = check(&tree).is_ok_and(|t| skeleton(&t));
                    (title, rows(&tree).unwrap_or_else(|e| said(&e)), reading)
                }
                crate::Drawing::Page(html) => {
                    (path.to_string(), said(&crate::forms::text_of(&html)), false)
                }
            },
            dash_core::Answer::Body(status, kind, body) if kind == dash_core::HTML => (
                status.to_string(),
                said(&crate::forms::text_of(&body)),
                false,
            ),
            dash_core::Answer::Body(status, _, body) => {
                (status.to_string(), said(&body), status.starts_with("202"))
            }
        };
        self.shown(target, &title, rows, rail);
        self.page.reading = reading;
    }

    /// A part of the page shown still reading: `run` draws it again a second later, as the window refills a skeleton.
    pub fn reading(&self) -> bool {
        self.page.reading
    }

    /// /tasks' list from its tree: /design's picture of `dash tui`, and the tests'.
    pub fn of(tree: &str) -> Result<Tui, String> {
        let mut tui = Tui::default();
        tui.tree("/tasks", "Tasks", tree, "")?;
        Ok(tui)
    }

    /// A page's tree shown as `target`, titled `title`: what `go` draws of a tree page, and the tests' and /design's own.
    pub fn tree(
        &mut self,
        target: &str,
        title: &str,
        tree: &str,
        rail: &str,
    ) -> Result<(), String> {
        let rows = rows(tree)?;
        self.shown(target, title, rows, rail);
        Ok(())
    }

    fn shown(&mut self, target: &str, title: &str, mut rows: Vec<Row>, rail: &str) {
        rows.extend(said(rail));
        // the same page again (a comment posted) keeps its cursor; another one starts at its first task, as /tasks' list
        // selects one, else its first link, else its top
        let at = match self.returning.take() {
            Some(at) => at,
            None if target == self.page.target => self.at,
            None => {
                if !self.page.target.is_empty() {
                    self.back.push((self.page.target.clone(), self.at));
                }
                let task = rows.iter().position(|r| r.task.is_some());
                task.or_else(|| rows.iter().position(|r| r.href.is_some()))
                    .unwrap_or(0)
            }
        };
        self.at = at.min(rows.len().saturating_sub(1));
        self.page = Shown {
            target: target.to_string(),
            title: title.to_string(),
            rows,
            reading: false,
        };
        self.top.set(0);
        self.pointing = None;
    }

    /// The URL of the page shown.
    pub fn target(&self) -> &str {
        &self.page.target
    }

    /// The page shown, as a comment on it is filed: its path.
    pub fn path(&self) -> &str {
        self.page.target.split('?').next().unwrap_or_default()
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

    /// The task a comment goes to: the one on the cursor's row, as /tasks' list comments on the task selected.
    fn task(&self) -> Option<&str> {
        self.page.rows.get(self.at)?.task.as_deref()
    }

    /// What c comments on: the task on the cursor's row, else the page shown.
    fn commented(&self) -> String {
        self.task()
            .map_or_else(|| self.path().to_string(), |t| crate::task_href(t, None))
    }

    /// The first row in sight in a page `height` rows tall: the last draw's, moved as little as keeps the row pointed at,
    /// else the cursor's, in sight.
    fn top(&self, height: u16) -> usize {
        let last = height.saturating_sub(1) as usize;
        let at = self.pointing.unwrap_or(self.at);
        self.top.get().min(at).max(at.saturating_sub(last))
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
        let text = self.page.rows[i]
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
            .page
            .rows
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
        self.page.rows.get(self.pointing?)?.anchor.as_deref()
    }

    /// The compass's tap in the terminal: the anchored row pointed at gets the comment box, quoting it as the web does.
    fn point(&mut self, i: usize) {
        let Some(anchor) = self.page.rows[i].anchor.clone() else {
            return;
        };
        self.pointed = Some((anchor, self.quote_at(i)));
        // the cursor where pointing took it, so the row stays in sight once the box opens (/code-review)
        self.at = i;
        self.pointing = None;
        self.typing = Some(String::new());
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
        let rows = &self.page.rows;
        let anchored = |i: &usize| rows[*i].anchor.is_some();
        let next = match down {
            true => (at + 1..rows.len()).find(anchored),
            false => (0..at).rev().find(anchored),
        };
        self.pointing = Some(next.unwrap_or(at));
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
                        Some((_, quote)) => (self.path().to_string(), quote.clone()),
                        None => (self.commented(), String::new()),
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
                Key::Esc => self.pointing = None,
                _ => {}
            }
            return None;
        }
        if let Some((entries, picked)) = &mut self.menu {
            let last = entries.len().saturating_sub(1);
            match key {
                Key::Up => *picked = picked.saturating_sub(1),
                Key::Down => *picked = (*picked + 1).min(last),
                Key::Enter => {
                    let at = entries.get(*picked).map(|(_, at)| Act::Go(at.clone()));
                    self.menu = None;
                    return at;
                }
                Key::Esc | Key::Menu | Key::Char('m') => self.menu = None,
                _ => {}
            }
            return None;
        }
        let rows = self.page.rows.len();
        match key {
            // Esc goes back, never out: q alone quits, as the keys say (/code-review)
            Key::Char('q') => return Some(Act::Quit),
            Key::Char('m') | Key::Menu => {
                let entries = menu();
                // on the page shown: its URL, else its path's entry (/tasks?layout=list is Tasks'; seen in a real terminal)
                let here = (entries.iter().position(|(_, at)| at == self.target()))
                    .or_else(|| entries.iter().position(|(_, at)| at == self.path()));
                self.menu = Some((entries, here.unwrap_or(0)));
            }
            Key::Char('c') => self.typing = Some(String::new()),
            // the compass: from the cursor's row to the first anchored one
            Key::Char('p') => {
                let anchored = |i: &usize| self.page.rows[*i].anchor.is_some();
                self.pointing = (self.at..rows)
                    .find(anchored)
                    .or_else(|| (0..self.at).rev().find(anchored));
                if self.pointing.is_none() {
                    self.say("Nothing here to point at");
                }
            }
            Key::Esc => {
                let (target, at) = self.back.pop()?;
                self.returning = Some(at);
                return Some(Act::Go(target));
            }
            Key::Up => self.at = self.at.saturating_sub(1),
            Key::Down => self.at = (self.at + 1).min(rows.saturating_sub(1)),
            // a link of a query alone (a table's next page) is the page shown's
            Key::Enter => {
                let href = self.page.rows.get(self.at)?.href.clone()?;
                let href = match href.starts_with('?') {
                    true => format!("{}{href}", self.path()),
                    false => href,
                };
                return Some(Act::Go(href));
            }
            _ => {}
        }
        None
    }

    /// The keys this state takes, each its Kbd's terminal form then what it does.
    fn keys(&self) -> Line<'static> {
        let back = !self.back.is_empty();
        let pairs: Vec<(&str, &str)> = match &self.typing {
            Some(_) => vec![("Enter", "send"), ("Esc", "cancel")],
            None if self.pointing.is_some() => vec![
                ("↑↓", "point"),
                ("Enter", "comment on it"),
                ("Esc", "cancel"),
            ],
            None if self.menu.is_some() => {
                vec![("↑↓", "pick"), ("Enter", "open"), ("Esc", "close")]
            }
            None => {
                let mut keys = vec![
                    ("↑↓", "move"),
                    ("Enter", "open"),
                    ("m", "menu"),
                    ("c", "comment"),
                    ("p", "point"),
                ];
                keys.extend(back.then_some(("Esc", "back")));
                keys.push(("q", "quit"));
                keys
            }
        };
        let mut spans = vec![];
        for (k, does) in pairs {
            spans.extend([key(k), format!(" {does}  ").into()]);
        }
        Line::from(spans)
    }

    /// The whole screen into `buf`'s `area`: its title, the page (or the page menu over it), the line typed or the last note,
    /// then the keys. The area is kept: `spots` reads where each row went from it.
    pub fn draw(&self, area: Rect, buf: &mut Buffer) {
        self.drawn.set(area);
        let [head, body, note, keys] = parts(area);
        let title = Style::new().fg(crate::theme::tone("primary")).bold();
        Line::from(Span::styled(self.page.title.clone(), title)).render(head, buf);
        let lines: Vec<Line<'static>> = match &self.menu {
            Some((entries, picked)) => {
                let picked = *picked;
                let entries = (entries.iter())
                    .map(|(name, _)| Line::from(name.clone()))
                    .collect();
                let lines = list_terminal(entries, Some(picked)).lines;
                // the entry picked kept in sight
                lines
                    .into_iter()
                    .skip((picked + 1).saturating_sub(body.height as usize))
                    .collect()
            }
            None => {
                let top = self.top(body.height);
                self.top.set(top);
                (self.page.rows.iter().enumerate().skip(top))
                    .map(|(i, r)| match (Some(i) == self.pointing, i == self.at) {
                        // the row pointed at underlined, as the web compass outlines it
                        (true, _) => r.line.clone().patch_style(Modifier::UNDERLINED),
                        (false, true) if self.pointing.is_none() => {
                            r.line.clone().patch_style(Modifier::REVERSED)
                        }
                        _ => r.line.clone(),
                    })
                    .collect()
            }
        };
        Paragraph::new(lines).render(body, buf);
        let target = match &self.pointed {
            Some((anchor, _)) => anchor.clone(),
            None => self
                .task()
                .map_or_else(|| self.path().to_string(), String::from),
        };
        let said = match (&self.typing, self.pointed_at()) {
            (Some(text), _) => Line::from(vec![
                Span::raw(format!("Comment on {target}: ")).bold(),
                Span::raw(format!("{text}▏")),
            ]),
            // the compass names the anchor pointed at
            (None, Some(anchor)) => Line::from(format!("Pointing at {anchor}").bold()),
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

/// Whether `dash tui` has a terminal to draw in: its input and its output each one. Without, it says so, never ratatui's
/// panic (task rust-tui-app).
pub fn terminal() -> Result<(), String> {
    use std::io::IsTerminal;
    match std::io::stdin().is_terminal() && std::io::stdout().is_terminal() {
        true => Ok(()),
        false => {
            Err("dash tui: no terminal here: run it in one, its input and output a terminal".into())
        }
    }
}

/// A key as `Tui` reads it: a press, never its release; Ctrl-K the menu, as the web's ⌘ K; Ctrl-C is none, `run` ends on it.
fn key_of(k: ratatui::crossterm::event::KeyEvent) -> Option<Key> {
    use ratatui::crossterm::event::{KeyCode, KeyModifiers};
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    Some(match k.code {
        KeyCode::Char('k') if ctrl => Key::Menu,
        _ if ctrl => return None,
        KeyCode::Up => Key::Up,
        KeyCode::Down => Key::Down,
        KeyCode::Enter => Key::Enter,
        KeyCode::Esc => Key::Esc,
        KeyCode::Backspace => Key::Backspace,
        KeyCode::Char(c) => Key::Char(c),
        _ => return None,
    })
}

/// `tui` in this terminal, each key until he quits: `act` does what a key asks (the binary's: a page drawn, a comment
/// posted), true to go on; a failed act is said under the page, never the end of it.
pub fn run(
    mut tui: Tui,
    mut act: impl FnMut(&mut Tui, Act) -> Result<bool, String>,
) -> Result<(), String> {
    use ratatui::crossterm::event::{
        self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
    };
    use ratatui::crossterm::execute;
    // ratatui's init panics with no terminal: said first
    terminal()?;
    // a paste arrives whole, never as keys: its line break no Enter, its q no quit (/code-review)
    execute!(std::io::stdout(), EnableBracketedPaste).map_err(|e| e.to_string())?;
    // the terminal's modes given back on any way out, a panic's unwinding included (/code-review)
    struct Modes;
    impl Drop for Modes {
        fn drop(&mut self) {
            let _ = execute!(
                std::io::stdout(),
                DisableBracketedPaste,
                DisableMouseCapture
            );
        }
    }
    let _modes = Modes;
    let ran = ratatui::run(|terminal| -> std::io::Result<()> {
        let mut captured = false;
        let mut redraw = true;
        loop {
            // the mouse captured while the compass is on alone, a click being its tap (task rust-tui-compass): else his
            // terminal selects and copies as ever, and no move reaches us (/code-review)
            if tui.pointing() != captured {
                captured = tui.pointing();
                match captured {
                    true => execute!(std::io::stdout(), EnableMouseCapture)?,
                    false => execute!(std::io::stdout(), DisableMouseCapture)?,
                }
            }
            if redraw {
                terminal.draw(|f| tui.draw(f.area(), f.buffer_mut()))?;
            }
            redraw = true;
            // a page still reading drawn again a second later, as the window refills a skeleton (/code-review)
            // ponytail: for as long as it reads; the window's five-minute cap when a read never ends bites
            if tui.reading() && !event::poll(std::time::Duration::from_secs(1))? {
                let again = Act::Go(tui.target().to_string());
                if let Err(e) = act(&mut tui, again) {
                    tui.say(e);
                }
                continue;
            }
            let k = match event::read()? {
                Event::Key(k) => k,
                Event::Paste(text) => {
                    tui.paste(&text);
                    continue;
                }
                Event::Mouse(m) if m.kind == MouseEventKind::Down(MouseButton::Left) => {
                    tui.click(m.column, m.row);
                    continue;
                }
                Event::Resize(..) => continue,
                // a move, a focus change: nothing to draw again
                _ => {
                    redraw = false;
                    continue;
                }
            };
            if k.kind != KeyEventKind::Press {
                continue;
            }
            if k.modifiers.contains(KeyModifiers::CONTROL) && k.code == KeyCode::Char('c') {
                return Ok(());
            }
            let Some(asked) = key_of(k).and_then(|key| tui.key(key)) else {
                continue;
            };
            match act(&mut tui, asked) {
                Ok(true) => {}
                Ok(false) => return Ok(()),
                Err(e) => tui.say(e),
            }
        }
    });
    ran.map_err(|e| format!("dash tui: {e}"))
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
        let mut buf = Buffer::empty(Rect::new(0, 0, 90, 8));
        tui.draw(buf.area, &mut buf);
        buf
    }

    /// Done-when 3's surface half (rust-tui-tracer), and rust-tui-app's page model: the list drawn from the tree, the cursor on a
    /// task, Enter following its link, Esc back to the list with the cursor where it was, the keys shown at the foot.
    #[dash_test]
    fn the_list_selects_and_opens_a_task_its_keys_at_the_foot() {
        let mut tui = Tui::of(&sample()).expect("a tree");
        let shown = lines(&screen(&tui));
        assert_eq!(shown[0], "Tasks");
        assert!(shown.contains(&"a Port the board".to_string()), "{shown:?}");
        assert_eq!(
            shown[7],
            " ↑↓  move   Enter  open   m  menu   c  comment   p  point   q  quit"
        );
        // the cursor is drawn reversed, on the first task's row
        let buf = screen(&tui);
        let row = (0..8)
            .find(|y| lines(&buf)[*y as usize].starts_with("a Port"))
            .expect("a's row");
        assert!(buf[(0, row)].modifier.contains(Modifier::REVERSED));
        assert_eq!(tui.key(Key::Down), None);
        assert_eq!(tui.key(Key::Enter), Some(Act::Go("/tasks/b".into())));
        let page = json!({"root": "p", "elements": {"p": {"type": "text", "props": {"text": "Its brief"}}}});
        tui.tree("/tasks/b", "Task b", &page.to_string(), "")
            .expect("a tree");
        let shown = lines(&screen(&tui));
        assert_eq!(shown[0], "Task b");
        assert_eq!(shown[1], "Its brief");
        assert_eq!(
            shown[7],
            " ↑↓  move   Enter  open   m  menu   c  comment   p  point   Esc  back   q  quit"
        );
        // no link on its row: Enter stays
        assert_eq!(tui.key(Key::Enter), None);
        // Esc asks for the list again, drawn with its cursor back on b
        assert_eq!(tui.key(Key::Esc), Some(Act::Go("/tasks".into())));
        tui.tree("/tasks", "Tasks", &sample(), "").expect("a tree");
        let buf = screen(&tui);
        assert_eq!(lines(&buf)[0], "Tasks");
        let b = (0..8)
            .find(|y| lines(&buf)[*y as usize].starts_with("b Draw"))
            .expect("b's row");
        assert!(buf[(0, b)].modifier.contains(Modifier::REVERSED));
        assert_eq!(
            tui.key(Key::Esc),
            None,
            "nothing before the list: Esc never quits"
        );
        assert_eq!(tui.key(Key::Char('q')), Some(Act::Quit));
    }

    /// Task rust-tui-app's charts on a page: a page's tree holding a Gantt (/plan's) and a treemap (/deps') draws them in
    /// `dash tui` through ratatui, read off the buffer: the Gantt its Chart, a bar a row in its mark's theme colour, its names
    /// on the y axis, today in the accent colour; the treemap its BarChart laid flat, largest box first, its size on it.
    #[dash_test]
    fn a_page_s_gantt_and_treemap_draw_ratatui_s_chart_and_bars() {
        let today = crate::components::today();
        let day = |n: i64| dash_core::date(today + n);
        let tree = json!({"root": "p", "elements": {
            "p": {"type": "stack", "children": ["g", "t"]},
            "g": {"type": "gantt", "props": {"rows": [
                {"id": "a", "name": "Port the board", "start": day(-10), "target": day(-2), "mark": "done"},
                {"id": "b", "name": "Linear sync", "start": day(-6), "target": day(4), "mark": "failed"},
                {"id": "c", "name": "Plan page", "start": day(2), "target": day(12), "mark": "next"},
            ]}},
            "t": {"type": "treemap", "props": {"boxes": [
                {"name": "Core", "children": [{"name": "core", "size": 700}]},
                {"name": "Rings", "children": [{"name": "store", "size": 900}, {"name": "surface", "size": 1400}]},
            ]}},
        }});
        let mut tui = Tui::default();
        tui.tree("/plan", "Plan", &tree.to_string(), "")
            .expect("a tree");
        let mut buf = Buffer::empty(Rect::new(0, 0, 90, 30));
        tui.draw(buf.area, &mut buf);
        let shown = lines(&buf);
        let tone = crate::theme::tone;
        let cells_in = |t: &str| {
            buf.content
                .iter()
                .filter(|c| c.fg == tone(t) && !c.symbol().trim().is_empty())
                .count()
        };
        // the Gantt: a row a bar in its mark's colour, its name on the axis, today's column
        for (name, look) in [
            ("Port the board", "primary"),
            ("Linear sync", "error"),
            ("Plan page", "info"),
        ] {
            assert!(shown.iter().any(|l| l.contains(name)), "{name}: {shown:#?}");
            assert!(cells_in(look) > 3, "{look}: {shown:#?}");
        }
        assert!(cells_in("accent") >= 2, "today: {shown:#?}");
        let top = shown
            .iter()
            .position(|l| l.contains("Port the board"))
            .expect("a");
        let low = shown
            .iter()
            .position(|l| l.contains("Plan page"))
            .expect("c");
        assert!(top < low, "rows top to bottom in their order: {shown:#?}");
        // the treemap: Rings (2,300) before Core (700), each its size on its bar
        let rings = shown
            .iter()
            .position(|l| l.contains("Rings") && l.contains("2,300"))
            .unwrap_or_else(|| panic!("{shown:#?}"));
        let core = shown
            .iter()
            .position(|l| l.contains("Core") && l.contains("700"))
            .unwrap_or_else(|| panic!("{shown:#?}"));
        assert!(rings < core && core > low, "{shown:#?}");
        assert!(shown[rings].contains('█'), "a bar: {}", shown[rings]);
    }

    /// Done-when 1's surface half (rust-tui-app): m or Ctrl-K (the web's ⌘ K) opens the page menu, the web menu's links that a
    /// Rust page serves (here /design, the one this crate registers), drawn by ratatui's List, the entry picked reversed under
    /// the theme's arrow; Esc closes it, Enter asks for the page picked.
    #[dash_test]
    fn the_page_menu_lists_the_rust_pages_and_opens_the_one_picked() {
        let menu = menu();
        let design = (menu.iter())
            .position(|(name, at)| name == "Design" && at == "/design")
            .unwrap_or_else(|| panic!("{menu:?}"));
        let rust = |at: &str| dash_core::page(at.split('?').next().unwrap_or_default()).is_some();
        assert!(menu.iter().all(|(_, at)| rust(at)), "{menu:?}");
        let mut tui = Tui::of(&sample()).expect("a tree");
        assert_eq!(tui.key(Key::Menu), None);
        for _ in 0..design {
            tui.key(Key::Down);
        }
        let buf = screen(&tui);
        let shown = lines(&buf);
        assert_eq!(shown[7], " ↑↓  pick   Enter  open   Esc  close");
        let row = (1..7)
            .find(|y| shown[*y as usize] == "▸ Design")
            .unwrap_or_else(|| panic!("{shown:#?}"));
        assert!(
            buf[(2, row)].modifier.contains(Modifier::REVERSED),
            "picked: {shown:#?}"
        );
        assert_eq!(buf[(0, row)].fg, crate::theme::tone("primary"));
        assert_eq!(tui.key(Key::Esc), None);
        assert_eq!(lines(&screen(&tui))[1], "Tasks", "closed, the page again");
        tui.key(Key::Char('m'));
        for _ in 0..design {
            tui.key(Key::Down);
        }
        assert_eq!(tui.key(Key::Enter), Some(Act::Go("/design".into())));
        assert_eq!(
            lines(&screen(&tui))[1],
            "Tasks",
            "the menu closes on its pick"
        );
        // it opens on the page shown, a query aside: /design?tab=charts is Design's entry
        tui.tree("/design?tab=charts", "Design", &sample(), "")
            .expect("a tree");
        tui.key(Key::Menu);
        let shown = lines(&screen(&tui));
        assert!(shown.iter().any(|l| l == "▸ Design"), "{shown:#?}");
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
        assert_eq!(tui.key(Key::Enter), Some(Act::Go("/tasks/a".into())));
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
        // Esc closes the box and forgets what it pointed at: c then comments on the cursor's row, which pointing moved to the
        // title, so on the page
        tui.key(Key::Esc);
        tui.key(Key::Char('c'));
        tui.paste("x");
        assert_eq!(lines(&screen(&tui))[6], "Comment on /tasks: x▏");
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
        tui.tree("/tasks/a", "Task a", &long.to_string(), "")
            .expect("a tree");
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
        tui.tree("/tasks/b", "Task b", &two.to_string(), "")
            .expect("a tree");
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
