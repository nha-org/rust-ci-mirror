//! Core's TaskList on the older app's CLI (section 20b's bridge), deleted with its part: its list stays the list of tasks and
//! its judge reads each report, so moving a task back loses nothing.
use dash_core::tasks::{Queue, Queued, TaskList, Verdict};
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

fn queue_of(text: &str) -> Result<Queue, String> {
    let answer = json(text)?;
    let seats = answer["seats"]
        .as_u64()
        .ok_or(format!("no seats in: {text}"))?;
    let tasks = answer["tasks"]
        .as_array()
        .ok_or(format!("no tasks in: {text}"))?;
    let task = |t: &Value| Queued {
        id: t["id"].as_str().unwrap_or_default().into(),
        title: t["title"].as_str().unwrap_or_default().into(),
    };
    Ok(Queue {
        seats: seats as usize,
        tasks: tasks.iter().map(task).collect(),
    })
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

impl TaskList for Dash {
    fn queue(&self) -> Result<Queue, String> {
        queue_of(&self.call(&["task", "queue"], CALL)?)
    }

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

    fn ended(&self, id: &str, why: &str) -> Result<(), String> {
        self.call(&["task", "ended", id, why], CALL).map(drop)
    }

    fn flag(&self, id: &str, what: &str) -> Result<(), String> {
        self.call(&["task", "flag", id, what], CALL).map(drop)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::memory::TestBudget;

    #[test]
    fn the_app_s_answers_read_as_a_queue_and_a_verdict() {
        let _budget = TestBudget::new();
        let queue = queue_of(r#"{"seats":5,"tasks":[{"id":"a","title":"A"}]}"#).expect("queue");
        assert_eq!(queue.seats, 5);
        assert_eq!(queue.tasks[0].id, "a");
        assert!(queue_of(r#"{"tasks":[]}"#).is_err());
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
