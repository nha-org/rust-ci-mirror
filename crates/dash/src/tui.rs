//! `dash tui`: /tasks in the terminal, drawn by ratatui from the same UI tree the web page draws (rust/CLAUDE.md rule 4, task
//! rust-tui-tracer; @nha, 2026-11-13: « better get this in early to get it right »). The board draws the trees, surface's
//! `tui::Tui` draws them and reads the keys; each act goes through the web's own handler: a task's page as /tasks/<id> draws
//! it, a comment as `POST /compass` lands it.
use crate::tasks::{Board, now_ms};
use dash_surface::tui::{Act, Key, Tui};
use dash_surface::{Drawing, TaskView};
use ratatui::crossterm::event::{
    self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEventKind, KeyModifiers,
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
        Act::Comment { task, text } => {
            let page = format!("/tasks/{task}");
            let id = crate::comments::post(dir, &page, "", &text, &[], now_ms())?;
            tui.posted(format!("Comment {id} posted on {task}"));
            // its page drawn again, as the web's 303 back to it does: the comment shows there (/code-review)
            if tui.open_task() == Some(task.as_str()) {
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
    let ran = ratatui::run(|terminal| -> std::io::Result<()> {
        loop {
            terminal.draw(|f| tui.draw(f.area(), f.buffer_mut()))?;
            let k = match event::read()? {
                Event::Key(k) => k,
                Event::Paste(text) => {
                    tui.paste(&text);
                    continue;
                }
                _ => continue,
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
    let _ = execute!(std::io::stdout(), DisableBracketedPaste);
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
        assert_eq!(list[23], " ↑↓  select   Enter  open   c  comment   q  quit");
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
        assert_eq!(page[23], " ↑↓  scroll   c  comment   Esc  back   q  quit");
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
            said.starts_with("Comment compass-") && said.ends_with(" posted on b-task"),
            "{said}"
        );
        assert!(!act(&board, &dir.0, &mut tui, Act::Quit).expect("quit"));
    }
}
