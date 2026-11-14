//! The fallback ends at half (rfc-rust-unified, task rust-fallback-off; @nha, 2026-11-11: « If we have say more than 50%
//! support, and we can build as we go maybe stop falling back altogether and treat each need as the impetus to build the tracer
//! bullet replacement. »). The front door counts each page and command he asks (`dash_rust.front_door.asked`) and each it hands
//! to the older app (`.forwarded`), in Datadog; bin/dash's commands and the bridge's calls all pass its `/dashboard/cli`, so it
//! counts theirs too. `dash fallback off --data <dir>` ends the forward once the week's numbers say Rust answers more than half:
//! a path or command Rust lacks then answers « not built yet », files its tracer-bullet task once, and links the old page on
//! :7799 meanwhile.
use crate::tasks::{now_ms, older_app};
use dash_gate::served::{self, Week};
pub use dash_gate::served::{command_tag, page_tag};
use std::path::Path;
use std::time::Duration;

/// The switch's file in the data folder: « off » ends the forward, anything else, or none, keeps it.
const SWITCH: &str = "fallback";
/// Where each path or command filed once is kept, a file a key: its first request and what the filing said.
const FILED: &str = "not-built";
/// The frozen Elixir app, the old page meanwhile (Makefile's rust/serve, `DASH_REFERENCE`).
const REFERENCE: u16 = 7799;
/// A filing waits on the older app's `dash task file`, as the bridge's calls do.
const FILE: Duration = Duration::from_secs(60);

/// Whether the forward to the older app has ended in `dir`.
pub fn ended(dir: &Path) -> bool {
    std::fs::read_to_string(dir.join(SWITCH)).is_ok_and(|s| s.trim() == "off")
}

/// What asked for a missing thing: a page's path, or a command's words.
pub enum Asked {
    Page { path: String, line: String },
    Command { args: Vec<String> },
}

impl Asked {
    /// One file a path or command, so each is filed once.
    fn key(&self) -> String {
        let key = match self {
            Asked::Page { path, .. } => format!("page{path}"),
            Asked::Command { args } => format!("command-{}", command_tag(args)),
        };
        key.chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .take(120)
            .collect()
    }

    fn what(&self) -> String {
        match self {
            Asked::Page { path, .. } => path.clone(),
            Asked::Command { args } => format!("dash {}", command_tag(args).replace('_', " ")),
        }
    }

    fn request(&self) -> String {
        let said = match self {
            Asked::Page { line, .. } => line.trim().to_string(),
            Asked::Command { args } => format!("dash {}", args.join(" ")),
        };
        said.chars().take(300).collect()
    }

    /// The old page on :7799 at `host` (the Host he asked on, its port dropped); a command's is the old app's home.
    pub fn old_page(&self, host: &str) -> String {
        let host = host.rsplit_once(':').map_or(host, |(h, _)| h);
        let host = if host.is_empty() { "127.0.0.1" } else { host };
        let port = crate::reference::asked().unwrap_or(REFERENCE);
        match self {
            Asked::Page { path, .. } => format!("http://{host}:{port}{path}"),
            Asked::Command { .. } => format!("http://{host}:{port}/"),
        }
    }
}

/// The tracer-bullet task for `asked`, filed once in `dir` by `file` (its words in, what it said out), then « not built yet »:
/// what it was, the task filed for it, the old page meanwhile.
pub fn not_built(
    dir: &Path,
    asked: &Asked,
    old_page: &str,
    file: impl FnOnce(&str) -> Result<String, String>,
) -> String {
    let what = asked.what();
    let folder = dir.join(FILED);
    let mark = folder.join(asked.key());
    // the mark is made before the filing, create_new being the one atomic step: two asks at once file one task
    let first = std::fs::create_dir_all(&folder).and_then(|()| {
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&mark)
    });
    let filed = match first {
        Ok(mut kept) => {
            // the request first: a mark with no second line is a filing under way
            let _ =
                std::io::Write::write_all(&mut kept, format!("{}\n", asked.request()).as_bytes());
            let words = format!(
                "Build {what} in Rust: not built yet, asked as « {} » on {} once the forward to the old app ended \
                 (task rust-fallback-off). Its tracer bullet: one thin slice through every layer it needs. The old \
                 page meanwhile: {old_page}",
                asked.request(),
                crate::comments::iso(now_ms())
            );
            match file(&words) {
                Ok(said) => {
                    let said = said.trim().to_string();
                    let _ = std::io::Write::write_all(&mut kept, format!("{said}\n").as_bytes());
                    said
                }
                // no mark kept: the next ask files it again (/code-review: a failed filing was never retried)
                Err(e) => {
                    let _ = std::fs::remove_file(&mark);
                    format!("not filed ({}), asked again on the next visit", e.trim())
                }
            }
        }
        Err(_) => std::fs::read_to_string(&mark)
            .ok()
            .and_then(|t| t.lines().nth(1).map(str::to_string))
            .filter(|s| !s.is_empty())
            .map_or("being filed".into(), |s| format!("filed already: {s}")),
    };
    format!(
        "not built yet: {what}\nIts tracer-bullet task: {filed}\nThe old page meanwhile: {old_page}\n"
    )
}

/// `not_built` on a thread of its own, its answer awaited `wait` at most: a slow older app never holds a page or command
/// past it (rust/CLAUDE.md rule 5); the filing goes on, and a later ask names its task.
pub fn not_built_within(
    dir: &Path,
    asked: Asked,
    old_page: String,
    file: impl FnOnce(&str) -> Result<String, String> + Send + 'static,
    wait: Duration,
) -> String {
    let (dir, what) = (dir.to_path_buf(), asked.what());
    let (sender, said) = std::sync::mpsc::sync_channel(1);
    let old = old_page.clone();
    let filing = std::thread::Builder::new().spawn(move || {
        let _ = sender.send(not_built(&dir, &asked, &old, file));
    });
    match filing.ok().and_then(|_| said.recv_timeout(wait).ok()) {
        Some(said) => said,
        None => format!(
            "not built yet: {what}\nIts tracer-bullet task: being filed\nThe old page meanwhile: {old_page}\n"
        ),
    }
}

/// The task filed through the older app's own `dash task file`, on its port: his list of tasks is still its (bin/dash with
/// DASHBOARD_URL, past the front door that no longer forwards). ponytail: once Rust holds the list, filed there instead.
pub fn file_on(older: Option<u16>) -> impl FnOnce(&str) -> Result<String, String> + Send + 'static {
    move |words| {
        let port = older.ok_or("no older app to hold the list of tasks")?;
        let url = format!("http://127.0.0.1:{port}");
        let bin = older_app().bin;
        let env = [("DASHBOARD_URL", url.as_str())];
        let args = ["task", "file", "site_perso", words];
        let (status, text, why) = dash_process::answer(&bin, &args, None, &env, FILE)?;
        match status.success() {
            true => Ok(text),
            false => Err(format!("{status}: {} {}", why.trim(), text.trim())),
        }
    }
}

/// The page form of the answer: what it was, the task filed, the old page, drawn by the design system's heading, text and
/// cards (Home's unit), a card opening the old page.
pub fn tree(said: &str, old_page: &str) -> String {
    let mut lines = said.lines();
    let title = lines.next().unwrap_or("not built yet");
    let filed = lines.next().unwrap_or_default();
    serde_json::json!({"root": "page", "elements": {
        "page": {"type": "stack", "children": ["title", "filed", "list"]},
        "title": {"type": "heading", "props": {"text": title}},
        "filed": {"type": "text", "props": {"text": filed}},
        "list": {"type": "list", "children": ["old"]},
        "old": {"type": "card", "props": {"title": "The old page meanwhile", "line": old_page, "href": old_page}},
    }})
    .to_string()
}

/// `dash fallback [on|off] --data <dir>`: the week's share Rust answered and its forwards by command, most first
/// (`dash_gate::served`), then the switch, saved. Off is refused at half or under, or when the share cannot be read: the RFC
/// ends the forward past half.
pub fn command(
    dir: &Path,
    to: Option<bool>,
    week: impl FnOnce() -> Result<Week, String>,
) -> Result<String, String> {
    let read = week();
    let line = match &read {
        Ok(w) => w.said(),
        Err(e) => format!("the last 7 days: not read: {e}"),
    };
    match (to, &read) {
        (Some(false), Ok(w)) if w.served() > 50.0 => write(dir, "off")?,
        (Some(false), _) => {
            return Err(format!(
                "{line}\nfallback: still on: it ends past half Rust-served"
            ));
        }
        (Some(true), _) => write(dir, "on")?,
        (None, _) => {}
    }
    let state = if ended(dir) {
        "off: a missing path or command answers « not built yet »"
    } else {
        "on"
    };
    Ok(format!("{line}\nfallback: {state}"))
}

fn write(dir: &Path, value: &str) -> Result<(), String> {
    std::fs::create_dir_all(dir)
        .and_then(|()| std::fs::write(dir.join(SWITCH), value))
        .map_err(|e| format!("{}: {e}", dir.display()))
}

/// The week from Datadog (`served::week`).
pub fn week() -> Result<Week, String> {
    served::week(now_ms() / 1000, &|pairs| {
        crate::alerts::get("https://api.datadoghq.eu/api/v1/query", pairs)
    })
}

/// The week's share and forwards, a pass of its own for as long as `dash orch` runs: /status/front-door draws its beat, and
/// it gates nothing (@nha, 2026-11-14, in task rust-forwarded-top's dialog: « the live share shows on /status and gates
/// nothing. »).
pub fn watch(dir: std::path::PathBuf) {
    let sleep = Duration::from_millis(crate::watchers::FRONT_DOOR.every_ms as u64);
    crate::watchers::run(dir, &crate::watchers::FRONT_DOOR, sleep, |_| {
        week().map(|w| w.said())
    });
}

/// /status/front-door's tree: the beat's lines and when it was read (`dash_surface::lines`).
fn tree_of(beats: Result<Vec<crate::watchers::Beat>, String>) -> String {
    let row = beats.map(|rows| {
        rows.into_iter()
            .find(|r| r.0 == crate::watchers::FRONT_DOOR.name)
    });
    let said = match row {
        Ok(Some((_, Some(at), _, said, _))) => format!("read {}\n{said}", crate::comments::iso(at)),
        Ok(_) => "no pass has read Datadog yet".into(),
        Err(e) => format!("the row did not read: {e}"),
    };
    dash_surface::lines("Front door", &said)
}

// One URL, both forms (rust/CLAUDE.md point 4): a browser gets HTML, an agent's curl the text.
inventory::submit! {
    crate::pages::Page { path: "/status/front-door", step: "route_status_front_door", reads: crate::pages::Reads::Nothing, redraw: true, serve: |a| {
        a.form(Some(crate::pages::serve_from_dir().map(|dir| dash_surface::Drawing::Tree {
            title: "Front door".into(),
            tree: tree_of(crate::watchers::beats(dir)),
        })))
    } }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    /// Done-when 3: the answer files the tracer-bullet task with the request that asked for it, once per path, and links :7799.
    #[dash_meter::dash_test]
    fn a_missing_path_files_its_task_once_with_its_request_and_links_7799() {
        let dir = dash_meter::TestDir::new("not-built");
        let filed = Cell::new(0);
        let asked = Asked::Page {
            path: "/mail".into(),
            line: "GET /mail?from=x HTTP/1.1\r\n".into(),
        };
        let old = asked.old_page("100.72.158.115:7788");
        assert_eq!(old, "http://100.72.158.115:7799/mail");
        let words = std::cell::RefCell::new(String::new());
        let file = |w: &str| {
            filed.set(filed.get() + 1);
            *words.borrow_mut() = w.to_string();
            Ok("site_perso: filed build-mail-in-rust\n".to_string())
        };
        let said = not_built(&dir.0, &asked, &old, file);
        assert_eq!(
            said,
            "not built yet: /mail\nIts tracer-bullet task: site_perso: filed build-mail-in-rust\n\
             The old page meanwhile: http://100.72.158.115:7799/mail\n"
        );
        assert!(
            words.borrow().contains("« GET /mail?from=x HTTP/1.1 »"),
            "{}",
            words.borrow()
        );
        assert!(
            words.borrow().contains("http://100.72.158.115:7799/mail"),
            "{}",
            words.borrow()
        );
        // the same path again, its query another: no second task, the first named
        let again = Asked::Page {
            path: "/mail".into(),
            line: "GET /mail HTTP/1.1".into(),
        };
        let said = not_built(&dir.0, &again, &old, |_| -> Result<String, String> {
            panic!("filed twice")
        });
        assert!(
            said.contains("filed already: site_perso: filed build-mail-in-rust"),
            "{said}"
        );
        assert_eq!(filed.get(), 1);
        // a command is filed once by its name, never its id or words
        let args: Vec<String> = ["queue", "done", "q-1", "oui"].map(String::from).into();
        let asked = Asked::Command { args };
        let said = not_built(&dir.0, &asked, &asked.old_page("127.0.0.1:7788"), |w| {
            assert!(w.contains("« dash queue done q-1 oui »"), "{w}");
            Ok("filed build-queue-done".into())
        });
        assert!(
            said.starts_with("not built yet: dash queue done\n"),
            "{said}"
        );
        assert!(said.ends_with("http://127.0.0.1:7799/\n"), "{said}");
        let other: Vec<String> = ["queue", "done", "q-2"].map(String::from).into();
        let said = not_built(&dir.0, &Asked::Command { args: other }, "", |_| {
            panic!("filed twice")
        });
        assert!(said.contains("filed already"), "{said}");
        // a filing that fails keeps no mark: the next ask files it (/code-review)
        let asked = || Asked::Page {
            path: "/o".into(),
            line: "GET /o HTTP/1.1".into(),
        };
        let said = not_built(&dir.0, &asked(), "", |_| Err("no older app".into()));
        assert!(
            said.contains("not filed (no older app), asked again on the next visit"),
            "{said}"
        );
        let said = not_built(&dir.0, &asked(), "", |_| Ok("filed build-o".into()));
        assert!(
            said.contains("Its tracer-bullet task: filed build-o\n"),
            "{said}"
        );
    }

    /// A slow older app never holds the answer: past its wait it says the task is being filed, and a later ask names it.
    #[dash_meter::dash_test]
    #[dash_meter::wall_budget(
        "400 ms: under the slow filing's 500 ms sleep, so an answer that waited it fails; a runner adds no 100 ms to a 50 ms wait"
    )]
    fn a_slow_filing_never_holds_the_answer() {
        let dir = dash_meter::TestDir::new("not-built-slow");
        let asked = || Asked::Page {
            path: "/v".into(),
            line: "GET /v HTTP/1.1".into(),
        };
        let slow = |_: &str| {
            std::thread::sleep(Duration::from_millis(500));
            Ok("filed build-v".to_string())
        };
        let started = std::time::Instant::now();
        let said = not_built_within(
            &dir.0,
            asked(),
            "old".into(),
            slow,
            Duration::from_millis(50),
        );
        assert!(
            started.elapsed() < Duration::from_millis(400),
            "waited {:?}",
            started.elapsed()
        );
        assert!(
            said.contains("Its tracer-bullet task: being filed\n"),
            "{said}"
        );
        let said = not_built(&dir.0, &asked(), "old", |_| panic!("filed twice"));
        assert!(said.contains("being filed"), "under way: {said}");
        std::thread::sleep(Duration::from_millis(700));
        let said = not_built(&dir.0, &asked(), "old", |_| panic!("filed twice"));
        assert!(said.contains("filed already: filed build-v"), "{said}");
    }

    /// Done-when 2: the switch ends the forward only past half Rust-served; at half, or unread, it stays on.
    /// Task rust-forwarded-top, done-when 3's other half: /status/front-door shows the share's beat, line by line, and
    /// before any pass says so.
    #[dash_meter::dash_test]
    fn status_front_door_shows_the_share_s_beat() {
        let text = |beats| dash_surface::text(&tree_of(beats)).expect("its text");
        assert!(text(Ok(vec![])).contains("no pass has read Datadog yet"));
        let said = "the last 7 days: 10 asked, 4 forwarded to the older app, 60% Rust-served\nforwarded, most first:";
        let row = (
            crate::watchers::FRONT_DOOR.name.into(),
            Some(1_794_428_220_000),
            true,
            said.into(),
            None,
        );
        let shown = text(Ok(vec![row]));
        assert!(shown.contains("read 2026-11-11T20:17:00Z"), "{shown}");
        assert!(
            shown.contains("60% Rust-served") && shown.contains("forwarded, most first:"),
            "{shown}"
        );
    }

    #[dash_meter::dash_test]
    fn the_switch_ends_the_forward_only_past_half() {
        let dir = dash_meter::TestDir::new("fallback-switch");
        let week = |asked, forwarded| {
            move || {
                Ok(Week {
                    asked,
                    forwarded,
                    by: vec![],
                })
            }
        };
        let refused = command(&dir.0, Some(false), week(100.0, 50.0)).expect_err("at half");
        assert!(
            refused.contains("50% Rust-served\nfallback: still on"),
            "{refused}"
        );
        assert!(command(&dir.0, Some(false), || Err("no keys".into())).is_err());
        assert!(!ended(&dir.0));
        let said = command(&dir.0, Some(false), week(100.0, 40.0)).expect("past half");
        assert!(said.ends_with("60% Rust-served\nfallback: off: a missing path or command answers « not built yet »"), "{said}");
        assert!(ended(&dir.0));
        command(&dir.0, Some(true), || Err("unread".into())).expect("on, whatever the share");
        assert!(!ended(&dir.0));
    }
}
