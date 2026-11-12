//! The older app's list of tasks on its CLI (section 20b's bridge), deleted with its part: its list stays the list of tasks and
//! its judge reads each report, so moving a task back loses nothing. Core's TaskList is the binary's, which writes each brief
//! itself (task brief-tests-prepush-skip): `dash task brief` is never asked.
use dash_core::tasks::{Outcome, Verdict};
use dash_process as process;
use serde_json::Value;
use std::time::Duration;

pub struct Dash {
    pub bin: String,
}

/// A read's limit; a judge waits on its model, up to 50 s (PM's own step allows 90).
const CALL: Duration = Duration::from_secs(60);
const JUDGE: Duration = Duration::from_secs(120);
/// A block sits at a message's end, so its tail holds it; a word on a command line has a size.
const SAID: usize = 8000;

impl Dash {
    /// bin/dash gives up after its own 60 s unless `DASH_TIMEOUT` says otherwise, so the limit asked here is passed to it: a
    /// read given more time gets it (`dash feedback count` failed at bin/dash's 60 s under its 120 s limit, task
    /// rust-home-prompt's live check).
    fn call(&self, args: &[&str], limit: Duration) -> Result<String, String> {
        self.call_fed(args, None, limit)
    }

    /// `call`, `stdin` given to the command, as a `-` word asks.
    fn call_fed(
        &self,
        args: &[&str],
        stdin: Option<&str>,
        limit: Duration,
    ) -> Result<String, String> {
        self.call_env(args, stdin, &[], limit)
    }

    /// bin/dash's whole answer, told the limit as `DASH_TIMEOUT` unless `env` names its own, then `env`; no answer keeps
    /// whether it ran (`NoAnswer::started`).
    fn answer(
        &self,
        args: &[&str],
        stdin: Option<&str>,
        env: &[(&str, &str)],
        limit: Duration,
    ) -> Result<(std::process::ExitStatus, String, String), process::NoAnswer> {
        let secs = limit.as_secs().to_string();
        let mut all = vec![("DASH_TIMEOUT", secs.as_str())];
        // a later pair of one name wins, as Command::env sets them in turn
        all.extend_from_slice(env);
        process::answer_fed(&self.bin, args, None, &all, stdin, limit)
    }

    /// `call_fed` with `env` for bin/dash.
    fn call_env(
        &self,
        args: &[&str],
        stdin: Option<&str>,
        env: &[(&str, &str)],
        limit: Duration,
    ) -> Result<String, String> {
        let (status, text, reason) = self.answer(args, stdin, env, limit)?;
        match status.success() {
            true => Ok(text),
            false => Err(format!(
                "{} {}: {status}: {} {}",
                self.bin,
                args.join(" "),
                reason.trim(),
                text.trim()
            )),
        }
    }
}

fn json(text: &str) -> Result<Value, String> {
    serde_json::from_str(text).map_err(|e| format!("dash's answer: {e}: {text}"))
}

/// One check of a task's done-when. `met`: the last judgement's word on it, `None` before one.
#[derive(Clone, Debug, PartialEq)]
pub struct Check {
    pub text: String,
    pub met: Option<bool>,
}

/// What the older app holds of an open task that Rust's store does not: what its page shows.
#[derive(Clone, Debug, PartialEq)]
pub struct Issue {
    pub id: String,
    pub title: String,
    pub why: String,
    pub checks: Vec<Check>,
    /// The words of each open comment of his this task handles.
    pub comments: Vec<String>,
}

fn issues_of(text: &str) -> Result<Vec<Issue>, String> {
    let answer = json(text)?;
    let issues = answer.as_array().ok_or(format!("no issues in: {text}"))?;
    let word = |v: &Value| v.as_str().unwrap_or_default().to_string();
    let list = |v: &Value| v.as_array().cloned().unwrap_or_default();
    let check = |c: &Value| Check {
        text: word(&c["text"]),
        met: c["met"].as_bool(),
    };
    let issue = |i: &Value| Issue {
        id: word(&i["id"]),
        title: word(&i["title"]),
        why: word(&i["why"]),
        checks: list(&i["checks"]).iter().map(check).collect(),
        comments: list(&i["comments"]).iter().map(word).collect(),
    };
    Ok(issues.iter().map(issue).collect())
}

/// An open task of any project, in its project's own status words (« open », « To Do », « Active »).
#[derive(Clone, Debug, PartialEq)]
pub struct Open {
    pub project: String,
    pub id: String,
    pub status: String,
    pub title: String,
}

/// `dash task ls`'s rows: the project, id, status and title a tab apart, after a count line per project.
fn open_of(text: &str) -> Vec<Open> {
    let row = |line: &str| {
        let mut cols = line.split('\t');
        let (project, id, status, title) = (cols.next()?, cols.next()?, cols.next()?, cols.next()?);
        let open = Open {
            project: project.into(),
            id: id.into(),
            status: status.into(),
            title: title.into(),
        };
        Some(open).filter(|o| !o.id.is_empty())
    };
    text.lines().filter_map(row).collect()
}

impl Dash {
    /// Each open task Rust owns, held or not, in the list's order: a page's read, so not one of the picker's ports.
    pub fn issues(&self) -> Result<Vec<Issue>, String> {
        issues_of(&self.call(&["task", "issues"], CALL)?)
    }

    /// His new issue from the board's form (task rust-board-buttons-new-issue): `dash task file`, which files his words on
    /// `project`'s backlog, its id and title made from them; another project's goes to its own repo. What it printed.
    pub fn file(&self, project: &str, words: &str) -> Result<String, String> {
        self.call(&["task", "file", project, words], CALL)
    }

    /// One field of a task he set from its page's property menu (task rust-board-buttons-new-issue): `dash task update <id>
    /// <flag> <value>...`, `--priority 2`, `--deadline 2026-11-15`, `--labels Bug --labels Docs` (a flag given once per
    /// value). What it printed.
    pub fn update(&self, id: &str, flags: &[(&str, &str)]) -> Result<String, String> {
        let mut args = vec!["task", "update", id];
        args.extend(flags.iter().flat_map(|(f, v)| [*f, *v]));
        self.call(&args, CALL)
    }

    /// A text he wrote on a task's page (task rust-task-page-edits): `dash task update <id> <flag> -`, the text on stdin, so words
    /// that start with `-` are his words, never a flag (/code-review). What it printed.
    pub fn update_text(&self, id: &str, flag: &str, text: &str) -> Result<String, String> {
        self.call_fed(&["task", "update", id, flag, "-"], Some(text), CALL)
    }

    /// A field of a tasks.edn row that `dash task update` takes no flag for (task rust-issue-sidebar-menus): `dash store set
    /// tasks.edn <id> --<field> <value>`, `--cycle 2026-11-09` or `--remind nil`. What it printed.
    pub fn set(&self, id: &str, field: &str, value: &str) -> Result<String, String> {
        let flag = format!("--{field}");
        self.call(&["store", "set", "tasks.edn", id, &flag, value], CALL)
    }

    /// Every project's open tasks, as `dash task ls` lists them: the board's rows beside Rust's own (task rust-nav-polish).
    /// Its rows whatever its exit: one project's unreadable file fails the command, and the other projects' rows still stand
    /// (/code-review of rust-nav-polish).
    pub fn every_task(&self) -> Result<Vec<Open>, String> {
        let (status, text, why) = process::answer(&self.bin, &["task", "ls"], None, &[], CALL)?;
        let rows = open_of(&text);
        match rows.is_empty() && !status.success() {
            true => Err(format!("{} task ls: {status}: {why}", self.bin)),
            false => Ok(rows),
        }
    }

    /// What waits on him, as `dash handoff` prints it: Home's first part (task rust-nav-board).
    pub fn handoff(&self) -> Result<String, String> {
        self.call(&["handoff"], CALL)
    }

    /// Every command it answers, one a line, its label first: `dash help all`, what a fold's « dash <command> » is checked
    /// against (task rust-fold-port).
    pub fn help_all(&self) -> Result<String, String> {
        self.call(&["help", "all"], CALL)
    }

    /// Today's daily: the one step `dash progress` names, and what it lists.
    pub fn progress(&self) -> Result<String, String> {
        self.call(&["progress"], CALL)
    }

    /// His feedback against the work done, `dash feedback count`'s tables: /insights'. It took 48 s on 2026-11-09, so it has
    /// the longer limit.
    pub fn feedback_count(&self) -> Result<String, String> {
        self.call(&["feedback", "count"], JUDGE)
    }

    /// Tokens and hours per estimate point, `dash cost fit`: /insights' last part.
    pub fn cost_fit(&self) -> Result<String, String> {
        self.call(&["cost", "fit"], CALL)
    }

    /// The agent kind its row names (`run.agent`), claude when it names none, as the older app's launch reads it.
    pub fn agent(&self, id: &str) -> Result<String, String> {
        Ok(agent_of(&self.call(&["task", "show", id], CALL)?))
    }

    /// A hook's read (`dash store text`, `dash model drafts`): 10 s and no restart wait, so a session is never held on an
    /// app that is down; stdout, or why not.
    pub fn quick(&self, args: &[&str]) -> Result<String, String> {
        let env = [("DASH_TIMEOUT", "10"), ("DASH_RESTART_TRIES", "0")];
        self.call_env(args, None, &env, Duration::from_secs(20))
    }

    /// One line of the older app's outbound log (`dash model outbound add`), where its drafts and leads read what went: a send
    /// made in Rust (task rust-mail-thread) and each send dash hook outbound-log sees. `env` for bin/dash, `limit` its end.
    /// A line the app surely did not take is kept in `main`'s private/live/pending/, which the app appends at its next check
    /// (Store.take_pending!/0); one it may have taken (its limit passed, a signal, « may still be running ») is never kept,
    /// so never added twice (/code-review of send-strong-fits). Ok once logged; Err says what was done instead.
    pub fn outbound(
        &self,
        line: &str,
        env: &[(&str, &str)],
        limit: Duration,
        main: &std::path::Path,
    ) -> Result<(), String> {
        let unconfirmed = |why: &str| {
            format!(
                "dashboard log unconfirmed ({why}): not appended again; check private/dashboard/bin/dash store text outbound.jsonl"
            )
        };
        match self.answer(&["model", "outbound", "add", line], None, env, limit) {
            Ok((status, ..)) if status.success() => return Ok(()),
            Err(no) if no.started => return Err(unconfirmed(&no.why)),
            Err(_) => {}
            Ok((status, text, err)) => {
                let partway =
                    err.contains("may still be running") || err.contains("may have run partway");
                // no code: a signal ended it, maybe once the app had the line
                if matches!(status.code(), None | Some(124))
                    || (status.code() == Some(3) && partway)
                {
                    let why = format!("{}: {status}: {} {}", self.bin, err.trim(), text.trim());
                    return Err(unconfirmed(why.trim()));
                }
            }
        }
        match pending(line, main) {
            Ok(folder) => Err(format!(
                "outbound send kept in {}: the dashboard logs it at its next check",
                folder.display()
            )),
            Err(e) => Err(format!("outbound send not logged nor kept: {e}: {line}")),
        }
    }
}

/// The older app's LinkedIn gate (Dashboard.Linkedin), asked by dash hook linkedin-budget before each LinkedIn call: its JSON
/// answer to `op`, or None when it does not answer, which the hook reads as a refusal. `LINKEDIN_GATE_URL`, as the fork asks
/// it, never a worktree's own app (/code-review of #159).
pub fn linkedin_gate(op: &str, body: &Value) -> Option<Value> {
    let gate = std::env::var("LINKEDIN_GATE_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:7788/dashboard/linkedin/".into());
    let url = format!("{}/{op}", gate.trim_end_matches('/'));
    let body = body.to_string();
    let args = [
        "-sS",
        "--max-time",
        "5",
        "-X",
        "POST",
        "-H",
        "content-type: application/json",
        "-H",
        "x-dashboard-cli: 1",
        "--data-binary",
        &body,
        "-w",
        "\n%{http_code}",
        &url,
    ];
    let (status, text, _) =
        process::answer("curl", &args, None, &[], Duration::from_secs(8)).ok()?;
    let (got, code) = text.trim_end().rsplit_once('\n')?;
    // an HTTP error status, a cut body or bad JSON: no answer, so a refusal
    (status.success() && code.starts_with('2')).then(|| serde_json::from_str(got).ok())?
}

/// Keeps `line` as its own file in `main`'s private/live/pending/, renamed in once whole: an append to one shared file could
/// land in the copy the app was taking and be removed with it (/code-review of PR 551).
fn pending(line: &str, main: &std::path::Path) -> std::io::Result<std::path::PathBuf> {
    let folder = main.join("private/live/pending");
    std::fs::create_dir_all(&folder)?;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let id = format!("{nanos:x}{:x}", std::process::id());
    let tmp = folder.join(format!(".tmp-{id}"));
    std::fs::write(&tmp, format!("{}\n", line.trim_end_matches('\n')))?;
    std::fs::rename(&tmp, folder.join(format!("outbound.jsonl.line-{id}")))?;
    Ok(folder)
}

fn agent_of(shown: &str) -> String {
    let named = shown.lines().find_map(|l| l.strip_prefix("run.agent: "));
    named.map_or("claude", str::trim).to_string()
}

fn verdict_of(text: &str) -> Result<Verdict, String> {
    let answer = json(text)?;
    let said = |field: &str| answer[field].as_str().unwrap_or_default().to_string();
    match answer["verdict"].as_str() {
        Some("met") => Ok(Verdict::Met),
        Some("not-met") => Ok(Verdict::NotMet(said("missing"))),
        Some("blocked") => Ok(Verdict::Blocked(said("needs"))),
        Some("refused") => Ok(Verdict::NotOurs(said("why"))),
        Some("unanswered") => Err("the judge gave no answer".to_string()),
        _ => Err(format!("no verdict in: {text}")),
    }
}

/// `dash task waiting`'s word for what a session waits on.
fn waiting_word(on: Option<&Outcome>) -> &'static str {
    match on {
        Some(Outcome::Offline(_)) => "network",
        Some(Outcome::Limited(_)) => "limit",
        _ => "over",
    }
}

impl Dash {
    pub fn judge(&self, id: &str, said: &str) -> Result<Verdict, String> {
        let tail = said.chars().rev().take(SAID).collect::<Vec<_>>();
        let tail: String = tail.into_iter().rev().collect();
        verdict_of(&self.call(&["task", "judge", id, &tail], JUDGE)?)
    }

    pub fn merged(&self, id: &str, pr: &str) -> Result<(), String> {
        let note = "Rust's orchestrator merged it, the PM check's verdict met";
        let args = ["task", "done", id, pr, "merged", note];
        self.call(&args, CALL).map(drop)
    }

    pub fn merged_later(&self, id: &str, pr: &str) -> Result<(), String> {
        let note = "merged after its Rust run ended, judged again by the PM check: met";
        let args = ["task", "done", id, pr, "merged", note];
        self.call(&args, CALL).map(drop)
    }

    pub fn ended(&self, id: &str, why: &str) -> Result<(), String> {
        self.call(&["task", "ended", id, why], CALL).map(drop)
    }

    pub fn reopen(&self, id: &str) -> Result<(), String> {
        self.call(&["task", "reopen", id], CALL).map(drop)
    }

    pub fn waiting(&self, id: &str, on: Option<&Outcome>) -> Result<(), String> {
        let on = waiting_word(on);
        self.call(&["task", "waiting", id, on], CALL).map(drop)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;

    /// Task rust-home-prompt: bin/dash is told the bridge's limit, never left at its own 60 s.
    #[dash_test]
    fn a_call_tells_bin_dash_its_limit() {
        let dash = Dash {
            bin: "/usr/bin/printenv".into(),
        };
        assert_eq!(dash.call(&["DASH_TIMEOUT"], JUDGE).as_deref(), Ok("120\n"));
        assert!(dash.call(&["NO_SUCH_VAR_HERE"], CALL).is_err());
    }

    #[dash_test]
    fn every_project_s_open_tasks_are_read_off_task_ls() {
        let ls = "klar: 1 open\nsite_perso: 1 open\n\nklar\t12\tTo Do\tSign up\nsite_perso\tnow-note\topen\tdash now prints it\t3pt\n";
        let open = |p: &str, id: &str, s: &str, t: &str| Open {
            project: p.into(),
            id: id.into(),
            status: s.into(),
            title: t.into(),
        };
        assert_eq!(
            open_of(ls),
            vec![
                open("klar", "12", "To Do", "Sign up"),
                open("site_perso", "now-note", "open", "dash now prints it")
            ]
        );
    }

    #[dash_test]
    fn a_row_s_agent_is_its_run_agent_line_and_claude_without_one() {
        let shown = "id: t\nrun.agent: codex\nrun.model: opus\nrun.prompt: run.agent: claude";
        assert_eq!(agent_of(shown), "codex");
        assert_eq!(
            agent_of("id: t\nwhy: run.agent: codex"),
            "claude",
            "only its own line"
        );
    }

    #[dash_test]
    fn the_app_s_answer_reads_as_a_verdict() {
        let verdict = |t: &str| verdict_of(t).expect("verdict");
        assert_eq!(verdict(r#"{"verdict":"met"}"#), Verdict::Met);
        assert_eq!(
            verdict(r#"{"verdict":"not-met","missing":"part 2"}"#),
            Verdict::NotMet("part 2".into())
        );
        assert_eq!(
            verdict(r#"{"verdict":"blocked","needs":"a tap"}"#),
            Verdict::Blocked("a tap".into())
        );
        assert_eq!(
            verdict(r#"{"verdict":"refused","why":"its owner is not rust"}"#),
            Verdict::NotOurs("its owner is not rust".into())
        );
        assert!(verdict_of(r#"{"verdict":"unanswered"}"#).is_err());
        assert!(verdict_of("{}").is_err());
        assert!(verdict_of("nope").is_err());
    }

    #[dash_test]
    fn the_app_s_answer_reads_as_issues() {
        let issues = issues_of(
            r#"[{"id":"a","title":"A","why":"w","comments":["c"],
                "checks":[{"text":"one","met":true},{"text":"two","met":false},{"text":"three","met":null}]},
               {"id":"b"}]"#,
        )
        .expect("issues");
        assert_eq!(issues[0].comments, ["c"]);
        let met: Vec<Option<bool>> = issues[0].checks.iter().map(|c| c.met).collect();
        assert_eq!(met, [Some(true), Some(false), None]);
        assert_eq!(issues[0].checks[1].text, "two");
        assert_eq!(issues[1].title, "");
        assert!(issues[1].checks.is_empty());
        assert!(issues_of("{}").is_err());
        assert!(issues_of("nope").is_err());
    }

    #[dash_test]
    fn a_wait_names_itself_in_dash_s_words() {
        let dash = Dash {
            bin: "/bin/echo".into(),
        };
        let offline = Outcome::Offline("x".into());
        assert!(dash.waiting("a", Some(&offline)).is_ok());
        assert_eq!(waiting_word(Some(&offline)), "network");
        assert_eq!(waiting_word(Some(&Outcome::Limited("x".into()))), "limit");
        assert_eq!(waiting_word(None), "over");
    }

    /// dash hook outbound-log and dash mail send: a line the app surely did not take (a refusal, no bin/dash) waits in
    /// private/live/pending, whole; one it may have taken (a timeout, a signal, « may still be running ») is never kept; the
    /// caller's env reaches bin/dash, its own `DASH_TIMEOUT` over the limit's.
    #[dash_test]
    fn an_outbound_line_is_logged_kept_or_never_kept_twice() {
        let dir = std::env::temp_dir().join(format!("bridge-outbound-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let bin = dir.join("dash");
        let script = "#!/bin/sh\necho \"env $DASH_TIMEOUT $DASH_RESTART_TRIES\" >&2\ncase \"$4\" in ok) exit 0 ;; late) exit 124 ;; partway) echo 'may still be running' >&2; exit 3 ;; killed) kill -9 $$ ;; *) echo refused >&2; exit 3 ;; esac\n";
        std::fs::write(&bin, script).expect("stand-in");
        std::fs::set_permissions(&bin, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .expect("chmod");
        let dash = Dash {
            bin: bin.display().to_string(),
        };
        let env = [("DASH_TIMEOUT", "10"), ("DASH_RESTART_TRIES", "2")];
        let log = |dash: &Dash, line: &str| dash.outbound(line, &env, CALL, &dir);
        let kept = || {
            std::fs::read_dir(dir.join("private/live/pending"))
                .map(|d| d.flatten().map(|e| e.path()).collect::<Vec<_>>())
                .unwrap_or_default()
        };
        assert_eq!(log(&dash, "ok"), Ok(()));
        for line in ["late", "partway", "killed"] {
            let why = log(&dash, line).expect_err(line);
            assert!(
                why.starts_with("dashboard log unconfirmed ("),
                "{line}: {why}"
            );
            assert!(kept().is_empty(), "{line} kept, so it may be added twice");
        }
        assert!(
            log(&dash, "late").is_err_and(|w| w.contains("env 10 2")),
            "the caller's env"
        );
        let why = log(&dash, "refused").expect_err("refused");
        assert!(why.starts_with("outbound send kept in "), "{why}");
        let gone = Dash {
            bin: dir.join("none").display().to_string(),
        };
        assert!(log(&gone, "{\"a\":1}").is_err_and(|w| w.starts_with("outbound send kept in ")));
        let mut lines: Vec<String> = kept()
            .iter()
            .map(|p| std::fs::read_to_string(p).expect("line"))
            .collect();
        lines.sort();
        assert_eq!(lines, ["refused\n", "{\"a\":1}\n"]);
        assert!(kept().iter().all(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("outbound.jsonl.line-"))
        }));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// dash hook linkedin-budget's gate, down: no answer, so the hook refuses (it fails closed).
    #[dash_test]
    fn linkedin_gate_down_is_no_answer() {
        // SAFETY: this test alone reads the variable, and no other thread of it runs
        unsafe {
            std::env::set_var(
                "LINKEDIN_GATE_URL",
                "http://127.0.0.1:9/dashboard/linkedin/",
            )
        };
        let ask = serde_json::json!({"tool": "get_inbox", "pages": 1});
        assert_eq!(linkedin_gate("check", &ask), None);
    }

    #[dash_test]
    fn a_call_names_its_words_and_a_failure_keeps_what_it_said() {
        let dash = Dash {
            bin: "/bin/echo".into(),
        };
        assert_eq!(dash.reopen("a"), Ok(()));
        assert_eq!(dash.agent("a").expect("echo"), "claude");
        let long = "é".repeat(9000);
        assert!(dash.judge("a", &long).is_err_and(|e| e.contains(&"é".repeat(SAID)) && !e.contains(&"é".repeat(SAID + 1))));
        let dash = Dash {
            bin: "/usr/bin/false".into(),
        };
        assert!(dash.merged("a", "u").is_err());
    }
}
