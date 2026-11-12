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

/// The page: the address its heading, then `part`, the screen or why there is none.
fn drawing(address: &str, part: serde_json::Value) -> surface::Drawing {
    let title = format!("Terminal {address}");
    let tree = json!({"root": "page", "elements": {
        "page": {"type": "stack", "children": ["title", SCREEN]},
        "title": {"type": "heading", "props": {"text": title}},
        SCREEN: part}});
    surface::Drawing::Tree {
        title,
        tree: tree.to_string(),
    }
}

/// The page as read: the screen and the line's form; with no hc to run, a 200 saying how to install it, never a 500 (task
/// rust-terminal-no-hc: the live page tripped the 400-and-over monitor); a terminal hc does not know, a 404 naming it.
fn served(a: &Asked, terminal: &dyn Terminal) -> Response {
    let id = a.id();
    // drawn as any other page, a 404 with an Alert: a closed terminal's page says so in his browser too (/code-review)
    let gone = |title: String, why: String| {
        let alert = json!({"type": "alert", "props": {"title": title, "text": why}});
        let (_, kind, body) = a.form(Some(Ok(drawing(&id, alert))));
        ("404 Not Found", kind, body)
    };
    let address = match address(&id) {
        Ok(address) => address,
        Err(e) => return gone("No such terminal".into(), e),
    };
    let part = match terminal.read(address) {
        Ok(text) => json!({"type": "terminal", "props": {
            "text": text,
            "action": format!("{}/send", surface::terminal_href(address))}}),
        Err(e) if dash_terminal::no_hc(&e) => json!({"type": "alert", "props": {
            "title": "No hc to run",
            "text": format!("dash shows terminals through herdr-core's hc, and cannot run it ({e}). Install hc at \
                ~/.local/bin/hc, or set HC_BIN to its path in dash serve's environment, then reload this page.")}}),
        Err(e) if dash_terminal::unknown(&e) => {
            let why = format!("hc knows no terminal « {address} »: it ended, or was never opened.");
            return gone("No such terminal".into(), why);
        }
        Err(e) => return a.failed(e),
    };
    a.form(Some(Ok(drawing(address, part))))
}

// The window draws it once a visit (`redraw: false`): a draw costs an hc call, and a window that follows the screen waits for
// hc's events (RFC 20f, slice 3).
inventory::submit! {
    pages::Page { path: "/terminal/*", step: "route_terminal", reads: pages::Reads::Nothing, redraw: false,
        serve: |a: &Asked| served(a, terminal()) }
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

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::TestDir;
    use dash_meter::dash_test;

    fn asked(path: &str, html: bool) -> Response {
        let a = Asked {
            path,
            query: "",
            html,
            board: None,
            registry: None,
        };
        served(&a, &dash_terminal::Hc::from_env())
    }

    /// Done-when 2 and 3 of rust-terminal-no-hc, through the real Hc: with no hc at HC_BIN, the page answers 200 with the
    /// design system's Alert naming hc and how to install it, its text form the same words; a terminal hc does not know
    /// answers 404 naming its address.
    #[dash_test]
    fn no_hc_is_a_notice_and_an_unknown_terminal_a_404() {
        let dir = TestDir::new("terminal-no-hc");
        unsafe { std::env::set_var("HC_BIN", dir.0.join("hc")) };
        let (status, kind, html) = asked("/terminal/t7", true);
        assert_eq!((status, kind), ("200 OK", pages::HTML), "{html}");
        // the Alert's markup is surface's test (an_alert_draws_its_title_then_its_words)
        assert!(html.contains("No hc to run"), "{html}");
        assert!(
            html.contains("Install hc at ~/.local/bin/hc, or set HC_BIN"),
            "{html}"
        );
        let (status, kind, text) = asked("/terminal/t7", false);
        assert_eq!((status, kind), ("200 OK", TEXT), "{text}");
        let missing = dir.0.join("hc");
        let want = format!(
            "Terminal t7\nNo hc to run\ndash shows terminals through herdr-core's hc, and cannot run it ({} {}: No such file \
             or directory (os error 2)). Install hc at ~/.local/bin/hc, or set HC_BIN to its path in dash serve's environment, \
             then reload this page.\n",
            dash_terminal::NO_HC,
            missing.display()
        );
        assert_eq!(text, want);

        // hc's own answer to an address it does not know (herdr-core's cli/output.rs), exit 3
        let hc = "#!/bin/sh\necho '{\"error\":{\"kind\":\"not_found\",\"message\":\"no terminal\",\"hint\":\"\"}}' >&2\nexit 3\n";
        std::fs::write(&missing, hc).expect("stand-in");
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::Permissions::from_mode(0o755);
        std::fs::set_permissions(&missing, mode).expect("mode");
        let (status, kind, said) = asked("/terminal/t9", false);
        let want = "Terminal t9\nNo such terminal\nhc knows no terminal « t9 »: it ended, or was never opened.\n";
        assert_eq!((status, kind, said.as_str()), ("404 Not Found", TEXT, want));
        let (status, kind, html) = asked("/terminal/t9", true);
        assert_eq!((status, kind), ("404 Not Found", pages::HTML), "{html}");
        assert!(html.contains("hc knows no terminal « t9 »"), "{html}");
        let (status, _, said) = asked("/terminal/-h", false);
        let want = "Terminal -h\nNo such terminal\nno terminal is named « -h »\n";
        assert_eq!((status, said.as_str()), ("404 Not Found", want));
    }
}
