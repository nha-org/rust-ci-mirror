//! dash doc ready (crates/hooks/src/doc_ready.rs): doc_ready.py's selftest, case for case, and done-when 5 of task
//! rust-jev-tool, an unchanged document's second ask paying nothing. Here, not in the module: its pages are HTML, which the
//! gate keeps out of every crate's src.
use dash_core::registry::{Contribution, Memory, Registry, Tool};
use dash_hooks::doc_ready::*;
use dash_meter::dash_test;
use serde_json::{Value, json};
use std::cell::{Cell, RefCell};

/// A fixture beside doc_ready.py, which Readiness's test reads too: none on the public mirror, which holds rust/ alone.
fn fixture(name: &str) -> Option<String> {
    let dir = dash_meter::monorepo()?.join(".claude/skills/nicolas-daily");
    Some(std::fs::read_to_string(dir.join(name)).expect("the fixture"))
}

/// doc_ready.py's selftest, case for case: the reading text Readiness makes from the same fixture, the split it makes
/// on the same cases, markers no entity forges, the reader line, the gate.
#[dash_test]
fn the_reading_text_and_its_sections_are_readiness_s() {
    if let (Some(html), Some(txt), Some(json)) = (
        fixture("doc-ready-text.html"),
        fixture("doc-ready-text.txt"),
        fixture("doc-ready-text-sections.json"),
    ) {
        assert_eq!(text(&html), txt);
        let cases: Vec<(String, Vec<String>)> = serde_json::from_str(&json).expect("cases");
        for (case, want) in cases {
            assert_eq!(sections(&case), want, "{case:?}");
        }
    }
    let page = r#"<h1>T</h1><p><b>Audience :</b> un conseiller</p><details class="wn"><ul><li>note</li></ul></details><p>a &amp; b</p>"#;
    assert_eq!(text(page), "# T\nAudience : un conseiller\na & b");
    assert_eq!(reader_of(page), "un conseiller");
    assert_eq!(
        reader_of("<p>x</p>"),
        "the person this document is written for"
    );
    assert_eq!(
        text("<ul><li><p>-20 % de coûts</p></li><li></li></ul><h2></h2><p>Le budget</p>"),
        "- -20 % de coûts\nLe budget"
    );
    assert_eq!(text("<p>a&#xFFFE;b&#xFDD0;c</p>"), "abc");
    assert_eq!(
        text("<p>a</p><svg viewBox='0 0 1 1'><text>label</text></svg><p>b</p>"),
        "a\nb"
    );
    assert_eq!(md_text("# T\n\n- a  b\n- c\n"), "# T\n- a b\n- c");
    assert_eq!(
        md_text("# T\n<!-- TODO -->\n<details class=\"wn\">note</details>\na &amp; <b>b</b>\n"),
        "# T\na & b"
    );
    let gate = json!({"not_ready_max": 0.5});
    assert_eq!(
        failures(&json!({"not_ready": {"noul": 0.8}}), &gate, &json!({})),
        ["not_ready 0.80 > 0.5"]
    );
    assert!(failures(&json!({"not_ready": {"noul": 0.2}}), &gate, &json!({})).is_empty());
    assert_eq!(
        failures(
            &json!({"not_ready": {"noul": 0.8}}),
            &gate,
            &json!({"not_ready": "a blank"})
        ),
        ["not_ready 0.80 > 0.5: a blank"]
    );
    assert_eq!(
        failures(&json!({}), &gate, &json!({})),
        ["not_ready not answered"]
    );
    assert!(
        control_ok(
            "not-ready-gap-x.html",
            &["not_ready_gap 0.78 > 0.5: …".into()]
        )
        .unwrap()
    );
    assert!(
        !control_ok(
            "not-ready-gap-x.html",
            &["not_ready_gap not answered".into()]
        )
        .unwrap()
    );
    assert!(
        control_ok("ready-x.html", &[]).unwrap()
            && control_ok("not-ready-terms-x.html", &[]).is_err()
    );
    // the tool's own words, never a reading of Jev's codes here (rfc-rust-unified#tools: its policy lives in the tool)
    assert_eq!(
        meaning("Jev answered 503: boom"),
        "Jev could not answer: Jev answered 503: boom"
    );
    assert!(meaning(TOO_LONG).starts_with("too long"));
    assert_eq!(
        meaning("no Jev key left: x"),
        "Jev could not answer: no Jev key left: x"
    );
}

/// A stand-in Jev: counts its fresh calls, refuses a text over `limit` characters as too long, scores the rest 0.1.
struct Standin(Cell<u32>, usize);

impl Tool for Standin {
    fn description(&self) -> &str {
        "stand-in Jev"
    }
    fn input_schema(&self) -> &str {
        "{}"
    }
    fn call(&self, arguments: &str) -> Result<String, String> {
        self.0.set(self.0.get() + 1);
        let asked: Value = serde_json::from_str(arguments).expect("JSON");
        let long = asked["state"]["doc"].as_str().unwrap_or("").len() > self.1;
        Ok(match long {
            true => json!({"too_long": true}),
            false => json!({"answers": {"q": {"noul": 0.1}}, "usage": {"input_tokens": 1000}}),
        }
        .to_string())
    }
    fn cost(&self, _arguments: &str, _answer: &Result<String, String>) -> f64 {
        1.0
    }
}

/// Done-when 5: a document judged twice, unchanged, goes through the core's tool both times; the first pays its asks
/// (whole and by section, as it is too long for one read), the second is the cache's: no fresh call, no cost, no new
/// line in Jev's log, the same verdict.
#[dash_test]
fn an_unchanged_doc_s_second_ask_pays_nothing() {
    let dir = dash_meter::TestDir::new("doc-ready-twice");
    let path = dir.0.join("long.html");
    let page = format!(
        "<h1>T</h1><h2>A</h2><p>{}</p><h2>B</h2><p>{}</p>",
        "a ".repeat(30),
        "b ".repeat(30)
    );
    std::fs::write(&path, page).expect("page");
    let mut r = Registry::default();
    let tool = Box::new(Standin(Cell::new(0), 100)) as Box<dyn Tool>;
    r.add(
        "stand-in",
        Contribution {
            tools: vec![("jev".into(), tool)],
            ..Default::default()
        },
    )
    .unwrap();
    let memory = Memory::default();
    let clock = || 0;
    let ports = memory.ports(&clock);
    // the binary's jev::ask in small: the tool through the core, and a fresh answer's log line written on the tool's side
    let logged = RefCell::new(vec![]);
    let ask = |q: &Value, s: Value, a: &str| {
        let hits = memory.calls.borrow().iter().filter(|(_, hit)| *hit).count();
        let asked = json!({"questions": q, "state": s, "agent": a, "model": "m"}).to_string();
        let text = r.call("jev", &asked, &ports).expect("jev").0?;
        let cached = memory.calls.borrow().iter().filter(|(_, hit)| *hit).count() > hits;
        if !cached {
            logged
                .borrow_mut()
                .extend(dash_hooks::jev::log_row(&asked, &text, "t"));
        }
        let mut v: Value = serde_json::from_str(&text).expect("JSON");
        v["cached"] = json!(cached);
        Ok(v)
    };
    let judge = Judge {
        ask: &ask,
        banned: &|_| Ok(vec![]),
        started: std::time::Instant::now(),
    };
    let spec = json!({"model": "m", "questions": {"q": {}}, "gate": {"q_max": 0.5}});
    let mut first = String::new();
    assert_eq!(
        judge.judge(
            &path,
            "private/docs/long.html",
            None,
            &spec,
            true,
            &mut first
        ),
        Ok(vec![])
    );
    let paid = memory.spent.borrow().clone();
    let cost = |m: &Memory| m.kept.borrow().iter().map(|c| c.cost).sum::<f64>();
    assert_eq!(
        (cost(&memory), logged.borrow().len()),
        (4.0, 4),
        "whole, preamble, A, B: {first}"
    );
    assert!(first.contains("judged by section") && first.contains("(reader: the person"));
    let mut second = String::new();
    assert_eq!(
        judge.judge(
            &path,
            "private/docs/long.html",
            None,
            &spec,
            true,
            &mut second
        ),
        Ok(vec![])
    );
    assert_eq!(
        (cost(&memory), logged.borrow().len()),
        (4.0, 4),
        "nothing paid, nothing logged: {second}"
    );
    // what each judge said, for the report: `--success-output immediate` prints it
    println!("first ask:\n{first}second ask:\n{second}");
    let hits = memory.calls.borrow().iter().filter(|(_, hit)| *hit).count();
    println!(
        "calls through the core: {}, of them the cache's: {hits}",
        memory.kept.borrow().len()
    );
    assert_eq!(hits, 4, "the second judge's four asks are the cache's");
    assert_eq!(
        memory.kept.borrow().len(),
        8,
        "every ask reached the core, the hits too"
    );
    assert_eq!(*memory.spent.borrow(), paid);
    assert_eq!(first.lines().last(), second.lines().last());
    assert_eq!(
        logged.borrow()[0]["status"],
        400,
        "the whole text's too-long answer is a line, as Readiness reads it"
    );
    assert_eq!(logged.borrow()[1]["section"], INTRO);
}
