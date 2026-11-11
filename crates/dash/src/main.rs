//! The one binary (section 3). `dash serve --data <dir>`, on loopback, prints `port <n>`, then answers GET /ping, /memory,
//! /scan, /search?q=<words>, /ui/<slot> and /deps, the crate and module graphs as built. With `--port <n>` it holds that port, on his tailnet address too, serves Home
//! at / (what waits on him, today's daily, what runs), the tasks at /tasks as the board (?layout=list the list, ?layout=graph the
//! graph, also at /graph), each task at /tasks/<id>, the code map at /deps, every access at /tools (`dash access` prints the same: access.rs), and `--data` defaults to the live folder; `--older <n>` makes it the front door, every other path the older app's on loopback port n. With DASH_ORCH set it also keeps `dash orch` running, its child (`keep_orch`). `dash ui` opens the app in the Mac window, each page rendered in its process from the same folder, no server under it. Plain std::net until the surface crate grows HTTP. On the same data folder:
//! `dash mcp` serves the registry's tools over stdio, `dash ingest --mime <key> <file>` runs an extractor and stores its
//! text, `dash extensions` lists every key with what serves it (section 11). `dash task add|waits|close|show|run` and `dash orch` are the
//! orchestrator of section 20b: tasks.rs, with `dash claude`, one agent isolated by MXC in a pane. `dash telemetry [on|off] --data <dir>` is the switch of the numbers they send. `dash draft` keeps each draft and its outcome:
//! drafts.rs. `dash comment pass` runs one pass of the comment queue `dash orch` runs every 30 s, `dash comment work` one
//! comment of its agent's: comments.rs.
mod access;
mod agents;
mod alerts;
mod automations;
mod chances;
mod codemap;
mod comments;
mod data;
mod design;
mod dmarc;
mod drafts;
mod fallback;
mod folds;
mod fresh;
mod home;
mod inbox;
mod keys;
mod kinds;
mod launchd;
mod layers;
mod mail;
mod pages;
mod parity;
mod proxy;
mod reference;
mod reporting;
mod reports;
mod roadmap;
mod sandbox;
mod search;
mod settings;
mod ship;
mod tasks;
mod temps;
mod terminal;
mod today;
mod watchers;

use dash_core::registry::{Hit, Registry};
use dash_core::tasks::{Queue, TaskStore};
use dash_core::{Store, ping};
use dash_ext as ext;
use pages::{HTML, Response, TEXT, drawn, form_decoded, param, view_named, wants_html};
// Linked for what it submits to the registry: nothing here names it, and an unnamed crate is not linked at all.
use dash_ingest as _;
use dash_meter::cap::Capped;
use dash_meter::memory::memory;
use dash_statsd as statsd;
use dash_statsd::Statsd;
use dash_store as store;
use dash_surface::{self as surface, TaskView};
use reporting::{armed, reporting, say_memory};
use serde_json::json;
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::os::fd::AsFd;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{SyncSender, sync_channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[cfg(not(any(feature = "mimalloc", feature = "jemalloc")))]
#[global_allocator]
static ALLOCATOR: Capped<std::alloc::System> = Capped(std::alloc::System, reporting::at_stop);

#[cfg(feature = "mimalloc")]
#[global_allocator]
static ALLOCATOR: Capped<mimalloc::MiMalloc> = Capped(mimalloc::MiMalloc, reporting::at_stop);

#[cfg(feature = "jemalloc")]
#[global_allocator]
static ALLOCATOR: Capped<tikv_jemallocator::Jemalloc> =
    Capped(tikv_jemallocator::Jemalloc, reporting::at_stop);

/// One slow or endless client must neither stall the serial loop nor grow its memory past the budget: a connection has this long
/// to its first byte, then the whole request has this long again, however its bytes trickle in.
const REQUEST_DEADLINE: Duration = Duration::from_secs(5);
/// How long a connection waits on the serial store: past a slow /scan or /search, never a limit a route has to meet.
/// ponytail: an abandoned request still runs when its turn comes; a flag on it if slow searches pile up.
const ANSWER_DEADLINE: Duration = Duration::from_secs(60);
const REQUEST_LIMIT: u64 = 64 * 1024;
/// Tailscale down at a start, as at a boot, or the bind failing: tried again this often.
const TAILNET_RETRY: Duration = Duration::from_secs(60);

/// Stdout is what a command answers: a port, a listing, a text, the MCP protocol. A crate's `println!` or a child that
/// inherits it would mix into it, so from here on the rest of the process writes to stderr and only the commands hold this.
fn claim_stdout() -> File {
    let answers = std::io::stdout()
        .as_fd()
        .try_clone_to_owned()
        .expect("dup stdout");
    assert!(unsafe { libc::dup2(2, 1) } == 1, "stdout to stderr");
    File::from(answers)
}

/// A command's failure said on stderr, exit 1.
fn exit_on(done: Result<(), String>) {
    if let Err(e) = done {
        eprintln!("{e}");
        std::process::exit(1);
    }
}

/// `dash roadmap set`: Rust's store in `dir` takes the plan.
fn roadmap_set(dir: &Path, id: &str, flags: &[&str], out: File) {
    exit_on(
        tasks::open_tasks(dir)
            .and_then(|mut t| roadmap::set(&mut t, &tasks::older_db(), id, flags, out)),
    )
}

/// `dash linear roadmap`: the roadmap's plans written to its Linear projects, each line printed; exit 1 when Linear does not
/// hold them (roadmap::write_linear).
fn linear_roadmap(dir: &Path, mut out: File) {
    let done = (|| {
        let plans = dash_store::tasks::read_plans(dir)?;
        let items = roadmap::items(&tasks::older_db(), &plans)?;
        let mut post = roadmap::linear_post()?;
        roadmap::write_linear(&items, &mut post)
    })();
    exit_on(done.and_then(|lines| {
        lines
            .iter()
            .try_for_each(|l| writeln!(out, "{l}"))
            .map_err(|e| e.to_string())
    }))
}

fn main() {
    let began = Instant::now();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let out = claim_stdout();
    // A stop at the cap names its command only once armed: a new command calls `reporting` first, or takes its folder by `armed`.
    match args.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        ["serve", "--data", dir] => serve(PathBuf::from(dir), None, began, out),
        ["serve", "--port", port] => serve(tasks::live_data(), Some(port_of(port)), began, out),
        ["serve", "--data", dir, "--port", port] => {
            serve(PathBuf::from(dir), Some(port_of(port)), began, out)
        }
        ["serve", "--port", port, "--older", older] => {
            let _ = OLDER.set(port_of(older));
            serve(tasks::live_data(), Some(port_of(port)), began, out)
        }
        ["serve", "--data", dir, "--port", port, "--older", older] => {
            let _ = OLDER.set(port_of(older));
            serve(PathBuf::from(dir), Some(port_of(port)), began, out)
        }
        ["launchd", "run", "--host", host, "--repo", repo] => {
            launchd::run(host, Path::new(repo), &home())
        }
        [
            "launchd",
            "install",
            "--host",
            host,
            "--repo",
            repo,
            "--bin",
            bin,
        ] => launchd_install(&args, bin, host, repo),
        ["launchd", "jobs", "--host", host] => {
            let (jobs, said) = launchd::all(host, &home());
            for j in jobs.values() {
                println!("{} {:?}", j.label, j);
            }
            said.iter().for_each(|e| eprintln!("{e}"));
            std::process::exit(i32::from(!said.is_empty()));
        }
        ["ui"] => ui(tasks::live_data()),
        ["ui", "--data", dir] => ui(PathBuf::from(dir)),
        ["ui", "--shot", target, png] => ui_shot(tasks::live_data(), target, Path::new(png)),
        ["ui", "--data", dir, "--shot", target, png] => {
            ui_shot(PathBuf::from(dir), target, Path::new(png))
        }
        ["ui", "--design"] => design_window(),
        ["ui", "--check", png] => ui_check(Path::new(png), false),
        ["ui", "--check", png, "--stop-front-door"] => ui_check(Path::new(png), true),
        ["mcp", "--data", dir] => mcp(Path::new(dir), out),
        ["mcp-proxy", "--cache", cache, "--", ref server @ ..] if !server.is_empty() => {
            let server: Vec<String> = server.iter().map(|w| w.to_string()).collect();
            let input = std::io::stdin().lock();
            proxy::serve(Path::new(cache), &server, input, out).expect("the MCP stream");
        }
        ["extensions", "--data", dir] => extensions(armed(dir, "extensions"), out),
        ["land", pr] => tasks::land_pr(pr, out),
        ["parity", "menu"] => parity::run(&parity::here(), "/", "/dashboard", true, out),
        ["parity", rust] => parity::run(&parity::here(), rust, rust, false, out),
        ["parity", rust, elixir] => parity::run(&parity::here(), rust, elixir, false, out),
        ["pr", "hold", "--data", dir, pr, why] => tasks::pr_hold(Path::new(dir), pr, why, out),
        ["pr", "hold", pr, why] => tasks::pr_hold(&tasks::live_data(), pr, why, out),
        ["pr", "release", "--data", dir, pr] => tasks::pr_release(Path::new(dir), pr, out),
        ["pr", "release", pr] => tasks::pr_release(&tasks::live_data(), pr, out),
        ["ingest", "--data", dir, "--mime", key, file] => ingest(Path::new(dir), key, file, out),
        ["doc", "extract", source] => doc_extract(Path::new(source), out),
        ["doc", "import", source] => doc_import(Path::new(source), out),
        ["task", "add", "--data", dir, id] => tasks::add(armed(dir, "add"), id, "", out),
        ["task", "add", "--data", dir, id, "--waits-on", waits] => {
            tasks::add(armed(dir, "add"), id, waits, out)
        }
        ["task", "waits", "--data", dir, id, "--on", waits] => {
            tasks::waits(armed(dir, "waits"), id, waits, out)
        }
        ["task", "close", "--data", dir, id, why] => {
            tasks::close(armed(dir, "close"), id, why, out)
        }
        ["task", "show", "--data", dir, id] => tasks::show(armed(dir, "show"), id, out),
        ["task", "brief", id] => tasks::print_brief(id, out),
        ["task", "run", "--data", dir, id] => {
            tasks::run(Path::new(dir), id, (None, None), false, out)
        }
        ["task", "run", "--data", dir, id, "--account", account] => {
            tasks::run(Path::new(dir), id, (Some(account), None), false, out)
        }
        ["task", "run", "--data", dir, id, "--model", model] => {
            tasks::run(Path::new(dir), id, (None, Some(model)), false, out)
        }
        [
            "task",
            "run",
            "--data",
            dir,
            id,
            "--account",
            account,
            "--model",
            model,
        ] => tasks::run(Path::new(dir), id, (Some(account), Some(model)), false, out),
        ["task", "run", "--data", dir, id, "--adopt"] => {
            tasks::run(Path::new(dir), id, (None, None), true, out)
        }
        ["access"] => access::list(out),
        ["alerts", "--data", dir] => alerts::once(Path::new(dir), out),
        ["chances", "--data", dir] => {
            if let Err(e) = chances::list(Path::new(dir), out) {
                eprintln!("chances: {e}");
                std::process::exit(1);
            }
        }
        ["watchers", "--data", dir] | ["watchers", "--data", dir, "--tab", "automations"] => {
            watchers::list(
                Path::new(dir),
                &comments::fed(Path::new(dir)),
                automations::rows(Path::new(dir)),
                args.get(4).map_or("", String::as_str),
                out,
            )
        }
        ["agents", ref rest @ ..] => keys_command(rest, out),
        ["login", ref rest @ ..] => agents::login(rest, out),
        ["limits"] => tasks::limits(out),
        ["dmarc", file] => dmarc::run(file, out),
        // a task's checks read a copy, as its sandbox writes none of the live folder (task rust-live-copy)
        ["data", "copy", dir] => {
            armed(&tasks::live_data().display().to_string(), "data copy");
            data::run(Path::new(dir), out)
        }
        // Claude's PostToolUse hook on Bash and Read: a sandbox denial becomes a candidate (task rust-denial-hook)
        ["sandbox", "denied"] => sandbox::hook(out),
        // the crates a diff touches and their dependents; after `--`, cargo run on them alone (task rust-affected-tests)
        ["ci", "affected", base, ref rest @ ..] => ci_affected(base, rest, out),
        ["ci", "list", ref rest @ ..] => ci_list(rest, out),
        // the PR gate's no-elixir on a pull request's own change, the pre-commit's step on what is staged (task rust-git-hooks-port)
        ["ci", "no-elixir", base, head] => exit_on(
            dash_gate::ratchet::top()
                .and_then(|top| dash_gate::hook::no_elixir::range(&top, base, head)),
        ),
        // git's hooks: .githooks/pre-commit and pre-push call these in one line (task rust-git-hooks-port)
        ["hook", "pre-commit"] => hook_pre_commit(),
        ["hook", "pre-push", remote, url] => hook_pre_push(remote, url),
        ["scripts", "count"] => scripts_count(false, None, out),
        ["scripts", "count", "--check"] => scripts_count(true, None, out),
        ["scripts", "count", "--check", "--base", base] => scripts_count(true, Some(base), out),
        ["scripts", "count", "--check", "--base", base, "--changed"] => {
            let said =
                dash_gate::ratchet::top().and_then(|top| dash_gate::ratchet::changed(&top, base));
            match said {
                Ok(said) => writeln!(&mut { out }, "{said}").unwrap_or_default(),
                Err(why) => {
                    eprintln!("scripts count: {why}");
                    std::process::exit(1);
                }
            }
        }
        // the sidebar's menu as text (task rust-menu-text)
        ["menu"] => write!(&mut { out }, "{}", surface::forms::menu_text()).expect("stdout"),
        ["code", "map", "--data", dir] => match codemap::run(Path::new(dir), &tasks::repo()) {
            Ok(said) => write!(&mut { out }, "{said}").unwrap_or_default(),
            Err(e) => {
                eprintln!("code map: {e}");
                std::process::exit(1);
            }
        },
        ["claude"] => tasks::claude(&tasks::live_data(), out),
        ["claude", "--data", dir] => tasks::claude(Path::new(dir), out),
        ["claude", "--in-pane", "--data", dir] => tasks::claude_in_pane(Path::new(dir)),
        ["orch", "--data", dir] => {
            let _look = one_look(Path::new(dir), true);
            comments::watch(PathBuf::from(dir));
            alerts::watch(PathBuf::from(dir));
            automations::watch(PathBuf::from(dir));
            chances::watch(PathBuf::from(dir), tasks::older_db());
            comments::work(PathBuf::from(dir));
            tasks::orchestrate(Path::new(dir), false, began, out)
        }
        ["orch", "--data", dir, "--once"] => {
            let _look = one_look(Path::new(dir), false);
            tasks::orchestrate(Path::new(dir), true, began, out)
        }
        ["orch", "--data", dir, "--sync"] => tasks::sync_once(Path::new(dir), out),
        ["orch", "--data", dir, "--reap"] => tasks::reap_once(Path::new(dir), out),
        ["orch", "--data", dir, "--deploy"] => tasks::redeploy_once(Path::new(dir), out),
        ["roadmap", "set", "--data", dir, id, ref flags @ ..] => {
            roadmap_set(armed(dir, "roadmap"), id, flags, out)
        }
        ["roadmap", "show", "--data", dir] => exit_on(roadmap::show(
            armed(dir, "roadmap"),
            &tasks::older_db(),
            out,
        )),
        ["roadmap", "set", id, ref flags @ ..] => roadmap_set(&tasks::live_data(), id, flags, out),
        ["roadmap", "show"] => exit_on(roadmap::show(&tasks::live_data(), &tasks::older_db(), out)),
        ["linear", "roadmap", "--data", dir] => linear_roadmap(armed(dir, "linear"), out),
        ["linear", "roadmap"] => linear_roadmap(&tasks::live_data(), out),
        ["orch", "next"] => tasks::orch_next(&tasks::live_data(), out),
        ["orch", "next", "--data", dir] => tasks::orch_next(Path::new(dir), out),
        ["orch", "route", "--data", dir, ref rest @ ..] => {
            tasks::orch_route(Path::new(dir), rest, out)
        }
        ["draft", "import", "--data", dir, "--from", from] => {
            drafts::import(armed(dir, "draft"), Path::new(from), out)
        }
        ["draft", "new", "--data", dir, "--file", file] => {
            drafts::new(armed(dir, "draft"), Path::new(file), None, out)
        }
        [
            "draft",
            "new",
            "--data",
            dir,
            "--file",
            file,
            "--from",
            from,
        ] => drafts::new(
            armed(dir, "draft"),
            Path::new(file),
            Some(Path::new(from)),
            out,
        ),
        ["draft", "show", "--data", dir, id] => drafts::show(armed(dir, "draft"), id, out),
        ["draft", "rates", "--data", dir] => drafts::rates(armed(dir, "draft"), out),
        ["comment", "pass", "--data", dir] => comments::once(armed(dir, "comment"), out),
        ["comment", "work", "--data", dir] => comments::work_one(armed(dir, "comment"), out),
        ["fallback", "--data", dir] => fallback_switch(armed(dir, "fallback"), None, out),
        ["fallback", "on", "--data", dir] => {
            fallback_switch(armed(dir, "fallback"), Some(true), out)
        }
        ["fallback", "off", "--data", dir] => {
            fallback_switch(armed(dir, "fallback"), Some(false), out)
        }
        ["inbox", "pass", "--data", dir] => inbox::once(armed(dir, "inbox"), out),
        ["inbox", "ls", "--data", dir] => inbox::ls(armed(dir, "inbox"), None, out),
        ["inbox", "ls", "--data", dir, state] => inbox::ls(armed(dir, "inbox"), Some(state), out),
        ["inbox", "rules"] => inbox::rules_listed(None, out),
        ["inbox", "rules", "--data", dir] => inbox::rules_listed(Some(armed(dir, "inbox")), out),
        ["inbox", "raise", "--data", dir, id, ref flags @ ..] => {
            inbox::raise(armed(dir, "inbox"), id, flags, out)
        }
        ["inbox", "answer", "--data", dir, id, words] => {
            inbox::answer(armed(dir, "inbox"), id, words, out)
        }
        ["inbox", "gate", "lift", "--data", dir, gate, words] => {
            inbox::lift(armed(dir, "inbox"), gate, words, out)
        }
        ["telemetry", "--data", dir] => telemetry(armed(dir, "telemetry"), None, out),
        ["telemetry", "on", "--data", dir] => telemetry(armed(dir, "telemetry"), Some(true), out),
        ["telemetry", "off", "--data", dir] => telemetry(armed(dir, "telemetry"), Some(false), out),
        // its lines are its answer, git-ship's on stdout: stdout goes back to the caller
        ["ship", ref rest @ ..] => {
            assert!(
                unsafe { libc::dup2(std::os::fd::AsRawFd::as_raw_fd(&out), 1) } == 1,
                "stdout back"
            );
            std::process::exit(ship::run(rest))
        }
        _ => usage(),
    }
}

/// `dash scripts count [--check [--base <rev>]]`: the script ratchet outside rust/ (dash_gate::ratchet), exit 1 on a fail.
/// `dash hook pre-commit`, run by git: each step's refusal ends the commit with its reason, exit 1.
fn hook_pre_commit() -> ! {
    use dash_gate::hook;
    let done = hook::from_hook_env("pre-commit").and_then(|ctx| hook::pre_commit::run(&ctx));
    if let Err(why) = done {
        hook::tell(&format!("{why}\n"));
        std::process::exit(1);
    }
    std::process::exit(0)
}

/// `dash hook pre-push <remote> <url>`, run by git with the pushed refs on stdin.
fn hook_pre_push(remote: &str, url: &str) -> ! {
    use dash_gate::hook;
    use std::io::Read;
    let mut ctx = hook::from_hook_env("pre-push").unwrap_or_else(|why| {
        hook::tell(&format!("pre-push: {why}\n"));
        std::process::exit(1)
    });
    let mut refs = String::new();
    let _ = std::io::stdin().read_to_string(&mut refs);
    std::process::exit(hook::pre_push::main(&mut ctx, remote, url, &refs))
}

fn scripts_count(check: bool, base: Option<&str>, mut out: File) {
    match dash_gate::ratchet::top().and_then(|top| dash_gate::ratchet::run(&top, check, base)) {
        Ok(said) => writeln!(out, "{said}").unwrap_or_default(),
        Err(why) => {
            eprintln!("scripts count: {why}");
            std::process::exit(1);
        }
    }
}

/// `dash ci affected <base> [--pushed <sha>] [-- <cargo arguments>]`: the affected crates one a line, or cargo's run on them
/// with its exit. `--pushed`: the pre-push's run, skipped with a note when this checkout is not that commit as pushed.
fn ci_affected(base: &str, rest: &[&str], mut out: File) {
    use dash_gate::affected;
    let (pushed, rest) = match rest {
        ["--pushed", sha, rest @ ..] => (Some(*sha), rest),
        rest => (None, rest),
    };
    // cargo needs its subcommand: a bare `--` is a usage error, never a run of `cargo -p ..` (/code-review)
    let args = match rest {
        [] => None,
        ["--", args @ ..] if !args.is_empty() => Some(args),
        _ => usage(),
    };
    let fail = |why: String| -> ! {
        eprintln!("ci affected: {why}");
        std::process::exit(1);
    };
    if let Some(sha) = pushed {
        match affected::not_as_pushed(Path::new("."), sha) {
            Ok(None) => {}
            Ok(Some(why)) => {
                return writeln!(
                    out,
                    "ci affected: {why}: its tests were not run here, CI runs them"
                )
                .unwrap_or_default();
            }
            Err(why) => fail(why),
        }
    }
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let crates = affected::of_repo(Path::new("."), base, &cargo).unwrap_or_else(|why| fail(why));
    let Some(args) = args else {
        return crates
            .iter()
            .for_each(|c| writeln!(out, "{c}").unwrap_or_default());
    };
    if crates.is_empty() {
        return writeln!(
            out,
            "ci affected: no crate changed since {base}, nothing to run"
        )
        .unwrap_or_default();
    }
    let args = affected::cargo_args(
        &crates,
        &args.iter().map(|a| a.to_string()).collect::<Vec<_>>(),
    );
    eprintln!("ci affected: cargo {}", args.join(" "));
    let (manifest, _, _) = affected::workspace(Path::new(".")).unwrap_or_else(|why| fail(why));
    // from the workspace's folder, so nextest finds its .config/nextest.toml
    let status = std::process::Command::new(&cargo)
        .args(&args)
        .current_dir(manifest.parent().expect("a folder"))
        .status();
    std::process::exit(status.map_or(127, |s| s.code().unwrap_or(1)));
}

/// `dash ci list [--repo <owner/name>] [--from <day>] [--days <n>]`: where CI's jobs ran and how long they waited, over whole
/// UTC days, by default the 7 before today (dash_forge::runs; task rust-ci-off-mac).
fn ci_list(args: &[&str], mut out: File) {
    let (mut slug, mut from, mut days) = ("nha-org/site_perso".to_string(), None, 7);
    for pair in args.chunks(2) {
        match pair {
            ["--repo", r] => slug = r.to_string(),
            ["--from", d] => from = Some(d.to_string()),
            ["--days", n] => days = n.parse().unwrap_or_else(|_| usage()),
            _ => usage(),
        }
    }
    let today = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64 / 86400);
    let from = from.unwrap_or_else(|| dash_core::date(today - days));
    let cache = tasks::repo().join(format!(
        ".claude/run/ci-jobs-{}.jsonl",
        slug.replace('/', "-")
    ));
    let ask = |words: &[&str]| dash_process::run("gh", words, None, Duration::from_secs(300));
    match dash_forge::runs::list(&ask, &slug, &from, days, &cache) {
        Ok(said) => write!(out, "{said}").unwrap_or_default(),
        Err(why) => {
            eprintln!("ci list: {why}");
            std::process::exit(1);
        }
    }
}

/// `dash agents`, on the live data unless `--data <dir>` names another folder, wherever it sits.
fn keys_command(args: &[&str], out: File) {
    let mut args = args.to_vec();
    let live = tasks::live_data().display().to_string();
    let dir = match args.iter().position(|a| *a == "--data") {
        Some(at) if at + 1 < args.len() => args.drain(at..at + 2).nth(1).unwrap_or_default(),
        Some(_) => usage(),
        None => &live,
    };
    let dir = armed(dir, "agents");
    let tools = &keys::live_tools(&tasks::repo());
    match args[..] {
        [] => keys::list(dir, out),
        ["key", "add", kind, name] => keys::add(dir, tools, kind, name, None, out),
        ["key", "add", kind, name, "--cap", cap] => {
            keys::add(dir, tools, kind, name, Some(cap), out)
        }
        ["key", "rm", kind] => keys::rm(dir, kind, out),
        ["key", "run", kind, prompt] => keys::run(dir, tools, kind, prompt, out),
        _ => usage(),
    }
}

/// Every command this binary answers, one a line: its usage, and the labels a fold's « dash <command> » is checked
/// against (folds.rs).
const USAGE: &str = "usage: dash serve|mcp|extensions --data <dir>\n       dash mcp-proxy --cache <file> -- <server> [<argument>...]\n       dash serve [--data <dir>] --port <n> [--older <n>]\n       dash ui [--data <dir>] [--shot <page> <png>]|--design|--check <png> [--stop-front-door]\n       dash ingest --data <dir> --mime <key> <file>\n       dash doc extract|import <file>\n       dash task add --data <dir> <id> [--waits-on <ids, a comma between>]\n       dash task waits --data <dir> <id> --on <ids, a comma between>\n       dash task close --data <dir> <id> <why>\n       dash task show|run --data <dir> <id> [--account <name>|--adopt]\n       dash orch --data <dir> [--once|--sync|--reap|--deploy]\n       dash orch next [--data <dir>]\n       dash roadmap show [--data <dir>]\n       dash roadmap set [--data <dir>] <id> [--start <day>] [--target <day>] [--after <ids>] [--milestone '<day> <name>']...\n       dash linear roadmap [--data <dir>]\n       dash pr hold [--data <dir>] <n> <why>\n       dash pr release [--data <dir>] <n>\n       dash claude [--data <dir>]\n       dash limits\n       dash dmarc <file of domains, one a line>\n       dash data copy <dir>\n       dash menu\n       dash parity <rust-path|rust-url> [<elixir-path>]|menu\n       dash scripts count [--check [--base <rev> [--changed]]]\n       dash ci list [--repo <owner/name>] [--from <day>] [--days <n>]\n       dash ci affected <base> [--pushed <sha>] [-- <cargo arguments>]\n       dash ci no-elixir <base> <head>\n       dash hook pre-commit|pre-push <remote> <url> (git's hooks)\n       dash telemetry [on|off] --data <dir>\n       dash fallback [on|off] --data <dir>\n       dash alerts --data <dir>\n       dash draft import --data <dir> --from <export>\n       dash draft new --data <dir> --file <labelled.md> [--from <export>]\n       dash draft show --data <dir> <id>\n       dash draft rates --data <dir>\n       dash comment pass|work --data <dir>\n       dash inbox pass|ls [<state>]|rules --data <dir>\n       dash inbox raise --data <dir> <id> --asker <who> --what <line> [--why <reason>] [--pick <pick>] [--link <url>] [--approval --sent-by <tool>] [--kind|--pr|--task|--draft|--supersedes|--not-before|--deadline <value>]\n       dash inbox answer --data <dir> <id> <words>\n       dash inbox gate lift --data <dir> <gate> <his words>\n       dash agents [--data <dir>]\n       dash agents key add <kind> <secret name> [--data <dir>] [--cap <usd>]\n       dash agents key rm|run <kind> [<prompt>] [--data <dir>]\n       dash login add <name> [--dry-run]|sign-in <name>|status|remove <name>\n       dash access\n       dash sandbox denied (a hook: its input on stdin)\n       dash ship \"<message>\" <file>...";

fn usage() -> ! {
    eprintln!("{USAGE}");
    std::process::exit(2);
}

fn port_of(word: &str) -> u16 {
    word.parse().unwrap_or_else(|_| {
        eprintln!("{word}: not a port");
        std::process::exit(2);
    })
}

/// The compiled-in entries, then extensions.d. Each refused file is answered, to name it; the app starts without it.
fn registry(dir: &Path) -> (Registry, Vec<String>) {
    let mut registry = Registry::with_compiled().expect("a key taken twice among our own crates");
    // the daily's « today » lines (task rust-daily-today): each plugin's own, the page reads them by slot
    for (origin, slot) in [("drafts", drafts::slot(dir)), ("mail", mail::slot())] {
        registry
            .add(origin, slot)
            .expect("a slot taken twice among our own crates");
    }
    let refused = ext::load_with(&mut registry, dir, &|tree| surface::check_tree(tree));
    (registry, refused)
}

fn registry_naming_refusals(dir: &Path) -> Registry {
    let (registry, refused) = registry(dir);
    refused.iter().for_each(|r| eprintln!("refused {r}"));
    registry
}

fn extensions(dir: &Path, mut out: File) {
    let (registry, refused) = registry(dir);
    let lines = registry.listing().into_iter();
    for line in lines.chain(refused.iter().map(|r| format!("refused {r}"))) {
        writeln!(out, "{line}").expect("stdout");
    }
}

/// rfc-rust-unified section 20i's first door: the source's items, one JSON line each, as Flare's `extract` prints them, its
/// header and count on stderr.
fn doc_extract(source: &Path, mut out: File) {
    let (format, items) = dash_ingest::doc::extract(source).unwrap_or_else(|e| fail(&e));
    eprintln!("# header: {}", serde_json::json!({ "format": format }));
    for item in &items {
        let mut line = serde_json::json!({ "title": item.title, "fts": item.fts });
        if let Some(page) = item.page {
            line["page"] = page.into();
        }
        writeln!(out, "{line}").expect("stdout");
    }
    eprintln!("# done: {} items", items.len());
}

/// Its second door: the source as a document of the dash, as canvist saves it, images and diagrams in it.
fn doc_import(source: &Path, mut out: File) {
    let doc = dash_ingest::doc::import(source).unwrap_or_else(|e| fail(&e));
    writeln!(out, "{doc}").expect("stdout");
}

fn fail(why: &str) -> ! {
    eprintln!("{why}");
    std::process::exit(1)
}

fn ingest(dir: &Path, key: &str, file: &str, mut out: File) {
    let report = reporting(dir, "ingest");
    let (registry, refused) = registry(dir);
    refused.iter().for_each(|r| eprintln!("refused {r}"));
    if registry.extractors.get(key).is_none() {
        // A refused file may be the one that serves it: the default's copy would be stored for good.
        if !refused.is_empty() {
            eprintln!("{key}: no entry while a file is refused, so nothing is stored");
            std::process::exit(1);
        }
        eprintln!("{key}: no entry, the default serves it");
    }
    let done = dash_ingest::read_file(file)
        .and_then(|input| registry.extract(key, &input))
        .and_then(|text| {
            let db = store::open(dir).map_err(|e| e.to_string())?;
            let title = Path::new(file)
                .file_name()
                .unwrap_or_default()
                .to_string_lossy();
            db.add_fact("file", &title, &text)
                .map_err(|e| e.to_string())?;
            Ok(text)
        });
    say_memory(&report);
    match done {
        Ok(text) => writeln!(out, "{text}").expect("stdout"),
        Err(e) => {
            eprintln!("{key}: {e}");
            std::process::exit(1);
        }
    }
}

fn mcp(dir: &Path, out: File) {
    let report = reporting(dir, "mcp");
    let registry = registry_naming_refusals(dir);
    surface::serve_mcp(&registry, std::io::stdin().lock(), out).expect("the MCP stream");
    say_memory(&report);
}

/// `dash fallback`: the week's share Rust served, then the forward's switch, saved (`fallback::command`).
fn fallback_switch(dir: &Path, to: Option<bool>, mut out: File) {
    match fallback::command(dir, to, fallback::week) {
        Ok(said) => writeln!(out, "{said}").expect("stdout"),
        Err(why) => {
            writeln!(out, "{why}").expect("stdout");
            std::process::exit(1);
        }
    }
}

/// `dash telemetry`: the switch, saved, then what it now says.
fn telemetry(dir: &Path, on: Option<bool>, mut out: File) {
    if let Some(on) = on
        && let Err(e) = statsd::set(dir, on)
    {
        eprintln!("{}: {e}", dir.display());
        std::process::exit(1);
    }
    let state = statsd::endpoint(dir).map_or("off".into(), |to| format!("on, statsd to {to}"));
    writeln!(out, "telemetry: {state}").expect("stdout");
}

/// A request's step: a page's registered one, else the routes below by name, so a client's paths never mint a metric. Only GET
/// is routed.
fn route_step(line: &str) -> &'static str {
    let mut words = line.split_whitespace();
    if words.next() != Some("GET") {
        return "route_other";
    }
    let target = words.next().unwrap_or_default();
    let path = target.split('?').next().unwrap_or_default();
    if let Some(page) = pages::find(path) {
        return page.step;
    }
    match path {
        path if path.starts_with("/projects/") => "route_project",
        "/ping" => "route_ping",
        "/built" => "route_built",
        "/memory" => "route_memory",
        "/scan" => "route_scan",
        "/compass" => "route_compass",
        path if path.starts_with("/ui/") => "route_ui",
        _ => "route_other",
    }
}

fn serve(dir: PathBuf, port: Option<u16>, began: Instant, mut out: File) {
    let report = reporting(&dir, "serve");
    // a page render is a child span of its request (task rust-dd-spans): surface names no tracer, the binary hands it this
    surface::screen::set_render_span(render_span);
    pages::serve_from(&dir);
    let db = report.step("open_store", || store::open(&dir).expect("open the store"));
    let registry = report.step("registry", || registry_naming_refusals(&dir));
    let board = report.step("board", || {
        port.map(|_| tasks::Board::open(&dir).expect("open the tasks"))
    });
    if let Some(board) = board {
        let _ = BOARD.set(Mutex::new(board));
    }
    let listener = TcpListener::bind(("127.0.0.1", port.unwrap_or(0))).expect("bind");
    // bound first, so a slow store delays no connection: a page after a deploy shows what was read, never its loader (task
    // rust-page-latency); the board's lock is held only for its own look, as a request's
    if let Some(board) = BOARD.get() {
        std::thread::spawn(move || {
            let board = board
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            tasks::start_reads(&board);
        });
    }
    let bound = listener.local_addr().expect("address").port();
    // the front door, while the older app exists: the frozen Elixir app on DASH_REFERENCE's port (Makefile's rust/serve: :7799),
    // each page's menu linking its own path there
    if let Some(at) = reference::asked()
        && port.is_some()
        && OLDER.get().is_some()
    {
        reference::hold(pages::repo(), at, Arc::clone(&report));
        surface::forms::link_reference(at);
    }
    writeln!(out, "port {bound}").expect("stdout");
    // A connection is read and written on its own thread, so an idle browser socket never holds the next request. The store stays
    // on this one, which only routes.
    let (sender, requests) = sync_channel::<Request>(CONNECTIONS);
    let live = Arc::new(AtomicUsize::new(0));
    if port.is_some() {
        let (sender, live, report) = (sender.clone(), Arc::clone(&live), Arc::clone(&report));
        std::thread::spawn(move || accept_on_tailnet(bound, &sender, &live, &report));
    }
    let doors = Arc::clone(&report);
    std::thread::spawn(move || accept(&listener, &sender, &live, &doors));
    if std::env::var_os(ORCH).is_some() {
        let dir = dir.clone();
        std::thread::spawn(move || keep_orch(&dir));
    }
    report.started(began);
    for (line, accept, queued, reply, span) in requests {
        // The serial route answers on this thread, so the request's root span is this thread's current one.
        statsd::set_current(Some(span));
        let response = route(&line, &accept, &db, &registry);
        statsd::set_current(None);
        let took = queued.elapsed();
        // After the reply: no client waits on its own number.
        let _ = reply.send(response);
        report.step_took(route_step(&line), took);
    }
}

/// Set, `dash serve` also runs the look: launchd's one dash job, rust-serve, starts the watchers itself (task rust-one-launchd;
/// @nha, 2026-11-09: « launchd: is there only one ? Rust? Which starts the app which starts the watchers ? »). A variable, not a
/// flag: the binary before this one ignores it, so a rollback to `dash.prev` or a restart before the deploy still serves.
const ORCH: &str = "DASH_ORCH";

/// The look and its watchers, `dash orch --data <dir>`, as this server's child, started again once it ends, 30 s after its
/// start at the soonest (launchd's throttle before). A child, not a thread: a deploy kills the look by its command line
/// (`Forge::restart_look`) and the server by its port, each coming back on the new binary. In a process group of its own: launchd
/// ends rust-serve's group when the server exits, and the look that runs a deploy outlives the server it restarts, to record it.
/// A look whose server is gone ends between passes (`tasks::orchestrate`) or while it waits (`one_look`), so none piles up and a
/// bootout of rust-serve stops it too; until it ends, it holds `one_look`'s lock and this child waits behind it.
fn keep_orch(dir: &Path) {
    loop {
        let started = Instant::now();
        let me = std::env::current_exe().map_err(|e| e.to_string());
        let ran = me.and_then(|me| {
            let mut look = std::process::Command::new(me);
            // never handed down: a test a task runs would start looks of its own on its scratch servers
            look.args(["orch", "--data"]).arg(dir).env_remove(ORCH);
            // nor the reference's port: a task's test server would take :7799 whenever it is free
            look.env_remove("DASH_REFERENCE");
            std::os::unix::process::CommandExt::process_group(&mut look, 0);
            look.status().map_err(|e| e.to_string())
        });
        let how = ran.map_or_else(|e| e, |status| status.to_string());
        let wait = Duration::from_secs(30).saturating_sub(started.elapsed());
        eprintln!(
            "serve: dash orch ended ({how}), started again in {} s",
            wait.as_secs()
        );
        std::thread::sleep(wait);
    }
}

/// The home folder, as launchd's agents read it.
fn home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").expect("HOME"))
}

/// `dash launchd install`, run again in a process group of its own and waited for: its caller, a sync run by comment-wake, is
/// a job of the agent it may boot out, whose stop ends that job's group, never this one.
fn launchd_install(args: &[String], bin: &str, host: &str, repo: &str) -> ! {
    const DETACHED: &str = "DASH_LAUNCHD_DETACHED";
    if std::env::var_os(DETACHED).is_none() {
        let me = std::env::current_exe().expect("its binary");
        let mut c = std::process::Command::new(me);
        c.args(args).env(DETACHED, "1");
        std::os::unix::process::CommandExt::process_group(&mut c, 0);
        let status = c.status().expect("dash launchd install");
        std::process::exit(status.code().unwrap_or(1));
    }
    match launchd::install(bin, host, Path::new(repo), &home()) {
        Ok(said) => said.iter().for_each(|l| println!("dash launchd: {l}")),
        Err(e) => {
            eprintln!("dash launchd install: {e}");
            std::process::exit(1);
        }
    }
    std::process::exit(0)
}

/// One look per data folder: `dash orch` holds `<dir>/orch.lock` for as long as it runs, so serve's child and another look
/// never pick twice. `wait`: the second says so once, its pid named, and takes over when the first ends (the kernel frees a dead
/// one's lock); else it stops, as `--once` beside a running look does.
fn one_look(dir: &Path, wait: bool) -> File {
    let path = dir.join("orch.lock");
    let exit = |e: &dyn std::fmt::Display| -> ! {
        eprintln!("dash orch: {}: {e}", path.display());
        std::process::exit(1)
    };
    let file = File::create(&path).unwrap_or_else(|e| exit(&e));
    if file.try_lock().is_err() {
        if !wait {
            exit(&"another look holds it");
        }
        let (me, parent) = (std::process::id(), unsafe { libc::getppid() });
        eprintln!(
            "dash orch {me}: another look holds {}, waiting for it to end",
            path.display()
        );
        // polled, not a blocking lock: a waiting look whose server is gone ends, so restarts pile none up
        while file.try_lock().is_err() {
            if unsafe { libc::getppid() } != parent {
                exit(&"its server is gone, the next one's look takes over");
            }
            std::thread::sleep(Duration::from_secs(1));
        }
    }
    file
}

/// The board, once `serve --port` opened it: the pages that read it lock it on their connection's thread.
static BOARD: std::sync::OnceLock<Mutex<tasks::Board>> = std::sync::OnceLock::new();
/// What needs neither the store nor the registry, /ping, /memory and every page that reads no registry, answered on its
/// connection's own thread: none waits behind a slow /scan or /search on the serial one (rust/CLAUDE.md rule 5: one slow
/// request never stalls the others). Only a page registered as reading the board, or the older app's link to a task, waits on
/// its lock: /mail on its server (10 s at most) holds no board page. `None`: the serial route's.
/// ponytail: one board lock, its pages reading polled caches; a lock per read if a board page grows slow.
fn unqueued(line: &str, accept: &str, board: Option<&Mutex<tasks::Board>>) -> Option<Response> {
    let mut words = line.split_whitespace();
    if words.next() != Some("GET") {
        return None;
    }
    let target = words.next().unwrap_or_default();
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let page = page_of(path, query).filter(|p| p.reads != pages::Reads::Registry);
    let locks = page.map_or(old_issue(path, query).is_some(), |p| {
        p.reads == pages::Reads::Board
    });
    let held = (board.filter(|_| locks))
        .map(|b| b.lock().unwrap_or_else(std::sync::PoisonError::into_inner));
    let board = held.as_deref();
    // The older app's link to a task Rust holds opens Rust's page (task rust-task-page-linear); a 404 leaves any other, and a
    // list not read, to the older app, which still answers it. Here, on the connection's thread, where the board is held.
    if let Some(id) = old_issue(path, query) {
        return Some(match board.map(|board| board.holds(&id)) {
            Some(Ok(true)) => ("303 See Other", TEXT, surface::task_href(&id, None)),
            Some(Err(e)) => {
                // Behind the front door this 404 goes to the older app and its text to nobody: the log keeps it.
                eprintln!("old-issue redirect: the task list is not read: {e}");
                (
                    "404 Not Found",
                    TEXT,
                    format!("the task list is not read: {e}"),
                )
            }
            _ => ("404 Not Found", TEXT, "no task of that id here".into()),
        });
    }
    Some(match path {
        "/ping" => ("200 OK", TEXT, ping().into()),
        "/built" => ("200 OK", TEXT, tasks::built()),
        "/memory" => {
            let m = memory();
            let body = format!("{:.1} {:.1}", m.current_mb, m.peak_mb);
            ("200 OK", TEXT, body)
        }
        _ => page_answer(page?, accept, (path, query), board, None),
    })
}

/// A request's first line, its Accept header, when it joined the queue, where its answer goes, and its root span: a route's
/// time starts at the queue, and a page's render is a child of that span.
type Request = (String, String, Instant, SyncSender<Response>, statsd::Span);

/// A request's root span, finished when the request ends: a guard, so every answer's return path finishes it. It also holds
/// the thread's current span, so a page render and a store read make their children of it.
struct RootSpan {
    span: Option<statsd::Span>,
}

impl RootSpan {
    fn new(span: statsd::Span) -> Self {
        statsd::set_current(Some(span.clone()));
        Self { span: Some(span) }
    }

    fn handle(&self) -> statsd::Span {
        self.span.clone().expect("a span")
    }
}

impl Drop for RootSpan {
    fn drop(&mut self) {
        if let Some(span) = self.span.take() {
            span.finish();
        }
        statsd::set_current(None);
    }
}

/// A page render's span, under the request's root: the surface crate names no tracer, so the binary gives it this one.
/// The closure finishes the span, so the surface crate holds no span type of its own.
fn render_span(name: &str, resource: &str) -> Option<Box<dyn FnOnce()>> {
    statsd::current().map(|parent| {
        let span = parent.child(name, resource);
        Box::new(move || span.finish()) as Box<dyn FnOnce()>
    })
}

/// The path of a request's first line, without its query: a span's resource. The board's patch stream is its own: it stays
/// open while the page does, so under /tasks its minutes read as the page's (task rust-page-latency).
fn request_path(line: &str) -> &str {
    if asks_patches(line) {
        return PATCHES;
    }
    if terminal::follows(line).is_some() {
        return terminal::PATCHES;
    }
    reference::line_path(line)
}

/// One thread per connection, each over by four deadlines (its first byte, its read, the store's answer, its write): past this many at once, a
/// new one is closed and logged, so a client opening sockets cannot grow memory past the budget.
const CONNECTIONS: usize = 64;
/// The status tag of a connection closed for want of room: no answer, but a request refused all the same.
const CLOSED: &str = "closed";

fn accept(
    listener: &TcpListener,
    requests: &SyncSender<Request>,
    live: &Arc<AtomicUsize>,
    report: &Arc<Statsd>,
) {
    for stream in listener.incoming() {
        let stream = match stream {
            Ok(stream) => stream,
            Err(e) => {
                // Out of file handles, say: the next try would fail at once, for as long as it lasts.
                eprintln!("no connection accepted: {e}");
                report.refused(CLOSED, "route_other");
                std::thread::sleep(Duration::from_secs(1));
                continue;
            }
        };
        let held = Held::take(live);
        if held.past_the_cap {
            eprintln!("past {CONNECTIONS} connections: one closed");
            report.refused(CLOSED, "route_other");
            continue;
        }
        let (requests, its_report) = (requests.clone(), Arc::clone(report));
        on_its_thread(std::thread::Builder::new(), report, move || {
            let _held = held;
            answer(
                &stream,
                &requests,
                &its_report,
                REQUEST_DEADLINE,
                ANSWER_DEADLINE,
            );
        });
    }
}

/// No thread to be had must not end the listener while the process stays up, where launchd would never restart it: `work`, its
/// connection and its slot, drops here, and that is a refused request.
fn on_its_thread(
    thread: std::thread::Builder,
    report: &Statsd,
    work: impl FnOnce() + Send + 'static,
) {
    if let Err(e) = thread.spawn(work) {
        eprintln!("no thread for a connection: {e}");
        report.refused(CLOSED, "route_other");
    }
}

/// A connection's slot, freed when it drops, a panic included: a leaked slot would refuse every connection in the end, with the
/// process still up, where launchd would never restart it.
struct Held {
    live: Arc<AtomicUsize>,
    past_the_cap: bool,
}

impl Held {
    fn take(live: &Arc<AtomicUsize>) -> Held {
        Held {
            live: Arc::clone(live),
            past_the_cap: live.fetch_add(1, Ordering::Relaxed) >= CONNECTIONS,
        }
    }
}

impl Drop for Held {
    fn drop(&mut self) {
        self.live.fetch_sub(1, Ordering::Relaxed);
    }
}

/// Each answer of 400 and over is counted and logged here, the one place they all pass. A connection that sent nothing gets no
/// answer: a browser's preconnect left open would show a 400 as its page (@nha, 2026-11-06: « had one "bad request" already fyi.
/// we need to monitor »).
fn answer(
    stream: &TcpStream,
    requests: &SyncSender<Request>,
    report: &Statsd,
    limit: Duration,
    wait: Duration,
) {
    if sent_nothing(stream, limit) {
        return;
    }
    let local = stream.local_addr().ok();
    let arrived = request_line(stream, limit);
    let route = route_step(match &arrived {
        Ok(head) => &head.line,
        Err(line) => line,
    });
    // One root span per request, its resource the path: a page's render and a store read are its children.
    let root = RootSpan::new(report.span(
        "http.request",
        request_path(match &arrived {
            Ok(head) => &head.line,
            Err(line) => line,
        }),
    ));
    // the forward ended (`dash fallback off`): nothing goes to the older app, a missing path answers « not built yet »
    let ended = forward_ended();
    let older = OLDER.get().copied().filter(|_| !ended);
    if let Ok(head) = &arrived
        && !is_a_cli_post(&head.line)
        && his_page(head)
    {
        let path = request_path(&head.line);
        report.front_door("asked", "page", &fallback::page_tag(path));
    }
    let response = match &arrived {
        Err(_) => None,
        Ok(head) => {
            let host = head.host.as_deref();
            let ours = matches!((host, local), (Some(host), Some(local)) if allowed_host(host, local.ip()));
            if ours && is_a_cli_post(&head.line) && cli(stream, head, older, report, limit) {
                return;
            }
            // Behind the front door the older app judges its own Host (Tailscale Serve's name among them), and answers
            // what is not ours, a browser's own fetches included.
            if let Some(port) = older
                && !(ours && !head.upgrade && served_here(&head.line))
            {
                return to_the_older_app(stream, head, port, report);
            }
            // the backup is bytes, a file to save: written here, never through a page's text answer
            if ours && head.line.starts_with(&format!("GET {} ", settings::BACKUP)) {
                let data =
                    pages::serve_from_dir().map_or_else(|_| tasks::live_data(), Path::to_path_buf);
                let day = comments::iso(tasks::now_ms());
                settings::download(stream, &data, &day[..10]);
                return;
            }
            // a comment's picture or PDF, shown in place: bytes too (task rust-comment-files-filter); one no comment keeps goes
            // on to the routes, which find no page there
            let dir = (ours && head.line.starts_with("GET /compass/files/"))
                .then(pages::serve_from_dir)
                .and_then(Result::ok);
            if let Some((file, kind)) = dir.and_then(|d| comments::attached(d, &head.line)) {
                settings::file(
                    stream,
                    std::fs::read(file).map_err(|e| e.to_string()),
                    kind,
                    None,
                );
                return;
            }
            // the Notion document's PDF, this page printed by headless Chrome: bytes too (task notion-page-menu-export)
            if ours && asks_document_pdf(&head.line) {
                settings::file(
                    stream,
                    surface::design::document_pdf(),
                    "application/pdf",
                    Some("rendering-plan.pdf"),
                );
                return;
            }
            if ours && asks_patches(&head.line) {
                return board_patches(stream, BOARD.get());
            }
            if ours && let Some(address) = terminal::follows(&head.line) {
                let mut changes = terminal::changes(terminal::terminal(), address);
                return patches(stream, Ok(move |wait| Ok(changes(wait))));
            }
            Some(if !ours {
                ("403 Forbidden", TEXT, "not this server's host".into())
            } else if asked_by_a_browser_itself(&head.line) {
                ("200 OK", TEXT, String::new())
            } else if is_a_compass_post(&head.line) {
                compass_post(stream, head, limit)
            } else if let Some(which) = settings_post(&head.line) {
                let most = if which == settings::RESTORE {
                    settings::UPLOAD_MOST
                } else {
                    COMPASS_BODY
                };
                match body_posted(stream, head, limit, most) {
                    Err(refused) => refused,
                    Ok(body) => {
                        let data = pages::serve_from_dir()
                            .map_or_else(|_| tasks::live_data(), Path::to_path_buf);
                        let drawing = match which == settings::RESTORE {
                            true => settings::read_upload(
                                &data,
                                &body,
                                &header(head, "content-type").unwrap_or_default(),
                            ),
                            false => settings::confirm(&data),
                        };
                        match drawn(true, false, Some(Ok(drawing))) {
                            (status, HTML, page) => (
                                status,
                                HTML,
                                surface::forms::with_menu(&page, settings::PAGE, ""),
                            ),
                            answer => answer,
                        }
                    }
                }
            } else if let Some(name) = pause_post(&head.line) {
                // a page of this app only (body_posted checks its Origin); a 303 back to /watchers shows the switch flipped
                match body_posted(stream, head, limit, COMPASS_BODY) {
                    Err(refused) => refused,
                    Ok(_) => {
                        let data = pages::serve_from_dir()
                            .map_or_else(|_| tasks::live_data(), Path::to_path_buf);
                        let query = head
                            .line
                            .split_whitespace()
                            .nth(1)
                            .and_then(|t| t.split_once('?'));
                        let tab = query.and_then(|(_, q)| param(q, "tab"));
                        comments::switched(&data, &name, tab.as_deref())
                    }
                }
            } else if is_a_move(&head.line) {
                card_moved(stream, head, BOARD.get(), limit)
            } else if is_a_new_issue(&head.line) {
                new_issue(stream, head, limit)
            } else if let Some(id) = tasks::set_posted(&head.line) {
                property_set(stream, head, &id, BOARD.get(), limit)
            } else if let Some(id) = drafts::set_posted(&head.line) {
                match form_posted(stream, head, limit) {
                    Ok(form) => draft_set(&id, &form),
                    Err(refused) => refused,
                }
            } else if let Some(thread) = a_mail_reply(&head.line) {
                mail_reply(stream, head, &thread, limit)
            } else if is_a_new_mail(&head.line) {
                mail_compose(stream, head, limit)
            } else if let Some(address) = terminal::posted(&head.line) {
                match form_posted(stream, head, limit) {
                    Ok(form) => terminal::send(terminal::terminal(), &address, &form),
                    Err(refused) => refused,
                }
            } else if let Some(page) = compass_read(&head.line) {
                // on its connection's thread, never the serial route's: every page view asks it (/code-review)
                match pages::serve_from_dir().and_then(|dir| comments::compass(dir, &page)) {
                    Ok(body) => ("200 OK", "application/json", body),
                    Err(e) => ("500 Internal Server Error", TEXT, e),
                }
            } else if let Some(response) = {
                let begun = Instant::now();
                let response = unqueued(&head.line, &head.accept, BOARD.get());
                // a UDP send, never a wait: timed as the serial route's are
                if response.is_some() {
                    report.step_took(route, begun.elapsed());
                }
                response
            } {
                response
            } else {
                let (reply, answered) = sync_channel(1);
                let routed = requests
                    .try_send((
                        head.line.clone(),
                        head.accept.clone(),
                        Instant::now(),
                        reply,
                        root.handle(),
                    ))
                    .ok()
                    .and_then(|()| answered.recv_timeout(wait).ok());
                routed.unwrap_or(("503 Service Unavailable", TEXT, "busy".into()))
            })
        }
    };
    // A path of ours with nothing there (a task the older app knows, say) is the older app's to answer.
    if let (Some(port), Ok(head), Some(("404 Not Found", _, _))) = (older, &arrived, &response) {
        return to_the_older_app(stream, head, port, report);
    }
    let response = match (&arrived, response) {
        (Ok(head), Some(("404 Not Found", _, said))) if ended && said == NO_SUCH_PATH => {
            Some(not_built(head))
        }
        (_, response) => response,
    };
    let (status, kind, body) =
        response.unwrap_or_else(|| ("400 Bad Request", TEXT, "bad request".into()));
    let mut timed = ByDeadline {
        stream,
        deadline: Instant::now() + limit,
    };
    // a redirect's body is where it leads, a path of ours that `task_href` encoded
    let location = match status.starts_with('3') {
        true => format!("Location: {body}\r\n"),
        false => String::new(),
    };
    let sent = write!(
        timed,
        "HTTP/1.1 {status}\r\n{location}Content-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .is_ok();
    // After the write: a log that fails costs a line, never the answer.
    if counts_as_refused(status, sent) {
        let code = &status[..3];
        report.refused(code, route);
        let gone = if sent { "" } else { ", the client gone" };
        eprintln!("answered {code} on {route}{gone}");
    }
}

/// A 4xx that reached nobody refused nobody: a browser cancelling mid-request, as with a silent close. A 5xx is this server's fault
/// whoever is left to read it. Every status is three digits, so the text compares as the number.
fn counts_as_refused(status: &str, sent: bool) -> bool {
    status >= "500" || (status >= "400" && sent)
}

/// What a browser fetches by itself, never a page's link: an empty 200 with no turn on the serial store, or each visit could count
/// as a refusal, a 404, or a 503 behind a slow /search.
fn asked_by_a_browser_itself(line: &str) -> bool {
    let mut words = line.split_whitespace();
    let (method, target) = (words.next(), words.next().unwrap_or_default());
    let path = target.split('?').next().unwrap_or_default();
    method == Some("GET")
        && (path == "/favicon.ico"
            || path.starts_with("/apple-touch-icon")
            || path == "/.well-known/appspecific/com.chrome.devtools.json")
}

/// No byte by the limit, or the client gone first. Peeks, so the request stays to read. Any other error, a signal's for one, is left
/// for the read that follows: it retries, or fails into a counted 400.
fn sent_nothing(stream: &TcpStream, limit: Duration) -> bool {
    use std::io::ErrorKind::{
        ConnectionAborted, ConnectionReset, NotConnected, TimedOut, WouldBlock,
    };
    if stream.set_read_timeout(Some(limit)).is_err() {
        return true;
    }
    match stream.peek(&mut [0]) {
        Ok(n) => n == 0,
        Err(e) => matches!(
            e.kind(),
            TimedOut | WouldBlock | ConnectionReset | ConnectionAborted | NotConnected
        ),
    }
}

/// His tailnet address, never every interface: /search reads private text, and a Host header is whatever a client on his
/// Wi-Fi writes. Read again each minute: a new address (a re-login) ends the process for launchd to start it on that one, since
/// a listener in accept() cannot be told. Tailscale merely down keeps the listener.
fn accept_on_tailnet(
    port: u16,
    requests: &SyncSender<Request>,
    live: &Arc<AtomicUsize>,
    report: &Arc<Statsd>,
) {
    let mut bound = None;
    loop {
        let ifconfig = std::process::Command::new("/sbin/ifconfig").output();
        let text = ifconfig.map_or(String::new(), |o| {
            String::from_utf8_lossy(&o.stdout).into_owned()
        });
        let here = tailnet_in(&text);
        if tailnet_moved(bound, here) {
            eprintln!(
                "the tailnet's address moved from {bound:?} to {here:?}: ending, for a restart"
            );
            std::process::exit(1);
        }
        if let (None, Some(ip)) = (bound, here) {
            match TcpListener::bind((ip, port)) {
                Ok(listener) => {
                    eprintln!("on {}", listener.local_addr().expect("address"));
                    bound = Some(ip);
                    let (requests, live, report) =
                        (requests.clone(), Arc::clone(live), Arc::clone(report));
                    std::thread::spawn(move || accept(&listener, &requests, &live, &report));
                }
                Err(e) => eprintln!("the tailnet's port {port}: {e}"),
            }
        }
        std::thread::sleep(TAILNET_RETRY);
    }
}

fn tailnet_moved(bound: Option<Ipv4Addr>, here: Option<Ipv4Addr>) -> bool {
    matches!((bound, here), (Some(old), Some(new)) if old != new)
}

/// This Mac's Tailscale address: a 100.64.0.0/10 one on a `utun` interface. A CGNAT Wi-Fi or hotspot gives en0 a 100.x too, and
/// binding there would open this to that network (as `Dashboard.Tailnet`).
fn tailnet_in(ifconfig: &str) -> Option<Ipv4Addr> {
    let mut on_utun = false;
    for line in ifconfig.lines() {
        if !line.starts_with(char::is_whitespace) {
            on_utun = line.starts_with("utun");
        } else if on_utun
            && let Some(rest) = line.trim_start().strip_prefix("inet ")
            && let Some(Ok(ip)) = rest.split_whitespace().next().map(str::parse::<Ipv4Addr>)
            && ip.octets()[0] == 100
            && (64..128).contains(&ip.octets()[1])
        {
            return Some(ip);
        }
    }
    None
}

/// A stream whose reads, or writes, all end by one deadline: a timeout per call restarts at each byte.
struct ByDeadline<'a> {
    stream: &'a TcpStream,
    deadline: Instant,
}

impl Read for ByDeadline<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let left = self.deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(std::io::ErrorKind::TimedOut.into());
        }
        self.stream.set_read_timeout(Some(left))?;
        (&*self.stream).read(buf)
    }
}

impl Write for ByDeadline<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let left = self.deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(std::io::ErrorKind::TimedOut.into());
        }
        self.stream.set_write_timeout(Some(left))?;
        (&*self.stream).write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// /search reads private text, and any web page can reach loopback by a DNS name of its own: a Host check stops that.
fn is_loopback_host(host: &str) -> bool {
    matches!(host_name(host), "127.0.0.1" | "localhost")
}

fn host_name(host: &str) -> &str {
    host.rsplit_once(':').map_or(host, |(name, _port)| name)
}

/// A connection names the address it came in on: loopback's names on loopback, the tailnet's own address there.
fn allowed_host(host: &str, local: IpAddr) -> bool {
    if local.is_loopback() {
        is_loopback_host(host)
    } else {
        host_name(host) == local.to_string()
    }
}

/// A request's head as it arrived. `headers`: each line as sent, its line end kept, for the older app; `rest`: what was read
/// past the head, a body's start.
struct Head {
    line: String,
    host: Option<String>,
    /// Its Accept header's value, or empty: each page answers HTML or text by it (`wants_html`).
    accept: String,
    /// A WebSocket's: it stays open both ways.
    upgrade: bool,
    headers: Vec<String>,
    rest: Vec<u8>,
}

/// The request's first line and its Host, its headers read to the blank line: closing on unread bytes sends a reset
/// that can destroy the answer. Bytes that end before that line are a bad request: a client that gave up has no wrong Host.
/// `Err` holds what arrived of the first line, so a request cut short still names its route.
fn request_line(stream: &TcpStream, limit: Duration) -> Result<Head, String> {
    let timed = ByDeadline {
        stream,
        deadline: Instant::now() + limit,
    };
    let mut reader = BufReader::new(timed.take(REQUEST_LIMIT));
    let mut first = String::new();
    if reader.read_line(&mut first).is_err() {
        return Err(first);
    }
    let mut head = Head {
        line: String::new(),
        host: None,
        accept: String::new(),
        upgrade: false,
        headers: vec![],
        rest: vec![],
    };
    loop {
        let mut header = String::new();
        match reader.read_line(&mut header) {
            Ok(n) if n > 0 => {}
            _ => return Err(first),
        }
        if header == "\r\n" || header == "\n" {
            head.line = first;
            head.rest = reader.buffer().to_vec();
            return Ok(head);
        }
        if let Some((name, value)) = header.split_once(':') {
            let value = value.trim();
            match name.trim().to_ascii_lowercase().as_str() {
                "host" => head.host = Some(value.to_string()),
                "accept" => head.accept = value.to_string(),
                // h2c and the like are refused upgrades: the request stays a plain one
                "upgrade" => {
                    head.upgrade =
                        (value.split(',')).any(|p| p.trim().eq_ignore_ascii_case("websocket"))
                }
                _ => {}
            }
        }
        head.headers.push(header);
    }
}

use tasks::FRONT_DOOR;
/// The older app's port behind the front door (`dash serve --older <n>`): what this server does not serve goes there.
static OLDER: std::sync::OnceLock<u16> = std::sync::OnceLock::new();
/// Paths both apps serve, kept the older app's until a page of ours replaces its own: its ping that bin/comment-wake reads.
/// /agents is ours since rust-agents-comments-tabs, /search since rust-search-port (FindLive unreachable through the front door).
const THE_OLDER_APPS: [&str; 1] = ["/ping"];

/// A GET of a path this server routes, and the compass's post; every other method, the paths above and the older agents
/// page's own query (`agents::OLDER_PAGE`) are the older app's.
fn served_here(line: &str) -> bool {
    let mut words = line.split_whitespace();
    let (method, target) = (words.next(), words.next().unwrap_or_default());
    let path = target.split('?').next().unwrap_or_default();
    (method == Some("GET")
        && !THE_OLDER_APPS.contains(&path)
        && target != agents::OLDER_PAGE
        && !asked_by_a_browser_itself(line))
        || is_a_compass_post(line)
        || settings_post(line).is_some()
        || pause_post(line).is_some()
        || is_a_move(line)
        || is_a_new_issue(line)
        || tasks::set_posted(line).is_some()
        || drafts::set_posted(line).is_some()
        || a_mail_reply(line).is_some()
        || is_a_new_mail(line)
        || terminal::posted(line).is_some()
}

/// `POST /settings/restore` or its confirm: which.
fn settings_post(line: &str) -> Option<&'static str> {
    let mut words = line.split_whitespace();
    (words.next() == Some("POST")).then_some(())?;
    match words.next()?.split('?').next()? {
        p if p == settings::RESTORE => Some(settings::RESTORE),
        p if p == settings::CONFIRM => Some(settings::CONFIRM),
        _ => None,
    }
}

/// `POST /watchers/pause?name=<name>`: his switch on /watchers, the watcher or automation it names (task rust-automations).
fn pause_post(line: &str) -> Option<String> {
    let mut words = line.split_whitespace();
    (words.next() == Some("POST")).then_some(())?;
    let (path, query) = words.next()?.split_once('?')?;
    (path == watchers::PAUSE).then(|| param(query, "name"))?
}

/// `POST /mail/t/<thread>/reply`: the thread, decoded.
fn a_mail_reply(line: &str) -> Option<String> {
    let mut words = line.split_whitespace();
    (words.next() == Some("POST")).then_some(())?;
    let path = words.next()?.split('?').next()?;
    let thread = path.strip_prefix(mail::THREAD)?.strip_suffix("/reply")?;
    (!thread.is_empty() && !thread.contains('/')).then(|| form_decoded(&thread.replace('+', "%2B")))
}

/// `POST /mail/compose`: a new mail (task rust-mail-parity).
fn is_a_new_mail(line: &str) -> bool {
    let mut words = line.split_whitespace();
    words.next() == Some("POST")
        && words
            .next()
            .is_some_and(|t| t.split('?').next() == Some(mail::COMPOSE))
}

fn is_a_compass_post(line: &str) -> bool {
    let mut words = line.split_whitespace();
    words.next() == Some("POST")
        && words
            .next()
            .is_some_and(|t| t.split('?').next() == Some("/compass"))
}

/// `POST /tasks/move`: the board's drag of a card to a column (task rust-board-linear-order).
fn is_a_move(line: &str) -> bool {
    let mut words = line.split_whitespace();
    words.next() == Some("POST")
        && words
            .next()
            .is_some_and(|t| t.split('?').next() == Some("/tasks/move"))
}

/// `POST /tasks/new`: the board's « + New issue » form (task rust-board-buttons-new-issue).
fn is_a_new_issue(line: &str) -> bool {
    let mut words = line.split_whitespace();
    words.next() == Some("POST")
        && words
            .next()
            .is_some_and(|t| t.split('?').next() == Some(surface::components::NEW_ISSUE))
}

/// The project and the words a New issue form files: its title on the first line, which `dash task file` keeps as the task's
/// title, then its description below; no project, the unfiltered board's, files on site_perso. A title is required, and
/// neither it nor the project is ever one `dash task file` would read as its `--on` (/code-review of PR 1395).
fn issue_words(form: &str) -> Result<(Option<String>, String), (String, String)> {
    let field = |name| param(form, name).unwrap_or_default().trim().to_string();
    let (title, description) = (field("title"), field("description"));
    let words = match description.is_empty() {
        true => title.clone(),
        false => format!("{title}\n\n{description}"),
    };
    let refused = |why: &str| Err((why.to_string(), words.clone()));
    if title.is_empty() {
        return refused("a new issue needs a title");
    }
    if title.starts_with('-') {
        return refused("a title starts with a word, not a dash");
    }
    let project = Some(field("project")).filter(|p| !p.is_empty());
    // a folder of ~/repo, as the older app's repo check reads it: a word, never a flag nor a path
    let named = |p: &str| {
        p.starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_')
            && p.chars()
                .all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c))
    };
    if project.as_deref().is_some_and(|p| !named(p)) {
        return refused("no such project");
    }
    Ok((project, words))
}

/// `POST /tasks/new`: his new issue, filed by `dash task file` through the older app, then back to the board he was on.
/// Only a page of this app posts it (`form_posted`). A refusal sends his words back with why, so nothing he wrote is lost.
/// ponytail: it waits on `dash task file` on its own connection, up to the bridge's 60 s (another project's starts a
/// delegate); an answer at once and a filing in the background when that wait shows.
fn new_issue(stream: &TcpStream, head: &Head, limit: Duration) -> Response {
    let form = match form_posted(stream, head, limit) {
        Ok(form) => form,
        Err(refused) => return refused,
    };
    let (project, words) = match issue_words(&form) {
        Ok(asked) => asked,
        Err((why, words)) => {
            return (
                "400 Bad Request",
                TEXT,
                format!("Not filed: {why}\n\nYour words:\n{words}\n"),
            );
        }
    };
    let on = project.as_deref().unwrap_or("site_perso");
    match tasks::older_app().file(on, &words) {
        // the board he posted from, filtered as it was
        Ok(_) => (
            "303 See Other",
            TEXT,
            surface::TaskView::Board.href(project.as_deref()),
        ),
        // a timeout may still file it (another project's delegate starts): never « not filed », so no retry files it twice
        Err(e) => (
            "502 Bad Gateway",
            TEXT,
            format!(
                "Not confirmed: {e}\n\nIt may be filed still: check {on}'s board before posting again.\n\nYour words:\n{words}\n"
            ),
        ),
    }
}

/// `POST /tasks/<id>/set`: his pick in a task page's menu, one field, then back to the page. Status moves its card on Rust's
/// board (`Board::move_to`, its own refusal said); priority 1 to 4 and a due day go to the list through `dash task update`.
/// Only a page of this app posts it (`form_posted`).
fn property_set(
    stream: &TcpStream,
    head: &Head,
    id: &str,
    board: Option<&Mutex<tasks::Board>>,
    limit: Duration,
) -> Response {
    let form = match form_posted(stream, head, limit) {
        Ok(form) => form,
        Err(refused) => return refused,
    };
    let back = surface::task_href(id, None);
    // a plain form's post: its answer is the page he reads, so a refusal leads back (/code-review of PR 1395)
    let refused = |why: String| {
        (
            "400 Bad Request",
            TEXT,
            format!("Not set: {why}\n\nBack to the task: {back}\n"),
        )
    };
    let board_of = || board.map(|b| b.lock().unwrap_or_else(std::sync::PoisonError::into_inner));
    let set = if let Some(column) = param(&form, "status") {
        let Some(board) = board else {
            return (
                "503 Service Unavailable",
                TEXT,
                "no board in this process".into(),
            );
        };
        let mut board = board
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match board.move_to(id, &column) {
            Ok(Err(why)) => return refused(why),
            moved => moved.map(|_| String::new()),
        }
    } else if let Some(priority) = param(&form, "priority") {
        // none clears it, as Linear's « No priority »: the list's « nil »
        let level = match priority.as_str() {
            "none" => None,
            "1" | "2" | "3" | "4" => priority.parse().ok(),
            _ => return refused(format!("{priority}: a priority is 1 to 4, or none")),
        };
        let level_word = level.map_or("nil".into(), |l: u8| l.to_string());
        let set = tasks::older_app().update(id, &[("--priority", level_word.as_str())]);
        if set.is_ok()
            && let Some(b) = board_of()
        {
            b.noted(id, Some(level), None);
        }
        set
    } else if let Some(day) = param(&form, "due") {
        let day = match day.as_str() {
            "none" => None,
            d if d.len() == 10 && dash_core::days(d).is_some() => Some(day.clone()),
            _ => return refused(format!("{day}: a due day is written 2026-11-15, or none")),
        };
        let set = tasks::older_app().update(id, &[("--deadline", day.as_deref().unwrap_or("nil"))]);
        if set.is_ok()
            && let Some(b) = board_of()
        {
            b.noted(id, None, Some(day));
        }
        set
    } else if let Some((field, value)) = tasks::SIDE_FIELDS
        .iter()
        .find_map(|f| param(&form, f).map(|v| (*f, v)))
    {
        // the task's sidebar as the list holds it now, so a label turns on or off from what it carries (task
        // rust-issue-sidebar-menus); the epics only for a project
        let db = tasks::older_db();
        let read = dash_store::older::sidebar(&db, id).and_then(|side| {
            let epics = match field {
                "project" => dash_store::older::epics(&db)?,
                _ => vec![],
            };
            Ok(side.map(|s| (s, epics)))
        });
        let (side, epics) = match read {
            Ok(Some(read)) => read,
            Ok(None) => return refused(format!("no task {id} in the list")),
            Err(e) => {
                return (
                    "502 Bad Gateway",
                    TEXT,
                    format!("Not set: {e}\n\nBack to the task: {back}\n"),
                );
            }
        };
        // the page's own day, its fallback included, so the cycles it offered are those taken (/code-review of PR 1399)
        let today = tasks::today_of(tasks::now_ms());
        let on = match param(&form, "on").as_deref() {
            Some("1") => Some(true),
            Some("0") => Some(false),
            _ => None,
        };
        let write = match tasks::side_write(field, &value, on, &side, &epics, today) {
            Ok(write) => write,
            Err(why) => return refused(why),
        };
        let set = write.apply(&tasks::older_app(), id);
        if set.is_ok()
            && let Some(b) = board_of()
        {
            b.noted_side(id);
        }
        set
    } else {
        return refused(
            "no field to set: status, priority, due, assignee, labels, project, cycle or remind"
                .into(),
        );
    };
    match set {
        Ok(_) => ("303 See Other", TEXT, back),
        Err(e) => (
            "502 Bad Gateway",
            TEXT,
            format!("Not set: {e}\n\nBack to the task: {back}\n"),
        ),
    }
}

/// `POST /drafts/<id>/set`: State's Save, the day draft `id` goes out (task rust-draft-state-edit), then back to its page; a
/// refusal says why and links back.
fn draft_set(id: &str, form: &str) -> Response {
    let back = drafts::moved(id);
    let set = match param(form, "send-on") {
        Some(day) => drafts::send_on(id, &day),
        None => Err(("400 Bad Request", "no field to set: send-on".into())),
    };
    match set {
        Ok(()) => ("303 See Other", TEXT, back),
        Err((status, why)) => (
            status,
            TEXT,
            format!("Not set: {why}\n\nBack to the draft: {back}\n"),
        ),
    }
}

/// `POST /tasks/move`: a form of `id` and `column`, his drag of that card on /tasks, kept by the board. Only a page of this app
/// posts it (`form_posted`). A refusal says why, which the board's script writes on the card.
fn card_moved(
    stream: &TcpStream,
    head: &Head,
    board: Option<&Mutex<tasks::Board>>,
    limit: Duration,
) -> Response {
    let form = match form_posted(stream, head, limit) {
        Ok(form) => form,
        Err(refused) => return refused,
    };
    let Some(board) = board else {
        return (
            "503 Service Unavailable",
            TEXT,
            "no board in this process".into(),
        );
    };
    let field = |name| param(&form, name).unwrap_or_default();
    let mut board = board
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    match board.move_to(&field("id"), &field("column")) {
        Ok(Ok(())) => ("200 OK", TEXT, "moved".into()),
        Ok(Err(refused)) => ("400 Bad Request", TEXT, refused),
        Err(e) => ("500 Internal Server Error", TEXT, e),
    }
}

/// `GET /design/rendering-plan.pdf`, any query after it: the Notion document's Export PDF (task notion-page-menu-export).
fn asks_document_pdf(line: &str) -> bool {
    let mut words = line.split_whitespace();
    let (method, target) = (words.next(), words.next().unwrap_or_default());
    method == Some("GET") && target.split('?').next() == Some(surface::design::DOCUMENT_PDF)
}

/// The patch stream's span resource.
const PATCHES: &str = "/tasks?view=patches";

/// `GET /tasks?view=patches`: the board page's stream of patches (task rust-view-patches).
fn asks_patches(line: &str) -> bool {
    let mut words = line.split_whitespace();
    let (method, target) = (words.next(), words.next().unwrap_or_default());
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    method == Some("GET")
        && path == tasks::BOARD_PAGE
        && param(query, "view").as_deref() == Some("patches")
}

/// How often a patch stream says it is still there when no diff came: a page closed meanwhile fails the write, and its
/// subscription ends with it.
const STILL_THERE: Duration = Duration::from_secs(15);

/// The board page's patches as server-sent events, for as long as the page is open: one event a diff of the board view, the
/// ids of the elements it changes as a JSON list (`tasks::patched`), which the page's LIVE_JS swaps in (task rust-view-patches;
/// @nha, 2026-11-09: « both should work together locally too, via the timely dataflow updates for views. »). No lock held while
/// it waits. ponytail: one of `CONNECTIONS` per open board tab, and an older app in front relays it as any long answer.
fn board_patches(stream: &TcpStream, board: Option<&Mutex<tasks::Board>>) {
    let follow = board.map(|b| {
        b.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .patches()
    });
    let follow = follow.unwrap_or_else(|| Err("no board in this process".into()));
    patches(
        stream,
        follow.map(|ids| {
            move |wait| match ids.recv_timeout(wait) {
                Ok(ids) => Ok(Some(ids)),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Ok(None),
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => Err(()),
            }
        }),
    )
}

/// A page's patches as server-sent events, from `follow`: each call waits up to the time it is given for the ids of the next
/// change, `Ok(None)` when none came, `Err` once its source ended. The board's view diffs and a terminal's screen
/// (`terminal::changes`, task rust-terminal-page) both stream through it.
fn patches(
    stream: &TcpStream,
    follow: Result<impl FnMut(Duration) -> Result<Option<Vec<String>>, ()>, String>,
) {
    let mut out = ByDeadline {
        stream,
        deadline: Instant::now() + ANSWER_DEADLINE,
    };
    let mut next = match follow {
        Ok(next) => next,
        Err(why) => {
            let _ = write!(
                out,
                "HTTP/1.1 503 Service Unavailable\r\nContent-Type: {TEXT}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{why}",
                why.len()
            );
            return;
        }
    };
    let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n";
    let mut sent = out.write_all(head.as_bytes()).and_then(|()| out.flush());
    while sent.is_ok() {
        let event = match next(STILL_THERE) {
            Ok(Some(ids)) => format!("data: {}\n\n", json!(ids)),
            Ok(None) => ": still here\n\n".into(),
            Err(()) => return,
        };
        out.deadline = Instant::now() + ANSWER_DEADLINE;
        sent = out.write_all(event.as_bytes()).and_then(|()| out.flush());
    }
}

/// `GET /compass?page=<path>`, the compass's list of a page's comments as JSON for its script: the page asked.
fn compass_read(line: &str) -> Option<String> {
    let mut words = line.split_whitespace();
    let (method, target) = (words.next(), words.next().unwrap_or_default());
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    (method == Some("GET") && path == "/compass").then(|| param(query, "page").unwrap_or_default())
}

/// A comment's form at most: its words and quote at their limits, each byte of them encoded as %XX.
const COMPASS_BODY: usize = 32 * 1024;
/// A comment's form with its files at most: comments::FILES_MOST of comments::FILE_MOST each, and its words.
const COMPASS_FILES_BODY: usize = comments::FILES_MOST * comments::FILE_MOST + 64 * 1024;

/// A form a page of this app posted, read whole: its Origin is the Host it came to, so no other site a browser shows can post
/// it, and it is 32 KB at most. Its text, or the answer refusing it.
fn form_posted(stream: &TcpStream, head: &Head, limit: Duration) -> Result<String, Response> {
    let body = body_posted(stream, head, limit, COMPASS_BODY)?;
    Ok(String::from_utf8_lossy(&body).into_owned())
}

/// A request's header, by name in any case.
fn header(head: &Head, name: &str) -> Option<String> {
    head.headers.iter().find_map(|h| {
        let (n, v) = h.split_once(':')?;
        n.trim()
            .eq_ignore_ascii_case(name)
            .then(|| v.trim().to_string())
    })
}

/// A posted body of at most `most` bytes, from a page of this app only (its Origin is this host).
fn body_posted(
    stream: &TcpStream,
    head: &Head,
    limit: Duration,
    most: usize,
) -> Result<Vec<u8>, Response> {
    let named = |name: &str| header(head, name);
    let host = head.host.as_deref().unwrap_or_default();
    if named("origin") != Some(format!("http://{host}")) {
        return Err((
            "403 Forbidden",
            TEXT,
            "a form comes from a page of this app".into(),
        ));
    }
    let Some(length) = named("content-length").and_then(|l| l.parse::<usize>().ok()) else {
        return Err(("411 Length Required", TEXT, "a form has a length".into()));
    };
    if length > most {
        return Err((
            "413 Content Too Large",
            TEXT,
            format!("a form is {} KB at most", most / 1024),
        ));
    }
    let mut body = head.rest.clone();
    body.truncate(length);
    let mut timed = ByDeadline {
        stream,
        deadline: Instant::now() + limit,
    };
    let mut more = vec![0; length - body.len()];
    if timed.read_exact(&mut more).is_err() {
        return Err(("400 Bad Request", TEXT, "the form ended early".into()));
    }
    body.extend(more);
    Ok(body)
}

/// `POST /mail/t/<thread>/reply`: his reply, sent from the address the form names (task rust-mail-thread), then back to the
/// thread saying so. On the connection's thread, never the serial route's: the mail server answers in seconds.
fn mail_reply(stream: &TcpStream, head: &Head, thread: &str, limit: Duration) -> Response {
    let form = match form_posted(stream, head, limit) {
        Ok(form) => form,
        Err(refused) => return refused,
    };
    let field = |name| param(&form, name).unwrap_or_default();
    let account = field("account");
    let sent =
        pages::serve_from_dir().and_then(|dir| mail::reply(dir, &account, thread, &field("text")));
    match sent {
        Ok(email) => (
            "303 See Other",
            TEXT,
            mail::thread_href(thread, &account, &email),
        ),
        // his words come back with why, so nothing he wrote is lost
        Err(e) => (
            "502 Bad Gateway",
            TEXT,
            format!("Not sent: {e}\n\nYour words:\n{}\n", field("text")),
        ),
    }
}

/// `POST /mail/compose`: his new mail, sent from the address the form names, then back to Compose saying so; his words come
/// back with why when it did not go. On the connection's thread, as a reply.
fn mail_compose(stream: &TcpStream, head: &Head, limit: Duration) -> Response {
    let form = match form_posted(stream, head, limit) {
        Ok(form) => form,
        Err(refused) => return refused,
    };
    let field = |name| param(&form, name).unwrap_or_default();
    let account = field("account");
    let new = dash_mail::New::of(
        &field("to"),
        &field("cc"),
        &field("bcc"),
        &field("subject"),
        &field("text"),
        &field("draft"),
    );
    // all he typed comes back with why, so nothing is typed twice
    let typed = format!(
        "To: {}\nCc: {}\nBcc: {}\nSubject: {}\nYour words:\n{}\n",
        field("to"),
        field("cc"),
        field("bcc"),
        field("subject"),
        field("text")
    );
    // his own input refused is his to fix (422); the mail server's refusal is the gateway's (502) (/code-review)
    let new = match new {
        Ok(new) => new,
        Err(e) => {
            return (
                "422 Unprocessable Content",
                TEXT,
                format!("Not sent: {e}\n\n{typed}"),
            );
        }
    };
    match pages::serve_from_dir().and_then(|dir| mail::compose(dir, &account, &new)) {
        Ok(email) => ("303 See Other", TEXT, mail::compose_href(&account, &email)),
        Err(e) => ("502 Bad Gateway", TEXT, format!("Not sent: {e}\n\n{typed}")),
    }
}

/// `POST /compass`: the sidebar's comment, a form of `page`, `quote` and `text`, one row of the comment queue (task
/// rust-compass-queue). Only a page of this app posts it: its Origin is the Host it came to, so no other site a browser shows
/// can put words before the queue's agent. A plain form post (a Composer: Home's prompt, a task's comment box) is sent back
/// to its page at the comment, which the page's rail lists (task rust-task-brief-linear: a bare « Landed as » page had left him
/// one click from where he wrote); the rail's fetch follows the same 303 and reloads.
fn compass_post(stream: &TcpStream, head: &Head, limit: Duration) -> Response {
    // the rail's box sends a multipart form, its images and PDFs with it (task rust-comment-files-filter); the compass's own
    // box a plain one
    let kind = header(head, "content-type").unwrap_or_default();
    let multipart = kind.starts_with("multipart/form-data");
    let most = if multipart {
        COMPASS_FILES_BODY
    } else {
        COMPASS_BODY
    };
    let body = match body_posted(stream, head, limit, most) {
        Ok(body) => body,
        Err(refused) => return refused,
    };
    let (mut fields, mut files) = (std::collections::HashMap::new(), vec![]);
    if multipart {
        let parts = pages::parts(&body, &kind);
        if parts.is_empty() {
            return (
                "400 Bad Request",
                TEXT,
                "the form's parts did not read: send it again".into(),
            );
        }
        for part in parts {
            match part.file {
                // a picker left empty sends a part with no name and no bytes
                Some(name) if name.is_empty() && part.bytes.is_empty() => {}
                Some(name) => files.push((name, part.bytes)),
                None => {
                    let value = String::from_utf8_lossy(part.bytes).into_owned();
                    fields.entry(part.name).or_insert(value);
                }
            }
        }
    } else {
        let form = String::from_utf8_lossy(&body).into_owned();
        for name in ["page", "quote", "text"] {
            fields.insert(name.to_string(), param(&form, name).unwrap_or_default());
        }
    }
    let field = |name: &str| fields.get(name).cloned().unwrap_or_default();
    let posted = pages::serve_from_dir().and_then(|dir| {
        comments::post(
            dir,
            &field("page"),
            &field("quote"),
            &field("text"),
            &files,
            tasks::now_ms(),
        )
    });
    match posted {
        // the page is a path of ours with no control character (comments::post), so it is a header's safe value
        Ok(id) => ("303 See Other", TEXT, format!("{}#c-{id}", field("page"))),
        Err(e) if e.starts_with("a comment") => ("400 Bad Request", TEXT, e),
        Err(e) => ("500 Internal Server Error", TEXT, e),
    }
}

/// A command's form with its first `args[]=<flag>` (`--layer`, `--deadline`) and the `args[]=<value>` after it taken out, every
/// other byte as it came.
fn without(body: &[u8], flag: &str, value: &str) -> Vec<u8> {
    let form = String::from_utf8_lossy(body);
    let pairs: Vec<&str> = form.split('&').collect();
    let arg = |pair: &str| {
        let (key, value) = pair.split_once('=')?;
        (form_decoded(key) == "args[]").then(|| form_decoded(value))
    };
    let at = pairs.iter().position(|p| arg(p).as_deref() == Some(flag));
    let next = at.and_then(|at| (at + 1..pairs.len()).find(|i| arg(pairs[*i]).is_some()));
    let gone =
        |i: usize| Some(i) == at || (Some(i) == next && arg(pairs[i]).as_deref() == Some(value));
    let kept: Vec<&str> = (pairs.iter().enumerate())
        .filter(|(i, _)| !gone(*i))
        .map(|(_, p)| *p)
        .collect();
    kept.join("&").into_bytes()
}

fn is_a_cli_post(line: &str) -> bool {
    let mut words = line.split_whitespace();
    words.next() == Some("POST") && words.next() == Some("/dashboard/cli")
}

/// A command's words, its form's `args[]` in their order.
fn args_of(form: &str) -> Vec<String> {
    (form.split('&'))
        .filter_map(|pair| pair.split_once('='))
        .filter(|(key, _)| form_decoded(key) == "args[]")
        .map(|(_, value)| form_decoded(value))
        .collect()
}

/// A command's form at most: bin/dash sends its words, its folder and its whole environment.
const CLI_BODY: usize = 1024 * 1024;

/// `dash outbox`, answered here (task rust-daily-today): bin/dash posts every command to /dashboard/cli, read whole here when it
/// fits `CLI_BODY`. This one Rust answers as the older app's CLI does (`x-exit`, `x-stdout-bytes`: DashboardWeb.ApiController),
/// on loopback only with bin/dash's header and no forward, as that CLI refuses any other caller. Any other command goes on to
/// the older app, its body as read. False: nothing done, the request goes its usual way.
fn cli(
    stream: &TcpStream,
    head: &Head,
    older: Option<u16>,
    report: &Statsd,
    limit: Duration,
) -> bool {
    let named = |name: &str| {
        head.headers.iter().find_map(|h| {
            let (n, v) = h.split_once(':')?;
            n.trim().eq_ignore_ascii_case(name).then(|| v.trim())
        })
    };
    let Some(length) = named("content-length").and_then(|l| l.parse::<usize>().ok()) else {
        return false;
    };
    if length > CLI_BODY || head.rest.len() > length {
        return false;
    }
    let mut body = head.rest.clone();
    let mut more = vec![0; length - body.len()];
    let mut timed = ByDeadline {
        stream,
        deadline: Instant::now() + limit,
    };
    // curl asks before a body past 1 KB and waits a second for the answer: every dash command would wait it
    let expects = named("expect").is_some_and(|e| e.eq_ignore_ascii_case("100-continue"));
    if expects && !more.is_empty() && timed.write_all(b"HTTP/1.1 100 Continue\r\n\r\n").is_err() {
        return true;
    }
    if timed.read_exact(&mut more).is_err() {
        let _ = write!(
            timed,
            "HTTP/1.1 400 Bad Request\r\nContent-Type: {TEXT}\r\nContent-Length: 30\r\nConnection: close\r\n\r\nthe command's form ended early"
        );
        report.refused("400", "route_cli");
        return true;
    }
    body.extend(more);
    let form = String::from_utf8_lossy(&body);
    let args = args_of(&form);
    report.front_door("asked", "command", &fallback::command_tag(&args));
    let loopback = |a: std::io::Result<SocketAddr>| a.is_ok_and(|a| a.ip().is_loopback());
    let from_bin_dash = loopback(stream.local_addr())
        && loopback(stream.peer_addr())
        && named("x-dashboard-cli").is_some()
        && named("x-forwarded-for").is_none();
    // the command may take long (ci wait: up to three hours): the answer's own deadline starts after it (/code-review)
    let answered = |(exit, out, err): (i32, String, String)| {
        let mut timed = ByDeadline {
            stream,
            deadline: Instant::now() + limit,
        };
        let _ = write!(
            timed,
            "HTTP/1.1 200 OK\r\nContent-Type: {TEXT}\r\nx-exit: {exit}\r\nx-stdout-bytes: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{out}{err}",
            out.len(),
            out.len() + err.len()
        );
        true
    };
    if args == ["outbox"] && from_bin_dash {
        return answered(match drafts::outbox_lines() {
            Ok(lines) if lines.is_empty() => (0, lines, String::new()),
            Ok(lines) => (0, lines + "\n", String::new()),
            Err(e) => (1, String::new(), format!("outbox: {e}\n")),
        });
    }
    // a sandbox denial's rule, written in the main checkout this process runs in: a task session cannot write it there
    if let [sandbox, propose, rest @ ..] = &args[..]
        && (sandbox.as_str(), propose.as_str()) == ("sandbox", "propose")
        && from_bin_dash
    {
        return answered(sandbox::answer(rest));
    }
    if let [ci, wait, target] = &args[..]
        && (ci.as_str(), wait.as_str()) == ("ci", "wait")
        && from_bin_dash
    {
        let cwd = param(&form, "cwd").unwrap_or_default();
        return answered(ci_wait(stream, target, &cwd));
    }
    // A run Rust's orchestrator holds is closed here: the older app finds no pane of its own and answers « nothing runs it »
    // (rust-task-close-rust-runs). In a process of its own, as `dash task run`: the close waits on its agent's leave.
    let data = pages::serve_from_dir().ok();
    let close = match args.as_slice() {
        [task, close, id] if task == "task" && close == "close" => Some((id, false)),
        [task, close, id, after]
            if task == "task" && close == "close" && after == "--after-report" =>
        {
            Some((id, true))
        }
        _ => None,
    };
    if let (Some((id, after_report)), true, Some(data)) = (close, from_bin_dash, data)
        && tasks::holds_run(data, id)
    {
        // Its run closes its own pane once its report is judged: nothing waits here (/code-review of baa197583).
        if after_report {
            let said = format!(
                "{id}: its Rust run closes its pane once its report is judged; nothing more to do\n"
            );
            return answered((0, said, String::new()));
        }
        let why = "asked by dash task close";
        let ran = std::env::current_exe().and_then(|me| {
            let mut command = std::process::Command::new(me);
            command.args(["task", "close", "--data"]).arg(data);
            command.args([id.as_str(), why]).output()
        });
        return answered(match ran {
            Ok(ran) => (
                ran.status.code().unwrap_or(1),
                String::from_utf8_lossy(&ran.stdout).into_owned(),
                String::from_utf8_lossy(&ran.stderr).into_owned(),
            ),
            Err(e) => (1, String::new(), format!("task close: {e}\n")),
        });
    }
    // a lesson's fold is Rust's: the older app's `PM.fold/3` is never asked (task rust-fold-port)
    if let [task, run, folded, rest @ ..] = &args[..]
        && (task.as_str(), run.as_str(), folded.as_str()) == ("task", "run", "--folded")
        && from_bin_dash
    {
        return answered(folds::answer(rest, USAGE));
    }
    // a done report whose parts quote no proof of their own is bounced before the older app files it (task report-proof-quotes)
    if from_bin_dash
        && let Some(why) = reports::bounced(
            &args,
            &param(&form, "cwd").unwrap_or_default(),
            &tasks::older_db(),
        )
    {
        return answered((1, String::new(), format!("{why}\n")));
    }
    // A new task names its epic or is urgent with a deadline, and a Rust task its layer, or it is refused before the older app
    // files it; the older app knows no --layer or --deadline, so it goes on without them, Rust's store keeping each (tasks
    // rust-task-names-layer, rust-orch-pick-dynamic). Not from bin/dash: the older app refuses both flags anyway.
    // `load`: what its layer's open tasks are, said after the older app's answer (task rust-add-layer-load).
    let mut load = String::new();
    // the forward ended: nothing to file through the older app, so nothing kept for it either (/code-review)
    let ended = forward_ended();
    if from_bin_dash && !ended {
        // the RFCs read once, whether the layer check or the load asks; a done-when given as « - » checked as bin/dash sent
        // it, on stdin (/code-review)
        let rfc = std::cell::OnceCell::new();
        let sections = || {
            rfc.get_or_init(|| layers::rfc_sections(&tasks::repo()))
                .clone()
        };
        let stdin = param(&form, "stdin").unwrap_or_default();
        let checked: Vec<String> = (args.iter().enumerate())
            .map(
                |(at, a)| match at > 0 && a == "-" && args[at - 1] == "--done-when" {
                    true => stdin.clone(),
                    false => a.clone(),
                },
            )
            .collect();
        match layers::filing(&checked, &sections) {
            layers::Filing::Pass => {}
            layers::Filing::Refused(why) => {
                return answered((1, String::new(), format!("{why}\n")));
            }
            layers::Filing::Keep {
                id,
                layer,
                deadline,
                dry,
                update,
                owned,
                same_layer,
                args: kept,
            } => {
                let mut opened = pages::serve_from_dir().and_then(tasks::open_tasks);
                // the older app's list synced first, so each add filed since the last pass counts; a sync that fails leaves
                // the last list (/code-review: a burst of adds read none of their own)
                if let (Some(_), Ok(t)) = (&layer, &mut opened) {
                    let _ = tasks::sync_list(t, &tasks::older_db());
                }
                let queue = (opened.as_ref().map_err(String::clone)).and_then(|t| t.queue());
                // a crowded layer refuses it before anything is kept; a list never read says so and files it
                if let Some(layer) = &layer {
                    load = match &queue {
                        Ok(q) => {
                            let rfc = sections();
                            match layers::load(&id, layer, &q.tasks, &rfc, same_layer.as_deref()) {
                                Ok(said) => said,
                                Err(why) => {
                                    return answered((1, String::new(), format!("{why}\n")));
                                }
                            }
                        }
                        Err(e) => format!("{id}: its layer's open tasks could not be read: {e}\n"),
                    };
                }
                // kept first: a new task the older app then refuses leaves a layer no task reads, and the next add writes over
                // it. An add of a task listed already is a duplicate it refuses: the running task's layer stays (/code-review).
                let stored = (!dry).then(|| {
                    let mut tasks = opened?;
                    let listed = |q: &Queue| q.tasks.iter().any(|t| t.id == id);
                    if !update && queue.as_ref().is_ok_and(listed) {
                        return Ok(());
                    }
                    if let Some(layer) = &layer {
                        tasks.layer(&id, layer)?;
                    }
                    deadline
                        .as_ref()
                        .map_or(Ok(()), |day| tasks.deadline(&id, day))
                });
                if let Some(Err(e)) = stored {
                    return answered((
                        1,
                        String::new(),
                        format!("{id}: its layer or deadline was not kept: {e}\n"),
                    ));
                }
                let named = [("layer", &layer), ("deadline", &deadline)];
                // `task update <id> --layer <l>` alone: nothing left for the older app
                if update && kept.len() == 3 {
                    let said = if dry { "would keep" } else { "keeps" };
                    let what: Vec<String> = (named.iter())
                        .filter_map(|(name, v)| v.as_ref().map(|v| format!("its {name}, {v}")))
                        .collect();
                    let out = format!("{load}{id}: Rust's store {said} {}\n", what.join(" and "));
                    return answered((0, out, String::new()));
                }
                for (name, value) in named {
                    if let Some(value) = value {
                        body = without(&body, &format!("--{name}"), value);
                    }
                }
                // a new task naming a layer and no owner goes on `--owner rust` (task rust-layer-means-owner)
                if owned {
                    body.extend_from_slice(b"&args%5B%5D=--owner&args%5B%5D=rust");
                }
            }
        }
    }
    // `dash pr hold|release`: Rust's store holds each PR (task rust-pr-one-owner), so bin/dash's call never reaches the older app
    if from_bin_dash
        && let [pr, verb, rest @ ..] = &args[..]
        && pr == "pr"
        && ["hold", "release"].contains(&verb.as_str())
    {
        let Some(data) = data else {
            return answered((1, String::new(), "pr: no data folder here\n".into()));
        };
        let ran = std::env::current_exe().and_then(|me| {
            let mut command = std::process::Command::new(me);
            command
                .args(["pr", verb.as_str(), "--data"])
                .arg(data)
                .args(rest)
                .output()
        });
        return answered(match ran {
            Ok(ran) => (
                ran.status.code().unwrap_or(1),
                String::from_utf8_lossy(&ran.stdout).into_owned(),
                String::from_utf8_lossy(&ran.stderr).into_owned(),
            ),
            Err(e) => (1, String::new(), format!("pr: {e}\n")),
        });
    }
    // `dash orch next`: the next task and why, from the picker's own plan, in a process of its own as `task close` runs; `dash orch
    // route` the same way (RFC 20h: each demand's route, a row of Rust's store)
    let sub = args
        .get(1)
        .filter(|s| s.as_str() == "next" || s.as_str() == "route");
    let sub = sub.filter(|_| args[0] == "orch" && (args.len() == 2 || args[1] == "route"));
    if let (true, Some(sub)) = (from_bin_dash, sub) {
        let Some(data) = data else {
            return answered((
                1,
                String::new(),
                format!("orch {sub}: no data folder here\n"),
            ));
        };
        let ran = std::env::current_exe().and_then(|me| {
            let mut command = std::process::Command::new(me);
            command
                .args(["orch", sub, "--data"])
                .arg(data)
                .args(&args[2..])
                .output()
        });
        return answered(match ran {
            Ok(ran) => (
                ran.status.code().unwrap_or(1),
                String::from_utf8_lossy(&ran.stdout).into_owned(),
                String::from_utf8_lossy(&ran.stderr).into_owned(),
            ),
            Err(e) => (1, String::new(), format!("orch {sub}: {e}\n")),
        });
    }
    // `dash fallback [on|off]` from bin/dash, the switch's way back once nothing is forwarded (/code-review)
    if from_bin_dash
        && let [fallback, rest @ ..] = &args[..]
        && fallback == "fallback"
        && let Some(data) = data
    {
        let ran = std::env::current_exe().and_then(|me| {
            let mut command = std::process::Command::new(me);
            command
                .arg("fallback")
                .args(rest)
                .arg("--data")
                .arg(data)
                .output()
        });
        return answered(match ran {
            Ok(ran) => (
                ran.status.code().unwrap_or(1),
                String::from_utf8_lossy(&ran.stdout).into_owned(),
                String::from_utf8_lossy(&ran.stderr).into_owned(),
            ),
            Err(e) => (1, String::new(), format!("fallback: {e}\n")),
        });
    }
    // the forward ended: a command Rust lacks is « not built yet », its task filed once (task rust-fallback-off)
    if ended && let Some(dir) = data {
        let asked = fallback::Asked::Command { args };
        let old = asked.old_page(head.host.as_deref().unwrap_or_default());
        let filing = fallback::file_on(OLDER.get().copied());
        let said = fallback::not_built_within(dir, asked, old, filing, FILING_WAIT);
        return answered((1, String::new(), said));
    }
    // the body is read now: the usual way would answer a request whose body is gone (/code-review)
    let Some(port) = older else {
        let why = "no older app here to run this command";
        let _ = write!(
            timed,
            "HTTP/1.1 404 Not Found\r\nContent-Type: {TEXT}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{why}",
            why.len()
        );
        report.refused("404", "route_cli");
        return true;
    };
    // the body is read: the older app's own 100 Continue would reach the client after ours; a layer taken out shortened it
    let named_one_of = |h: &String, names: [&str; 2]| {
        let name = h.split(':').next().unwrap_or_default().trim();
        names.iter().any(|n| name.eq_ignore_ascii_case(n))
    };
    let mut headers: Vec<String> = (head.headers.iter())
        .filter(|h| !named_one_of(h, ["expect", "content-length"]))
        .cloned()
        .collect();
    headers.push(format!("Content-Length: {}\r\n", body.len()));
    let whole = Head {
        line: head.line.clone(),
        host: head.host.clone(),
        accept: head.accept.clone(),
        upgrade: head.upgrade,
        headers,
        rest: body,
    };
    if from_bin_dash && args == ["claude-accounts"] {
        accounts_with_models(stream, &whole, port, report);
        return true;
    }
    // a new task in an epic that waits on nothing: its answer says so after it (task rust-graph-why-these)
    let warned = from_bin_dash.then(|| layers::warning(&args)).flatten();
    if let Some(warned) = (warned.is_some() || !load.is_empty())
        .then(|| format!("{load}{}", warned.unwrap_or_default()))
    {
        // no read deadline, as the plain way: an add checks overlaps and may start its task through the load cap. Said only
        // after an add that filed (/code-review of rust-graph-why-these: a refused add was told to update a task it never made)
        with_older_lines(stream, &whole, port, report, None, |raw| {
            if exit_of(raw) == Some(0) {
                warned
            } else {
                String::new()
            }
        });
        return true;
    }
    to_the_older_app(stream, &whole, port, report);
    true
}

/// `dash claude-accounts`: the older app's answer, each login's own model allowances under its lines (RFC 20h: Fable's, task
/// rust-pools-fable). An answer it cannot read goes on as it came.
fn accounts_with_models(client: &TcpStream, head: &Head, port: u16, report: &Statsd) {
    with_older_lines(client, head, port, report, Some(ANSWER_DEADLINE), |_| {
        std::env::var("HOME").map_or_else(
            |e| format!("models: HOME: {e}\n"),
            |home| agents::model_lines(Path::new(&home)),
        )
    });
}

/// The older app's answer to bin/dash, the `lines` its answer calls for after its stdout (`with_lines`), read within `deadline`
/// (`None`: as long as it takes); a client that stops reading is cut, as the plain way cuts it.
/// ponytail: buffered, so a client gone mid-answer is seen only once the older app ends; `to_the_older_app`'s two threads if
/// a slow command ever needs it.
fn with_older_lines(
    client: &TcpStream,
    head: &Head,
    port: u16,
    report: &Statsd,
    deadline: Option<Duration>,
    lines: impl FnOnce(&[u8]) -> String,
) {
    let Some(upstream) = asked_of_the_older_app(client, head, port, report) else {
        return;
    };
    let _ = upstream.set_read_timeout(deadline);
    let mut raw = vec![];
    let _ = (&upstream).read_to_end(&mut raw);
    let _ = client.set_write_timeout(Some(ANSWER_DEADLINE));
    let _ = (&*client).write_all(&with_lines(&raw, &lines(&raw)));
}

/// bin/dash's exit code in the older app's answer, its `x-exit` header; `None` when it has none.
fn exit_of(raw: &[u8]) -> Option<i32> {
    let at = raw.windows(4).position(|w| w == b"\r\n\r\n")?;
    let head = std::str::from_utf8(&raw[..at]).ok()?;
    head.lines().find_map(|l| {
        let (name, value) = l.split_once(':')?;
        name.trim()
            .eq_ignore_ascii_case("x-exit")
            .then(|| value.trim().parse().ok())?
    })
}

/// `raw`, bin/dash's answer, with `lines` after its stdout, its x-stdout-bytes and Content-Length grown by them. Any other
/// answer (a refusal, a body of another length) as it is.
fn with_lines(raw: &[u8], lines: &str) -> Vec<u8> {
    let Some(at) = raw.windows(4).position(|w| w == b"\r\n\r\n") else {
        return raw.to_vec();
    };
    let (Ok(head), body) = (std::str::from_utf8(&raw[..at]), &raw[at + 4..]) else {
        return raw.to_vec();
    };
    let value = |name: &str| {
        let line = head.lines().find_map(|l| {
            l.split_once(':')
                .filter(|(n, _)| n.trim().eq_ignore_ascii_case(name))
        });
        line.and_then(|(_, v)| v.trim().parse::<usize>().ok())
    };
    let ok = head
        .lines()
        .next()
        .is_some_and(|l| l.split_whitespace().nth(1) == Some("200"));
    let (Some(out), Some(length), true) = (value("x-stdout-bytes"), value("content-length"), ok)
    else {
        return raw.to_vec();
    };
    if length != body.len() || out > length {
        return raw.to_vec();
    }
    let grown = |l: &str| match l.split_once(':') {
        Some((n, _)) if n.trim().eq_ignore_ascii_case("x-stdout-bytes") => {
            format!("{n}: {}", out + lines.len())
        }
        Some((n, _)) if n.trim().eq_ignore_ascii_case("content-length") => {
            format!("{n}: {}", length + lines.len())
        }
        _ => l.to_string(),
    };
    let head: Vec<String> = head.lines().map(grown).collect();
    let mut answer = format!("{}\r\n\r\n", head.join("\r\n")).into_bytes();
    answer.extend_from_slice(&body[..out]);
    answer.extend_from_slice(lines.as_bytes());
    answer.extend_from_slice(&body[out..]);
    answer
}

/// `dash ci wait <pr|sha>` (task ci-wait-gh-retry), on its connection's thread: a poll a minute through the merge hook of the
/// checkout `DASH_REPO` names, run in the caller's folder, as the older app ran it; a caller gone between two polls ends it, its
/// slot freed. `DASH_CI_POLL_MS` shortens the minute for a test.
fn ci_wait(stream: &TcpStream, target: &str, cwd: &str) -> (i32, String, String) {
    let cwd = Path::new(cwd);
    if !cwd.is_dir() {
        let why = format!(
            "ci wait: no folder {} to read the checks in\n",
            cwd.display()
        );
        return (2, String::new(), why);
    }
    let hook = tasks::repo().join(".claude/hooks/merge-claim.py");
    if !hook.is_file() {
        return (
            3,
            String::new(),
            format!("ci wait: no merge hook at {}\n", hook.display()),
        );
    }
    let every = std::env::var("DASH_CI_POLL_MS")
        .ok()
        .and_then(|ms| ms.parse().ok());
    let every = Duration::from_millis(every.unwrap_or(60_000));
    let listens = || {
        std::thread::sleep(every);
        // bin/dash sends nothing after its form: a read of 0 bytes, or a reset, is its socket closed (an interrupted dash)
        use std::io::ErrorKind;
        let mut byte = [0];
        let open = stream.set_nonblocking(true).is_ok()
            && match stream.peek(&mut byte) {
                Ok(n) => n > 0,
                Err(e) => matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::Interrupted),
            };
        let _ = stream.set_nonblocking(false);
        open
    };
    let read = || dash_forge::hook_rows(&hook, cwd, target);
    dash_forge::ci_wait(read, listens, Instant::now() + dash_forge::CI_DEADLINE)
}

/// The head the older app gets. Connection: close, or a browser would send its next request, a page of ours too, down the same
/// socket; a WebSocket keeps its own. A caller outside this Mac is named in X-Forwarded-For, any it wrote dropped: the older app
/// reads a loopback peer with that header as Tailscale Serve's forward (`Dashboard.Handoff.caller/2`, /approve's tap).
fn forwarded(head: &Head, peer: Option<IpAddr>) -> String {
    let outside = peer.filter(|ip| !ip.is_loopback());
    let mut text = head.line.clone();
    for header in &head.headers {
        let name = header
            .split(':')
            .next()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        let dropped = (outside.is_some() && name == "x-forwarded-for")
            || (!head.upgrade && matches!(name.as_str(), "connection" | "keep-alive"));
        if !dropped {
            text += header;
        }
    }
    if let Some(ip) = outside {
        text += &format!("X-Forwarded-For: {ip}\r\n");
    }
    if !head.upgrade {
        text += "Connection: close\r\n";
    }
    text + "\r\n"
}

/// The request to the older app on loopback, its head and the body's start sent; or the 502 its client gets.
fn asked_of_the_older_app(
    client: &TcpStream,
    head: &Head,
    port: u16,
    report: &Statsd,
) -> Option<TcpStream> {
    // every forward passes here, a page's, a command's from bin/dash or the bridge, and a browser's own fetches (task
    // rust-fallback-off); a command's body is read whole by then (`cli`)
    let (kind, what) = match (is_a_cli_post(&head.line), his_page(head)) {
        // a command's form unread (`cli` passed it on) was never counted asked: a fetch, so the share stays whole (/code-review)
        (true, _) => match args_of(&String::from_utf8_lossy(&head.rest)) {
            args if args.is_empty() => ("fetch", "dashboard".into()),
            args => ("command", fallback::command_tag(&args)),
        },
        (false, true) => ("page", fallback::page_tag(request_path(&head.line))),
        (false, false) => ("fetch", fallback::page_tag(request_path(&head.line))),
    };
    report.front_door("forwarded", kind, &what);
    let to = (Ipv4Addr::LOCALHOST, port).into();
    let upstream = TcpStream::connect_timeout(&to, Duration::from_secs(2)).and_then(|upstream| {
        let mut sent = &upstream;
        sent.write_all(forwarded(head, client.peer_addr().ok().map(|a| a.ip())).as_bytes())?;
        sent.write_all(&head.rest)?;
        Ok(upstream)
    });
    match upstream {
        Ok(upstream) => Some(upstream),
        Err(e) => {
            let body = format!("the older app on :{port} does not answer: {e}");
            let _ = write!(
                &mut &*client,
                "HTTP/1.1 502 Bad Gateway\r\nContent-Type: {TEXT}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            report.refused("502", alerts::OLDER_ROUTE);
            eprintln!("answered 502 on {}: {e}", alerts::OLDER_ROUTE);
            None
        }
    }
}

/// The view an older page is asked in: a page a browser opens at the top is `html`, and `?view=html|text|both` names one, as
/// on our pages (rust/CLAUDE.md rule 4). `None`: any other request, which goes on as bytes: a frame inside a page
/// (Sec-Fetch-Dest: iframe, as the older app's previews), a script's or a crawler's (no HTML asked).
fn older_view(head: &Head) -> Option<&'static str> {
    if head.upgrade || !head.line.starts_with("GET ") {
        return None;
    }
    let target = head.line.split_whitespace().nth(1).unwrap_or_default();
    let query = target.split_once('?').map_or("", |(_, q)| q);
    let dest = head.headers.iter().find_map(|h| {
        let (name, value) = h.split_once(':')?;
        name.trim()
            .eq_ignore_ascii_case("sec-fetch-dest")
            .then(|| value.trim().to_ascii_lowercase())
    });
    match view_named(query) {
        Some(view) => Some(view),
        None if head.accept.contains("text/html")
            && dest.as_deref().is_none_or(|d| d == "document") =>
        {
            Some("html")
        }
        None => None,
    }
}

/// An older page through the front door (task rust-nav-board; the orchestrator's answer, 2026-11-08: « Older pages too »):
/// asked as a browser asks, with no view of ours, no compression and no validator, so a 304 never meets a dressed page. Its
/// head is read first: only a page `older_dressable` takes is read whole and handed on with the menu, or as its words;
/// anything else streams on as it comes, as the plain byte pump does.
fn dressed_by_the_front_door(
    client: &TcpStream,
    head: &Head,
    port: u16,
    report: &Statsd,
    view: &str,
) {
    let target = head.line.split_whitespace().nth(1).unwrap_or_default();
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    if client.set_write_timeout(Some(ANSWER_DEADLINE)).is_err() {
        return;
    }
    let ours = |p: &&str| p.starts_with("view=") && view_named(p).is_some();
    let kept: Vec<&str> = query
        .split('&')
        .filter(|p| !p.is_empty() && !ours(p))
        .collect();
    let bare = if kept.is_empty() {
        path.to_string()
    } else {
        format!("{path}?{}", kept.join("&"))
    };
    let left_out = |h: &String| {
        let name = h
            .split(':')
            .next()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        matches!(
            name.as_str(),
            "accept" | "accept-encoding" | "if-none-match" | "if-modified-since"
        )
    };
    let mut headers: Vec<String> = head
        .headers
        .iter()
        .filter(|h| !left_out(h))
        .cloned()
        .collect();
    headers.push("Accept: text/html\r\n".into());
    let asked = Head {
        line: head.line.replacen(target, &bare, 1),
        host: head.host.clone(),
        accept: String::new(),
        upgrade: false,
        headers,
        rest: head.rest.clone(),
    };
    let Some(upstream) = asked_of_the_older_app(client, &asked, port, report) else {
        return;
    };
    let mut raw = vec![];
    let mut chunk = [0u8; 8192];
    let ends = loop {
        if let Some(at) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
            break Some(at);
        }
        match (&upstream).read(&mut chunk) {
            Ok(n) if n > 0 && raw.len() < REQUEST_LIMIT as usize => {
                raw.extend_from_slice(&chunk[..n])
            }
            _ => break None,
        }
    };
    let older = ends.and_then(|at| Some((at, std::str::from_utf8(&raw[..at]).ok()?.to_string())));
    let length = older
        .as_ref()
        .and_then(|(_, h)| surface::forms::older_dressable(h));
    if let (Some((at, older)), Some(length)) =
        (older, length.filter(|n| *n as u64 <= OLDER_PAGE_LIMIT))
    {
        let missing = (at + 4 + length).saturating_sub(raw.len());
        let _ = (&upstream).take(missing as u64).read_to_end(&mut raw);
        let body = std::str::from_utf8(&raw[at + 4..]).ok();
        let page = body.and_then(|body| {
            surface::forms::older_page(&older, body, path, &kept.join("&"), view != "html")
        });
        if let Some(page) = page {
            let _ = (&*client).write_all(&page);
            let _ = client.shutdown(std::net::Shutdown::Both);
            return;
        }
    }
    let _ = (&*client).write_all(&raw);
    let _ = std::io::copy(&mut &upstream, &mut &*client);
    let _ = client.shutdown(std::net::Shutdown::Both);
}

/// The most of an older page read whole to dress it: its pages run to a few hundred KB (/mail, 223 KB on 2026-11-08).
const OLDER_PAGE_LIMIT: u64 = 16 * 1024 * 1024;

/// The request to the older app on loopback, then its bytes both ways until either side ends: a body, a long answer and a
/// WebSocket alike; a browser's page is dressed with the menu first. ponytail: a WebSocket holds one of `CONNECTIONS` while
/// open; a cap of their own if tabs ever crowd it out.
fn to_the_older_app(client: &TcpStream, head: &Head, port: u16, report: &Statsd) {
    if let Some(view) = older_view(head) {
        return dressed_by_the_front_door(client, head, port, report, view);
    }
    let Some(upstream) = asked_of_the_older_app(client, head, port, report) else {
        return;
    };
    // No read deadline either way: bin/dash's long commands (ci wait, task run) stay silent for minutes, and a WebSocket for as
    // long as it lives. The older app closes a plain answer itself (Connection: close). A client that stops reading is cut.
    let (Ok(()), Ok(()), Ok(()), Ok(up), Ok(down)) = (
        client.set_read_timeout(None),
        client.set_write_timeout(Some(ANSWER_DEADLINE)),
        upstream.set_read_timeout(None),
        client.try_clone(),
        upstream.try_clone(),
    ) else {
        return;
    };
    let sending = std::thread::Builder::new().spawn(move || {
        let _ = std::io::copy(&mut &up, &mut &down);
        // the client gone (or done) ends the upstream side too: a hung older app never holds this slot past its client
        let _ = down.shutdown(std::net::Shutdown::Both);
    });
    let _ = std::io::copy(&mut &upstream, &mut &*client);
    // the client's read above ends here, whether it closed or not
    let _ = client.shutdown(std::net::Shutdown::Both);
    let _ = upstream.shutdown(std::net::Shutdown::Both);
    if let Ok(sending) = sending {
        let _ = sending.join();
    }
}

/// `dash ui`: the app in the Mac window (task rust-window-in-process; @nha, 2026-11-09: « the native app will probably be the
/// real thing for end users. browser is useful in dev. »): each page `window_page` renders, so it opens with nothing on any port.
fn ui(dir: PathBuf) {
    pages::serve_from(&dir);
    let (board, changes) = tasks::Board::open(&dir)
        .and_then(|board| board.patches().map(|changes| (board, changes)))
        .unwrap_or_else(|e| {
            eprintln!("dash ui: {e}");
            std::process::exit(1);
        });
    #[cfg(feature = "window")]
    dash_ui::open_pages(
        tasks::BOARD_PAGE,
        "dash",
        window_pages(board),
        follows_views,
        changes,
    );
    #[cfg(not(feature = "window"))]
    {
        let _ = (window_pages(board), changes, follows_views("", ""));
        eprintln!("dash ui: this dash was built without its window: cargo build --features window");
        std::process::exit(2);
    }
}

/// Whether the window's page follows the board view, so it is drawn again only on a diff, as a patch (task rust-view-patches):
/// /tasks' board, once no skeleton is left on it; until then it is rendered again as any page that reads the store.
/// ponytail: the board's own columns alone; its other parts (what runs, the older app's lines) show anew on the next diff.
fn follows_views(target: &str, page: &str) -> bool {
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    path == tasks::BOARD_PAGE
        && TaskView::of(param(query, "layout").as_deref()) == TaskView::Board
        && !page.contains(BEING_READ)
}

/// The pages that read the store, registered with `redraw`, rendered again as the window asks; any other is rendered once a
/// visit: /mail's or /agents' reads cost a server or a terminal, and dash serve too reads them only when asked.
fn reads_the_store(target: &str) -> bool {
    let path = target.split('?').next().unwrap_or_default();
    pages::find(path).is_some_and(|p| p.redraw)
}

/// The window's render: `window_page`, kept for a page that reads no store until another page is asked, unless `refill` says
/// its skeleton is drawn again.
fn window_pages(board: tasks::Board) -> impl FnMut(&str) -> String + Send + 'static {
    // the page shown, when its visit began, when it was last drawn
    let mut kept: Option<(String, String, Instant, Instant)> = None;
    move |target| {
        let now = Instant::now();
        match &mut kept {
            Some((at, page, _, _)) if at == target && reads_the_store(target) => {
                *page = window_page(target, &board);
                page.clone()
            }
            Some((at, page, began, drawn)) if at == target => {
                if refill(page, now - *began, now - *drawn) {
                    *page = window_page(target, &board);
                    *drawn = now;
                }
                page.clone()
            }
            _ => {
                let page = window_page(target, &board);
                kept = Some((target.to_string(), page.clone(), now, now));
                page
            }
        }
    }
}

/// Whether a page that reads no store is drawn again: the window runs no script, so it fills a skeleton as the browser's
/// LIVE_JS does, once a second while one is on the page, for five minutes a visit (/design's catalog keeps one for good).
fn refill(page: &str, visit: Duration, since: Duration) -> bool {
    page.contains(BEING_READ) && since >= Duration::from_secs(1) && visit < Duration::from_secs(300)
}

/// What a page holds while a read of the older app is still on its thread: a skeleton.
const BEING_READ: &str = surface::components::BUSY;

/// `dash ui --shot <page> <png>`: that page of the window painted into a PNG, no window opened, once the board's reads landed
/// or 10 s passed, as the window shows it then.
fn ui_shot(dir: PathBuf, target: &str, png: &Path) {
    pages::serve_from(&dir);
    let shot = tasks::Board::open(&dir).and_then(|board| {
        let until = Instant::now() + Duration::from_secs(10);
        let mut page = window_page(target, &board);
        while page.contains(BEING_READ) && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(250));
            page = window_page(target, &board);
        }
        painted(page, png)
    });
    if let Err(e) = shot {
        eprintln!("dash ui: {e}");
        std::process::exit(1);
    }
}

/// `dash ui --check <png> [--stop-front-door]`: done-when 1 of rust-window-in-process, the window on his screen with nothing on
/// :7788, run from his terminal (a task's sandbox can neither stop launchd's front door nor read the screen). It refuses while
/// anything listens on :7788, or with --stop-front-door boots launchd's job out for the run and back in after; then it opens
/// `dash ui`, times it to its window in the macOS window list (10 s at most), captures that window and closes it.
fn ui_check(png: &Path, stop_front_door: bool) {
    // from the start, before any bootout: a Ctrl-C ends the window alone (it shares the terminal's process group), never this
    // process before FrontDoor's Drop loads the front door back
    unsafe { libc::signal(libc::SIGINT, libc::SIG_IGN) };
    let said = FrontDoor::stopped(stop_front_door).and_then(|_door| window_check(png));
    match said {
        Ok(line) => println!("dash ui --check: {line}"),
        Err(why) => {
            eprintln!("dash ui --check: {why}");
            std::process::exit(1);
        }
    }
}

fn window_check(png: &Path) -> Result<String, String> {
    if let Some(holder) = on_7788() {
        return Err(format!(
            "{holder} listens on :7788: stop it first, or add --stop-front-door"
        ));
    }
    let began = Instant::now();
    let me = std::env::current_exe().map_err(|e| format!("this dash: {e}"))?;
    let child = std::process::Command::new(me).arg("ui").spawn();
    let mut window = Window(child.map_err(|e| format!("dash ui: {e}"))?);
    let (id, size, asked, seen) = loop {
        if let Ok(Some(status)) = window.0.try_wait() {
            return Err(format!("dash ui ended before its window showed: {status}"));
        }
        let asked = began.elapsed();
        match on_screen(window.0.id())? {
            Some((id, size)) => break (id, size, asked, began.elapsed()),
            None if began.elapsed() > Duration::from_secs(10) => {
                return Err("no window on screen within 10 s".into());
            }
            None => std::thread::sleep(Duration::from_millis(100)),
        }
    };
    // its first page drawn, then :7788 asked again, so the window started nothing there either, then the picture
    std::thread::sleep(Duration::from_secs(2));
    if let Some(holder) = on_7788() {
        return Err(format!("{holder} took :7788 while the window ran"));
    }
    let captured = std::process::Command::new("/usr/sbin/screencapture")
        .args(["-o", "-x", "-l", &id])
        .arg(png)
        .status();
    if !captured.is_ok_and(|s| s.success()) {
        return Err(format!(
            "window {id} on screen, but screencapture failed (Screen Recording granted to this terminal?)"
        ));
    }
    Ok(format!(
        "nothing on :7788, window {id} {size} on screen after {:.2} to {:.2} s (the window list asked then answered), captured {}; closed",
        asked.as_secs_f64(),
        seen.as_secs_f64(),
        png.display()
    ))
}

/// The check's `dash ui`, closed however the check ends, a panic included.
struct Window(std::process::Child);

impl Drop for Window {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// launchd's job that keeps the front door on :7788 (Makefile's rust/serve), with KeepAlive: a kill alone restarts it. The bare
/// name is what launch.request takes (comment-wake matches it in sandbox/jobs.conf), the label what launchctl takes.
const FRONT_JOB: &str = "rust-serve";
const FRONT_LABEL: &str = "com.nicolasha.rust-serve";

/// The front door booted out for the check, bootstrapped again when dropped, a panic or an early return included. It boots
/// nothing out it cannot load back: a sandboxed launchctl bootstraps nothing (« Bootstrap failed: 5 », the orchestrator's run
/// of 2026-11-09 left :7788 dark), so a sandbox or an unreadable plist refuses first; a bootstrap that fails anyway asks
/// comment-wake, on the host, through .claude/run/launch.request.
struct FrontDoor(Option<(String, PathBuf)>);

impl FrontDoor {
    fn stopped(stop: bool) -> Result<FrontDoor, String> {
        if !stop {
            return Ok(FrontDoor(None));
        }
        let refused = |why: String| {
            Err(format!(
                "--stop-front-door cannot load {FRONT_LABEL} back ({why}), so nothing was stopped; run it from his terminal"
            ))
        };
        let home = match std::env::var("HOME") {
            Ok(home) if Path::new(&home).is_absolute() => home,
            _ => return refused("HOME is not an absolute path".into()),
        };
        let plist = Path::new(&home).join(format!("Library/LaunchAgents/{FRONT_LABEL}.plist"));
        if let Some(why) = cannot_bootstrap() {
            return refused(why.into());
        }
        if let Err(e) = File::open(&plist) {
            return refused(format!("{}: {e}", plist.display()));
        }
        let domain = format!("gui/{}", unsafe { libc::getuid() });
        // armed before the bootout: a bootout that errs while launchd still unloads the job is loaded back on the way out too
        let door = FrontDoor(Some((domain.clone(), plist)));
        launchctl(&["bootout", &format!("{domain}/{FRONT_LABEL}")])?;
        let until = Instant::now() + Duration::from_secs(10);
        while on_7788().is_some() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(200));
        }
        Ok(door)
    }
}

impl Drop for FrontDoor {
    fn drop(&mut self) {
        let Some((domain, plist)) = &self.0 else {
            return;
        };
        // a bootout launchd has not finished refuses a bootstrap (« already in progress », 5): tried again for 10 s, so the
        // request below is written only once the job is gone (comment-wake skips a job launchctl print still finds)
        let bootstrap = || launchctl(&["bootstrap", domain, &plist.to_string_lossy()]);
        let until = Instant::now() + Duration::from_secs(10);
        let mut tried = bootstrap();
        while tried.is_err() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(500));
            tried = bootstrap();
        }
        let Err(e) = tried else {
            eprintln!("dash ui --check: the front door is back ({FRONT_LABEL})");
            return;
        };
        let request = tasks::repo().join(".claude/run/launch.request");
        let asked = std::fs::create_dir_all(request.parent().expect("a folder")).and_then(|()| {
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&request)?;
            writeln!(file, "{FRONT_JOB}")
        });
        let by_hand = format!("launchctl bootstrap {domain} {}", plist.display());
        match asked {
            Ok(()) => eprintln!(
                "dash ui --check: the front door did not come back ({e}); {FRONT_JOB} written to {}, comment-wake loads it within a minute, else: {by_hand}",
                request.display()
            ),
            Err(w) => eprintln!(
                "dash ui --check: the front door did not come back ({e}), nor could {} be written ({w}): {by_hand}",
                request.display()
            ),
        }
    }
}

/// Why this process cannot run launchctl bootstrap, if it cannot: a macOS sandbox (sandbox_check of libSystem, no operation
/// named) answers « Bootstrap failed: 5 », and launchd is macOS's alone.
#[cfg(target_os = "macos")]
fn cannot_bootstrap() -> Option<&'static str> {
    unsafe extern "C" {
        fn sandbox_check(
            pid: libc::pid_t,
            operation: *const libc::c_char,
            kind: libc::c_int,
            ...
        ) -> libc::c_int;
    }
    // 1 sandboxed, 0 not; an error (-1) is not taken for a sandbox, so his terminal is never refused on it
    let sandboxed = unsafe { sandbox_check(libc::getpid(), std::ptr::null(), 0) > 0 };
    sandboxed.then_some("this process is sandboxed: launchctl bootstrap fails there")
}

#[cfg(not(target_os = "macos"))]
fn cannot_bootstrap() -> Option<&'static str> {
    Some("launchd is macOS's")
}

fn launchctl(args: &[&str]) -> Result<(), String> {
    let out = std::process::Command::new("/bin/launchctl")
        .args(args)
        .output()
        .map_err(|e| format!("launchctl: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!(
            "launchctl {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

/// Who listens on :7788 on any address, as lsof names it; with no lsof, "something" when 127.0.0.1:7788 answers.
fn on_7788() -> Option<String> {
    let lsof = std::process::Command::new("lsof")
        .args(["-nP", "-iTCP:7788", "-sTCP:LISTEN", "-Fpc"])
        .output();
    if let Ok(out) = lsof {
        let text = String::from_utf8_lossy(&out.stdout);
        let field = |tag| {
            text.lines()
                .find_map(|l| l.strip_prefix(tag))
                .map(str::to_string)
        };
        if let Some(pid) = field('p') {
            return Some(format!("{}, pid {pid},", field('c').unwrap_or_default()));
        }
    }
    let front = SocketAddr::from(([127, 0, 0, 1], 7788));
    TcpStream::connect_timeout(&front, Duration::from_millis(500))
        .is_ok()
        .then(|| "something".to_string())
}

/// The id and size of `pid`'s first window on screen, from the macOS window list (osascript's JavaScript reads CoreGraphics);
/// an osascript that fails says why, never taken for a window not shown yet.
fn on_screen(pid: u32) -> Result<Option<(String, String)>, String> {
    const LIST: &str = r#"ObjC.import("CoreGraphics");
function run(argv) {
  const all = ObjC.deepUnwrap(ObjC.castRefToObject($.CGWindowListCopyWindowInfo($.kCGWindowListOptionOnScreenOnly, 0)));
  const w = all.find(w => w.kCGWindowOwnerPID === parseInt(argv[0]) && w.kCGWindowLayer === 0);
  return w ? `${w.kCGWindowNumber} ${w.kCGWindowBounds.Width}x${w.kCGWindowBounds.Height}` : "";
}"#;
    let out = std::process::Command::new("/usr/bin/osascript")
        .args(["-l", "JavaScript", "-e", LIST, &pid.to_string()])
        .output()
        .map_err(|e| format!("osascript: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "osascript: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let line = String::from_utf8_lossy(&out.stdout);
    Ok(line
        .trim()
        .split_once(' ')
        .map(|(id, size)| (id.to_string(), size.to_string())))
}

#[cfg(feature = "window")]
fn painted(html: String, png: &Path) -> Result<(), String> {
    dash_ui::shot(html, 1280, 860, png)
}

#[cfg(not(feature = "window"))]
fn painted(_: String, png: &Path) -> Result<(), String> {
    Err(format!(
        "{}: this dash was built without its window",
        png.display()
    ))
}

/// A page as the window draws it: the very HTML `forms` gives a browser at `target`, sidebar and style included, rendered here.
/// An answer that is not HTML (a page's text view, a 404, an older app's page) is the same sidebar over its words.
fn window_page(target: &str, board: &tasks::Board) -> String {
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let page = pages::find(path);
    let (status, kind, body) = match page {
        Some(page) => page_answer(page, "text/html", (path, query), Some(board), None),
        None => ("404 Not Found", TEXT, "no such path".into()),
    };
    if kind == HTML {
        return body;
    }
    let tree = if page.is_some() {
        json!({"root": "page", "elements": {
            "page": {"type": "stack", "children": ["status", "said"]},
            "status": {"type": "heading", "props": {"text": status}},
            "said": {"type": "quote", "props": {"text": body}},
        }})
    } else {
        // the older app's: a card that opens it at the front door, in the browser
        let line = "The older app's page: the window draws Rust's pages, the browser the rest.";
        let href = format!("{FRONT_DOOR}{target}");
        json!({"root": "page", "elements": {
            "page": {"type": "stack", "children": ["status", "cards"]},
            "status": {"type": "heading", "props": {"text": path}},
            "cards": {"type": "list", "children": ["older"]},
            "older": {"type": "card", "props": {"title": "Open it in the browser", "line": line, "href": href}},
        }})
    };
    let page = surface::screen::page(status, &tree.to_string()).unwrap_or_else(|e| e);
    surface::forms::with_menu(&page, path, query)
}

/// `dash ui --design`: /design in the Mac window, the component dash serve renders there (task rust-design-page).
fn design_window() {
    #[cfg(feature = "window")]
    dash_ui::open(
        surface::design::Design,
        surface::design::DesignProps::default(),
        "Design system",
    );
    #[cfg(not(feature = "window"))]
    {
        eprintln!("dash ui: this dash was built without its window: cargo build --features window");
        std::process::exit(2);
    }
}

/// The store's hits, in the order the rerank hooks leave them.
fn search(query: &str, db: &dyn Store, registry: &Registry) -> Result<Vec<Hit>, String> {
    let (hits, refused) = registry.rerank(query, db.search(query, search::LIMIT)?);
    refused.iter().for_each(|r| eprintln!("{r}"));
    Ok(hits)
}

/// /inbox's search (`in=inbox`): the serial route answers it with the store's reranked hits, its results on /inbox; any other
/// /search is the search page's, on its connection's thread (task rust-search-port).
fn inbox_search(query: &str) -> bool {
    param(query, "in").as_deref() == Some(tasks::INBOX_SEARCH)
}

/// The page registered at `path`, but for /inbox's search, which the serial route answers.
fn page_of(path: &str, query: &str) -> Option<&'static pages::Page> {
    pages::find(path).filter(|_| !(path == search::PAGE && inbox_search(query)))
}

/// The older app's link to a site_perso task, `/projects/site_perso?issue=<id>` or `/projects/site_perso/issue/<id>`: its id.
fn old_issue(path: &str, query: &str) -> Option<String> {
    let id = match path.strip_prefix("/projects/site_perso")? {
        "" | "/" => param(query, "issue"),
        // a path keeps its +, as /tasks/<id>'s
        rest => (rest.strip_prefix("/issue/")).map(|id| form_decoded(&id.replace('+', "%2B"))),
    };
    id.filter(|id| !id.is_empty())
}

/// A registered page's answer in the view asked, its HTML in the menu's frame (rust/CLAUDE.md rule 4).
fn page_answer(
    page: &pages::Page,
    accept: &str,
    (path, query): (&str, &str),
    board: Option<&tasks::Board>,
    registry: Option<&Registry>,
) -> Response {
    dressed(accept, path, query, |html| {
        let board = board.map(|b| b as &dyn std::any::Any);
        let asked = pages::Asked {
            path,
            query,
            html,
            board,
            registry,
        };
        (page.serve)(&asked)
    })
}

/// A page's answer in the view asked, its HTML in the menu's frame; `page` answers HTML or its text, `html` asking which.
fn dressed(accept: &str, path: &str, query: &str, page: impl FnOnce(bool) -> Response) -> Response {
    let html = wants_html(accept, query);
    match page(html) {
        // the page's comments in its rail, each with its answer, in its HTML and its text (task rust-comment-answer-on-page)
        (status, HTML, body) => {
            let threads = comments::threads_on(path);
            (
                status,
                HTML,
                surface::forms::with_menu_and(&body, path, query, &threads),
            )
        }
        // a page's text, never a redirect's Location or an error's words; the menu last, as `dash menu` prints it (task
        // rust-menu-text)
        (status, TEXT, body) if status.starts_with("200") => {
            let threads = comments::threads_on(path);
            let menu = surface::forms::menu_text();
            (
                status,
                TEXT,
                body + &comments::threads_text(&threads) + "\n" + menu,
            )
        }
        answer => answer,
    }
}

/// The serial route: what reads the store or the registry, one at a time on `serve`'s thread.
fn route(line: &str, accept: &str, db: &dyn Store, registry: &Registry) -> Response {
    let failed = |e| ("500 Internal Server Error", TEXT, e);
    let mut words = line.split_whitespace();
    let (method, target) = (words.next(), words.next().unwrap_or_default());
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    // a page that reads the registry, as /today, routes here (pages::Reads::Registry)
    if method == Some("GET")
        && let Some(page) = page_of(path, query)
    {
        return page_answer(page, accept, (path, query), None, Some(registry));
    }
    match (method, path) {
        (Some("GET"), "/scan") => match db.scan() {
            Ok(s) => ("200 OK", TEXT, format!("{} {}", s.rows, s.bytes)),
            Err(e) => failed(e),
        },
        // only /inbox's field reaches here (`page_of`): any other /search is the search page's
        (Some("GET"), p) if p == search::PAGE => {
            let words = param(query, "q").unwrap_or_default();
            let hits = search(&words, db, registry);
            // a failed search is said in /inbox's Results, its field kept (/code-review)
            let page = tasks::inbox_page(Some((&words, hits)));
            let data = view_named(query) == Some("data");
            dressed(accept, tasks::INBOX_PAGE, query, |html| {
                drawn(html, data, Some(Ok(page)))
            })
        }
        (Some("GET"), slot) if slot.starts_with("/ui/") => {
            let name = &slot["/ui/".len()..];
            match registry.ui.get(name) {
                Some(tree) => {
                    let drawing = surface::Drawing::Tree {
                        title: name.to_string(),
                        tree: tree.to_string(),
                    };
                    let data = view_named(query) == Some("data");
                    let drawing = if data {
                        drawing
                    } else {
                        surface::queried(drawing, query)
                    };
                    drawn(wants_html(accept, query), data, Some(Ok(drawing)))
                }
                None => ("404 Not Found", TEXT, "no such slot".into()),
            }
        }
        _ => ("404 Not Found", TEXT, NO_SUCH_PATH.into()),
    }
}

/// Whether this front door's forward has ended (`dash fallback off`): only a server with an older app has one to end (/code-review:
/// a plain `dash serve` on a folder switched off filed tasks it could not file). ponytail: the switch's file read on each request,
/// a few µs; held in memory with its mtime if requests ever crowd.
fn forward_ended() -> bool {
    OLDER.get().is_some() && pages::serve_from_dir().is_ok_and(fallback::ended)
}

/// How long a « not built yet » waits on its filing before answering « being filed » (rust/CLAUDE.md rule 5).
const FILING_WAIT: Duration = Duration::from_secs(3);

/// Rust's answer to a path it routes nowhere: the older app's to answer while the forward lasts, « not built yet » after.
const NO_SUCH_PATH: &str = "no such path";

/// A page he asked, a browser's at its top or an agent's: never a browser's own fetch (an asset, a frame, a live update), which
/// the page it serves asked for. What the front door counts as asked, and files once the forward has ended.
fn his_page(head: &Head) -> bool {
    let dest = head.headers.iter().find_map(|h| {
        let (name, value) = h.split_once(':')?;
        name.trim()
            .eq_ignore_ascii_case("sec-fetch-dest")
            .then(|| value.trim().to_ascii_lowercase())
    });
    head.line.starts_with("GET ")
        && !head.upgrade
        && dest.as_deref().is_none_or(|d| d == "document")
        && !asked_by_a_browser_itself(&head.line)
        // a program's probe, never his page: comment-wake's poll of /ping alone was thousands a week (/code-review)
        && !matches!(request_path(&head.line), "/ping" | "/memory" | "/scan" | "/built")
        && !asks_patches(&head.line)
        && terminal::follows(&head.line).is_none()
}

/// A path Rust lacks once the forward has ended (task rust-fallback-off): his page files its tracer-bullet task once and links
/// the old page on :7799, drawn by the design system; any other request gets the words alone.
fn not_built(head: &Head) -> Response {
    let path = request_path(&head.line);
    let asked = fallback::Asked::Page {
        path: path.to_string(),
        line: head.line.clone(),
    };
    let old = asked.old_page(head.host.as_deref().unwrap_or_default());
    if !his_page(head) {
        return ("404 Not Found", TEXT, format!("not built yet: {path}\n"));
    }
    let Ok(dir) = pages::serve_from_dir() else {
        return ("404 Not Found", TEXT, format!("not built yet: {path}\n"));
    };
    let filing = fallback::file_on(OLDER.get().copied());
    let said = fallback::not_built_within(dir, asked, old.clone(), filing, FILING_WAIT);
    let target = head.line.split_whitespace().nth(1).unwrap_or_default();
    let query = target.split_once('?').map_or("", |(_, q)| q);
    let drawing = surface::Drawing::Tree {
        title: "Not built yet".into(),
        tree: fallback::tree(&said, &old),
    };
    let html = wants_html(&head.accept, query);
    let (_, kind, body) = drawn(html, view_named(query) == Some("data"), Some(Ok(drawing)));
    ("404 Not Found", kind, body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::memory::TestBudget;
    use std::net::TcpListener;

    /// Task rust-automations: his switch posts its name, decoded; a GET or another path is not it, and it is this server's own.
    #[test]
    fn his_pause_switch_posts_the_name_it_flips() {
        let _budget = TestBudget::new();
        let post = "POST /watchers/pause?name=comment+queue HTTP/1.1";
        assert_eq!(pause_post(post).as_deref(), Some("comment queue"));
        assert!(served_here(post), "Rust answers it, never the older app");
        assert_eq!(
            pause_post("GET /watchers/pause?name=morning-run HTTP/1.1"),
            None
        );
        assert_eq!(pause_post("POST /watchers?name=morning-run HTTP/1.1"), None);
        assert_eq!(pause_post("POST /watchers/pause HTTP/1.1"), None);
    }

    /// Task rust-board-buttons-new-issue, done-when 2: the board's New issue form files its title and description through
    /// `dash task file` on the board's project, then goes back to that board; no title, or another site's post, files nothing.
    #[test]
    fn a_new_issue_form_files_its_words_and_goes_back_to_its_board() {
        let _budget = TestBudget::new();
        let dir = no_app("new-issue");
        let said = dir.0.join("said");
        let older = dir.0.join("older-dash");
        let script = format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\necho filed\n",
            said.display()
        );
        std::fs::write(&older, script).expect("the older app");
        std::fs::set_permissions(&older, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .expect("chmod");
        unsafe { std::env::set_var("DASH_CLI", &older) };
        let post = |form: &str, origin: &str| {
            let request = format!(
                "POST /tasks/new HTTP/1.1\r\nHost: localhost\r\nOrigin: {origin}\r\nContent-Length: {}\r\n\r\n{form}",
                form.len()
            );
            answered(
                |c| c.write_all(request.as_bytes()).expect("write"),
                Duration::from_secs(10),
            )
            .0
        };
        assert!(served_here("POST /tasks/new HTTP/1.1"), "Rust answers it");

        let filed = post(
            "title=Fix+the+toggle&description=its+pill+group&project=klar",
            "http://localhost",
        );
        assert!(
            filed.starts_with("HTTP/1.1 303 See Other\r\nLocation: /tasks?project=klar\r\n"),
            "{filed}"
        );
        let args = std::fs::read_to_string(&said).expect("dash was asked");
        assert_eq!(args, "task\nfile\nklar\nFix the toggle\n\nits pill group\n");

        std::fs::remove_file(&said).expect("reset");
        let bare = post("title=Only+a+title", "http://localhost");
        assert!(
            bare.starts_with("HTTP/1.1 303 See Other\r\nLocation: /tasks\r\n"),
            "{bare}"
        );
        let args = std::fs::read_to_string(&said).expect("dash was asked");
        assert_eq!(args, "task\nfile\nsite_perso\nOnly a title\n");

        std::fs::remove_file(&said).expect("reset");
        for (form, origin, status) in [
            ("title=&description=x", "http://localhost", "400"),
            ("title=--on+2026-11-10", "http://localhost", "400"),
            ("title=x&project=--on", "http://localhost", "400"),
            ("title=x&project=../klar", "http://localhost", "400"),
            ("title=x", "http://elsewhere", "403"),
        ] {
            let refused = post(form, origin);
            assert!(
                refused.starts_with(&format!("HTTP/1.1 {status} ")),
                "{form}: {refused}"
            );
        }
        assert!(!said.exists(), "nothing filed");
        // a refusal gives his words back, so nothing he typed is lost
        let kept = post(
            "title=-+bullets&description=a+long+thought",
            "http://localhost",
        );
        assert!(
            kept.ends_with("Your words:\n- bullets\n\na long thought\n"),
            "{kept}"
        );
    }

    /// A sidebar menu's post on `a-task` (task rust-issue-sidebar-menus), against an older app's list holding it, owned by
    /// Rust, labelled Docs, in project rust-poc; another task's epic is crm. Each answer, and what dash was asked, a line a word.
    fn sidebar_posts(name: &str, forms: &[&str]) -> Vec<(String, String)> {
        let dir = no_app(name);
        let said = dir.0.join("said");
        let older = dir.0.join("older-dash");
        let script = format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\necho set\n",
            said.display()
        );
        std::fs::write(&older, script).expect("the older app");
        std::fs::set_permissions(&older, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .expect("chmod");
        unsafe { std::env::set_var("DASH_CLI", &older) };
        let db = tasks::older_db();
        std::fs::create_dir_all(db.parent().expect("its folder")).expect("its folder");
        let list = dash_store::Connection::open(&db).expect("the list");
        list.execute_batch(
            "CREATE TABLE tasks (collection TEXT, pos INTEGER, id TEXT, stage TEXT, data TEXT NOT NULL, owner TEXT,
               epic TEXT, cycle TEXT);
             INSERT INTO tasks VALUES ('tasks.edn', 1, 'a-task', 'open', '{\"labels\":[\"Docs\"]}', 'rust', 'rust-poc', NULL);
             INSERT INTO tasks VALUES ('tasks.edn', 2, 'b-task', 'done', '{}', NULL, 'crm', NULL);",
        )
        .expect("its rows");
        let post = |path: &str, form: &str| {
            let request = format!(
                "POST {path} HTTP/1.1\r\nHost: localhost\r\nOrigin: http://localhost\r\nContent-Length: {}\r\n\r\n{form}",
                form.len()
            );
            answered(
                |c| c.write_all(request.as_bytes()).expect("write"),
                Duration::from_secs(10),
            )
            .0
        };
        let mut answers = vec![];
        for form in forms {
            let _ = std::fs::remove_file(&said);
            let (path, form) = form.split_once(' ').unwrap_or(("/tasks/a-task/set", form));
            let answer = post(path, form);
            let asked = std::fs::read_to_string(&said)
                .unwrap_or_default()
                .replace('\n', " ");
            answers.push((
                answer.lines().next().unwrap_or("").to_string(),
                asked.trim().to_string(),
            ));
        }
        answers
    }

    const SET: &str = "HTTP/1.1 303 See Other";
    const REFUSED: &str = "HTTP/1.1 400 Bad Request";

    /// Task rust-issue-sidebar-menus, done-when 2: the assignee menu sets the task's owner, one of the list's; another is refused.
    #[test]
    fn the_assignee_menu_sets_its_owner() {
        let _budget = TestBudget::new();
        let got = sidebar_posts("side-assignee", &["assignee=nicolas", "assignee=bob"]);
        assert_eq!(
            got[0],
            (SET.into(), "task update a-task --owner nicolas".into())
        );
        assert_eq!(got[1], (REFUSED.into(), String::new()));
    }

    /// Task rust-issue-sidebar-menus, done-when 2: a label turns on beside the ones it holds, off as asked, the last off clears
    /// them; on for a label it holds already keeps it (/code-review of PR 1399: never a flip of what is stored). A pick with
    /// no state, an agent's label, a word of no set, or a task the list does not hold is refused.
    #[test]
    fn the_labels_menu_turns_one_label_on_or_off() {
        let _budget = TestBudget::new();
        let got = sidebar_posts(
            "side-labels",
            &[
                "labels=Bug&on=1",
                "labels=Docs&on=0",
                "labels=Docs&on=1",
                "labels=Bug",
                "labels=Claude&on=1",
                "labels=site_perso&on=1",
                "/tasks/z-task/set labels=Bug&on=1",
            ],
        );
        let asked = |words: &str| (SET.to_string(), format!("task update a-task {words}"));
        assert_eq!(got[0], asked("--labels Docs --labels Bug"));
        assert_eq!(got[1], asked("--labels nil"));
        assert_eq!(got[2], asked("--labels Docs"), "held already: kept on");
        for refused in &got[3..] {
            assert_eq!(*refused, (REFUSED.into(), String::new()));
        }
    }

    /// Task rust-issue-sidebar-menus, done-when 2: the project menu sets its epic, one the list holds, or none.
    #[test]
    fn the_project_menu_sets_its_epic() {
        let _budget = TestBudget::new();
        let got = sidebar_posts(
            "side-project",
            &["project=crm", "project=none", "project=nope"],
        );
        assert_eq!(got[0], (SET.into(), "task update a-task --epic crm".into()));
        assert_eq!(got[1], (SET.into(), "task update a-task --epic nil".into()));
        assert_eq!(got[2], (REFUSED.into(), String::new()));
    }

    /// Task rust-issue-sidebar-menus, done-when 2: the cycle menu sets this week's or the next through `dash store set`, or
    /// none; a cycle long past is refused.
    #[test]
    fn the_cycle_menu_sets_its_cycle() {
        let _budget = TestBudget::new();
        let today = tasks::local_today().expect("today");
        let monday = dash_core::date(today - tasks::weekday(today));
        let next = dash_core::date(today - tasks::weekday(today) + 7);
        let got = sidebar_posts(
            "side-cycle",
            &[
                &format!("cycle={monday}"),
                &format!("cycle={next}"),
                "cycle=none",
                "cycle=2020-01-06",
            ],
        );
        assert_eq!(
            got[0],
            (
                SET.into(),
                format!("store set tasks.edn a-task --cycle {monday}")
            )
        );
        assert_eq!(
            got[1],
            (
                SET.into(),
                format!("store set tasks.edn a-task --cycle {next}")
            )
        );
        assert_eq!(
            got[2],
            (SET.into(), "store set tasks.edn a-task --cycle nil".into())
        );
        assert_eq!(got[3], (REFUSED.into(), String::new()));
    }

    /// Task rust-issue-sidebar-menus, done-when 2: the « Remind me » menu sets its day through `dash store set`, or none.
    #[test]
    fn the_remind_menu_sets_its_day() {
        let _budget = TestBudget::new();
        let got = sidebar_posts(
            "side-remind",
            &["remind=2026-11-20", "remind=none", "remind=soon"],
        );
        assert_eq!(
            got[0],
            (
                SET.into(),
                "store set tasks.edn a-task --remind 2026-11-20".into()
            )
        );
        assert_eq!(
            got[1],
            (SET.into(), "store set tasks.edn a-task --remind nil".into())
        );
        assert_eq!(got[2], (REFUSED.into(), String::new()));
    }

    /// Task rust-board-buttons-new-issue, done-when 3: a task page's property menus post one field to /tasks/<id>/set; priority
    /// and a due day go to `dash task update`, then back to the page; a bad value, an id read as a flag or another site's
    /// post sets nothing.
    #[test]
    fn a_property_menu_sets_its_field_and_goes_back_to_its_page() {
        let _budget = TestBudget::new();
        let dir = no_app("property-set");
        let said = dir.0.join("said");
        let older = dir.0.join("older-dash");
        let script = format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" >> '{}'\necho updated\n",
            said.display()
        );
        std::fs::write(&older, script).expect("the older app");
        std::fs::set_permissions(&older, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .expect("chmod");
        unsafe { std::env::set_var("DASH_CLI", &older) };
        let post = |path: &str, form: &str, origin: &str| {
            let request = format!(
                "POST {path} HTTP/1.1\r\nHost: localhost\r\nOrigin: {origin}\r\nContent-Length: {}\r\n\r\n{form}",
                form.len()
            );
            answered(
                |c| c.write_all(request.as_bytes()).expect("write"),
                Duration::from_secs(10),
            )
            .0
        };
        assert!(
            served_here("POST /tasks/a-task/set HTTP/1.1"),
            "Rust answers it"
        );
        assert_eq!(
            tasks::set_posted("POST /tasks/a%2Db/set HTTP/1.1").as_deref(),
            Some("a-b")
        );
        assert_eq!(
            tasks::set_posted("POST /tasks/-h/set HTTP/1.1"),
            None,
            "never a flag"
        );
        assert_eq!(tasks::set_posted("GET /tasks/a/set HTTP/1.1"), None);
        // a path's '+' stays a plus (/code-review of PR 1395)
        assert_eq!(
            tasks::set_posted("POST /tasks/c++-port/set HTTP/1.1").as_deref(),
            Some("c++-port")
        );

        let set = post("/tasks/a-task/set", "priority=2", "http://localhost");
        assert!(
            set.starts_with("HTTP/1.1 303 See Other\r\nLocation: /tasks/a-task\r\n"),
            "{set}"
        );
        let due = post("/tasks/a-task/set", "due=2026-11-15", "http://localhost");
        assert!(due.starts_with("HTTP/1.1 303 "), "{due}");
        // none clears either, the list's « nil »
        for form in ["priority=none", "due=none"] {
            let cleared = post("/tasks/a-task/set", form, "http://localhost");
            assert!(cleared.starts_with("HTTP/1.1 303 "), "{form}: {cleared}");
        }
        let args = std::fs::read_to_string(&said).expect("dash was asked");
        assert_eq!(
            args,
            "task\nupdate\na-task\n--priority\n2\ntask\nupdate\na-task\n--deadline\n2026-11-15\n\
             task\nupdate\na-task\n--priority\nnil\ntask\nupdate\na-task\n--deadline\nnil\n"
        );

        for (form, origin, status) in [
            ("priority=9", "http://localhost", "400"),
            ("due=next+week", "http://localhost", "400"),
            ("due=2026-02-30", "http://localhost", "400"),
            ("color=red", "http://localhost", "400"),
            // no board in this process: a status move is the board's, never the list's
            ("status=todo", "http://localhost", "503"),
            ("priority=1", "http://elsewhere", "403"),
        ] {
            let refused = post("/tasks/a-task/set", form, origin);
            assert!(
                refused.starts_with(&format!("HTTP/1.1 {status} ")),
                "{form}: {refused}"
            );
            // a plain form's refusal is the page he reads: it leads back (/code-review of PR 1395)
            assert!(
                status != "400" || refused.ends_with("Back to the task: /tasks/a-task\n"),
                "{form}: {refused}"
            );
        }
        assert_eq!(
            std::fs::read_to_string(&said)
                .expect("read")
                .lines()
                .count(),
            20,
            "nothing more set"
        );
    }

    /// Task rust-page-latency, done-when 3: the board's patch stream is its own span resource, so its open minutes never read
    /// as /tasks' time (its 90 s spans were /tasks' on 2026-11-10); /tasks and any other query keep the path.
    #[test]
    fn the_patch_stream_is_its_own_span_resource() {
        let _budget = TestBudget::new();
        assert_eq!(
            request_path("GET /tasks?view=patches HTTP/1.1"),
            "/tasks?view=patches"
        );
        assert_eq!(
            request_path("GET /tasks?layout=board&view=patches HTTP/1.1"),
            "/tasks?view=patches"
        );
        assert_eq!(request_path("GET /tasks HTTP/1.1"), "/tasks");
        assert_eq!(request_path("GET /tasks?layout=list HTTP/1.1"), "/tasks");
        assert_eq!(request_path("GET /tasks?view=text HTTP/1.1"), "/tasks");
    }

    /// A report on a folder with no switch: it sends nothing.
    fn silent() -> Arc<Statsd> {
        Arc::new(Statsd::new(Path::new("/nonexistent"), "test"))
    }

    /// A report whose lines reach the socket it comes with. The folder holds the switch, so it stays as long as the report.
    fn reporting_to_a_listener(
        name: &str,
    ) -> (dash_meter::TestDir, std::net::UdpSocket, Arc<Statsd>) {
        let dir = dash_meter::TestDir::new(name);
        let port = std::net::UdpSocket::bind("127.0.0.1:0").expect("bind");
        port.set_read_timeout(Some(Duration::from_secs(2)))
            .expect("timeout");
        let to = port.local_addr().expect("address").to_string();
        std::fs::write(dir.0.join("telemetry"), to).expect("the switch");
        let report = Arc::new(Statsd::new(&dir.0, "test"));
        (dir, port, report)
    }

    fn assert_a_closed_count(port: &std::net::UdpSocket) {
        let mut line = [0u8; 256];
        let n = port.recv(&mut line).expect("a count");
        let line = String::from_utf8_lossy(&line[..n]);
        assert!(line.contains("status:closed,route:route_other"), "{line}");
    }

    fn head(headers: &[&str], upgrade: bool) -> Head {
        Head {
            line: "GET /approve HTTP/1.1\r\n".into(),
            host: Some("100.72.158.115:7788".into()),
            accept: String::new(),
            upgrade,
            headers: headers.iter().map(|h| format!("{h}\r\n")).collect(),
            rest: vec![],
        }
    }

    /// /approve refuses a tap from this Mac: a caller on the tailnet must reach the older app named, never as loopback, and
    /// never under an address it wrote itself. Tailscale Serve's own forward, from loopback, passes as it came.
    #[test]
    fn the_older_app_learns_who_called_and_no_caller_can_say_it_for_itself() {
        let _budget = TestBudget::new();
        let phone: IpAddr = "100.80.1.2".parse().expect("an address");
        let written = head(
            &[
                "Host: 100.72.158.115:7788",
                "X-Forwarded-For: 127.0.0.1",
                "Connection: keep-alive",
            ],
            false,
        );
        assert_eq!(
            forwarded(&written, Some(phone)),
            "GET /approve HTTP/1.1\r\nHost: 100.72.158.115:7788\r\nX-Forwarded-For: 100.80.1.2\r\nConnection: close\r\n\r\n"
        );
        let served = head(&["Host: mac.ts.net", "X-Forwarded-For: 100.80.1.2"], false);
        let loopback = Some(IpAddr::V4(Ipv4Addr::LOCALHOST));
        assert_eq!(
            forwarded(&served, loopback),
            "GET /approve HTTP/1.1\r\nHost: mac.ts.net\r\nX-Forwarded-For: 100.80.1.2\r\nConnection: close\r\n\r\n"
        );
        let socket = head(&["Connection: Upgrade", "Upgrade: websocket"], true);
        assert_eq!(
            forwarded(&socket, loopback),
            "GET /approve HTTP/1.1\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n\r\n"
        );
    }

    #[test]
    fn a_request_trickling_in_ends_at_its_deadline() {
        let _budget = TestBudget::new();
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let mut client =
            TcpStream::connect(listener.local_addr().expect("address")).expect("connect");
        let (server, _) = listener.accept().expect("accept");
        std::thread::spawn(move || {
            // each byte well inside what a timeout per read would allow
            while client.write_all(b"x").is_ok() {
                std::thread::sleep(Duration::from_millis(20));
            }
        });
        let start = Instant::now();
        assert!(request_line(&server, Duration::from_millis(200)).is_err());
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    /// Done-when 1 of rust-page-forms: the head holds Accept as its own field, the first line as sent.
    #[test]
    fn the_head_holds_its_accept_apart_from_its_first_line() {
        let _budget = TestBudget::new();
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let mut client =
            TcpStream::connect(listener.local_addr().expect("address")).expect("connect");
        let (server, _) = listener.accept().expect("accept");
        let sent = b"GET /tools HTTP/1.1\r\nHost: localhost\r\nAccept:  text/html,*/*\r\n\r\n";
        client.write_all(sent).expect("write");
        let head = request_line(&server, Duration::from_secs(2)).expect("a head");
        assert_eq!(
            (head.line.as_str(), head.accept.as_str()),
            ("GET /tools HTTP/1.1\r\n", "text/html,*/*")
        );
        assert!(wants_html(&head.accept, ""));
    }

    #[test]
    fn a_client_that_stops_reading_ends_at_the_deadline() {
        let _budget = TestBudget::new();
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let _client = TcpStream::connect(listener.local_addr().expect("address")).expect("connect");
        let (server, _) = listener.accept().expect("accept");
        let mut timed = ByDeadline {
            stream: &server,
            deadline: Instant::now() + Duration::from_millis(200),
        };
        let piece = vec![b'x'; 64 * 1024];
        let start = Instant::now();
        while timed.write_all(&piece).is_ok() {}
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn only_a_loopback_name_passes_the_host_check() {
        let _budget = TestBudget::new();
        for good in ["127.0.0.1", "127.0.0.1:7788", "localhost", "localhost:1"] {
            assert!(is_loopback_host(good), "{good}");
        }
        for bad in [
            "evil.example",
            "evil.example:80",
            "127.0.0.1.evil.example",
            "",
        ] {
            assert!(!is_loopback_host(bad), "{bad}");
        }
    }

    #[test]
    fn a_connection_must_name_the_address_it_came_in_on() {
        let _budget = TestBudget::new();
        let loopback: IpAddr = "127.0.0.1".parse().expect("ip");
        let tailnet: IpAddr = "100.72.158.115".parse().expect("ip");
        for good in ["100.72.158.115", "100.72.158.115:7788"] {
            assert!(allowed_host(good, tailnet), "{good}");
            assert!(!allowed_host(good, loopback), "{good}");
        }
        for good in ["127.0.0.1:7788", "localhost:7788"] {
            assert!(allowed_host(good, loopback), "{good}");
            assert!(!allowed_host(good, tailnet), "{good}");
        }
        for bad in [
            "100.72.158.116:7788",
            "evil.example:7788",
            "100.72.158.115.evil.example",
            "",
        ] {
            assert!(!allowed_host(bad, tailnet), "{bad}");
            assert!(!allowed_host(bad, loopback), "{bad}");
        }
    }

    #[test]
    fn only_a_utun_tailscale_address_is_the_tailnet() {
        let _budget = TestBudget::new();
        let hotspot = "en0: flags=8863<UP>\n\tinet 100.72.1.1 netmask 0xffffff00\nutun3: flags=8051<UP>\n\tinet6 fe80::1 prefixlen 64\n";
        assert_eq!(
            tailnet_in(hotspot),
            None,
            "a CGNAT Wi-Fi is not his tailnet"
        );
        let live = format!(
            "{hotspot}utun4: flags=8051<UP>\n\tinet 100.72.158.115 --> 100.72.158.115 netmask 0xffffffff\n"
        );
        assert_eq!(tailnet_in(&live), Some(Ipv4Addr::new(100, 72, 158, 115)));
        let outside = "utun5: flags=8051<UP>\n\tinet 100.128.0.1 --> 100.128.0.1\n";
        assert_eq!(tailnet_in(outside), None, "outside 100.64.0.0/10");
    }

    #[test]
    fn an_idle_connection_holds_no_other_request() {
        let _budget = TestBudget::new();
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let address = listener.local_addr().expect("address");
        let (sender, requests) = sync_channel::<Request>(16);
        let live = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&live);
        std::thread::spawn(move || accept(&listener, &sender, &counted, &silent()));
        std::thread::spawn(move || {
            for (_, _, _, reply, _) in requests {
                let _ = reply.send(("200 OK", TEXT, "ok".into()));
            }
        });
        // as a browser's preconnect: open, sends nothing
        let _idle = TcpStream::connect(address).expect("connect");
        std::thread::sleep(Duration::from_millis(50));
        let start = Instant::now();
        let mut stream = TcpStream::connect(address).expect("connect");
        write!(stream, "GET /x HTTP/1.1\r\nHost: localhost\r\n\r\n").expect("write");
        let mut text = String::new();
        stream.read_to_string(&mut text).expect("read");
        assert!(text.starts_with("HTTP/1.1 200"), "{text}");
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "{:?}",
            start.elapsed()
        );
    }

    #[test]
    fn a_request_s_time_starts_when_it_joins_the_queue() {
        let _budget = TestBudget::new();
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let address = listener.local_addr().expect("address");
        let (sender, requests) = sync_channel::<Request>(1);
        let live = Arc::new(AtomicUsize::new(0));
        std::thread::spawn(move || accept(&listener, &sender, &live, &silent()));
        let mut stream = TcpStream::connect(address).expect("connect");
        write!(stream, "GET /x HTTP/1.1\r\nHost: localhost\r\n\r\n").expect("write");
        let (_, _, queued, reply, _) = requests.recv().expect("the request");
        let before = queued.elapsed();
        std::thread::sleep(Duration::from_millis(50));
        assert!(queued.elapsed() >= before + Duration::from_millis(50));
        let _ = reply.send(("200 OK", TEXT, "ok".into()));
    }

    #[test]
    fn a_store_that_does_not_answer_is_a_503_by_the_deadline() {
        let _budget = TestBudget::new();
        let (text, _) = answered(
            |client| write!(client, "GET /x HTTP/1.1\r\nHost: localhost\r\n\r\n").expect("write"),
            Duration::from_millis(100),
        );
        assert!(text.starts_with("HTTP/1.1 503"), "{text}");
    }

    /// Task rust-pools-fable, done-when 1: `dash claude-accounts`' answer from the older app gets the model lines after its
    /// stdout, before its stderr, both lengths grown; a refusal or an answer of another length goes on as it came.
    #[test]
    fn the_older_app_s_accounts_answer_gets_the_model_lines_after_its_stdout() {
        let _budget = TestBudget::new();
        let answer = "HTTP/1.1 200 OK\r\ncontent-length: 7\r\nx-exit: 0\r\nX-Stdout-Bytes: 4\r\n\r\nout\nerr";
        let lines = "ok fable (default): its own allowance 2%\n";
        let grown = String::from_utf8(with_lines(answer.as_bytes(), lines)).expect("text");
        assert_eq!(
            grown,
            format!(
                "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nx-exit: 0\r\nX-Stdout-Bytes: {}\r\n\r\nout\n{lines}err",
                7 + lines.len(),
                4 + lines.len()
            )
        );
        for kept in [
            "HTTP/1.1 403 Forbidden\r\ncontent-length: 3\r\nx-stdout-bytes: 3\r\n\r\nno!",
            "HTTP/1.1 200 OK\r\ncontent-length: 9\r\nx-stdout-bytes: 3\r\n\r\nshort",
            "HTTP/1.1 200 OK\r\ncontent-length: 5\r\n\r\nnone!",
            "no head at all",
        ] {
            assert_eq!(
                with_lines(kept.as_bytes(), lines),
                kept.as_bytes(),
                "{kept}"
            );
        }
    }

    /// /code-review of rust-graph-why-these: `task add`'s warning follows only an add that filed, read off its x-exit.
    #[test]
    fn the_exit_code_is_read_off_the_answer_s_head_only() {
        let _budget = TestBudget::new();
        let answer = |head: &str, body: &str| format!("HTTP/1.1 200 OK\r\n{head}\r\n\r\n{body}");
        assert_eq!(exit_of(answer("X-Exit: 0", "").as_bytes()), Some(0));
        assert_eq!(
            exit_of(answer("x-exit: 1\r\nx-stdout-bytes: 0", "").as_bytes()),
            Some(1)
        );
        assert_eq!(
            exit_of(answer("content-length: 9", "x-exit: 0").as_bytes()),
            None
        );
        assert_eq!(exit_of(b"no head at all"), None);
    }

    /// `send` writes to the client, then `answer` runs with `wait` as its limit; what the client reads back.
    fn answered(send: impl FnOnce(&mut TcpStream), wait: Duration) -> (String, Duration) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let mut client =
            TcpStream::connect(listener.local_addr().expect("address")).expect("connect");
        let (server, _) = listener.accept().expect("accept");
        let (sender, _requests) = sync_channel::<Request>(1);
        send(&mut client);
        let start = Instant::now();
        answer(&server, &sender, &silent(), wait, wait);
        let took = start.elapsed();
        drop(server);
        let mut text = String::new();
        let _ = client.read_to_string(&mut text);
        (text, took)
    }

    #[test]
    fn a_connection_that_sends_nothing_closes_with_no_answer_at_its_limit() {
        let _budget = TestBudget::new();
        let limit = Duration::from_millis(150);
        let (text, took) = answered(|_| {}, limit);
        assert_eq!(text, "", "silent to the limit");
        // Half: a kernel's timeout may wake a hair early, and the point is that it waited.
        assert!(
            took >= limit / 2 && took < Duration::from_secs(2),
            "{took:?}"
        );
        let (text, took) = answered(
            |client| client.shutdown(std::net::Shutdown::Write).expect("close"),
            Duration::from_secs(5),
        );
        assert_eq!(text, "", "closed before a byte");
        assert!(took < Duration::from_secs(2), "at once: {took:?}");
    }

    #[test]
    fn a_request_begun_and_never_finished_is_still_a_400() {
        let _budget = TestBudget::new();
        let limit = Duration::from_millis(150);
        let (text, _) = answered(|client| client.write_all(b"GET /").expect("write"), limit);
        assert!(text.starts_with("HTTP/1.1 400"), "{text}");
        let (text, _) = answered(
            |client| {
                client.write_all(b"GET /").expect("write");
                client.shutdown(std::net::Shutdown::Write).expect("close");
            },
            limit,
        );
        assert!(
            text.starts_with("HTTP/1.1 400"),
            "a client that gave up: {text}"
        );
    }

    #[test]
    fn a_preconnect_used_late_has_the_whole_limit_for_its_request() {
        let _budget = TestBudget::new();
        let (text, _) = answered(
            |client| {
                let mut late = client.try_clone().expect("clone");
                std::thread::spawn(move || {
                    let step = Duration::from_millis(600);
                    std::thread::sleep(step);
                    late.write_all(b"GET /x HTTP/1.1\r\n").expect("write");
                    std::thread::sleep(step);
                    late.write_all(b"Host: localhost\r\n\r\n").expect("write");
                });
            },
            Duration::from_secs(1),
        );
        assert!(
            text.starts_with("HTTP/1.1 503"),
            "read whole, then no store: {text}"
        );
    }

    #[test]
    fn a_4xx_nobody_read_is_not_counted_and_a_5xx_always_is() {
        let _budget = TestBudget::new();
        assert!(counts_as_refused("400 Bad Request", true));
        assert!(!counts_as_refused("400 Bad Request", false));
        assert!(counts_as_refused("503 Service Unavailable", false));
        assert!(!counts_as_refused("200 OK", true));
    }

    #[test]
    fn what_a_browser_fetches_by_itself_is_known_by_its_path_and_method() {
        let _budget = TestBudget::new();
        for line in [
            "GET /favicon.ico HTTP/1.1\r\n",
            "GET /apple-touch-icon-precomposed.png HTTP/1.1\r\n",
            "GET /.well-known/appspecific/com.chrome.devtools.json?x=1 HTTP/1.1\r\n",
        ] {
            assert!(asked_by_a_browser_itself(line), "{line}");
        }
        assert!(!asked_by_a_browser_itself("POST /favicon.ico HTTP/1.1\r\n"));
        assert!(!asked_by_a_browser_itself("GET /secret HTTP/1.1\r\n"));
    }

    #[test]
    fn a_panic_frees_the_connection_slot() {
        let _budget = TestBudget::new();
        let live = Arc::new(AtomicUsize::new(0));
        let held = Held::take(&live);
        assert_eq!(live.load(Ordering::Relaxed), 1);
        let died = std::thread::spawn(move || {
            let _held = held;
            panic!("a connection's panic, on purpose");
        });
        assert!(died.join().is_err());
        assert_eq!(live.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn only_a_new_address_moves_the_tailnet() {
        let _budget = TestBudget::new();
        let (a, b) = (Ipv4Addr::new(100, 64, 0, 1), Ipv4Addr::new(100, 64, 0, 2));
        assert!(tailnet_moved(Some(a), Some(b)));
        for (bound, here) in [
            (Some(a), Some(a)),
            (Some(a), None),
            (None, Some(a)),
            (None, None),
        ] {
            assert!(!tailnet_moved(bound, here), "{bound:?} {here:?}");
        }
    }

    #[test]
    fn past_the_cap_a_connection_is_closed() {
        let _budget = TestBudget::new();
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let address = listener.local_addr().expect("address");
        let (sender, _requests) = sync_channel::<Request>(1);
        let live = Arc::new(AtomicUsize::new(CONNECTIONS));
        let counted = Arc::clone(&live);
        let (_dir, port, report) = reporting_to_a_listener("past-the-cap");
        std::thread::spawn(move || accept(&listener, &sender, &counted, &report));
        let mut stream = TcpStream::connect(address).expect("connect");
        let mut text = String::new();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .expect("timeout");
        assert_eq!(
            stream.read_to_string(&mut text).expect("closed, no answer"),
            0
        );
        assert_eq!(live.load(Ordering::Relaxed), CONNECTIONS, "as it was");
        assert_a_closed_count(&port);
    }

    #[test]
    fn no_thread_to_be_had_counts_a_closed_connection_and_frees_its_slot() {
        let _budget = TestBudget::new();
        let (_dir, port, report) = reporting_to_a_listener("no-thread");
        let live = Arc::new(AtomicUsize::new(0));
        let held = Held::take(&live);
        // A stack no address space holds: the one way to make a spawn fail in a test.
        let thread = std::thread::Builder::new().stack_size(1 << 62);
        on_its_thread(thread, &report, move || {
            let _held = held;
            panic!("this work must not run");
        });
        assert_a_closed_count(&port);
        assert_eq!(live.load(Ordering::Relaxed), 0, "its slot is free");
    }

    #[test]
    fn a_client_s_path_never_names_a_step() {
        let _budget = TestBudget::new();
        for (line, step) in [
            ("GET / HTTP/1.1\r\n", "route_page"),
            ("GET /graph HTTP/1.1\r\n", "route_graph"),
            ("GET /agents HTTP/1.1\r\n", "route_agents"),
            ("GET /tools HTTP/1.1\r\naccept: text/html\n", "route_tools"),
            ("GET /design?tab=data HTTP/1.1\r\n", "route_design"),
            ("GET /mail?page=2 HTTP/1.1\r\n", "route_mail"),
            ("GET /tasks/a-b HTTP/1.1\r\n", "route_task"),
            ("GET /drafts/a-draft?view=text HTTP/1.1\r\n", "route_draft"),
            ("GET /d/a-draft HTTP/1.1\r\n", "route_draft_moved"),
            ("GET /search?q=a HTTP/1.1\r\n", "route_search"),
            ("GET /ui/menu HTTP/1.1\r\n", "route_ui"),
            ("GET /whatever/they/like HTTP/1.1\r\n", "route_other"),
            ("POST /search HTTP/1.1\r\n", "route_other"),
            ("GET\t/ping HTTP/1.1\r\n", "route_ping"),
            ("", "route_other"),
        ] {
            assert_eq!(route_step(line), step, "{line}");
        }
    }

    /// rust/CLAUDE.md rule 5, one slow request never stalls the others: with the serial store's thread held by a slow /scan,
    /// /ping and a page still answer within 200 ms, and only the store's routes queue behind it (Elixir answered no dash
    /// command for 120 s under disk load, 2026-11-09).
    #[test]
    fn a_slow_request_leaves_the_ping_and_the_pages_answering_under_200_ms() {
        let _budget = TestBudget::new();
        let repo = dash_meter::TestDir::new("slow-request");
        // its own process (rust/CLAUDE.md rule 1): the scratch repo, never the live checkout
        unsafe { std::env::set_var("DASH_REPO", &repo.0) };
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let address = listener.local_addr().expect("address");
        let (sender, requests) = sync_channel::<Request>(1);
        let live = Arc::new(AtomicUsize::new(0));
        std::thread::spawn(move || accept(&listener, &sender, &live, &silent()));
        let ask = |path: &str| {
            let mut stream = TcpStream::connect(address).expect("connect");
            write!(stream, "GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n").expect("write");
            stream
        };
        // the slow one: taken off the queue and never answered while the others ask
        let _slow = ask("/scan");
        let (line, _, _, _held, _) = requests.recv().expect("the slow request");
        assert!(line.starts_with("GET /scan"), "{line}");
        // /deps is static text: a debug build's /design render under CI load is no part of this check
        for path in ["/ping", "/deps"] {
            let start = Instant::now();
            let mut text = String::new();
            ask(path).read_to_string(&mut text).expect("its answer");
            let took = start.elapsed();
            assert!(text.starts_with("HTTP/1.1 200"), "{path}: {text}");
            assert!(took < Duration::from_millis(200), "{path} took {took:?}");
        }
        assert!(
            unqueued("GET /scan HTTP/1.1", "*/*", None).is_none(),
            "the store's routes stay serial"
        );
        assert!(unqueued("POST /ping HTTP/1.1", "*/*", None).is_none());
        // the search page asks Flare for up to 3 s: on its own thread, never the serial one (task rust-search-port)
        assert!(unqueued("GET /search?q=x HTTP/1.1", "*/*", None).is_some());
        assert!(
            unqueued("GET /search?in=inbox&q=x HTTP/1.1", "*/*", None).is_none(),
            "/inbox's field reads the reranked store"
        );
    }

    /// Done-when 2 of rust-search-port: every browser's /search is ours, FindLive unreachable through the front door; /inbox's field stays the serial
    /// route's, and words that read as the field are the search page's.
    #[test]
    fn every_search_is_ours_and_only_the_inbox_s_field_is_the_serial_route_s() {
        let _budget = TestBudget::new();
        assert!(served_here("GET /search?in=inbox&q=rust HTTP/1.1"));
        assert!(served_here("GET /search?q=rust HTTP/1.1"));
        assert!(served_here("GET /search HTTP/1.1"));
        assert!(!served_here("GET /ping HTTP/1.1"), "the older app's ping");
        assert!(page_of(search::PAGE, "q=rust").is_some());
        assert!(page_of(search::PAGE, "q=rust&in=inbox").is_none());
        assert!(
            page_of(search::PAGE, "q=in=inbox").is_some(),
            "words that read as the field"
        );
        assert_eq!(route_step("GET /search?q=a HTTP/1.1"), "route_search");
    }

    /// /agents is ours since rust-agents-comments-tabs; the older agents page's own query stays the older app's.
    #[test]
    fn agents_is_ours_and_its_older_page_the_older_app_s() {
        let _budget = TestBudget::new();
        assert!(served_here("GET /agents HTTP/1.1"));
        assert!(
            !served_here("GET /agents?app=older HTTP/1.1"),
            "the orchestrator's controls stay reachable"
        );
    }

    #[test]
    fn a_browser_gets_html_an_agent_the_text_and_view_overrides_both() {
        let _budget = TestBudget::new();
        let (browser, curl) = ("text/html,application/xhtml+xml", "*/*");
        assert!(wants_html(browser, ""));
        assert!(!wants_html(curl, ""));
        assert!(!wants_html("", ""));
        assert!(!wants_html(browser, "view=text"));
        assert!(wants_html(curl, "a=1&view=html"));
        assert!(wants_html(curl, "view=HTML%0A"));
        assert!(wants_html(browser, "view="));
    }

    fn get(target: &str, accept: &str) -> Response {
        let dir = dash_meter::TestDir::new("forms");
        let db = store::open(&dir.0).expect("a store");
        let line = format!("GET {target} HTTP/1.1\r\n");
        unqueued(&line, accept, None).unwrap_or_else(|| {
            let registry = Registry::with_compiled().expect("a registry");
            route(&line, accept, &db, &registry)
        })
    }

    /// Done-when 4 of rust-daily-today: /today, as the server routes it, draws its one tree through dash-surface in both forms:
    /// its text for an agent, its HTML in the menu for him, drafts to send first; a line still reading answers an agent 202.
    #[test]
    fn today_answers_both_forms_through_the_surface_crate() {
        let _budget = TestBudget::new();
        let dir = dash_meter::TestDir::new("today-forms");
        let db = store::open(&dir.0).expect("a store");
        let today = |drafts: Option<usize>, accept: &str, query: &str| {
            let mut registry = Registry::default();
            let to_send: Box<dyn dash_core::registry::Slot> =
                Box::new(dash_drafts::outbox::ToSend(move || Ok(drafts)));
            let replies: Box<dyn dash_core::registry::Slot> =
                Box::new(dash_mail::RepliesWaiting(|| Ok(Some(2))));
            let slot = |key: &str, line| dash_core::registry::Contribution {
                slots: vec![(key.to_string(), line)],
                ..Default::default()
            };
            registry
                .add("mail", slot(mail::SLOT, replies))
                .expect("mail");
            registry
                .add("drafts", slot(drafts::SLOT, to_send))
                .expect("drafts");
            let line = format!("GET /today{query} HTTP/1.1\r\n");
            route(&line, accept, &db, &registry)
        };
        let (status, kind, text) = today(Some(3), "*/*", "?view=text");
        assert_eq!((status, kind), ("200 OK", TEXT), "{text}");
        assert_eq!(
            text,
            "Today\n3 to send\t/drafts?show=to-send\ndrafts approved, not yet sent\n2 replies waiting\t/mail\nunread answers in his inbox\n\n".to_string()
                + surface::forms::menu_text()
        );
        let (status, kind, html) = today(Some(3), "*/*", "?view=html");
        assert_eq!((status, kind), ("200 OK", HTML));
        let html = surface::markup(&html);
        let (first, second) = (html.find("3 to send"), html.find("2 replies waiting"));
        assert!(
            first.is_some_and(|f| second.is_some_and(|s| f < s)),
            "drafts first: {html}"
        );
        assert!(
            html.contains(r#"href="/drafts?show=to-send""#) && html.contains(r#"id="dash-menu""#),
            "the line's link and the menu"
        );
        assert_eq!(
            today(Some(3), "text/html", "").1,
            HTML,
            "a browser gets the HTML"
        );
        let (status, _, said) = today(None, "*/*", "");
        assert_eq!(status, "202 Accepted", "{said}");
        assert_eq!(
            today(None, "text/html", "").0,
            "200 OK",
            "he sees the line reading"
        );
    }

    /// Task rust-page-forms: /deps and /design answer by `drawn` too, HTML for a browser and their text for an agent.
    #[test]
    fn design_answers_both_forms_through_drawn() {
        let _budget = TestBudget::new();
        assert_eq!(get("/design", "text/html").1, HTML);
        let (status, kind, body) = get("/design", "*/*");
        assert_eq!((status, kind), ("200 OK", TEXT));
        assert!(
            !body.contains("class=") && !body.trim().is_empty(),
            "{body}"
        );
    }

    /// Done-when 1: /design is the spike's page, the Dioxus component's server render, and the window draws its words.
    #[test]
    fn design_is_the_component_and_the_window_draws_the_same_words() {
        let _budget = TestBudget::new();
        let (status, kind, html) = get("/design", "text/html");
        assert_eq!((status, kind), ("200 OK", HTML));
        assert!(html.contains(".ds-row"), "its style in /design");
        let html = surface::markup(&html);
        for part in [
            "Design system",
            r#"id="f-layout""#,
            r#"href="/design?tab=data&#38;theme=light""#,
            "Guidelines",
        ] {
            assert!(html.contains(part), "{part} in /design");
        }
        // Every word, the dark cell's too, as the window draws it: the first tab, and Feedback's live Stage toggle
        let words = |t: &str| t.split_whitespace().map(str::to_string).collect::<Vec<_>>();
        for tab in ["", "feedback"] {
            let window = dash_ui::shown(dioxus::prelude::VirtualDom::new_with_props(
                surface::design::Design,
                surface::design::DesignProps {
                    tab: surface::design::tab_of(tab),
                    ..Default::default()
                },
            ));
            assert_eq!(
                words(&window),
                words(&surface::forms::every_word(&surface::design::page(
                    tab, None, ""
                ))),
                "{tab}"
            );
        }
        // ?tab=notion is the Documents tab, the editor its first part: one document component, no tab of its own
        let notion = surface::design::page("notion", None, "");
        assert_eq!(notion, surface::design::page("documents", None, ""));
        let notion = surface::markup(&notion);
        assert!(
            notion.contains(r#"id="no-block""#)
                && notion.contains(r#"id="v-documents" class="ds-tab on""#)
                && !notion.contains("v-notion"),
            "{notion}"
        );
    }

    /// Done-when 2 and 3: every page answers its text, HTML with the toggle, and both side by side, at its one URL.
    #[test]
    fn every_page_answers_three_views_and_the_menu_links_to_none() {
        let _budget = TestBudget::new();
        let (_, kind, text) = get("/design?tab=forms&view=text", "text/html");
        assert_eq!(kind, TEXT);
        assert!(text.starts_with("Design\tLayout\tData\t"), "{text}");
        assert!(
            text.contains("\nForms\nWhat he presses, picks and types"),
            "{text}"
        );
        assert!(
            text.contains("\nPrimary\tApprove\tReject\tLock\tCancel\n"),
            "{text}"
        );
        assert!(!text.contains('<'), "no markup: {text}");
        assert_eq!(
            get("/design?tab=forms", "*/*").2,
            text,
            "an agent's default is the text"
        );
        // task rust-menu-text: the text ends with the menu, as `dash menu` prints it; a data view's JSON stays whole (below)
        assert!(
            text.ends_with(&format!("\n\n{}", surface::forms::menu_text())),
            "{text}"
        );
        let html = surface::markup(&get("/design?tab=forms&view=html", "*/*").2);
        // no view toggle in the menu (task rust-menu-no-view-toggle)
        assert!(!html.contains("view-toggle"), "{html}");
        // a family tab keeps the theme picked, as the window does
        let dark = surface::markup(&get("/design?tab=forms&theme=dark&view=html", "*/*").2);
        assert!(
            dark.contains("app dark ds-root")
                && dark.contains(r#"href="/design?tab=data&#38;theme=dark""#),
            "the theme stays"
        );
        for path in [
            "/",
            "/graph",
            "/agents",
            "/comments",
            "/tools",
            "/watchers",
            "/design",
            "/tasks/a-b",
            "/drafts/a-b",
            "/d/a-b",
        ] {
            assert!(pages::find(path).is_some(), "{path}");
        }
        // done-when 3 of rust-one-doc-component: /d/<id> is no page of its own, only where a link to it is moved on
        assert_eq!(
            pages::find("/d/a-b").map(|p| p.step),
            Some("route_draft_moved")
        );
        // the data view: a tree page's tree as pretty JSON, for a program
        let (status, kind, data) = get("/inbox?view=data", "text/html");
        assert_eq!((status, kind), ("200 OK", pages::JSON), "{data}");
        let tree: serde_json::Value = serde_json::from_str(&data).expect("JSON");
        assert!(
            tree["root"].is_string() && data.contains("\n  \""),
            "pretty: {data}"
        );
        // /search is a page since rust-search-port; /inbox's field the serial route's
        assert!(pages::find("/ping").is_none() && page_of(search::PAGE, "in=inbox").is_none());
        // A page that only draws gets its text from its HTML: /graph's 404 here stays its own words.
        assert_eq!(get("/graph?view=text", "*/*").2, "no port, no page");
    }

    #[test]
    fn a_form_s_query_decodes() {
        let _budget = TestBudget::new();
        assert_eq!(form_decoded("a+b%20c%C3%A9"), "a b c\u{e9}");
        assert_eq!(form_decoded("100%"), "100%");
        assert_eq!(form_decoded("%zz%2"), "%zz%2");
        assert_eq!(form_decoded("%+a"), "% a", "not a byte");
    }

    /// Done-when 1 and 3 of rust-window-in-process: the window's page is the web's answer at the same URL, sidebar and style
    /// included, rendered in this process with no server on any port; its document shows the menu. The web's answer is the
    /// route `dash serve` runs, which needs no window.
    #[test]
    fn the_window_draws_the_web_s_page_in_its_own_process() {
        let _budget = TestBudget::new();
        let dir = no_app("window");
        // the board as dash serve holds it, behind its lock
        let served = Mutex::new(tasks::Board::open(&dir.0).expect("the tasks"));
        let board = || served.lock().expect("the board");
        for target in [
            "/tasks",
            "/tasks?layout=list",
            "/design",
            "/deps",
            "/comments",
        ] {
            let line = format!("GET {target} HTTP/1.1\r\n");
            // the board's reads of the older app end on their threads, each the next render's: both read once they settled
            let (mut window, mut web) = (String::new(), String::from("unread"));
            for _ in 0..50 {
                let (status, kind, answer) =
                    unqueued(&line, "text/html", Some(&served)).expect("a page dash serve answers");
                assert_eq!((status, kind), ("200 OK", HTML), "{target}: {answer}");
                (window, web) = (window_page(target, &board()), answer);
                if window == web {
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            assert_eq!(window, web, "{target}");
            assert!(
                surface::markup(&window).contains(r#"id="dash-nav""#),
                "{target}: the sidebar"
            );
            assert!(window.contains(".app.light {"), "{target}: the theme");
            // done-when 4 of rust-home-prompt: the « dash » is the window's home link too
            assert!(
                surface::markup(&window).contains(r#"href="/" id="dash-home""#),
                "{target}: the home link"
            );
            let shown = dash_ui::shown(dioxus::prelude::VirtualDom::new_with_props(
                dash_ui::Page,
                dash_ui::PageProps { html: window },
            ));
            assert!(
                shown.contains("Inbox\nToday\nMail\nTasks\nsite_perso\n"),
                "{target}, the window's menu: {shown}"
            );
        }
        // a page that reads no store is rendered once a visit; the board each time it is asked
        assert!(reads_the_store("/tasks?layout=list") && reads_the_store("/tasks/a"));
        assert!(!reads_the_store("/mail") && !reads_the_store("/agents"));
        // a page that is not HTML, and one only the older app serves: the same sidebar over its words
        let text = surface::markup(&window_page("/deps?view=text", &board()));
        assert!(
            text.contains(r#"id="dash-nav""#) && text.contains("code map"),
            "{text}"
        );
        let older = surface::markup(&window_page("/approve", &board()));
        assert!(
            older.contains(r#"href="http://127.0.0.1:7788/approve""#),
            "{older}"
        );
    }

    /// Task rust-one-doc-component, done-when 1 and 3: a draft written before the one model, as the older app's export still
    /// holds it (its words a `body` of blank-line paragraphs, no private note), reads in as a document of public sections; a
    /// former /d/<id> link moves to /drafts/<id>, its query kept, and that view draws it through DocView's Gmail view.
    #[test]
    fn a_former_draft_link_opens_its_view_and_an_older_draft_reads_in_as_a_document() {
        let _budget = TestBudget::new();
        let dir = no_app("old-draft");
        // the older app's record in its file, as the page reads it (task rust-draft-page-cached)
        let db = dir.0.join("private/dashboard");
        std::fs::create_dir_all(&db).expect("its folder");
        let conn = store::Connection::open(db.join("dashboard_dev.db")).expect("its database");
        conn.execute_batch(
            "CREATE TABLE links (\"from\" TEXT, kind TEXT, \"to\" TEXT, pos INTEGER);
             CREATE TABLE drafts (collection TEXT, pos INTEGER, key TEXT, id TEXT, stage TEXT, data TEXT, body TEXT);
             CREATE TABLE contacts (id TEXT, name TEXT);
             CREATE TABLE companies (id TEXT, name TEXT);",
        )
        .expect("its tables");
        let record = serde_json::json!({"id": "old-draft", "status": "\u{E000}stage\u{E001}", "channel": "email",
            "to": "Ana (a@x.io)", "created": "2026-10-24"});
        conn.execute(
            "INSERT INTO drafts VALUES ('drafts', 0, '2026-10-24-old-draft.md', 'old-draft', 'pending', ?1, ?2)",
            (record.to_string(), "Bonjour Ana,\n\nUne question courte.\n\nNicolas"),
        )
        .expect("the draft");
        // task rust-draft-page-cached, done-when 1: no draft page request waits on the older app. Its stub takes 5 s; each
        // page below answers long before, its draft read from the file; /drafts/<id> draws the list too, whose refresh it
        // starts on its own thread
        let older = dir.0.join("older-dash");
        std::fs::write(&older, "#!/bin/sh\nsleep 5\necho '[]'\n").expect("the older app");
        std::fs::set_permissions(&older, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .expect("chmod");
        unsafe { std::env::set_var("DASH_CLI", &older) };
        let began = std::time::Instant::now();

        let (status, _, to) = get("/d/old-draft?view=text", "*/*");
        assert_eq!(
            (status, to.as_str()),
            ("308 Permanent Redirect", "/drafts/old-draft?view=text")
        );
        let (status, _, text) = get(&to, "*/*");
        assert_eq!(status, "200 OK", "{text}");
        assert!(
            text.contains("\nMessage\nBonjour Ana,\nUne question courte.\nNicolas\n"),
            "{text}"
        );
        let html = surface::markup(&get("/drafts/old-draft?view=html", "*/*").2);
        assert!(html.contains(r#"class="dv gmail""#), "{html}");
        assert_eq!(
            html.matches(r#"class="dv-section""#).count(),
            3,
            "a paragraph a section: {html}"
        );
        // an id the older app does not hold stays its own to answer
        assert_eq!(get("/d/no-such-draft", "*/*").0, "404 Not Found");
        let took = began.elapsed();
        assert!(
            took < Duration::from_secs(4),
            "a request waited on the older app: {took:?}"
        );
    }

    /// The older app for a draft's page, as a fixture: its draft a row of its database, which the page reads; its HTTP writes
    /// (`/d/<id>/body`, `/d/<id>/status`) and the stub CLI's `dash draft set <id> --send-on` a thread that writes the posted
    /// field into that row; and the draft's waits (`links`, `drafts`). Returns the HTTP app's port.
    fn older_draft_app(dir: &Path) -> u16 {
        // the draft in the older app's file, as the page reads it (task rust-draft-page-cached): its status the stage
        let record = serde_json::json!({"id": "grant-reply", "status": "\u{E000}stage\u{E001}", "channel": "form",
            "to": "Hostman (their form)", "created": "2026-11-10", "where": "Their form, its message field",
            "context": ["Fit 5.", "Six questions to answer."], "send-on": ""});
        let db = dir.join("private/dashboard");
        std::fs::create_dir_all(&db).expect("its folder");
        let db = db.join("dashboard_dev.db");
        let conn = store::Connection::open(&db).expect("its database");
        conn.execute_batch(
            "CREATE TABLE links (\"from\" TEXT, kind TEXT, \"to\" TEXT, pos INTEGER);
             CREATE TABLE drafts (collection TEXT, pos INTEGER, key TEXT, id TEXT, stage TEXT, data TEXT, body TEXT);
             CREATE TABLE contacts (id TEXT, name TEXT);
             CREATE TABLE companies (id TEXT, name TEXT);
             INSERT INTO drafts (id, stage, data) VALUES ('intro-first', 'pending', '{}'), ('old-one', 'sent', '{}');
             INSERT INTO links VALUES ('drafts:grant-reply', 'waits-on', 'drafts:old-one', 0),
               ('drafts:grant-reply', 'waits-on', 'drafts:intro-first', 1);",
        )
        .expect("its waits");
        conn.execute(
            "INSERT INTO drafts VALUES ('drafts', 0, '2026-11-10-grant-reply.md', 'grant-reply', 'pending', ?1, ?2)",
            (record.to_string(), "Hello,\n\nthe answers."),
        )
        .expect("the draft");
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("address").port();
        // `dash draft set <id> --send-on <day>` writes through the same stand-in as /d/<id>/<field>; `model drafts`, its
        // outbox facts, answers none
        let older = dir.join("older-dash");
        let script = format!(
            "#!/bin/sh\ncase \"$1 $2\" in\n'draft set') curl -sf -o /dev/null --data-urlencode \"send-on=$5\" \
             'http://127.0.0.1:{port}/d/'\"$3\"'/send-on' ;;\n*) echo '[]' ;;\nesac\n"
        );
        std::fs::write(&older, script).expect("the older app");
        std::fs::set_permissions(&older, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .expect("chmod");
        unsafe { std::env::set_var("DASH_CLI", &older) };
        std::thread::spawn(move || {
            for mut c in listener.incoming().flatten() {
                let mut raw = vec![];
                let mut buf = [0u8; 4096];
                let whole = |raw: &[u8]| {
                    let text = String::from_utf8_lossy(raw);
                    let (head, body) = text.split_once("\r\n\r\n")?;
                    let len = head.lines().find_map(|l| {
                        l.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .and_then(|v| v.trim().parse::<usize>().ok())
                    });
                    (body.len() >= len.unwrap_or(0)).then(|| (head.to_string(), body.to_string()))
                };
                let (head, form) = loop {
                    match c.read(&mut buf) {
                        Ok(0) | Err(_) => break whole(&raw).unwrap_or_default(),
                        Ok(n) => raw.extend_from_slice(&buf[..n]),
                    }
                    if let Some(read) = whole(&raw) {
                        break read;
                    }
                };
                let path = head
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or_default()
                    .to_string();
                let Some((id, what)) = path.strip_prefix("/d/").and_then(|p| p.split_once('/'))
                else {
                    continue;
                };
                // the older app's write, into its file: the status its stage, the text its body, any other field its data's
                let said = param(&form, what).unwrap_or_default();
                let conn = store::Connection::open(&db).expect("its database");
                let sql = match what {
                    "status" => "UPDATE drafts SET stage = ?2 WHERE id = ?3",
                    "body" => "UPDATE drafts SET body = ?2 WHERE id = ?3",
                    _ => {
                        "UPDATE drafts SET data = json_set(data, '$.\"' || ?1 || '\"', ?2) WHERE id = ?3"
                    }
                };
                conn.execute(sql, (what, said, id)).expect("the write");
                let said = format!(
                    "HTTP/1.1 303 See Other\r\nLocation: /d/{id}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                );
                let _ = c.write_all(said.as_bytes());
            }
        });
        port
    }

    /// A form posted to this app as a browser on it does, and its whole answer.
    fn posted(path: &str, form: &str) -> String {
        let request = format!(
            "POST {path} HTTP/1.1\r\nHost: localhost\r\nOrigin: http://localhost\r\nContent-Type: application/x-www-form-urlencoded\r\n\
             Content-Length: {}\r\n\r\n{form}",
            form.len()
        );
        answered(
            |c| c.write_all(request.as_bytes()).expect("write"),
            Duration::from_secs(10),
        )
        .0
    }

    /// Task rust-draft-state-edit, done-when 2: Change opens the text in place (`?edit=1`), and Save posts his words to the older
    /// app's write of his edit, through this app's front door, into the store: the page then reads the new text.
    #[test]
    fn change_edits_the_text_in_place_and_saves_it_through_the_store() {
        let _budget = TestBudget::new();
        let dir = no_app("draft-change");
        let port = older_draft_app(&dir.0);
        OLDER.set(port).expect("the older app's port, once");
        let (_, _, page) = get("/drafts/grant-reply?view=text", "*/*");
        assert!(
            page.contains("Message\nHello,\nthe answers.\nCopy the text\tChange\n"),
            "{page:?}"
        );
        // in place: the field holds the text where it stood, Save and Cancel beside it, Copy and Change gone
        let (_, _, page) = get("/drafts/grant-reply?edit=1&view=text", "*/*");
        assert!(
            page.contains("Message\nHello, the answers.\tSave\tCancel\nDecision\n"),
            "{page:?}"
        );
        let (_, _, tree) = get("/drafts/grant-reply?edit=1&view=data", "*/*");
        assert!(
            tree.contains(r#""change": "/drafts/grant-reply""#)
                && tree.contains(r#""decide": "/d/grant-reply""#)
                && tree.contains(r#""editing": true"#),
            "{tree}"
        );
        assert!(
            !served_here("POST /d/grant-reply/body HTTP/1.1"),
            "the older app's write of his edit"
        );
        let saved = posted("/d/grant-reply/body", "body=Hello%2C%0A%0Athe+new+answers.");
        assert!(
            saved.starts_with("HTTP/1.1 303 See Other\r\nLocation: /d/grant-reply\r\n"),
            "{saved}"
        );
        let (_, _, page) = get("/drafts/grant-reply?view=text", "*/*");
        assert!(
            page.contains("Message\nHello,\nthe new answers.\n"),
            "{page:?}"
        );
    }

    /// Done-when 3: State's Save keeps the day it goes out, written by `dash draft set <id> --send-on`, and the page then holds
    /// it; a day that is no date sets nothing. Handled another way closes it: posted to the older app's draft write as « moot »,
    /// the page then offers no Change and no day to set, only Back to review.
    #[test]
    fn state_is_kept_by_save_and_handled_another_way_closes_it() {
        let _budget = TestBudget::new();
        let dir = no_app("draft-state");
        let port = older_draft_app(&dir.0);
        OLDER.set(port).expect("the older app's port, once");
        let (_, _, page) = get("/drafts/grant-reply?view=text", "*/*");
        assert!(
            page.contains(
                "State\nStatus\npending · form · grant-reply\nWhere\nTheir form, its message field\n\
                 Bold\nNot checked: plain text until the form is checked (its :form-bold)\n\
                 Goes out on (empty: with the next outbox)\tSave\n"
            ),
            "{page:?}"
        );
        assert!(
            served_here("POST /drafts/grant-reply/set HTTP/1.1"),
            "Rust answers it"
        );
        let refused = posted("/drafts/grant-reply/set", "send-on=tomorrow");
        assert!(refused.starts_with("HTTP/1.1 400 Bad Request"), "{refused}");
        assert!(
            refused.contains("tomorrow: a day is written 2026-11-15, or left empty"),
            "{refused}"
        );
        let saved = posted("/drafts/grant-reply/set", "send-on=2026-11-20");
        assert!(
            saved.starts_with("HTTP/1.1 303 See Other\r\nLocation: /drafts/grant-reply\r\n"),
            "{saved}"
        );
        let (_, _, tree) = get("/drafts/grant-reply?view=data", "*/*");
        assert!(
            tree.contains(r#""send_on": "2026-11-20""#)
                && tree.contains(r#""schedule": "/drafts/grant-reply/set""#),
            "{tree}"
        );

        let (_, _, page) = get("/drafts/grant-reply?view=text", "*/*");
        assert!(page.contains("Reject\nHandled another way\n"), "{page:?}");
        let closed = posted("/d/grant-reply/status", "status=moot");
        assert!(
            closed.starts_with("HTTP/1.1 303 See Other\r\nLocation: /d/grant-reply\r\n"),
            "{closed}"
        );
        let (_, _, page) = get("/drafts/grant-reply?view=text", "*/*");
        assert!(
            page.contains("Status\nmoot · form · grant-reply\n"),
            "{page:?}"
        );
        assert!(page.contains("Decision\nBack to review\n"), "{page:?}");
        assert!(page.contains("Goes out on\n2026-11-20\n"), "{page:?}");
        assert!(
            !page.contains("Change") && !page.contains("(empty: with the next outbox)"),
            "{page:?}"
        );
    }

    /// Done-when 4, read from the older app's database: Waits lists each step still open, a closed one left out, linked.
    #[test]
    fn a_draft_s_waits_are_the_steps_still_open() {
        let _budget = TestBudget::new();
        let dir = no_app("draft-waits");
        older_draft_app(&dir.0);
        let (_, _, page) = get("/drafts/grant-reply?view=text", "*/*");
        assert!(
            page.contains("Why and context Waits on drafts:intro-first\n"),
            "{page:?}"
        );
        assert!(
            page.contains("Why this text\nFit 5.\nSix questions to answer.\n"),
            "{page:?}"
        );
        assert!(
            page.contains("Waits\nIn flight waits on\tdrafts:intro-first\n"),
            "{page:?}"
        );
        assert!(!page.contains("old-one"), "{page:?}");
        let (_, _, tree) = get("/drafts/grant-reply?view=data", "*/*");
        assert!(tree.contains(r#""href": "/drafts/intro-first""#), "{tree}");
    }

    /// rust-ui-hit-panic: each menu page drawn whole in the window's component, the pointer swept over it, then the next one
    /// drawn and swept again before any resolve, as the shell hit-tests a pointer move that lands between its poll and its
    /// redraw. blitz-dom 0.3.0-beta.4 unwrapped there the dropped page's `#dash-nav`, z-index 60 and so hoisted to the root's
    /// stacking context (node.rs:1472); rust/vendor/blitz-dom backports upstream's skip of a dropped node (blitz #1244).
    #[test]
    fn the_pointer_sweeps_each_menu_page_as_the_next_one_is_drawn() {
        let _budget = TestBudget::new();
        let dir = no_app("hit");
        let board = tasks::Board::open(&dir.0).expect("the tasks");
        let (drawn, renders) = futures_channel::mpsc::unbounded::<dash_ui::Drawn>();
        let vdom = dioxus::prelude::VirtualDom::new(dash_ui::Pages);
        vdom.provide_root_context(
            std::sync::Arc::new(Mutex::new(Some(renders))) as dash_ui::Renders
        );
        let mut doc = dash_ui::document(vdom);
        use blitz_dom::Document;
        let first = dash_ui::Drawn::Page(window_page("/tasks", &board));
        drawn.unbounded_send(first).expect("the window's component");
        while doc.poll(None) {}
        let targets: Vec<String> = {
            let doc = doc.inner();
            let links = doc.query_selector_all("#dash-menu a").expect("a selector");
            links
                .iter()
                .filter_map(|&a| doc.get_node(a)?.attr(blitz_dom::local_name!("href")))
                .map(str::to_string)
                .collect()
        };
        assert!(targets.len() >= 10, "the menu's pages: {targets:?}");
        // the paint lists of a page on screen, whatever the menu's order: the first swap drops its nodes too
        doc.inner_mut().resolve(0.0);
        for target in &targets {
            drawn
                .unbounded_send(dash_ui::Drawn::Page(window_page(target, &board)))
                .expect("the window's component");
            while doc.poll(None) {}
            // first the shell's state between its poll and its redraw: the new page's nodes in, the last resolve's paint lists
            for resolved in [false, true] {
                if resolved {
                    doc.inner_mut().resolve(0.0);
                }
                let doc = doc.inner();
                let points = (0..1280)
                    .step_by(16)
                    .flat_map(|x| (0..860).step_by(16).map(move |y| (x, y)));
                // a hit is a node of the tree, never one the swap dropped
                let hits: Vec<_> = points
                    .filter_map(|(x, y)| doc.hit(x as f32, y as f32))
                    .collect();
                assert!(!hits.is_empty(), "{target}: the pointer hit nothing");
                let gone = hits.iter().find(|h| doc.get_node(h.node_id).is_none());
                assert!(
                    gone.is_none(),
                    "{target}, resolved {resolved}: hit a dropped node"
                );
            }
        }
    }

    /// A data folder a window and a server read, with no older app, no worktrees and no real repo: the queue fails fast.
    /// nextest gives each test its own process (rust/CLAUDE.md rule 1), so nothing else reads these variables.
    fn no_app(name: &str) -> dash_meter::TestDir {
        let dir = dash_meter::TestDir::new(name);
        unsafe {
            std::env::set_var("DASH_CLI", "/nonexistent/dash");
            std::env::set_var("DASH_REPO", &dir.0);
            std::env::set_var("DASH_WORKTREES", &dir.0);
        }
        pages::serve_from(&dir.0);
        dir
    }

    /// A page that reads no store is drawn again only while a skeleton is on it, once a second, for five minutes a visit.
    #[test]
    fn the_window_fills_a_skeleton_once_a_second_for_five_minutes() {
        let _budget = TestBudget::new();
        let secs = Duration::from_secs;
        let busy = surface::screen::page(
            "200 OK",
            r#"{"root":"s","elements":{"s":{"type":"skeleton","props":{"what":"Usage"}}}}"#,
        )
        .expect("a page");
        assert!(refill(&busy, secs(2), secs(1)));
        assert!(
            !refill(&busy, secs(2), Duration::from_millis(250)),
            "not four times a second"
        );
        assert!(!refill(&busy, secs(300), secs(1)), "not past five minutes");
        let filled = busy.replace(BEING_READ, "");
        assert!(!refill(&filled, secs(2), secs(1)), "kept once filled");
    }

    /// The column a task's row sits in on a /tasks page: the status of the last group opened before it.
    fn column_of(page: &str, task: &str) -> Option<String> {
        let page = &surface::markup(page);
        let at = page.find(&format!(r#"data-task="{task}""#))?;
        let group = page[..at].rfind(r#"data-status=""#)? + r#"data-status=""#.len();
        let end = page[group..].find('"')?;
        Some(page[group..group + end].to_string())
    }

    /// Done-when 2 of rust-window-in-process: the window and dash serve each read the one store through their own differential
    /// dataflow views (`Board`'s, fed by its op log), and a write to it shows in both within 1 s: the window through its real
    /// render loop, the web through the page dash serve answers.
    #[test]
    fn a_store_write_shows_in_the_window_and_the_web_within_a_second() {
        use dash_core::tasks::TaskStore;
        let _budget = TestBudget::new();
        let bar = Duration::from_secs(1);
        // the window's timer leaves a render's time under the bar
        assert!(dash_ui::REREAD * 2 <= bar, "REREAD {:?}", dash_ui::REREAD);
        let dir = no_app("window-sync");
        let mut tasks = store::tasks::open(&dir.0, &dir.0).expect("the store's writer");
        tasks.add("w-task", &[]).expect("added");
        // two processes in life, two boards here: each its own views over the one store
        let web = Mutex::new(tasks::Board::open(&dir.0).expect("the web's board"));
        let window = tasks::Board::open(&dir.0).expect("the window's board");
        let (asks, asked) = std::sync::mpsc::channel::<dash_ui::Ask>();
        let (drew, drawn) = std::sync::mpsc::channel::<String>();
        std::thread::spawn(move || {
            let draw = |d| match d {
                dash_ui::Drawn::Page(page) | dash_ui::Drawn::Patch(page, _) => {
                    drew.send(page).is_ok()
                }
            };
            let pages = window_pages(window);
            dash_ui::render_loop("/tasks?layout=list", pages, follows_views, asked, draw)
        });
        let line = "GET /tasks?layout=list HTTP/1.1\r\n";
        let served = || unqueued(line, "text/html", Some(&web)).expect("a page").2;
        let first = drawn
            .recv_timeout(Duration::from_secs(10))
            .expect("the window's first page");
        assert_eq!(
            column_of(&first, "w-task").as_deref(),
            Some("backlog"),
            "{first}"
        );
        assert_eq!(column_of(&served(), "w-task").as_deref(), Some("backlog"));

        // both surfaces watched side by side, each timed on its own, 20 ms between the web's asks
        let wrote = Instant::now();
        TaskStore::claim(&mut tasks, "w-task").expect("claimed");
        let claimed = |page: &str| column_of(page, "w-task").as_deref() == Some("in-progress");
        let (mut window_took, mut web_took) = (None, None);
        while (window_took.is_none() || web_took.is_none()) && wrote.elapsed() < 5 * bar {
            for page in drawn.try_iter() {
                if window_took.is_none() && claimed(&page) {
                    window_took = Some(wrote.elapsed());
                }
            }
            if web_took.is_none() && claimed(&served()) {
                web_took = Some(wrote.elapsed());
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        // the render loop ends at its next wait
        drop(asks);
        eprintln!("the claim shown: window {window_took:?}, web {web_took:?}");
        let window_took = window_took.expect("the window drew the claim");
        let web_took = web_took.expect("the web served the claim");
        assert!(window_took < bar, "window: {window_took:?}");
        assert!(web_took < bar, "web: {web_took:?}");
    }

    /// Done-when 1 of rust-view-patches: one board view diff, a claim, reaches the window and the web page as a patch of the
    /// two columns the card left and entered, through each surface's real path: the window's render loop fed by the board's
    /// patches, the web's `GET /tasks?view=patches` on a served port. The board follows the view, so no timer redraws it.
    #[test]
    fn a_board_diff_reaches_the_window_and_the_web_as_a_patch_of_its_columns() {
        use dash_core::tasks::TaskStore;
        let _budget = TestBudget::new();
        let dir = no_app("view-patches");
        let mut tasks = store::tasks::open(&dir.0, &dir.0).expect("the store's writer");
        tasks.add("p-task", &[]).expect("added");
        let wait = Duration::from_secs(10);

        // the web: dash serve's own answer on a port, its board the one the pages lock
        let _ = BOARD.set(Mutex::new(
            tasks::Board::open(&dir.0).expect("the web's board"),
        ));
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let address = listener.local_addr().expect("address");
        let (sender, _requests) = sync_channel::<Request>(4);
        let live = Arc::new(AtomicUsize::new(0));
        std::thread::spawn(move || accept(&listener, &sender, &live, &silent()));
        let mut web = TcpStream::connect(address).expect("connect");
        write!(
            web,
            "GET /tasks?view=patches HTTP/1.1\r\nHost: localhost\r\n\r\n"
        )
        .expect("write");
        let mut web = std::io::BufReader::new(web);
        let mut head = String::new();
        while web.read_line(&mut head).expect("the head") > 2 {}
        assert!(
            head.starts_with("HTTP/1.1 200 OK\r\n") && head.contains("text/event-stream"),
            "{head}"
        );

        // the window: its real render loop on the /tasks board, fed by its own board's patches
        let window = tasks::Board::open(&dir.0).expect("the window's board");
        let changes = window.patches().expect("the window's patches");
        let (asks, asked) = std::sync::mpsc::channel::<dash_ui::Ask>();
        let told = asks.clone();
        std::thread::spawn(move || {
            for ids in changes {
                let _ = told.send(dash_ui::Ask::Changed(ids));
            }
        });
        let (drew, drawn) = std::sync::mpsc::channel::<dash_ui::Drawn>();
        std::thread::spawn(move || {
            let pages = window_pages(window);
            dash_ui::render_loop("/tasks", pages, follows_views, asked, |d| {
                drew.send(d).is_ok()
            })
        });
        // drawn whole until its skeletons fill, then it follows the view
        let end = Instant::now() + wait;
        loop {
            match drawn.recv_timeout(wait) {
                Ok(dash_ui::Drawn::Page(page)) if !page.contains(BEING_READ) => {
                    let page = surface::markup(&page);
                    assert_eq!(
                        column_of(&page, "p-task").as_deref(),
                        Some("backlog"),
                        "{page}"
                    );
                    assert!(
                        page.contains(r#"id="task-p-task""#),
                        "a card is named by its task: {page}"
                    );
                    break;
                }
                Ok(_) => assert!(Instant::now() < end, "the board's skeletons never filled"),
                Err(e) => panic!("no page drawn: {e}"),
            }
        }

        TaskStore::claim(&mut tasks, "p-task").expect("claimed");
        let columns = ["backlog".to_string(), "in-progress".to_string()];
        match drawn.recv_timeout(wait) {
            Ok(dash_ui::Drawn::Patch(page, ids)) => {
                assert_eq!(ids, columns);
                assert_eq!(column_of(&page, "p-task").as_deref(), Some("in-progress"));
            }
            Ok(dash_ui::Drawn::Page(page)) => panic!("drawn whole, not patched: {page}"),
            Err(e) => panic!("no patch drawn: {e}"),
        }
        let mut event = String::new();
        while !event.starts_with("data: ") {
            event.clear();
            assert!(
                web.read_line(&mut event).expect("an event") > 0,
                "the stream ended"
            );
        }
        assert_eq!(event, format!("data: {}\n", json!(columns)));
        drop(asks);
    }

    /// The views started again (`Board::board_view` on any failed read) end every subscription the old ones had: the board's
    /// patches follow the new ones, name the board whole for the diffs missed meanwhile, then each diff as before. Else the
    /// window, drawn again on a diff alone, would never show another write (/code-review of PR 1322).
    #[test]
    fn the_board_s_patches_follow_the_views_started_again() {
        use dash_core::tasks::TaskStore;
        let _budget = TestBudget::new();
        let dir = no_app("view-patches-restart");
        let mut tasks = store::tasks::open(&dir.0, &dir.0).expect("the store's writer");
        tasks.add("r-task", &[]).expect("added");
        let board = tasks::Board::open(&dir.0).expect("the board");
        let changes = board.patches().expect("the board's patches");
        let wait = Duration::from_secs(10);
        board.restart_views().expect("started again");
        assert_eq!(
            changes.recv_timeout(wait).expect("the board whole"),
            ["board"]
        );
        TaskStore::claim(&mut tasks, "r-task").expect("claimed");
        let columns = ["backlog".to_string(), "in-progress".to_string()];
        assert_eq!(changes.recv_timeout(wait).expect("the diff"), columns);
    }

    /// The elements a board diff names: a card's words changed in its column is that card; a move, an arrival or a departure
    /// is each column touched, and a card in a column drawn whole is not named again.
    #[test]
    fn a_board_diff_names_its_cards_or_its_columns() {
        let _budget = TestBudget::new();
        let row =
            |id: &str, column: &str, title: &str| dash_views::rules::board_row(id, column, title);
        let renamed = vec![(row("a", "todo", "A"), -1), (row("a", "todo", "A2"), 1)];
        assert_eq!(tasks::patched(&renamed), ["task-a"]);
        let moved = [
            renamed.clone(),
            vec![
                (row("b", "todo", "B"), -1),
                (row("b", "in-progress", "B"), 1),
            ],
        ]
        .concat();
        assert_eq!(tasks::patched(&moved), ["in-progress", "todo"]);
        let arrived = vec![
            (row("c", "backlog", "C"), 1),
            (row("d", "in-review", "D"), -1),
        ];
        assert_eq!(tasks::patched(&arrived), ["backlog", "in-review"]);
        let other = vec![(dash_views::rules::needs("k".into(), "task", "x".into()), 1)];
        assert!(
            tasks::patched(&other).is_empty(),
            "another view's rows patch nothing here"
        );
    }

    /// The lines a stand-in terminal was given, each « <address> <line> ».
    static TYPED: Mutex<Vec<String>> = Mutex::new(Vec::new());

    /// How many times the stand-in's screen was read.
    static READS: AtomicUsize = AtomicUsize::new(0);

    /// A terminal whose screen echoes each line typed into it, as a shell would.
    struct StandIn(Mutex<String>);

    impl dash_core::tasks::Terminal for StandIn {
        fn open(&self, _: &str, _: &str) -> Result<String, String> {
            Err("not the page's".into())
        }
        fn run(&self, _: &str, _: &str) -> Result<(), String> {
            Err("not the page's".into())
        }
        fn agent(&self, _: &str) -> Result<Option<dash_core::tasks::Seen>, String> {
            Err("not the page's".into())
        }
        fn sessions(&self) -> Result<usize, String> {
            Err("not the page's".into())
        }
        fn read(&self, pane: &str) -> Result<String, String> {
            READS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            match pane {
                "t7" => Ok(self.0.lock().expect("screen").clone()),
                _ => Err(format!("no terminal \"{pane}\"")),
            }
        }
        fn send(&self, pane: &str, text: &str) -> Result<(), String> {
            TYPED.lock().expect("typed").push(format!("{pane} {text}"));
            *self.0.lock().expect("screen") +=
                &format!("{text}\n{}\n$ ", text.trim_start_matches("echo "));
            Ok(())
        }
        fn key(&self, pane: &str, key: &str) -> Result<(), String> {
            TYPED
                .lock()
                .expect("typed")
                .push(format!("{pane} key {key}"));
            Ok(())
        }
        fn close(&self, _: &str) -> Result<(), String> {
            Err("not the page's".into())
        }
    }

    /// Done-when 2 of rust-terminal-page, through dash serve's own answer on a port: /terminal/t7 draws the stand-in's screen,
    /// its form posts a line to Terminal::send and leads back to the page, and the page's patch stream names the screen once
    /// the line changed it.
    #[test]
    fn a_line_posted_reaches_terminal_send_and_the_screen_follows_as_a_patch() {
        let _budget = TestBudget::new();
        assert!(
            terminal::TERMINAL
                .set(Box::new(StandIn(Mutex::new("$ ".into()))))
                .is_ok(),
            "nextest runs this test in a process of its own"
        );
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let address = listener.local_addr().expect("address");
        let (sender, _requests) = sync_channel::<Request>(4);
        let live = Arc::new(AtomicUsize::new(0));
        std::thread::spawn(move || accept(&listener, &sender, &live, &silent()));
        let ask = |request: String| {
            let mut web = TcpStream::connect(address).expect("connect");
            web.write_all(request.as_bytes()).expect("write");
            let mut answer = String::new();
            web.read_to_string(&mut answer).expect("the answer");
            answer
        };
        let page = ask(
            "GET /terminal/t7 HTTP/1.1\r\nHost: localhost\r\nAccept: text/plain\r\n\r\n".into(),
        );
        assert!(
            page.ends_with(&format!(
                "\r\n\r\nTerminal t7\n$ \n\nType a line: POST /terminal/t7/send, the line as text\n\n{}",
                surface::forms::menu_text()
            )),
            "{page}"
        );

        // the page's stream, open before the line is typed
        let mut stream = TcpStream::connect(address).expect("connect");
        write!(
            stream,
            "GET /terminal/t7?view=patches HTTP/1.1\r\nHost: localhost\r\n\r\n"
        )
        .expect("write");
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .expect("timeout");
        let mut stream = std::io::BufReader::new(stream);
        let mut head = String::new();
        while stream.read_line(&mut head).expect("the head") > 2 {}
        assert!(
            head.starts_with("HTTP/1.1 200 OK\r\n") && head.contains("text/event-stream"),
            "{head}"
        );
        // its first read comes after its head: the line is typed once the stream holds the screen it compares with
        let end = Instant::now() + Duration::from_secs(10);
        while READS.load(std::sync::atomic::Ordering::SeqCst) < 2 {
            assert!(Instant::now() < end, "the stream read the screen");
            std::thread::sleep(Duration::from_millis(10));
        }

        let post = |form: &str, origin: &str| {
            ask(format!(
                "POST /terminal/t7/send HTTP/1.1\r\nHost: localhost\r\nOrigin: {origin}\r\nContent-Length: {}\r\n\r\n{form}",
                form.len()
            ))
        };
        let refused = post("text=rm+x", "http://elsewhere");
        assert!(
            refused.starts_with("HTTP/1.1 403 "),
            "another site posts nothing: {refused}"
        );
        let sent = post("text=echo+hc-%2B42", "http://localhost");
        assert!(
            sent.starts_with("HTTP/1.1 303 See Other\r\nLocation: /terminal/t7\r\n"),
            "{sent}"
        );
        let enter = post("text=", "http://localhost");
        assert!(enter.starts_with("HTTP/1.1 303 "), "{enter}");
        assert_eq!(
            *TYPED.lock().expect("typed"),
            ["t7 echo hc-+42", "t7 key Enter"]
        );

        let mut event = String::new();
        stream
            .read_line(&mut event)
            .expect("an event within a second");
        assert_eq!(event, "data: [\"terminal-screen\"]\n");
        let page = ask(
            "GET /terminal/t7 HTTP/1.1\r\nHost: localhost\r\nAccept: text/plain\r\n\r\n".into(),
        );
        assert!(
            page.contains("\n$ echo hc-+42\nhc-+42\n$ \n"),
            "the page drawn again: {page}"
        );

        let gone = ask(
            "GET /terminal/t9 HTTP/1.1\r\nHost: localhost\r\nAccept: text/plain\r\n\r\n".into(),
        );
        assert!(gone.contains("no terminal \"t9\""), "{gone}");
        let flag = ask(
            "GET /terminal/-h HTTP/1.1\r\nHost: localhost\r\nAccept: text/plain\r\n\r\n".into(),
        );
        assert!(
            flag.contains("no terminal is named « -h »"),
            "never one of hc's flags: {flag}"
        );
        let typed = String::from(
            "POST /terminal/-h/send HTTP/1.1\r\nHost: localhost\r\nOrigin: http://localhost\r\nContent-Length: 6\r\n\r\ntext=x",
        );
        let flag = ask(typed);
        assert!(
            flag.starts_with("HTTP/1.1 404 "),
            "a client's error, never hc's: {flag}"
        );
        assert_eq!(TYPED.lock().expect("typed").len(), 2, "nothing typed");
    }
}
