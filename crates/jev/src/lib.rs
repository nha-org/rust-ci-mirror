//! Jev (TypeSafe AI) as a core tool (task rust-jev-tool; @nha, 2026-11-14: « make Jev a tool as well and cache by default »,
//! « TWO api keys and should rotate. again hidden transparently in the tool. »). Called through `Registry::call` alone, so
//! the core caches a repeat question and counts, charges and keeps every call; this crate holds what is Jev's: its body, its
//! price, and its two keys, each held to `CAP_USD`, the second taking a call when the first is spent or refused. Each
//! question is a « noul », 0 to 1. The secrets, the transport and the spend read come from the binary, so this crate takes
//! the core alone.
use dash_core::registry::{Contribution, Tool};
use serde_json::{Value, json};

pub const NAME: &str = "jev";
pub const MODEL: &str = "jev-1.13.0";
/// Its price in US dollars per input token, $0.042 a million (Klar's ledger, qualification/jev); its output is not priced.
pub const USD_PER_INPUT_TOKEN: f64 = 0.042 / 1e6;
/// What each key may spend, in US dollars (@nha, 2026-11-14, the task's done-when 4).
pub const CAP_USD: f64 = 5.0;
/// The keys' names in bin/secret's typesafe, two accounts, in the order they are spent (ats.py's `api_keys`).
pub const KEYS: [&str; 2] = ["TYPESAFE_API_KEY", "TYPESAFE_SECOND_API_KEY"];
/// An answer about the key, not the question: the next key takes it (ats.py's SWITCH, Klar's jev_classifier).
const SWITCH: [u16; 4] = [401, 402, 403, 429];
/// Jev's refusal of a text over its limit, which the tool answers as `{"too_long": true}`.
pub const TOO_LONG: &str = "max_tokens_exceeded";

/// A secret by its key's name; one POST of a body with a secret and a caller's name, its status and text; a key's spend.
pub type Secret = Box<dyn Fn(&str) -> Result<String, String>>;
pub type Post = Box<dyn Fn(&str, &str, &str) -> Result<(u16, String), String>>;
pub type Spent = Box<dyn Fn(&str) -> Result<f64, String>>;

pub struct Jev {
    pub secret: Secret,
    pub post: Post,
    pub spent: Spent,
}

/// The budget port's name for a key.
pub fn budget_key(name: &str) -> String {
    format!("{NAME}/{name}")
}

/// A call's arguments: the questions (a JSON object), the state they are asked of, and the caller's name for its User-Agent.
/// The model rides in them, so a new model is a new question, never an old answer (/code-review). A state key that
/// starts with « _ » (a document's path) keys the cache and is never sent.
pub fn arguments(questions: &Value, state: Value, agent: &str) -> String {
    json!({"questions": questions, "state": state, "agent": agent, "model": MODEL}).to_string()
}

/// This crate's part of the registry.
pub fn contribution(jev: Jev) -> Contribution {
    Contribution {
        tools: vec![(NAME.into(), Box::new(jev) as Box<dyn Tool>)],
        ..Default::default()
    }
}

/// The input tokens an answer was billed for: 0 when it does not say.
pub fn input_tokens(answer: &Value) -> u64 {
    answer["usage"]["input_tokens"].as_u64().unwrap_or(0)
}

fn parsed(answer: &Result<String, String>) -> Value {
    let text = answer.as_deref().unwrap_or("null");
    serde_json::from_str(text).unwrap_or_default()
}

impl Tool for Jev {
    fn description(&self) -> &str {
        "Jev (TypeSafe AI): each question's score, 0 to 1, on the state given"
    }

    fn input_schema(&self) -> &str {
        r#"{"type":"object","properties":{"questions":{"type":"object"},"state":{"type":"object"},"agent":{"type":"string"}},"required":["questions","state"]}"#
    }

    /// The first key under its cap asks; a key refused (spent, no secret, an answer about the key) passes the call to the
    /// next. The answer names the key that paid, by name, never its secret.
    fn call(&self, arguments: &str) -> Result<String, String> {
        let asked: Value =
            serde_json::from_str(arguments).map_err(|e| format!("Jev's arguments: {e}"))?;
        let mut state = asked["state"].clone();
        if let Some(fields) = state.as_object_mut() {
            fields.retain(|k, _| !k.starts_with('_'));
        }
        let body =
            json!({"state": state, "model": MODEL, "questions": asked["questions"]}).to_string();
        let agent = asked["agent"].as_str().unwrap_or("site_perso/jev");
        let mut refused = vec![];
        for name in KEYS {
            let spent = (self.spent)(&budget_key(name))?;
            if spent >= CAP_USD {
                refused.push(format!("{name} spent ${spent:.2} of its ${CAP_USD}"));
                continue;
            }
            let secret = match (self.secret)(name) {
                Ok(s) => s,
                Err(why) => {
                    refused.push(why);
                    continue;
                }
            };
            let (status, text) = (self.post)(&secret, &body, agent)?;
            if SWITCH.contains(&status) {
                refused.push(format!("{name}: Jev answered {status}"));
                continue;
            }
            // an answer about the question, the same each time it is asked: an answer, so the cache keeps it
            if status == 400 && text.contains(TOO_LONG) {
                return Ok(json!({"too_long": true}).to_string());
            }
            if status != 200 {
                let said: String = text.chars().take(300).collect();
                return Err(format!("Jev answered {status}: {said}"));
            }
            let mut answer: Value =
                serde_json::from_str(&text).map_err(|e| format!("Jev's answer: {e}"))?;
            // an answer with no score is asked again, never the cache's for good (doc_ready.py's rule, review of #177)
            if answer["answers"].as_object().is_none_or(|a| a.is_empty()) {
                return Err("Jev answered 200 with no answers".into());
            }
            answer["key"] = json!(name);
            return Ok(answer.to_string());
        }
        Err(format!("no Jev key left: {}", refused.join("; ")))
    }

    fn cost(&self, _arguments: &str, answer: &Result<String, String>) -> f64 {
        input_tokens(&parsed(answer)) as f64 * USD_PER_INPUT_TOKEN
    }

    fn key(&self, answer: &Result<String, String>) -> Option<String> {
        parsed(answer)["key"].as_str().map(budget_key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_core::registry::{Budget, Memory, Registry};
    use dash_meter::dash_test;
    use std::cell::RefCell;
    use std::rc::Rc;

    /// Jev on `memory`'s budget, each secret « s-<name> », answering `status` to the secrets in `refuses` and 200 with
    /// 10 million input tokens ($0.42) to the others; the secrets it was asked with.
    fn jev(
        memory: &Rc<Memory>,
        refuses: &'static [&str],
        status: u16,
    ) -> (Registry, Rc<RefCell<Vec<String>>>) {
        let used = Rc::new(RefCell::new(vec![]));
        let (seen, budget) = (used.clone(), memory.clone());
        let tool = Jev {
            secret: Box::new(|name| Ok(format!("s-{name}"))),
            post: Box::new(move |secret, _body, _agent| {
                seen.borrow_mut().push(secret.to_string());
                Ok(match refuses.contains(&secret) {
                    true => (status, "{}".into()),
                    false => (
                        200,
                        r#"{"answers":{"q":{"noul":0.1}},"usage":{"input_tokens":10000000}}"#
                            .into(),
                    ),
                })
            }),
            spent: Box::new(move |key| budget.spent(key)),
        };
        let mut r = Registry::default();
        r.add("dash-jev", contribution(tool)).unwrap();
        (r, used)
    }

    /// A Jev on `memory`'s budget whose post is never asked.
    fn jev_parts(memory: &Rc<Memory>) -> Jev {
        let budget = memory.clone();
        Jev {
            secret: Box::new(|name| Ok(format!("s-{name}"))),
            post: Box::new(|_, _, _| Err("not asked".into())),
            spent: Box::new(move |key| budget.spent(key)),
        }
    }

    fn ask(r: &Registry, memory: &Memory, q: &str) -> Result<Value, String> {
        let clock = || 0;
        let asked = arguments(&json!({q: {}}), json!({"doc": q}), "test");
        let (answer, kept) = r
            .call(NAME, &asked, &memory.ports(&clock))
            .expect("registered");
        assert_eq!(kept, Ok(()));
        answer.map(|a| serde_json::from_str(&a).expect("JSON"))
    }

    /// Done-when 4: callers ask one tool; the first key pays until it has spent $5, then the second, each charged its
    /// own spend; with both spent the call is refused, saying each; a key refused by Jev passes the call to the other.
    #[dash_test]
    fn each_key_is_held_to_5_dollars_and_the_tool_rotates_them_behind_one_name() {
        let memory = Rc::new(Memory::default());
        let (r, used) = jev(&memory, &[], 200);
        let [first, second] = KEYS.map(budget_key);
        assert_eq!(ask(&r, &memory, "a").unwrap()["key"], KEYS[0]);
        assert!((memory.spent(&first).unwrap() - 0.42).abs() < 1e-9);
        memory.charge(&first, CAP_USD).unwrap();
        assert_eq!(ask(&r, &memory, "b").unwrap()["key"], KEYS[1]);
        assert!((memory.spent(&second).unwrap() - 0.42).abs() < 1e-9);
        let said = |k: &str| format!("s-{k}");
        assert_eq!(*used.borrow(), [said(KEYS[0]), said(KEYS[1])]);
        memory.charge(&second, CAP_USD).unwrap();
        let refused = ask(&r, &memory, "c").expect_err("both spent");
        assert!(
            refused.contains("TYPESAFE_API_KEY spent $5.42")
                && refused.contains("TYPESAFE_SECOND_API_KEY spent $5.42"),
            "{refused}"
        );
        assert_eq!(used.borrow().len(), 2, "no key asked past its cap");
        assert_eq!(
            ask(&r, &memory, "a").unwrap()["key"],
            KEYS[0],
            "an answer given is the cache's, free"
        );

        let memory = Rc::new(Memory::default());
        let (r, used) = jev(&memory, &["s-TYPESAFE_API_KEY"], 429);
        assert_eq!(ask(&r, &memory, "a").unwrap()["key"], KEYS[1]);
        assert_eq!(used.borrow().len(), 2);
        assert_eq!(
            memory.spent(&first),
            Ok(0.0),
            "the refused key paid nothing"
        );
    }

    /// A state key starting with « _ » keys the cache, so two documents with one text are asked each, and is never sent.
    #[dash_test]
    fn a_state_key_starting_with_an_underscore_keys_the_cache_and_is_never_sent() {
        let memory = Rc::new(Memory::default());
        let bodies = Rc::new(RefCell::new(vec![]));
        let seen = bodies.clone();
        let tool = Jev {
            post: Box::new(move |_, body, _| {
                seen.borrow_mut().push(body.to_string());
                Ok((200, r#"{"answers":{"q":{"noul":0.1}}}"#.into()))
            }),
            ..jev_parts(&memory)
        };
        let mut r = Registry::default();
        r.add("dash-jev", contribution(tool)).unwrap();
        let clock = || 0;
        for path in ["a.html", "b.html", "a.html"] {
            let asked = arguments(&json!({}), json!({"doc": "same", "_for": path}), "test");
            r.call(NAME, &asked, &memory.ports(&clock))
                .unwrap()
                .0
                .unwrap();
        }
        assert_eq!(
            bodies.borrow().len(),
            2,
            "one ask a path, the third the cache's"
        );
        assert!(
            bodies
                .borrow()
                .iter()
                .all(|b| !b.contains("_for") && b.contains(MODEL)),
            "{bodies:?}"
        );
    }

    /// A 200 with no answers is an error, so the cache never keeps it and the next ask reaches Jev again.
    #[dash_test]
    fn an_answer_with_no_score_is_never_cached() {
        let memory = Rc::new(Memory::default());
        let asked = Rc::new(RefCell::new(0));
        let seen = asked.clone();
        let tool = Jev {
            post: Box::new(move |_, _, _| {
                *seen.borrow_mut() += 1;
                Ok((200, r#"{"answers":{}}"#.into()))
            }),
            ..jev_parts(&memory)
        };
        let mut r = Registry::default();
        r.add("dash-jev", contribution(tool)).unwrap();
        for _ in 0..2 {
            let said = ask(&r, &memory, "a").expect_err("no answers");
            assert!(said.contains("no answers"), "{said}");
        }
        assert_eq!(*asked.borrow(), 2, "asked each time");
    }

    /// A question Jev refuses (a text too long) is its error, said, never passed to the other key nor cached.
    #[dash_test]
    fn a_refused_question_is_said_and_asked_again_next_time() {
        let memory = Rc::new(Memory::default());
        let (r, used) = jev(&memory, &["s-TYPESAFE_API_KEY"], 400);
        let said = ask(&r, &memory, "a").expect_err("400");
        assert!(said.starts_with("Jev answered 400"), "{said}");
        let _ = ask(&r, &memory, "a");
        assert_eq!(used.borrow().len(), 2, "asked again, one key each time");
        let memory = Rc::new(Memory::default());
        let (r, used) = jev(&memory, &[], 200);
        let long = Jev {
            post: Box::new(|_, _, _| Ok((400, r#"{"error":"max_tokens_exceeded"}"#.into()))),
            ..jev_parts(&memory)
        };
        let mut r2 = Registry::default();
        r2.add("dash-jev", contribution(long)).unwrap();
        drop((r, used));
        for _ in 0..2 {
            assert_eq!(
                ask(&r2, &memory, "long").unwrap(),
                json!({"too_long": true})
            );
        }
        assert_eq!(
            memory.calls.borrow().iter().filter(|(_, hit)| *hit).count(),
            1,
            "too long once, then the cache's"
        );
    }
}
