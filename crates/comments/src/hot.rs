//! The hot files' check (task hot-files-fable; @nha, 2026-10-29: « run a periodic biweekly check on this (automated) and alert
//! the orchestrator », « you won't need to look if you automate. »): twice a week main's [fix] PRs of `HOT_DAYS` days are
//! ranked by file and the top 5 wake the orchestrator. A PR touching one past `HOT_FIXES` is reviewed on `HOT_MODEL`
//! (dash_orch's `reviewer_model`).
use crate::watch::{Item, Kind, land};
use dash_core::{
    HOT_DAYS, HOT_FIXES, HOT_MODEL, Host, OpLog, fix_counts, fix_log, hot, pr_files, title_task,
};
use std::io::Write;

/// Twice a week, in minutes.
pub const EVERY_MIN: i64 = 7 * 24 * 60 / 2;
const USAGE: &str = "dash orch --data <dir> --hot-files";

dash_core::submit! {
    dash_core::Automation {
        key: "hot-files",
        title: "Hot files: the files with the most fix PRs in 14 days wake the orchestrator",
        why: "(@nha, 2026-10-29: « when a file / feature gets too many bugs get a bigger model to review and/or fix it (Fable) ») \
              Run twice a week by its automation (task hot-files-fable): this task reopens when its command fails.",
        done_when: "1. dash orch --data <dir> --hot-files runs clean at the automation's next pass",
        epic: "orchestration",
        every: dash_core::Every::Minutes(EVERY_MIN),
        does: dash_core::Does::Command(&["orch", "--hot-files"]),
        mcp: &[],
        pause_file: None,
        urgent: None,
    }
}

dash_core::submit! { dash_core::Command { usage: USAGE, words: &["orch", "--hot-files"], run: Some(run) } }

/// Main's squash merges of `HOT_DAYS` days in the host's checkout, as the last fetch left them, ranked and woken on.
fn run(host: &dyn Host, args: &[&str], out: &mut dyn Write) {
    if !args.is_empty() {
        eprintln!("usage: {USAGE}");
        std::process::exit(2)
    }
    let said = fixes(host).and_then(|fixes| {
        let mut log = host.log(&host.data(), "comments.sqlite", &[crate::SCHEMA])?;
        wake(&mut *log, &fix_counts(&fixes), host.now_ms())
    });
    match said {
        Ok(said) => {
            let _ = writeln!(out, "{said}");
        }
        Err(e) => {
            eprintln!("hot files: {e}");
            std::process::exit(1)
        }
    }
}

/// `bin` run with `args` in the host's checkout (gh reads its repository there): its output, or why none. Unbounded here, as a
/// plugin takes no runner but dash-sdk's: both callers bound it, the automation's run (dash's `automations::RUN`) and
/// session.sh's `bounded 30` (/code-review of PR 1692).
fn live(host: &dyn Host, bin: &str, args: &[&str]) -> Result<String, String> {
    let ran = std::process::Command::new(bin)
        .args(args)
        .current_dir(host.repo())
        .output()
        .map_err(|e| format!("{bin}: {e}"))?;
    match ran.status.success() {
        true => Ok(String::from_utf8_lossy(&ran.stdout).into_owned()),
        false => Err(format!(
            "{bin}: {}",
            String::from_utf8_lossy(&ran.stderr).trim()
        )),
    }
}

/// Main's fix history (`fix_log`), as dash's GitHub forge reads it. ponytail: main typed, as the forge's own `base` is, every
/// Rust run being site_perso's; read forge.env's branch if another repo runs one.
fn fixes(host: &dyn Host) -> Result<String, String> {
    let args = fix_log("main");
    live(
        host,
        "git",
        &args.iter().map(String::as_str).collect::<Vec<_>>(),
    )
}

const MODEL_USAGE: &str = "dash review model <task id>";

dash_core::submit! { dash_core::Command { usage: MODEL_USAGE, words: &["review", "model"], run: Some(model_run) } }

/// The PM check's reviewer launch (herdr/session.sh, for a `review-<id>` task) asks it: `HOT_MODEL` printed when task `id`'s
/// open PR touches a hot file, else nothing, and the launch keeps its own model. A read that fails exits 1, the same.
fn model_run(host: &dyn Host, args: &[&str], out: &mut dyn Write) {
    let [id] = args else {
        eprintln!("usage: {MODEL_USAGE}");
        std::process::exit(2)
    };
    let gh = |args: &[&str]| live(host, "gh", args);
    match fixes(host).and_then(|log| reviewer_model(id, &gh, &log)) {
        // one start on it a task: a reviewer started again (a refusal, a limit, a failed start) keeps the launch's own model,
        // as CLAUDE.md says of a refusal (/code-review of PR 1692)
        Ok(Some(model)) if first_start(&host.data(), id) => {
            let _ = writeln!(out, "{model}");
        }
        Ok(Some(_)) => {}
        Ok(None) => {}
        Err(e) => {
            eprintln!("review model: {e}");
            std::process::exit(1)
        }
    }
}

/// Whether this is the first `HOT_MODEL` start of task `id`'s reviewer under `data`: marked once, so a second is not. A mark
/// that cannot be written answers no: the launch keeps its own model.
fn first_start(data: &std::path::Path, id: &str) -> bool {
    let dir = data.join("review-model");
    let _ = std::fs::create_dir_all(&dir);
    (std::fs::OpenOptions::new().write(true).create_new(true))
        .open(dir.join(id))
        .is_ok()
}

/// The model a reviewer of task `id`'s open PR starts on (task hot-files-fable): `HOT_MODEL` when it changes a file `log`
/// (`fix_log`'s output) shows fixed more than `HOT_FIXES` times, else none. `gh` is GitHub's CLI; the PR is the open one whose
/// title names `id` (`title_task`).
pub fn reviewer_model(
    id: &str,
    gh: &dyn Fn(&[&str]) -> Result<String, String>,
    log: &str,
) -> Result<Option<&'static str>, String> {
    let search = format!("{id} in:title");
    let rows = r#".[] | "\(.number) \(.title)""#;
    let listed = gh(&[
        "pr",
        "list",
        "--state",
        "open",
        "--search",
        &search,
        // a short id matches many titles: past gh's 30 its own could fall off
        "--limit",
        "200",
        "--json",
        "number,title",
        "-q",
        rows,
    ])?;
    let own = |line: &str| {
        let (n, title) = line.split_once(' ')?;
        (title_task(title).as_deref() == Some(id)).then(|| n.to_string())
    };
    let Some(n) = listed.lines().find_map(own) else {
        return Ok(None);
    };
    let args = pr_files(n.parse().map_err(|e| format!("PR number {n}: {e}"))?);
    let files = gh(&args.iter().map(String::as_str).collect::<Vec<_>>())?;
    Ok((!hot(&fix_counts(log), files.lines()).is_empty()).then_some(HOT_MODEL))
}

/// `counts`' 5 most fixed files landed in `log` as one wake at `now_ms`, a key a run so each run wakes, once one is past
/// `HOT_FIXES`. What it did.
pub fn wake(
    log: &mut dyn OpLog,
    counts: &[(String, usize)],
    now_ms: i64,
) -> Result<String, String> {
    let top: Vec<String> = counts
        .iter()
        .take(5)
        .map(|(f, n)| format!("{f} ({n})"))
        .collect();
    // a file fixed once is no news: only one past HOT_FIXES wakes it (/code-review of PR 1692)
    if !counts.iter().any(|(_, n)| *n > HOT_FIXES) {
        return Ok(format!(
            "no file past {HOT_FIXES} [fix] PRs merged in {HOT_DAYS} days: none woken"
        ));
    }
    let title = format!(
        "Hot files, by [fix] PRs merged in {HOT_DAYS} days: {}. A PR touching one past {HOT_FIXES} is reviewed on {HOT_MODEL}; \
         find what keeps breaking there and file its fix (task hot-files-fable)",
        top.join(", ")
    );
    land(
        log,
        Item::new(
            &format!("hot-files-{now_ms}"),
            Kind::Block,
            None,
            &title,
            "",
            now_ms,
        ),
    )?;
    Ok(format!("woke the orchestrator: {title}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;

    /// Task hot-files-fable, done-when 2, the PM check's reviewer (herdr/session.sh asks it for a `review-<id>` launch): a task
    /// whose open PR changes a file fixed 4 times starts its reviewer on fable; one changing none, or with no open PR, keeps the
    /// launch's own model. A PR whose title only mentions the id is not its own.
    #[dash_test]
    fn a_review_of_a_pr_on_a_hot_file_starts_on_fable() {
        let log: String = (1..=4)
            .map(|n| format!("\x1e[fix][t{n}] a fix (#{n})\n\nhot.rs\n"))
            .collect();
        let gh = |files: &'static str| {
            move |args: &[&str]| -> Result<String, String> {
                Ok(match args[0] {
                    "pr" => "8 [feat][other] mentions t1\n9 [fix][t1] its own\n".into(),
                    _ => {
                        assert_eq!(
                            args[1], "repos/{owner}/{repo}/pulls/9/files",
                            "its own PR's files"
                        );
                        files.into()
                    }
                })
            }
        };
        assert_eq!(
            reviewer_model("t1", &gh("a.rs\nhot.rs\n"), &log),
            Ok(Some("fable"))
        );
        assert_eq!(reviewer_model("t1", &gh("a.rs\n"), &log), Ok(None));
        assert_eq!(
            reviewer_model("t2", &gh("hot.rs\n"), &log),
            Ok(None),
            "no PR of its own"
        );
        let three: String = log
            .split('\x1e')
            .skip(2)
            .map(|c| format!("\x1e{c}"))
            .collect();
        assert_eq!(
            reviewer_model("t1", &gh("hot.rs\n"), &three),
            Ok(None),
            "3 is not past 3"
        );
        let refused = |_: &[&str]| -> Result<String, String> { Err("gh: no answer".into()) };
        assert!(reviewer_model("t1", &refused, &log).is_err());
    }

    /// /code-review of PR 1692: `dash review model` answers fable once a task, so a reviewer started again after a refusal
    /// runs on the launch's own opus; another task gets its own first start.
    #[dash_test]
    fn a_reviewer_gets_fable_once_a_task() {
        let data = std::env::temp_dir().join(format!("hot-first-start-{}", std::process::id()));
        assert!(first_start(&data, "t1"));
        assert!(!first_start(&data, "t1"), "a second start keeps opus");
        assert!(first_start(&data, "t2"));
        assert!(
            !first_start(std::path::Path::new("/dev/null/x"), "t3"),
            "an unwritable mark: no fable"
        );
    }

    /// Task hot-files-fable, done-when 1: twice a week the app's automation runs `dash orch --hot-files`, whose pass wakes the
    /// orchestrator with the 5 files of most [fix] PRs, a wake a run; none past 3 wakes nothing.
    #[dash_test]
    fn twice_a_week_the_top_5_fixed_files_wake_the_orchestrator() {
        use dash_core::{Automation, Does, Every};
        let a = (inventory::iter::<Automation>.into_iter())
            .find(|a| a.key == "hot-files")
            .expect("registered");
        assert!(matches!(a.every, Every::Minutes(5040)), "twice a week");
        assert!(matches!(a.does, Does::Command(["orch", "--hot-files"])));
        let command = (inventory::iter::<dash_core::Command>.into_iter())
            .find(|c| c.words == ["orch", "--hot-files"]);
        assert!(command.is_some_and(|c| c.run.is_some()), "its command runs");

        let mut log_text = String::new();
        for (n, files) in [
            (1, "a.rs\nb.rs\nc.rs"),
            (2, "a.rs\nb.rs\nd.rs"),
            (3, "a.rs\ne.rs\nf.rs"),
            (4, "a.rs"),
        ] {
            log_text += &format!("\x1e[fix][t{n}] a fix (#{n})\n\n{files}\n");
        }
        let counts = fix_counts(&log_text);
        let mut log = dash_core::MemoryLog::default();
        let said = wake(&mut log, &counts, 1_000).expect("woke");
        wake(&mut log, &counts, 2_000).expect("woke again");
        let wakes = crate::watch::load(&log).unwrap();
        let titles: Vec<(&str, &str)> = (wakes.iter())
            .map(|(k, w)| (k.as_str(), w.title.as_str()))
            .collect();
        println!("{said}\n{titles:?}");
        let top = "Hot files, by [fix] PRs merged in 14 days: a.rs (4), b.rs (2), c.rs (1), d.rs (1), e.rs (1). ";
        assert_eq!(titles.len(), 2, "a wake a run: {titles:?}");
        assert!(titles.iter().all(|(_, t)| t.starts_with(top)), "{titles:?}");
        assert!(!titles[0].1.contains("f.rs"), "the top 5 only");
        let none_hot = "no file past 3 [fix] PRs merged in 14 days: none woken";
        assert_eq!(wake(&mut log, &[], 3_000), Ok(none_hot.into()));
        let three = [("a.rs".to_string(), 3), ("b.rs".to_string(), 1)];
        assert_eq!(
            wake(&mut log, &three, 4_000),
            Ok(none_hot.into()),
            "3 is not past 3"
        );
    }
}
