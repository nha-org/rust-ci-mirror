//! The page at /agents, phase 3 of rfc-rust-unified 20b: each Claude login with its 5-hour and week use, as the tree the surface
//! crate draws (task rust-agents-page).
use crate::fresh::{self, Read};
use dash_agent::{ClaudeCode, logins};
use dash_core::tasks::Limit;
use dash_surface as surface;
use dash_terminal::Live;
use serde_json::{Map, json};
use std::path::Path;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// A percent as `Dashboard.ClaudeAccounts`' pct writes it: whole with no decimals, else one.
fn percent(used: f64) -> String {
    if used.fract() == 0.0 {
        format!("{used:.0}%")
    } else {
        format!("{used:.1}%")
    }
}

/// A Unix time as `Dashboard.ClaudeAccounts`' clock writes it: « Sun 15/11 05:00 UTC ».
fn clock(at: u64) -> String {
    let days = (at / 86_400) as i64;
    let day = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"][(days % 7) as usize];
    let date = dash_core::date(days);
    let (minutes, hours) = (at / 60 % 60, at / 3600 % 24);
    format!(
        "{day} {}/{} {hours:02}:{minutes:02} UTC",
        &date[8..10],
        &date[5..7]
    )
}

/// A window as `Dashboard.ClaudeAccounts.shown` words it. No reset read is no reading: never 0 %. A window past its reset is
/// open again, 0 %, as its `window/2` reads it: a read kept for 5 minutes may outlive its window.
fn shown(used: f64, resets: u64, now: u64) -> String {
    match resets {
        0 => "no reading yet".into(),
        r if r <= now => "0%, reset passed since the last reading".into(),
        r => format!("{}, resets {}", percent(used), clock(r)),
    }
}

/// One agent's card on /agents: its name, what it works on, where it opens.
type Card = (String, String, Option<String>);

/// The screen at /agents as a UI tree: each agent and what it works on, the comment queue's first (task
/// rust-agents-comments-tabs; @nha, 2026-11-09: « should be an agents tab, and then a comments queue »), then a section per
/// login, its two windows a line each, then how old the read is: a skeleton before the first read ends (task rust-skeleton-live).
fn tree(
    logins: &[String],
    limits: &[Limit],
    now: u64,
    note: Option<&str>,
    agents: &[Card],
    title: &str,
) -> String {
    let text = |t: String| json!({"type": "text", "props": {"text": t}});
    let mut elements = Map::new();
    let heading = json!({"type": "heading", "props": {"text": title}});
    elements.insert("title".into(), heading);
    let note = note.map_or_else(|| surface::skeleton("Claude's usage"), |n| text(n.into()));
    elements.insert("note".into(), note);
    let mut cards = vec![];
    for (i, (title, line, href)) in agents.iter().enumerate() {
        let name = format!("agent-{i}");
        let card = json!({"type": "card", "props": {"title": title, "line": line, "href": href}});
        elements.insert(name.clone(), card);
        cards.push(name);
    }
    elements.insert("agents".into(), json!({"type": "list", "children": cards}));
    let mut children = vec![
        "title".to_string(),
        "agents".to_string(),
        "note".to_string(),
    ];
    for (i, login) in logins.iter().enumerate() {
        let limit = limits.iter().find(|l| &l.account == login);
        let (five, week) = match limit {
            Some(l) => (
                shown(l.five_hours, l.five_hours_resets, now),
                shown(l.week, l.week_resets, now),
            ),
            None => ("no reading yet".into(), "no reading yet".into()),
        };
        let lines = [
            ("five", format!("5 h: {five}")),
            ("week", format!("Week: {week}")),
        ];
        let mut names = vec![];
        for (kind, line) in lines {
            let name = format!("{kind}-{i}");
            elements.insert(name.clone(), text(line));
            names.push(name);
        }
        let section = json!({"type": "section", "props": {"title": login}, "children": names});
        let name = format!("login-{i}");
        elements.insert(name.clone(), section);
        children.push(name);
    }
    elements.insert(
        "page".into(),
        json!({"type": "stack", "children": children}),
    );
    json!({"root": "page", "elements": elements}).to_string()
}

/// Each login's figures, as `dash claude-accounts` reads them: Claude's `get_usage` answer, and for a window it leaves with
/// no reading, or a login it does not answer for, the status line's newest file. `usage` stands in for Claude in a test.
fn readings(claude: &Path, logins: &[String], now: u64, usage: Usage) -> Vec<Limit> {
    let agent = ClaudeCode {
        projects: claude.join("projects"),
        limits: claude.join("weekly-limit"),
    };
    // ponytail: parses every file (1,955 on 2026-11-08) each read, off the routing thread and at most every `FRESH`; a scan
    // that stops once each login is seen if a read's time ever shows.
    let files = agent.limits_at(now, usize::MAX).unwrap_or_else(|e| {
        eprintln!("agents: {e}");
        vec![]
    });
    let read = |login: &String| {
        let config = (login != "default").then(|| claude.join("accounts").join(login));
        let file = files.iter().find(|l| &l.account == login);
        let asked = usage(login, config.as_deref()).map_err(|e| eprintln!("agents: {login}: {e}"));
        let Ok(mut asked) = asked else {
            return file.cloned();
        };
        if let Some(file) = file {
            if asked.five_hours_resets == 0 {
                (asked.five_hours, asked.five_hours_resets) =
                    (file.five_hours, file.five_hours_resets);
            }
            if asked.week_resets == 0 {
                (asked.week, asked.week_resets) = (file.week, file.week_resets);
            }
        }
        Some(asked)
    };
    // One thread a login: each call takes some 7.6 s and up to its 120 s limit, and none waits on another.
    std::thread::scope(|s| {
        let reads: Vec<_> = logins.iter().map(|l| s.spawn(|| read(l))).collect();
        let joined = reads.into_iter().filter_map(|r| r.join().ok());
        joined.flatten().collect()
    })
}

static READ: Mutex<Read<Vec<Limit>>> = Mutex::new(Read::new());

/// The older app's agents page, the front door's for this one query: the orchestrator's Start and its auto-start switch live
/// there until Rust has them (/code-review of rust-agents-comments-tabs).
pub const OLDER_PAGE: &str = "/agents?app=older";

/// Each agent's card: the comment queue's, then every one herdr shows, by its tab's title, its state and its folder, then
/// where the orchestrator's controls are.
/// The page's heading counts the agents and says how each stands (task agents-count; @nha, 2026-11-09: « should probably have a
/// count of agents »): the comment agent and each one herdr lists, « Agents · 3: 1 working, 2 idle ». The orchestrator's card is
/// its controls, no agent; herdr silent counts only the comment agent.
fn counted(queue: &str, live: &Result<Vec<Live>, String>) -> String {
    let comment = if queue.contains("works comment") {
        "working"
    } else {
        "idle"
    };
    let mut by: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    *by.entry(comment).or_default() += 1;
    for a in live.iter().flatten() {
        *by.entry(a.status.as_str()).or_default() += 1;
    }
    let total: usize = by.values().sum();
    // working first, then the rest by name
    let mut said: Vec<String> = by
        .get("working")
        .map(|n| format!("{n} working"))
        .into_iter()
        .collect();
    said.extend(
        by.iter()
            .filter(|(s, _)| **s != "working")
            .map(|(s, n)| format!("{n} {s}")),
    );
    format!("Agents · {total}: {}", said.join(", "))
}

fn cards(queue: String, live: &Result<Vec<Live>, String>) -> Vec<Card> {
    let mut cards = vec![("Comment agent".into(), queue, Some("/comments".into()))];
    let folder = |cwd: &str| match cwd.trim_end_matches('/').rsplit('/').next() {
        Some(f) if !f.is_empty() => f.to_string(),
        _ => "no folder herdr names".into(),
    };
    match live {
        Ok(live) => cards.extend(live.iter().map(|a| {
            (
                a.title.clone(),
                format!("{} in {}", a.status, folder(&a.cwd)),
                None,
            )
        })),
        Err(e) => cards.push(("herdr".into(), format!("did not answer: {e}"), None)),
    }
    let controls = "Start it, and its auto-start after a limit, on the older app's page";
    cards.push((
        "Orchestrator".into(),
        controls.into(),
        Some(OLDER_PAGE.into()),
    ));
    cards
}

/// The page, from `home`'s `.claude`. `get_usage` takes some 7.6 s a login, and the server routes on one thread: a view shows the
/// last read and, past `FRESH`, starts the next on its own thread, as /tasks reads the older app's queue.
pub fn page(home: &Path) -> Result<surface::Drawing, String> {
    view(home, dash_agent::usage)
}

/// The figures a login's usage reader gives: Claude's in the server, a stand-in in a test.
type Usage = fn(&str, Option<&Path>) -> Result<Limit, String>;

fn view(home: &Path, usage: Usage) -> Result<surface::Drawing, String> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH);
    let now = now.map_err(|e| e.to_string())?.as_secs();
    let claude = home.join(".claude");
    let logins = logins(&claude);
    let logins_read = logins.clone();
    // `get_usage` costs no model call; only a view starts one.
    let read = fresh::last(&READ, now, "agents", move || {
        readings(&claude, &logins_read, now, usage)
    })?;
    let (limits, note) = match read {
        Some((limits, at)) => (limits, Some(fresh::ago(now, at))),
        None => (vec![], None),
    };
    // herdr answers in milliseconds: read on each view, the page on a thread of its own (main's on_the_network)
    let live = dash_terminal::Herdr::from_env().live();
    let queue = crate::comments::agent_now();
    let title = counted(&queue, &live);
    let cards = cards(queue, &live);
    let tree = tree(&logins, &limits, now, note.as_deref(), &cards, &title);
    Ok(surface::Drawing::Tree {
        title: "Agents".into(),
        tree,
    })
}

/// `dash login add <name> [--dry-run]|sign-in <name>|status|remove <name>`, on `$HOME/.claude` (task rust-login-signin). An add
/// opens its sign-in pane at once, as `Dashboard.ClaudeAccounts.add`; a dry run prints the folder and sign-in it would make.
pub fn login(args: &[&str], mut out: std::fs::File) {
    use dash_agent::login::{self, Security};
    use std::io::Write;
    let home = std::env::var("HOME").map_err(|e| format!("HOME: {e}"));
    let done = home.and_then(|home| {
        let home = Path::new(&home);
        let claude = home.join(".claude");
        let seed = home.join(".claude.json");
        let herdr = dash_terminal::Herdr::from_env();
        match args {
            ["add", name, "--dry-run"] => login::add(&claude, &seed, name, &Security, false)
                .map(|plan| format!("{plan}\ndry run: nothing made, no pane opened")),
            ["add", name] => {
                let plan = login::add(&claude, &seed, name, &Security, true)?;
                if plan.signed_in {
                    return Ok(plan.to_string());
                }
                // The folder is made either way: a failed pane says so, and how to open it again.
                match login::sign_in(&claude, name, &Security, &herdr) {
                    Ok(pane) => Ok(format!("{plan}\npane: {pane}")),
                    Err(e) => Err(format!(
                        "{plan}\nno sign-in pane: {e}\nthe folder is made: dash login sign-in {name} opens it"
                    )),
                }
            }
            ["sign-in", name] => {
                login::sign_in(&claude, name, &Security, &herdr).map(|pane| format!("pane: {pane}"))
            }
            ["status"] => {
                // One item the keychain fails on costs only its own line.
                let line = |name: String| match login::status(&claude, &name, &Security) {
                    Ok(s) => format!("{name}: {s}"),
                    Err(e) => format!("{name}: unknown, {e}"),
                };
                let lines: Vec<String> = logins(&claude).into_iter().map(line).collect();
                Ok(lines.join("\n"))
            }
            ["remove", name] => login::remove(&claude, name, &herdr.claude_sessions()?)
                .map(|dir| format!("removed {}", dir.display())),
            _ => Err(
                "usage: dash login add <name> [--dry-run]|sign-in <name>|status|remove <name>"
                    .into(),
            ),
        }
    });
    match done {
        Ok(text) => writeln!(out, "{text}").expect("stdout"),
        Err(e) => {
            eprintln!("login: {e}");
            std::process::exit(1);
        }
    }
}

inventory::submit! {
    crate::pages::Page { path: "/agents", step: "route_agents", reads: crate::pages::Reads::Nothing, redraw: false, serve: |a| match a.home() {
        Ok(home) => a.form(Some(page(&home))),
        Err(failed) => failed,
    } }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::TestDir;
    use dash_meter::memory::TestBudget;

    #[test]
    fn each_login_shows_its_two_windows_and_a_login_with_no_reading_says_so() {
        let _budget = TestBudget::new();
        let dir = TestDir::new("agents-page");
        let claude = dir.0.join(".claude");
        for folder in ["accounts/second", "accounts/third", "weekly-limit"] {
            std::fs::create_dir_all(claude.join(folder)).expect("dirs");
        }
        let reading = |name: &str, config: &str, rates: serde_json::Value| {
            let text = json!({"config_dir": config, "rate_limits": rates}).to_string();
            std::fs::write(claude.join("weekly-limit").join(name), text).expect("reading");
        };
        // 2100-01-01 00:00 UTC, a Friday.
        const LATER: u64 = 4_102_444_800;
        let (later, now) = (LATER, LATER - 7200);
        reading(
            "a.json",
            "",
            json!({"five_hour": {"used_percentage": 12.5, "resets_at": later + 3600},
                   "seven_day": {"used_percentage": 99, "resets_at": later}}),
        );
        reading(
            "b.json",
            "/h/.claude/accounts/second",
            json!({"seven_day": {"used_percentage": 80, "resets_at": later}}),
        );
        // Claude answers for default with its week and no 5 hours, and not at all for second.
        let usage: Usage = |login, config| match (login, config) {
            ("default", None) => Ok(Limit {
                key: false,
                account: "default".into(),
                five_hours: 0.0,
                five_hours_resets: 0,
                week: 1.0,
                week_resets: LATER,
            }),
            ("second", Some(c)) if c.ends_with("accounts/second") => Err("no answer".to_string()),
            _ => Err(format!("asked {login} at {config:?}")),
        };
        let logins = logins(&claude);
        let tree = tree(
            &logins,
            &readings(&claude, &logins, now, usage),
            now,
            Some("Read 0 min ago."),
            &cards(
                "The comment agent works comment c1 on /tasks: « x »".into(),
                &Ok(vec![Live {
                    title: "Rust design components".into(),
                    status: "working".into(),
                    cwd: "/w/rust-rust-design-components".into(),
                }]),
            ),
            "Agents · 2: 2 working",
        );
        let tree: serde_json::Value = serde_json::from_str(&tree).expect("json");
        let login = |i: usize| {
            let section = &tree["elements"][format!("login-{i}")];
            let lines = section["children"].as_array().expect("lines").iter();
            let text = |name: &serde_json::Value| {
                tree["elements"][name.as_str().expect("a name")]["props"]["text"].clone()
            };
            let lines: Vec<serde_json::Value> = lines.map(text).collect();
            (section["props"]["title"].clone(), lines)
        };
        // Done-when 1 of rust-agents-comments-tabs: each agent with what it works on, the comment agent among them
        assert_eq!(tree["elements"]["page"]["children"][1], "agents");
        assert_eq!(tree["elements"]["agents"]["type"], "list");
        let card = |i: usize| tree["elements"][format!("agent-{i}")]["props"].clone();
        assert_eq!(
            card(0),
            json!({"title": "Comment agent", "line": "The comment agent works comment c1 on /tasks: « x »",
                   "href": "/comments"})
        );
        assert_eq!(
            card(1),
            json!({"title": "Rust design components", "line": "working in rust-rust-design-components",
                   "href": null})
        );
        assert_eq!(
            login(0),
            (
                json!("default"),
                vec![
                    json!("5 h: 12.5%, resets Fri 01/01 01:00 UTC"),
                    json!("Week: 1%, resets Fri 01/01 00:00 UTC")
                ]
            ),
            "Claude's week, the status line's 5 hours where Claude has none"
        );
        assert_eq!(
            login(1),
            (
                json!("second"),
                vec![
                    json!("5 h: no reading yet"),
                    json!("Week: 80%, resets Fri 01/01 00:00 UTC")
                ]
            ),
            "no answer: the status line's; a window with no reading says so, never 0 %"
        );
        assert_eq!(
            login(2),
            (
                json!("third"),
                vec![json!("5 h: no reading yet"), json!("Week: no reading yet")]
            ),
            "a login with no reading at all"
        );
    }

    #[test]
    fn agents_is_one_drawing_the_server_makes_html_or_text() {
        let _budget = TestBudget::new();
        let dir = TestDir::new("agents-first");
        let usage: Usage = |login, _| {
            Ok(Limit {
                key: false,
                account: login.into(),
                five_hours: 0.0,
                five_hours_resets: 1,
                week: 7.0,
                week_resets: 4_102_444_800,
            })
        };
        // Done-when 1 of rust-page-forms: the server's `drawn` makes the one drawing HTML or text
        let form = |html| crate::pages::drawn(html, false, Some(view(&dir.0, usage)));
        let (_, kind, html) = form(true);
        assert_eq!(kind, crate::pages::HTML);
        // before the first read its usage is a skeleton, filled in place (task rust-skeleton-live)
        assert!(
            html.contains(r#"aria-label="Claude&#39;s usage: being read""#),
            "{html}"
        );
        assert!(html.contains("no reading yet"), "{html}");
        for zero in ["5 h: 0%", "Week: 0%"] {
            assert!(!html.contains(zero), "before a read, never 0 %: {html}");
        }
        fresh::waited(&READ, std::time::Duration::from_secs(5)).expect("the read");
        let (_, kind, text) = form(false);
        assert_eq!(kind, crate::pages::TEXT);
        assert!(
            text.starts_with("Agents · ") && text.contains("Week: 7%"),
            "{text}"
        );
        let (_, _, html) = form(true);
        assert!(html.contains("Read 0 min ago."), "{html}");
        assert!(
            html.contains("Week: 7%, resets Fri 01/01 00:00 UTC"),
            "{html}"
        );
        assert!(
            html.contains("5 h: 0%, reset passed since the last reading"),
            "{html}"
        );
    }

    #[test]
    fn a_login_whose_read_panics_costs_only_its_own_figures() {
        let _budget = TestBudget::new();
        let dir = TestDir::new("agents-panic");
        std::fs::create_dir_all(dir.0.join("accounts/second")).expect("dirs");
        let usage: Usage = |login, _| match login {
            "default" => panic!("a bad answer"),
            _ => Ok(Limit {
                key: false,
                account: login.into(),
                five_hours: 3.0,
                five_hours_resets: 9,
                week: 4.0,
                week_resets: 9,
            }),
        };
        let read = readings(&dir.0, &logins(&dir.0), 1, usage);
        assert_eq!(
            read.iter().map(|l| l.account.as_str()).collect::<Vec<_>>(),
            ["second"]
        );
    }

    #[test]
    fn herdr_down_still_shows_the_comment_agent_says_why_and_keeps_the_orchestrator_s_controls() {
        let _budget = TestBudget::new();
        let line = |c: &[Card]| {
            c.iter()
                .map(|(t, l, _)| format!("{t}: {l}"))
                .collect::<Vec<_>>()
        };
        let controls =
            "Orchestrator: Start it, and its auto-start after a limit, on the older app's page";
        assert_eq!(
            line(&cards("idle".into(), &Err("no herdr".into()))),
            [
                "Comment agent: idle",
                "herdr: did not answer: no herdr",
                controls
            ]
        );
        let at = |cwd: &str| Live {
            title: "t".into(),
            status: "idle".into(),
            cwd: cwd.into(),
        };
        let folders = cards("idle".into(), &Ok(vec![at("/r/klar/"), at("")]));
        assert_eq!(
            line(&folders)[1..3],
            ["t: idle in klar", "t: idle in no folder herdr names"]
        );
        assert_eq!(folders[3].2.as_deref(), Some(OLDER_PAGE));
    }

    #[test]
    fn a_window_past_its_reset_says_so() {
        let _budget = TestBudget::new();
        assert_eq!(
            shown(40.0, 10, 20),
            "0%, reset passed since the last reading",
            "a window past its reset is open again"
        );
        assert_eq!(shown(0.0, 0, 20), "no reading yet");
        assert_eq!(clock(1_794_412_800), "Wed 11/11 16:00 UTC");
    }

    /// Task agents-count: the heading counts the comment agent and herdr's, each state said, working first.
    #[test]
    fn the_heading_counts_the_agents_and_how_each_stands() {
        let _budget = TestBudget::new();
        let live = |s: &str| Live {
            title: "t".into(),
            status: s.into(),
            cwd: "/r".into(),
        };
        let herdr = Ok(vec![live("working"), live("idle"), live("idle")]);
        assert_eq!(
            counted("The comment agent is idle: no comment waits.", &herdr),
            "Agents · 4: 1 working, 3 idle"
        );
        let busy = "The comment agent works comment c on /x: « a » (1 open).";
        assert_eq!(counted(busy, &herdr), "Agents · 4: 2 working, 2 idle");
        assert_eq!(counted(busy, &Err("down".into())), "Agents · 1: 1 working");
    }
}
