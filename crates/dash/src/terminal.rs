//! /terminal/<address>: a terminal hc runs, on the web and in the window (task rust-terminal-page, rfc-rust-unified 20f's
//! slice 2). The server holds the screen: the page draws what `Terminal::read` gives with the design system's Terminal, a
//! form posts his line to `Terminal::send`, and the screen follows through the page's patch stream (`?view=patches`).
//! ponytail: the stream reads the screen twice a second while a page is open; slice 3's events.subscribe replaces the poll.
use crate::pages::{self, Asked, Response, TEXT};
use dash_core::tasks::Terminal;
use dash_surface::{self as surface, components::terminal_screen};
use serde_json::json;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// The page's prefix: its address is the one segment after it.
const PAGE: &str = "/terminal/";
/// The screen's key in the page's tree.
const SCREEN: &str = "terminal";
/// The patch stream's span resource, one for every terminal: an address never mints one.
pub const PATCHES: &str = "/terminal?view=patches";
/// How often an open page's stream reads the screen.
const EVERY: Duration = Duration::from_millis(500);

/// The terminal the page drives: hc's, as its URL names hc's address, whatever `DASH_TERMINAL` picks for task runs. A test
/// sets its stand-in first.
pub static TERMINAL: OnceLock<Box<dyn Terminal + Send + Sync>> = OnceLock::new();

pub fn terminal() -> &'static dyn Terminal {
    TERMINAL
        .get_or_init(|| Box::new(dash_terminal::Hc::from_env()))
        .as_ref()
}

/// An address hc could name: never empty, never read as one of its flags.
fn address(id: &str) -> Result<&str, String> {
    match id.is_empty() || id.starts_with('-') {
        true => Err(format!("no terminal is named « {id} »")),
        false => Ok(id),
    }
}

/// The page: the address its heading, then the screen as read and the line's form.
fn drawing(terminal: &dyn Terminal, id: &str) -> Result<surface::Drawing, String> {
    let address = address(id)?;
    let text = terminal.read(address)?;
    let title = format!("Terminal {address}");
    let tree = json!({"root": "page", "elements": {
        "page": {"type": "stack", "children": ["title", SCREEN]},
        "title": {"type": "heading", "props": {"text": title}},
        SCREEN: {"type": "terminal", "props": {
            "text": text,
            "action": format!("{}/send", surface::terminal_href(address))}}}});
    Ok(surface::Drawing::Tree {
        title,
        tree: tree.to_string(),
    })
}

// The window draws it once a visit (`redraw: false`): a draw costs an hc call, and a window that follows the screen waits for
// hc's events (RFC 20f, slice 3).
inventory::submit! {
    pages::Page { path: "/terminal/*", step: "route_terminal", reads: pages::Reads::Nothing, redraw: false,
        serve: |a: &Asked| a.form(Some(drawing(terminal(), &a.id()))) }
}

/// The address in a request's path `/terminal/<address><tail>`, decoded, for `method`.
fn asked(line: &str, method: &str, tail: &str) -> Option<String> {
    let mut words = line.split_whitespace();
    (words.next() == Some(method)).then_some(())?;
    let target = words.next()?;
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let id = path.strip_prefix(PAGE)?.strip_suffix(tail)?;
    (!id.is_empty() && !id.contains('/'))
        .then(|| (pages::segment_decoded(id), query))
        .filter(|(_, query)| {
            tail == "/send" || pages::param(query, "view").as_deref() == Some("patches")
        })
        .map(|(id, _)| id)
}

/// `POST /terminal/<address>/send`: the terminal his line goes to.
pub fn posted(line: &str) -> Option<String> {
    asked(line, "POST", "/send")
}

/// `GET /terminal/<address>?view=patches`: the terminal whose screen the page follows.
pub fn follows(line: &str) -> Option<String> {
    asked(line, "GET", "")
}

/// His line, a form of `text`, typed into the terminal: hc pastes it and presses Enter; an empty one is Enter alone. Back to
/// the page, or, refused, why with his line, so nothing he typed is lost.
pub fn send(terminal: &dyn Terminal, id: &str, form: &str) -> Response {
    let text = pages::param(form, "text").unwrap_or_default();
    let address = match address(id) {
        Ok(address) => address,
        Err(e) => return ("404 Not Found", TEXT, e),
    };
    let sent = match text.is_empty() {
        true => terminal.key(address, "Enter"),
        false => terminal.send(address, &text),
    };
    match sent {
        Ok(()) => ("303 See Other", TEXT, surface::terminal_href(id)),
        Err(e) => (
            "502 Bad Gateway",
            TEXT,
            format!("Not typed: {e}\n\nYour line:\n{text}\n"),
        ),
    }
}

/// The screen's changes, for the patch stream: each call waits up to the time it is given, reading the screen every
/// `EVERY`, and names the screen's part once what it reads differs from the last; `None` when nothing changed meanwhile. A
/// read that fails is a change too, so the page shows why. Its first read is in its first call, after the stream's head: a
/// slow hc never holds the page's connect, whose open swaps the screen in whole anyway.
pub fn changes(
    terminal: &'static dyn Terminal,
    address: String,
) -> impl FnMut(Duration) -> Option<Vec<String>> {
    let mut last = None;
    move |wait| {
        let end = Instant::now() + wait;
        if last.is_none() {
            last = Some(terminal.read(&address));
        }
        loop {
            let now = Some(terminal.read(&address));
            if now != last {
                last = now;
                return Some(vec![terminal_screen(SCREEN)]);
            }
            let left = end.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return None;
            }
            std::thread::sleep(EVERY.min(left));
        }
    }
}
