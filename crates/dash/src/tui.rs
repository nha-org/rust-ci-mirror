//! `dash tui`: /tasks in the terminal, drawn by ratatui from the same UI tree the web page draws (rust/CLAUDE.md rule 4, task
//! rust-tui-tracer; @nha, 2026-11-13: « better get this in early to get it right »). The board draws the trees, surface's
//! `tui::Tui` draws them and reads the keys; each act goes through the web's own handler: a task's page as /tasks/<id> draws
//! it, a comment as `POST /compass` lands it.
use crate::tasks::{Board, now_ms};
use dash_surface::tui::{Act, Key, Tui};
use dash_surface::{Drawing, TaskView};
use ratatui::crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};
use ratatui::crossterm::execute;
use std::path::Path;

inventory::submit! { crate::usage::Command::usage("dash tui [--data <dir>] (/tasks in the terminal: select and open a task, comment on it)") }

/// A page's tree, as its Drawing holds it.
fn tree(drawing: Drawing) -> Result<String, String> {
    match drawing {
        Drawing::Tree { tree, .. } => Ok(tree),
        Drawing::Page(_) => Err("a page with no tree has no terminal form yet".into()),
    }
}

/// /tasks' list in the terminal, as `Board::tasks` draws it for the web.
pub fn opened(board: &Board) -> Result<Tui, String> {
    Tui::of(&tree(board.tasks(TaskView::List, None)?)?)
}

/// `task`'s page opened as /tasks/<id> serves it: its tree, then its comment rail's text form; none when it is not open.
fn open(board: &Board, tui: &mut Tui, task: &str) -> Result<(), String> {
    match board.task(task)? {
        Some(page) => {
            let rail = crate::comments::threads_text(&crate::comments::threads_on(&format!(
                "/tasks/{task}"
            )));
            tui.opened(task, &tree(page)?, &rail)
        }
        None => {
            tui.say(format!("No open task « {task} »"));
            Ok(())
        }
    }
}

/// What `asked` does, through the web's handlers; false once he quits.
pub fn act(board: &Board, dir: &Path, tui: &mut Tui, asked: Act) -> Result<bool, String> {
    match asked {
        Act::Quit => return Ok(false),
        Act::Open(task) => open(board, tui, &task)?,
        // the handler `POST /compass` runs, its quote the words pointed at: the web's rail shows the same thread
        Act::Comment { page, quote, text } => {
            let id = crate::comments::post(dir, &page, &quote, &text, &[], now_ms())?;
            tui.posted(format!("Comment {id} posted on {page}"));
            // its page drawn again, as the web's 303 back to it does: the comment shows there (/code-review)
            if let Some(task) = tui.open_task().map(str::to_string)
                && page == format!("/tasks/{task}")
            {
                open(board, tui, &task)?;
            }
        }
    }
    Ok(true)
}

/// A key as `Tui` reads it: a press, never its release; Ctrl-C is none, `run` ends on it.
fn key_of(code: KeyCode) -> Option<Key> {
    Some(match code {
        KeyCode::Up => Key::Up,
        KeyCode::Down => Key::Down,
        KeyCode::Enter => Key::Enter,
        KeyCode::Esc => Key::Esc,
        KeyCode::Backspace => Key::Backspace,
        KeyCode::Char(c) => Key::Char(c),
        _ => return None,
    })
}

/// `dash tui`: the list, then each key until he quits; a failed act is said under the page, never the end of it.
pub fn run(dir: &Path) -> Result<(), String> {
    // the reads through the data folder (a page's comment threads, kept reads) find it, as `dash ui`'s do (/code-review)
    crate::pages::serve_from(dir);
    let board = Board::open(dir)?;
    let mut tui = opened(&board)?;
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
            let Some(asked) = key_of(k.code).and_then(|key| tui.key(key)) else {
                continue;
            };
            match act(&board, dir, &mut tui, asked) {
                Ok(true) => {}
                Ok(false) => return Ok(()),
                Err(e) => tui.say(e),
            }
        }
    });
    ran.map_err(|e| format!("dash tui: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;
    use dash_store as store;
    use dash_surface::tui::lines;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;

    fn screen(tui: &Tui) -> Vec<String> {
        let mut buf = Buffer::empty(Rect::new(0, 0, 80, 24));
        tui.draw(buf.area, &mut buf);
        lines(&buf)
    }

    /// Done-when 3 and 4 of rust-tui-tracer: `dash tui` opens /tasks from the board's own tree, its rows the tasks, the keys at
    /// the foot; Enter opens the task selected, its page the tree /tasks/<id> draws; c, words and Enter post a comment through
    /// `POST /compass`'s handler, which lands it on the task's page as the web's rail lists it.
    #[dash_test]
    fn dash_tui_lists_opens_and_comments_on_a_task_through_the_web_s_handlers() {
        let dir = dash_meter::TestDir::new("tui-board");
        // no older app: the board draws from this store alone
        unsafe {
            std::env::set_var("DASH_CLI", "/nonexistent/dash");
            std::env::set_var("DASH_REPO", &dir.0);
            std::env::set_var("DASH_WORKTREES", &dir.0);
        }
        crate::pages::serve_from(&dir.0);
        let mut tasks = store::tasks::open(&dir.0, &dir.0).expect("the store");
        tasks.add("a-task", &[]).expect("added");
        tasks.add("b-task", &[]).expect("added");
        let board = Board::open(&dir.0).expect("the board");
        let issue = |id: &str, title: &str| dash_bridge::Issue {
            id: id.into(),
            title: title.into(),
            why: "Drawn in the terminal too.".into(),
            checks: vec![],
            comments: vec![],
        };
        board.read_as(vec![
            issue("a-task", "Port the board"),
            issue("b-task", "Draw the list"),
        ]);
        let mut tui = opened(&board).expect("the list");
        let list = screen(&tui);
        assert_eq!(list[0], "Tasks");
        let row = |id: &str| {
            list.iter()
                .position(|l| l.starts_with(id))
                .unwrap_or_else(|| panic!("{id}: {list:#?}"))
        };
        assert!(row("a-task") < row("b-task"), "{list:#?}");
        assert_eq!(
            list[23],
            " ↑↓  select   Enter  open   c  comment   p  point   q  quit"
        );
        // the selected row moves down, then Enter opens its task's page, drawn from the tree /tasks/<id> serves
        assert_eq!(tui.key(Key::Down), None);
        let asked = tui.key(Key::Enter).expect("an act");
        assert_eq!(asked, Act::Open("b-task".into()));
        assert!(act(&board, &dir.0, &mut tui, asked).expect("opened"));
        let page = screen(&tui);
        assert_eq!(page[0], "Task b-task");
        assert!(
            page.iter()
                .any(|l| l.contains("Drawn in the terminal too.")),
            "{page:#?}"
        );
        assert_eq!(
            page[23],
            " ↑↓  scroll   c  comment   p  point   Esc  back   q  quit"
        );
        // his comment, through the web's handler: it lands as the rail's own post does
        tui.key(Key::Char('c'));
        for c in "Looks right".chars() {
            tui.key(Key::Char(c));
        }
        let asked = tui.key(Key::Enter).expect("an act");
        assert!(act(&board, &dir.0, &mut tui, asked).expect("posted"));
        assert_eq!(crate::comments::open_on(&dir.0, "/tasks/b-task"), Ok(1));
        // the page drawn again, as the web's 303 back to it: his comment under it, in its rail's text form
        let mut tall = Buffer::empty(Rect::new(0, 0, 80, 80));
        tui.draw(tall.area, &mut tall);
        let page = lines(&tall);
        assert!(page.iter().any(|l| l.contains("Looks right")), "{page:#?}");
        let said = &screen(&tui)[22];
        assert!(
            said.starts_with("Comment compass-") && said.ends_with(" posted on /tasks/b-task"),
            "{said}"
        );
        assert!(!act(&board, &dir.0, &mut tui, Act::Quit).expect("quit"));
    }

    /// Done-when 2 of rust-tui-compass: a comment pointed at a row of a task's page goes through `POST /compass`'s handler
    /// with the row's words as its quote, and lands in the thread the web page's rail draws, in its HTML and in `dash tui`.
    #[dash_test]
    fn a_comment_pointed_in_the_tui_lands_in_the_web_page_s_thread() {
        let dir = dash_meter::TestDir::new("tui-compass");
        unsafe {
            std::env::set_var("DASH_CLI", "/nonexistent/dash");
            std::env::set_var("DASH_REPO", &dir.0);
            std::env::set_var("DASH_WORKTREES", &dir.0);
        }
        crate::pages::serve_from(&dir.0);
        let mut tasks = store::tasks::open(&dir.0, &dir.0).expect("the store");
        tasks.add("a-task", &[]).expect("added");
        let board = Board::open(&dir.0).expect("the board");
        board.read_as(vec![dash_bridge::Issue {
            id: "a-task".into(),
            title: "Port the board".into(),
            why: "Drawn in the terminal too.".into(),
            checks: vec![],
            comments: vec![],
        }]);
        let mut tui = opened(&board).expect("the list");
        let asked = tui.key(Key::Enter).expect("an act");
        assert!(act(&board, &dir.0, &mut tui, asked).expect("opened"));
        let shown = screen(&tui);
        // the compass on, a click on the row of its why: the anchor drawn in that cell named
        tui.key(Key::Char('p'));
        let (cell, anchor) = (tui.spots().into_iter())
            .find(|(r, _)| shown[r.y as usize].contains("Drawn in the terminal too."))
            .unwrap_or_else(|| panic!("no anchored row says the why: {shown:#?}"));
        assert_eq!(tui.click(cell.x, cell.y), Some(anchor.clone()));
        tui.paste("Say why it matters");
        let asked = tui.key(Key::Enter).expect("an act");
        let Act::Comment { page, quote, .. } = &asked else {
            panic!("{asked:?}")
        };
        assert_eq!(page, "/tasks/a-task");
        assert!(quote.contains("Drawn in the terminal too."), "{quote}");
        let quote = quote.clone();
        assert!(act(&board, &dir.0, &mut tui, asked).expect("posted"));
        // the same thread the web's rail lists for the page, his quote and words in it
        let threads = crate::comments::threads_on("/tasks/a-task");
        assert_eq!(
            threads.len(),
            1,
            "{:?}",
            threads.iter().map(|t| &t.quote).collect::<Vec<_>>()
        );
        assert_eq!(threads[0].quote, quote);
        assert_eq!(threads[0].said[0].words, "Say why it matters");
        // the web page's HTML draws it, as `dressed` serves /tasks/a-task
        let web = dash_surface::forms::with_menu_and("", "/tasks/a-task", "", &threads, None);
        assert!(
            web.contains(&quote) && web.contains("Say why it matters"),
            "{anchor}: {web}"
        );
        // and dash tui's page, drawn again, shows it under the page
        let mut tall = Buffer::empty(Rect::new(0, 0, 80, 80));
        tui.draw(tall.area, &mut tall);
        assert!(
            lines(&tall)
                .iter()
                .any(|l| l.contains("Say why it matters")),
            "{:#?}",
            lines(&tall)
        );
    }
}
