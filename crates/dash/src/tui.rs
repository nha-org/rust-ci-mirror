//! `dash tui`: the app in the terminal (task rust-tui-app; @nha, 2026-11-13: « I'd like a command to launch the app in terminal
//! mode too. »), drawn by ratatui from the same UI tree each page draws for the web (rust/CLAUDE.md rule 4). Surface's
//! `tui::Tui` draws every page through its registered handler and reads the keys; the binary hands it the board, each page's
//! comment rail, and posts a comment through `POST /compass`'s handler.
use crate::tasks::{Board, now_ms};
use dash_core::registry::Registry;
use dash_surface::tui::{Act, Tui};

dash_core::submit! { crate::usage::Command::usage("dash tui [--data <dir>] (the app in the terminal: its page menu, any page, comments on it)") }

/// Where `dash tui` opens: /tasks' list.
const HOME: &str = "/tasks?layout=list";

/// What `dash tui` hands its pages, as the server does: the board to a page that reads it, else the plugins' registry.
pub type Held = (Board, Registry);

/// `target` drawn as the server answers it, its comment rail under it.
fn go(host: &dyn dash_core::Host, (board, registry): &Held, tui: &mut Tui, target: &str) {
    let path = target.split('?').next().unwrap_or_default();
    let rail = crate::comments::threads_text(&crate::comments::threads_on(&host.data(), path));
    tui.go(host, (board, registry), target, &rail);
}

/// What `asked` does, through the web's handlers; false once he quits.
pub fn act(
    host: &dyn dash_core::Host,
    board: &Held,
    tui: &mut Tui,
    asked: Act,
) -> Result<bool, String> {
    let dir = &host.data();
    match asked {
        Act::Quit => return Ok(false),
        Act::Go(target) => go(host, board, tui, &target),
        // the handler `POST /compass` runs, its quote the words pointed at: the web's rail shows the same thread
        Act::Comment { page, quote, text } => {
            let id = crate::comments::post(dir, &page, &quote, &text, &[], now_ms())?;
            tui.posted(format!("Comment {id} posted on {page}"));
            // its page drawn again, as the web's 303 back to it does: the comment shows there (/code-review)
            if tui.path() == page {
                let target = tui.target().to_string();
                go(host, board, tui, &target);
            }
        }
    }
    Ok(true)
}

/// `dash tui`: /tasks' list, then each key until he quits; `registry` builds the plugins' registry, as `dash serve`'s, each
/// refusal said before the screen is taken. With no terminal it says so before the store opens.
pub fn run(
    host: &'static crate::pages::Binary,
    registry: fn(&'static crate::pages::Binary) -> Registry,
) -> Result<(), String> {
    dash_surface::tui::terminal()?;
    let board = (Board::open(&host.folder())?, registry(host));
    let mut tui = Tui::default();
    go(host, &board, &mut tui, HOME);
    dash_surface::tui::run(tui, |tui, asked| act(host, &board, tui, asked))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;
    use dash_store as store;
    use dash_surface::tui::{Key, lines};
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;

    fn screen(tui: &Tui) -> Vec<String> {
        let mut buf = Buffer::empty(Rect::new(0, 0, 80, 24));
        tui.draw(buf.area, &mut buf);
        lines(&buf)
    }

    /// Done-when 3 and 4 of rust-tui-tracer: `dash tui` opens /tasks from the board's own tree, its rows the tasks, the keys at
    /// the foot; Enter opens the task selected, its page the tree /tasks/<id> draws through its registered page; c, words and
    /// Enter post a comment through `POST /compass`'s handler, which lands it on the task's page as the web's rail lists it.
    #[dash_test]
    fn dash_tui_lists_opens_and_comments_on_a_task_through_the_web_s_handlers() {
        let dir = dash_meter::TestDir::new("tui-board");
        let host = crate::pages::Binary::served(&dir.0);
        // no older app: the board draws from this store alone
        unsafe {
            std::env::set_var("DASH_CLI", "/nonexistent/dash");
            std::env::set_var("DASH_REPO", &dir.0);
            std::env::set_var("DASH_WORKTREES", &dir.0);
        }
        let mut tasks = store::tasks::open(&dir.0, &dir.0).expect("the store");
        tasks.add("a-task", &[]).expect("added");
        tasks.add("b-task", &[]).expect("added");
        let board = (
            Board::open(&dir.0).expect("the board"),
            Registry::with_compiled().expect("the registry"),
        );
        let issue = |id: &str, title: &str| dash_bridge::Issue {
            id: id.into(),
            title: title.into(),
            why: "Drawn in the terminal too.".into(),
            checks: vec![],
            comments: vec![],
        };
        board.0.read_as(vec![
            issue("a-task", "Port the board"),
            issue("b-task", "Draw the list"),
        ]);
        let mut tui = Tui::default();
        go(host, &board, &mut tui, HOME);
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
            " ↑↓  move   Enter  open   m  menu   c  comment   p  point   q  quit"
        );
        // the selected row moves down, then Enter opens its task's page, drawn from the tree /tasks/<id> serves
        assert_eq!(tui.key(Key::Down), None);
        let asked = tui.key(Key::Enter).expect("an act");
        assert_eq!(asked, Act::Go("/tasks/b-task".into()));
        assert!(act(host, &board, &mut tui, asked).expect("opened"));
        let page = screen(&tui);
        assert_eq!(page[0], "Rust task: b-task");
        assert!(
            page.iter()
                .any(|l| l.contains("Drawn in the terminal too.")),
            "{page:#?}"
        );
        assert_eq!(
            page[23],
            " ↑↓  move   Enter  open   m  menu   c  comment   p  point   Esc  back   q  quit"
        );
        // his comment, through the web's handler: it lands as the rail's own post does
        tui.key(Key::Char('c'));
        for c in "Looks right".chars() {
            tui.key(Key::Char(c));
        }
        let asked = tui.key(Key::Enter).expect("an act");
        assert!(act(host, &board, &mut tui, asked).expect("posted"));
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
        assert!(!act(host, &board, &mut tui, Act::Quit).expect("quit"));
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
        let host = crate::pages::Binary::served(&dir.0);
        let mut tasks = store::tasks::open(&dir.0, &dir.0).expect("the store");
        tasks.add("a-task", &[]).expect("added");
        let board = (
            Board::open(&dir.0).expect("the board"),
            Registry::with_compiled().expect("the registry"),
        );
        board.0.read_as(vec![dash_bridge::Issue {
            id: "a-task".into(),
            title: "Port the board".into(),
            why: "Drawn in the terminal too.".into(),
            checks: vec![],
            comments: vec![],
        }]);
        let mut tui = Tui::default();
        go(host, &board, &mut tui, HOME);
        let asked = tui.key(Key::Enter).expect("an act");
        assert!(act(host, &board, &mut tui, asked).expect("opened"));
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
        assert!(act(host, &board, &mut tui, asked).expect("posted"));
        // the same thread the web's rail lists for the page, his quote and words in it
        let threads = crate::comments::threads_on(&dir.0, "/tasks/a-task");
        assert_eq!(
            threads.len(),
            1,
            "{:?}",
            threads.iter().map(|t| &t.quote).collect::<Vec<_>>()
        );
        assert_eq!(threads[0].quote, quote);
        assert_eq!(threads[0].said[0].words, "Say why it matters");
        // the web page's HTML draws it, as `dressed` serves /tasks/a-task
        let web = dash_surface::forms::with_menu_and(host, "", "/tasks/a-task", "", &threads, None);
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

    /// Done-when 1 of rust-tui-app: `dash tui`'s page menu, m and the arrows (its key handler, Rust's, as the web menu's), reaches
    /// every Rust page: each link of the web's menu a Rust page serves, then every other page registered at a path of its own,
    /// each drawn through its registered handler as the server answers it, never « No Rust page »; Ctrl-K opens it too.
    #[dash_test]
    fn the_page_menu_reaches_every_rust_page() {
        let dir = dash_meter::TestDir::new("tui-menu");
        unsafe {
            // no page reads his own tools or mail: /tools probes what HOME holds
            std::env::set_var("HOME", &dir.0);
            std::env::set_var("DASH_CLI", "/nonexistent/dash");
            std::env::set_var("DASH_REPO", &dir.0);
            std::env::set_var("DASH_WORKTREES", &dir.0);
        }
        let host = crate::pages::Binary::served(&dir.0);
        let board = (
            Board::open(&dir.0).expect("the board"),
            Registry::with_compiled().expect("the registry"),
        );
        board.0.read_as(vec![]);
        let mut tui = Tui::default();
        go(host, &board, &mut tui, HOME);
        let menu = dash_surface::tui::menu(host);
        let path = |at: &str| at.split('?').next().unwrap_or_default().to_string();
        // every page registered at a path of its own, and served, is an entry
        let own = (inventory::iter::<crate::pages::Page>.into_iter())
            .map(|p| p.path)
            .filter(|p| !p.contains(['*', '<']) && crate::pages::find(host, p).is_some());
        for page in own {
            assert!(
                menu.iter().any(|(_, at)| path(at) == page),
                "{page} in {menu:#?}"
            );
        }
        // the web menu's Rust links first, in its order
        let web: Vec<String> = (dash_surface::forms::menu_links(host).into_iter())
            .map(|(_, _, at)| at)
            .filter(|at| crate::pages::find(host, &path(at)).is_some())
            .collect();
        let firsts: Vec<String> = menu
            .iter()
            .take(web.len())
            .map(|(_, at)| at.clone())
            .collect();
        assert_eq!(firsts, web);
        let mut reached = vec![];
        for (i, (name, at)) in menu.iter().enumerate() {
            let open = if i == 0 { Key::Menu } else { Key::Char('m') };
            assert_eq!(tui.key(open), None);
            for _ in 0..menu.len() {
                tui.key(Key::Up);
            }
            for _ in 0..i {
                tui.key(Key::Down);
            }
            let asked = tui.key(Key::Enter).expect("an act");
            assert_eq!(asked, Act::Go(at.clone()), "{name}");
            assert!(act(host, &board, &mut tui, asked).expect("drawn"));
            assert_eq!(tui.target(), at);
            let shown = screen(&tui);
            assert!(
                !shown.iter().any(|l| l.contains("No Rust page")),
                "{name}: {shown:#?}"
            );
            reached.push(format!("{at}\t{}", shown[0]));
        }
        println!("reached {} pages:\n{}", reached.len(), reached.join("\n"));
    }
}
