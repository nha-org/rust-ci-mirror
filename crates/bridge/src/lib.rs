//! Core's TaskList on the older app's CLI (section 20b's bridge), deleted with its part: its list stays the list of tasks and
//! its judge reads each report, so moving a task back loses nothing.
use dash_core::tasks::{Outcome, TaskList, Verdict};
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
    fn call(&self, args: &[&str], limit: Duration) -> Result<String, String> {
        process::run(&self.bin, args, None, limit)
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

    /// Today's daily: the one step `dash progress` names, and what it lists.
    pub fn progress(&self) -> Result<String, String> {
        self.call(&["progress"], CALL)
    }

    /// The agent kind its row names (`run.agent`), claude when it names none, as the older app's launch reads it.
    pub fn agent(&self, id: &str) -> Result<String, String> {
        Ok(agent_of(&self.call(&["task", "show", id], CALL)?))
    }

    /// One line of the older app's outbound log (`dash model outbound add`), where its drafts and leads read what went: a send
    /// made in Rust stays seen there (task rust-mail-thread).
    pub fn outbound(&self, line: &str) -> Result<(), String> {
        self.call(&["model", "outbound", "add", line], CALL)
            .map(|_| ())
    }
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

impl TaskList for Dash {
    fn brief(&self, id: &str) -> Result<String, String> {
        self.call(&["task", "brief", id], CALL)
    }

    fn judge(&self, id: &str, said: &str) -> Result<Verdict, String> {
        let tail = said.chars().rev().take(SAID).collect::<Vec<_>>();
        let tail: String = tail.into_iter().rev().collect();
        verdict_of(&self.call(&["task", "judge", id, &tail], JUDGE)?)
    }

    fn merged(&self, id: &str, pr: &str) -> Result<(), String> {
        let note = "Rust's orchestrator merged it, the PM check's verdict met";
        let args = ["task", "done", id, pr, "merged", note];
        self.call(&args, CALL).map(drop)
    }

    fn merged_later(&self, id: &str, pr: &str) -> Result<(), String> {
        let note = "merged after its Rust run ended, judged again by the PM check: met";
        let args = ["task", "done", id, pr, "merged", note];
        self.call(&args, CALL).map(drop)
    }

    fn ended(&self, id: &str, why: &str) -> Result<(), String> {
        self.call(&["task", "ended", id, why], CALL).map(drop)
    }

    fn reopen(&self, id: &str) -> Result<(), String> {
        self.call(&["task", "reopen", id], CALL).map(drop)
    }

    fn flag(&self, id: &str, what: &str) -> Result<(), String> {
        self.call(&["task", "flag", id, what], CALL).map(drop)
    }

    fn waiting(&self, id: &str, on: Option<&Outcome>) -> Result<(), String> {
        let on = waiting_word(on);
        self.call(&["task", "waiting", id, on], CALL).map(drop)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::memory::TestBudget;

    #[test]
    fn every_project_s_open_tasks_are_read_off_task_ls() {
        let _budget = TestBudget::new();
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

    #[test]
    fn a_row_s_agent_is_its_run_agent_line_and_claude_without_one() {
        let _budget = TestBudget::new();
        let shown = "id: t\nrun.agent: codex\nrun.model: opus\nrun.prompt: run.agent: claude";
        assert_eq!(agent_of(shown), "codex");
        assert_eq!(
            agent_of("id: t\nwhy: run.agent: codex"),
            "claude",
            "only its own line"
        );
    }

    #[test]
    fn the_app_s_answer_reads_as_a_verdict() {
        let _budget = TestBudget::new();
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

    #[test]
    fn the_app_s_answer_reads_as_issues() {
        let _budget = TestBudget::new();
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

    #[test]
    fn a_wait_names_itself_in_dash_s_words() {
        let _budget = TestBudget::new();
        let dash = Dash {
            bin: "/bin/echo".into(),
        };
        let offline = Outcome::Offline("x".into());
        assert!(dash.waiting("a", Some(&offline)).is_ok());
        assert_eq!(waiting_word(Some(&offline)), "network");
        assert_eq!(waiting_word(Some(&Outcome::Limited("x".into()))), "limit");
        assert_eq!(waiting_word(None), "over");
    }

    #[test]
    fn a_call_names_its_words_and_a_failure_keeps_what_it_said() {
        let _budget = TestBudget::new();
        let dash = Dash {
            bin: "/bin/echo".into(),
        };
        assert_eq!(dash.brief("a").expect("echo").trim(), "task brief a");
        let long = "é".repeat(9000);
        assert!(dash.judge("a", &long).is_err_and(|e| e.contains(&"é".repeat(SAID)) && !e.contains(&"é".repeat(SAID + 1))));
        let dash = Dash {
            bin: "/usr/bin/false".into(),
        };
        assert!(dash.merged("a", "u").is_err());
    }
}
