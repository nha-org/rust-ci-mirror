//! His inbox, agent to him (private/docs/rfc-inbox.html; task rust-inbox-triage): the reverse of a comment. An ask is one
//! record with one state, moved only as section 3 decides: raised by an agent, checked by the rules, held behind a gate of
//! his, shown by the app, answered by him, dropped by a rule (@nha, 2026-11-11: « /inbox = agent to me. there needs to be a
//! "backlog" of things an agent / JEV has to double-check that they truly need me. We need to start building the rules now
//! as we find it »). The rules are rows of `inbox-rules.json`, each with an example and the verdict it must give; one table
//! test runs them all, and each rule counts its firings. Kept on the op log beside the comment queue, in table `ask_inbox`.
//! ponytail: rules in code, then the bar's own word check; section 5's Jev and agent steps are later rows of the same pass.
use dash_core::{FieldWrite, OpLog, Value};
use serde_json::{Value as Json, json};
use std::collections::{BTreeMap, BTreeSet};

pub const SCHEMA: &str =
    "CREATE TABLE IF NOT EXISTS ask_inbox (id TEXT PRIMARY KEY, record TEXT) WITHOUT ROWID;";
const TABLE: &str = "ask_inbox";
const FIELD: &str = "record";
const PAGE: i64 = 1000;
/// At most this many shown at once (rfc-inbox section 7; @nha, 2026-11-11: « 7 s fine. »).
pub const CAP: usize = 7;
/// The rules, tracked beside this code and reviewed like it (rfc-inbox section 9, « Where the rules live »).
pub const RULES: &str = include_str!("inbox-rules.json");

/// Section 3's six states.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum State {
    Raised,
    Checked,
    Held,
    Shown,
    Answered,
    Dropped,
}

impl State {
    pub const ALL: [State; 6] = [
        State::Raised,
        State::Checked,
        State::Held,
        State::Shown,
        State::Answered,
        State::Dropped,
    ];
    pub fn name(self) -> &'static str {
        match self {
            State::Raised => "raised",
            State::Checked => "checked",
            State::Held => "held",
            State::Shown => "shown",
            State::Answered => "answered",
            State::Dropped => "dropped",
        }
    }
    pub fn parse(s: &str) -> Option<State> {
        State::ALL.into_iter().find(|st| st.name() == s)
    }
}

/// The readiness bar's fixed reasons only he can answer (section 4).
pub const WHYS: [&str; 4] = ["outside-party", "wording", "his-hands", "one-way"];

/// Pages that list things, never the thing itself (section 4: « never a list page such as /mail »).
const LIST_PAGES: [&str; 7] = [
    "/", "/mail", "/review", "/drafts", "/inbox", "/tasks", "/today",
];

#[derive(Clone, Debug, PartialEq)]
pub struct Ask {
    pub id: String,
    /// Who raised it: a task id, `handoff` (the older app's page, read in), `pm-loop`, a session.
    pub asker: String,
    /// Where it was read from: `handoff` for the older app's page, else `raise`.
    pub source: String,
    /// The bar's five fields: what, why (one of `WHYS`), the agent's pick, the link, and for an approval who sends it.
    pub what: String,
    pub why: Option<String>,
    pub pick: String,
    pub link: String,
    pub approval: bool,
    pub sent_by: String,
    /// What sort of ask its asker says it is (`pitch`, `optional`, `after-merge`…), read by the never rules.
    pub kind: String,
    /// What a settled rule watches: its PR, its task, its draft, the ask it supersedes.
    pub pr: Option<u64>,
    pub task: Option<String>,
    pub draft: Option<String>,
    pub supersedes: Option<String>,
    /// A day before which it cannot act (a date gate), and its deadline, each `YYYY-MM-DD`.
    pub not_before: Option<String>,
    pub deadline: Option<String>,
    pub state: State,
    /// The rule that moved it last, the gate that holds it, what the bar found missing, his answer.
    pub rule: Option<String>,
    pub gate: Option<String>,
    pub lacks: Vec<String>,
    pub answer: Option<String>,
    /// Jev's scores and the agent's verdict, each with the words it judged: the same words are never judged twice.
    pub jev: Option<Scored>,
    pub agent: Option<Ruled>,
    pub raised_ms: i64,
    pub moved_ms: i64,
}

impl Ask {
    /// A new ask, raised.
    pub fn raised(id: &str, asker: &str, what: &str, now_ms: i64) -> Ask {
        Ask {
            id: id.into(),
            asker: asker.into(),
            source: "raise".into(),
            what: what.into(),
            why: None,
            pick: String::new(),
            link: String::new(),
            approval: false,
            sent_by: String::new(),
            kind: String::new(),
            pr: None,
            task: None,
            draft: None,
            supersedes: None,
            not_before: None,
            deadline: None,
            state: State::Raised,
            rule: None,
            gate: None,
            lacks: vec![],
            answer: None,
            jev: None,
            agent: None,
            raised_ms: now_ms,
            moved_ms: now_ms,
        }
    }

    pub fn to_json(&self) -> Json {
        json!({"id": self.id, "asker": self.asker, "source": self.source, "what": self.what, "why": self.why,
            "pick": self.pick, "link": self.link, "approval": self.approval, "sent_by": self.sent_by, "kind": self.kind,
            "pr": self.pr, "task": self.task, "draft": self.draft, "supersedes": self.supersedes,
            "not_before": self.not_before, "deadline": self.deadline, "state": self.state.name(), "rule": self.rule,
            "gate": self.gate, "lacks": self.lacks, "answer": self.answer, "raised_ms": self.raised_ms,
            "jev": self.jev.as_ref().map(|j| json!({"words": j.words, "plain": j.plain, "agent_can": j.agent_can})),
            "agent": self.agent.as_ref().map(|a| json!({"words": a.words, "his": a.his, "rule": a.rule, "why": a.why})),
            "moved_ms": self.moved_ms})
    }

    /// Read leniently, every field but `what` optional: a rule's example names only what it needs.
    pub fn from_json(j: &Json) -> Option<Ask> {
        let m = j.as_object()?;
        let s = |k: &str| m.get(k).and_then(Json::as_str).map(str::to_string);
        let text = |k: &str| s(k).unwrap_or_default();
        let ms = |k: &str| m.get(k).and_then(Json::as_i64).unwrap_or(0);
        Some(Ask {
            id: s("id").unwrap_or_else(|| "example".into()),
            asker: text("asker"),
            source: s("source").unwrap_or_else(|| "raise".into()),
            what: s("what")?,
            why: s("why"),
            pick: text("pick"),
            link: text("link"),
            approval: m.get("approval").and_then(Json::as_bool).unwrap_or(false),
            sent_by: text("sent_by"),
            kind: text("kind"),
            pr: m.get("pr").and_then(Json::as_u64),
            task: s("task"),
            draft: s("draft"),
            supersedes: s("supersedes"),
            not_before: s("not_before"),
            deadline: s("deadline"),
            state: s("state")
                .and_then(|st| State::parse(&st))
                .unwrap_or(State::Raised),
            rule: s("rule"),
            gate: s("gate"),
            lacks: (m
                .get("lacks")
                .and_then(Json::as_array)
                .into_iter()
                .flatten())
            .filter_map(|l| l.as_str().map(str::to_string))
            .collect(),
            answer: s("answer"),
            jev: m.get("jev").and_then(|j| {
                Some(Scored {
                    words: j.get("words")?.as_str()?.into(),
                    plain: j.get("plain")?.as_f64()?,
                    agent_can: j.get("agent_can")?.as_f64()?,
                })
            }),
            agent: m.get("agent").and_then(|a| {
                Some(Ruled {
                    words: a.get("words")?.as_str()?.into(),
                    his: a.get("his")?.as_bool()?,
                    rule: a.get("rule")?.as_str()?.into(),
                    why: a.get("why")?.as_str()?.into(),
                })
            }),
            raised_ms: ms("raised_ms"),
            moved_ms: ms("moved_ms"),
        })
    }
}

/// What the store and the world say now, as the settled, held and lift tests read it.
#[derive(Clone, Debug, Default)]
pub struct Facts {
    pub prs_closed: BTreeSet<u64>,
    pub tasks_done: BTreeSet<String>,
    pub drafts_settled: BTreeSet<String>,
    /// Asks a settled ask supersedes.
    pub superseded: BTreeSet<String>,
    /// The ids the older app's page lists now; `None` when it was not read, so nothing is taken as gone.
    pub listed: Option<BTreeSet<String>>,
    /// Gates of his that lifted (`cape` once the contract is signed).
    pub lifted: BTreeSet<String>,
    pub today: String,
}

impl Facts {
    fn from_json(j: &Json) -> Facts {
        let strs = |k: &str| -> BTreeSet<String> {
            (j.get(k).and_then(Json::as_array).into_iter().flatten())
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        };
        Facts {
            prs_closed: (j
                .get("prs_closed")
                .and_then(Json::as_array)
                .into_iter()
                .flatten())
            .filter_map(Json::as_u64)
            .collect(),
            tasks_done: strs("tasks_done"),
            drafts_settled: strs("drafts_settled"),
            superseded: strs("superseded"),
            listed: j.get("listed").map(|_| strs("listed")),
            lifted: strs("lifted"),
            today: (j.get("today").and_then(Json::as_str))
                .unwrap_or("2026-11-11")
                .into(),
        }
    }
}

/// One rule: a row of `inbox-rules.json`.
#[derive(Clone, Debug)]
pub struct Rule {
    pub id: String,
    /// `settled` or `never` drop, `held` holds behind `gate`, `his` checks.
    pub kind: String,
    /// The test against the ask and the facts, one this code defines (`WHENS`).
    pub when: String,
    pub words: Vec<String>,
    pub gate: Option<String>,
    pub said: String,
    pub example: Json,
    pub expect: String,
}

/// The tests a rule's `when` may name; a new one is code here with its own row.
pub const WHENS: [&str; 9] = [
    "pr-closed",
    "task-done",
    "draft-settled",
    "superseded",
    "gone-from-source",
    "kind",
    "says",
    "mentions",
    "not-before",
];
const KINDS: [&str; 4] = ["settled", "never", "held", "his"];

/// The rules file read, each row whole, or why it is not.
pub fn rules(text: &str) -> Result<Vec<Rule>, String> {
    let rows: Vec<Json> = serde_json::from_str(text).map_err(|e| format!("the rules: {e}"))?;
    let mut out = vec![];
    for r in rows {
        let s = |k: &str| r.get(k).and_then(Json::as_str).map(str::to_string);
        let id = s("id").ok_or("a rule with no id")?;
        let miss = |k: &str| format!("rule {id}: no {k}");
        let rule = Rule {
            kind: s("kind").ok_or_else(|| miss("kind"))?,
            when: s("when").ok_or_else(|| miss("when"))?,
            words: (r
                .get("words")
                .and_then(Json::as_array)
                .into_iter()
                .flatten())
            .filter_map(|w| w.as_str().map(str::to_string))
            .collect(),
            gate: s("gate"),
            said: s("said").ok_or_else(|| miss("said"))?,
            example: r.get("example").cloned().ok_or_else(|| miss("example"))?,
            expect: s("expect").ok_or_else(|| miss("expect"))?,
            id,
        };
        let known = KINDS.contains(&rule.kind.as_str())
            && (WHENS.contains(&rule.when.as_str()) || rule.when == "why");
        if !known || (rule.kind == "held") != rule.gate.is_some() {
            return Err(format!(
                "rule {}: kind {} or when {} unknown, or a gate on a rule that holds nothing",
                rule.id, rule.kind, rule.when
            ));
        }
        out.push(rule);
    }
    Ok(out)
}

/// Whether gate `gate` still holds `ask`.
fn shut(gate: &str, ask: &Ask, facts: &Facts) -> bool {
    match gate {
        "date" => ask
            .not_before
            .as_deref()
            .is_some_and(|d| d > facts.today.as_str()),
        g => !facts.lifted.contains(g),
    }
}

/// Whether `rule` fires on `ask`.
fn fires(rule: &Rule, ask: &Ask, facts: &Facts) -> bool {
    let any_word = |text: &str| {
        let lower = text.to_lowercase();
        (rule.words.iter()).any(|w| lower.contains(&w.to_lowercase()))
    };
    let hit = match rule.when.as_str() {
        "pr-closed" => ask.pr.is_some_and(|p| facts.prs_closed.contains(&p)),
        "task-done" => (ask.task.as_ref()).is_some_and(|t| facts.tasks_done.contains(t)),
        "draft-settled" => (ask.draft.as_ref()).is_some_and(|d| facts.drafts_settled.contains(d)),
        "superseded" => facts.superseded.contains(&ask.id),
        "gone-from-source" => {
            ask.source == "handoff" && (facts.listed.as_ref()).is_some_and(|l| !l.contains(&ask.id))
        }
        "kind" => rule.words.contains(&ask.kind),
        "says" => rule.words.iter().any(|w| ask.what.contains(w.as_str())),
        "mentions" => any_word(&ask.what),
        "not-before" => ask.not_before.is_some(),
        "why" => ask.why.as_ref().is_some_and(|w| rule.words.contains(w)),
        _ => false,
    };
    hit && rule.gate.as_deref().is_none_or(|g| shut(g, ask, facts))
}

/// What the readiness bar finds missing (section 4), none when the ask is ready.
pub fn lacks(ask: &Ask) -> Vec<String> {
    let mut out = vec![];
    let what = ask.what.trim();
    // a raw id (one word with dashes), a warning, a path or pasted text is no plain line he can act on
    let raw = !what.contains(' ') && what.contains('-');
    let jargon = what.starts_with("WARNING") || what.contains(".md") || what.contains("private/");
    if what.is_empty() {
        out.push("what it is".into());
    } else if raw || jargon || what.chars().count() > 200 {
        out.push("what it is, in plain words: no id, path, warning or pasted text".into());
    }
    if !ask.why.as_ref().is_some_and(|w| WHYS.contains(&w.as_str())) {
        out.push(format!("why only him, one of {}", WHYS.join(", ")));
    }
    if ask.pick.trim().is_empty() {
        out.push("the agent's pick".into());
    }
    let path = ask.link.split(['?', '#']).next().unwrap_or_default();
    let path = path
        .strip_prefix("http://100.72.158.115:7788")
        .unwrap_or(path);
    if ask.link.trim().is_empty() {
        out.push("a link that opens the thing itself".into());
    } else if LIST_PAGES.contains(&path.trim_end_matches('/')) || LIST_PAGES.contains(&path) {
        out.push(format!(
            "a link that opens the thing itself, not the list page {path}"
        ));
    }
    if ask.approval && ask.sent_by.trim().is_empty() {
        out.push("who sends it once he says yes, or why no tool can".into());
    }
    out
}

/// The check pass's verdict on one ask: its next state, the rule that decided it and the gate that holds it, or the bar's
/// findings when it goes back to its asker (state raised).
#[derive(Clone, Debug, PartialEq)]
pub struct Verdict {
    pub state: State,
    pub rule: Option<String>,
    pub gate: Option<String>,
    pub lacks: Vec<String>,
}

/// Section 5's steps, cheapest first: a settled or never rule drops it, a held rule holds it, then the bar, then a his rule
/// checks it. A ready ask no rule takes stays raised, for the agent's check.
pub fn check(ask: &Ask, rules: &[Rule], facts: &Facts) -> Verdict {
    let first = |kinds: &[&str]| {
        (rules.iter()).find(|r| kinds.contains(&r.kind.as_str()) && fires(r, ask, facts))
    };
    let verdict = |state, rule: &Rule| Verdict {
        state,
        rule: Some(rule.id.clone()),
        gate: rule.gate.clone(),
        lacks: vec![],
    };
    if let Some(r) = first(&["settled", "never"]) {
        return verdict(State::Dropped, r);
    }
    if let Some(r) = first(&["held"]) {
        return verdict(State::Held, r);
    }
    let lacks = lacks(ask);
    if !lacks.is_empty() {
        return Verdict {
            state: State::Raised,
            rule: Some("bar".into()),
            gate: None,
            lacks,
        };
    }
    match first(&["his"]) {
        Some(r) => verdict(State::Checked, r),
        None => Verdict {
            state: State::Raised,
            rule: None,
            gate: None,
            lacks: vec!["no rule decides it: the agent's check".into()],
        },
    }
}

/// Each rule's firings, by id.
pub type Counts = BTreeMap<String, i64>;

/// Jev's two scores on an ask's words, each 0 to 1: `plain`, he can act on the line alone; `agent_can`, an agent could do or
/// decide it without him.
#[derive(Clone, Debug, PartialEq)]
pub struct Scored {
    pub words: String,
    pub plain: f64,
    pub agent_can: f64,
}

/// The agent's verdict on an ask Jev doubts: his or not, and the rule of the rules file it cites, with its reason.
#[derive(Clone, Debug, PartialEq)]
pub struct Ruled {
    pub words: String,
    pub his: bool,
    pub rule: String,
    pub why: String,
}

/// Section 5's steps 2 and 3: Jev for words, then an agent for what Jev doubts (@nha, 2026-11-11: « there needs to be a
/// "backlog" of things an agent / JEV has to double-check that they truly need me »). The binary's asks Jev and Claude; a
/// test's stands in.
pub trait Judge {
    /// Jev's (plain, agent_can) on the ask.
    fn jev(&mut self, ask: &Ask) -> Result<(f64, f64), String>;
    /// The agent's (his, rule id, why), its rule one of `rules`.
    fn agent(&mut self, ask: &Ask, rules: &[Rule]) -> Result<(bool, String, String), String>;
    /// What the judge asks, its model and questions: kept with each judgement, so changed questions judge every ask again.
    fn version(&self) -> String {
        String::new()
    }
}

/// Under this, Jev finds the line not plain enough to act on alone: it goes back.
pub const PLAIN_MIN: f64 = 0.5;
/// At or over this, Jev doubts only he can: the agent's check decides.
pub const AGENT_MAX: f64 = 0.5;

/// The words Jev and the agent judge: what, why, pick and link. A change of any is judged again.
pub fn words(ask: &Ask) -> String {
    let why = ask.why.as_deref().unwrap_or("");
    format!("{}\n{why}\n{}\n{}", ask.what, ask.pick, ask.link)
}

fn back(step: &str, lack: String) -> Verdict {
    Verdict {
        state: State::Raised,
        rule: Some(step.into()),
        gate: None,
        lacks: vec![lack],
    }
}

/// Steps 2 and 3 on an ask the rules and the bar found his (`v`, checked): Jev scores its words; a line not plain goes back,
/// a line Jev finds only his is checked; one Jev doubts goes to the agent, whose verdict checks it or drops it by the rule it
/// cites; one it finds not his, or citing no fitting rule, goes back naming why. A judge that does not answer leaves it
/// raised for the next pass. No ask reaches checked without Jev's score.
pub fn screen(ask: &mut Ask, v: Verdict, rules: &[Rule], judge: &mut dyn Judge) -> Verdict {
    let w = format!("{}\n{}", judge.version(), words(ask));
    if ask.jev.as_ref().is_none_or(|s| s.words != w) {
        match judge.jev(ask) {
            Ok((plain, agent_can)) => {
                ask.jev = Some(Scored {
                    words: w.clone(),
                    plain,
                    agent_can,
                })
            }
            Err(e) => {
                // the error on stderr, the lack fixed: an outage's changing words never rewrite every ask each pass
                eprintln!("inbox: Jev on {}: {e}", ask.id);
                return back("jev", "Jev's check, not answered yet".into());
            }
        }
    }
    let scored = ask.jev.clone().expect("scored above");
    if scored.plain < PLAIN_MIN {
        let lack = format!(
            "what it is, in plain words: Jev scores it {:.2}",
            scored.plain
        );
        return back("jev", lack);
    }
    if scored.agent_can < AGENT_MAX {
        return v;
    }
    if ask.agent.as_ref().is_none_or(|r| r.words != w) {
        match judge.agent(ask, rules) {
            Ok((his, rule, why)) => {
                ask.agent = Some(Ruled {
                    words: w,
                    his,
                    rule,
                    why,
                })
            }
            Err(e) => {
                eprintln!("inbox: the agent on {}: {e}", ask.id);
                return back("agent", "the agent's check, not answered yet".into());
            }
        }
    }
    let r = ask.agent.clone().expect("ruled above");
    let kind = (rules.iter())
        .find(|x| x.id == r.rule)
        .map(|x| x.kind.as_str());
    let ruled = |state| Verdict {
        state,
        rule: Some(format!("agent:{}", r.rule)),
        gate: None,
        lacks: vec![],
    };
    match (r.his, kind) {
        (true, Some("his")) => ruled(State::Checked),
        // never final on a model's word: back to its asker, the rule and reason named (/code-review: a dropped ask is gone)
        (false, Some("settled" | "never")) => Verdict {
            lacks: vec![format!(
                "not his, the agent's check finds ({}): {}",
                r.rule, r.why
            )],
            ..ruled(State::Raised)
        },
        _ => back(
            "agent",
            format!(
                "the agent's check cites no fitting rule ({}, his: {}): {}",
                r.rule, r.his, r.why
            ),
        ),
    }
}

/// One pass over every ask (section 3): a held ask whose gate lifted goes back to raised and is checked again; raised,
/// checked and shown asks are checked, a raised one the rules find his then screened by Jev and the agent (`screen`); a
/// settled or never rule drops a checked or shown one, a held rule holds it; then the app shows checked asks, nearest
/// deadline first then oldest, up to `CAP` shown. Each move's rule counts a firing. How many moved.
pub fn pass(
    asks: &mut BTreeMap<String, Ask>,
    rules: &[Rule],
    facts: &Facts,
    counts: &mut Counts,
    judge: &mut dyn Judge,
    now_ms: i64,
) -> usize {
    let mut facts = facts.clone();
    // a settled successor settles what it supersedes
    facts.superseded.extend(
        (asks.values())
            .filter(|a| matches!(a.state, State::Answered | State::Dropped))
            .filter_map(|a| a.supersedes.clone()),
    );
    let mut moved = 0;
    // an agent's verdict counts under the rule it cites (/code-review)
    let fired = |counts: &mut Counts, rule: &Option<String>| {
        if let Some(r) = rule {
            let r = r.strip_prefix("agent:").unwrap_or(r);
            *counts.entry(r.to_string()).or_default() += 1;
        }
    };
    for ask in asks.values_mut() {
        if ask.state == State::Held && !ask.gate.as_deref().is_some_and(|g| shut(g, ask, &facts)) {
            ask.state = State::Raised;
            ask.rule = Some("gate-lifted".into());
            ask.gate = None;
            ask.moved_ms = now_ms;
            moved += 1;
        }
        if !matches!(ask.state, State::Raised | State::Checked | State::Shown) {
            continue;
        }
        let mut v = check(ask, rules, &facts);
        // a raised ask the rules and the bar found his reaches checked only through Jev and the agent's check; a checked one
        // kept before them, never scored, is screened before its turn (/code-review)
        let unscored = ask.state == State::Checked && ask.jev.is_none();
        let screened = v.state == State::Checked && (ask.state == State::Raised || unscored);
        if screened {
            v = screen(ask, v, rules, judge);
        }
        let next = match (ask.state, v.state) {
            // the bar and the his rules judge a raised ask only: one shown stays until he answers or a rule settles it
            (State::Checked, State::Raised) if screened => State::Raised,
            (State::Checked | State::Shown, State::Raised | State::Checked) => continue,
            (_, next) => next,
        };
        let same = next == ask.state && v.lacks == ask.lacks && v.rule == ask.rule;
        if same {
            continue;
        }
        // a firing is a rule moving an ask: the readiness bar and Jev sending one back are no rule of the file
        if next != ask.state || v.rule.as_deref().is_some_and(|r| r.starts_with("agent:")) {
            fired(counts, &v.rule);
        }
        if next != ask.state {
            moved += 1;
        }
        ask.state = next;
        ask.rule = v.rule;
        ask.gate = v.gate;
        ask.lacks = v.lacks;
        ask.moved_ms = now_ms;
    }
    moved + show(asks, now_ms)
}

/// The app's move: checked asks to shown, nearest deadline first, then oldest, while fewer than `CAP` are shown. How many.
pub fn show(asks: &mut BTreeMap<String, Ask>, now_ms: i64) -> usize {
    let shown = asks.values().filter(|a| a.state == State::Shown).count();
    let mut waiting: Vec<(Option<String>, i64, String)> = (asks.values())
        .filter(|a| a.state == State::Checked)
        .map(|a| (a.deadline.clone(), a.raised_ms, a.id.clone()))
        .collect();
    // no deadline sorts after every day
    waiting.sort_by(|a, b| {
        let day = |d: &Option<String>| d.clone().unwrap_or_else(|| "9999".into());
        (day(&a.0), a.1).cmp(&(day(&b.0), b.1))
    });
    let room = CAP.saturating_sub(shown);
    let mut n = 0;
    for (_, _, id) in waiting.into_iter().take(room) {
        let ask = asks.get_mut(&id).expect("listed above");
        ask.state = State::Shown;
        ask.moved_ms = now_ms;
        n += 1;
    }
    n
}

/// His move: a shown ask answered, his words kept so it is never asked again.
pub fn answer(ask: &mut Ask, words: &str, now_ms: i64) -> Result<(), String> {
    if ask.state != State::Shown {
        return Err(format!(
            "ask {} is {}: only a shown ask is his to answer",
            ask.id,
            ask.state.name()
        ));
    }
    ask.state = State::Answered;
    ask.answer = Some(words.into());
    ask.moved_ms = now_ms;
    Ok(())
}

/// What the store keeps: every ask, each rule's firings, each gate he lifted with his words.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Inbox {
    pub asks: BTreeMap<String, Ask>,
    pub counts: Counts,
    pub lifted: BTreeMap<String, String>,
}

fn row(key: &str, record: Json) -> FieldWrite {
    FieldWrite {
        table: TABLE.into(),
        row: key.into(),
        field: FIELD.into(),
        value: Value::Text(record.to_string()),
    }
}

/// The inbox, read from the log: rows `ask:<id>`, `rule:<id>` (its firings) and `gate:<id>` (his words lifting it).
pub fn load(log: &dyn OpLog) -> Result<Inbox, String> {
    let mut inbox = Inbox::default();
    let mut after: Option<(i64, String)> = None;
    loop {
        let ops = log.ops_after(after.as_ref().map(|(s, w)| (*s, w.as_str())), PAGE)?;
        let Some(last) = ops.last() else {
            return Ok(inbox);
        };
        after = Some((last.stamp, last.writer.clone()));
        for w in ops.iter().map(|op| &op.write) {
            let (TABLE, FIELD, Value::Text(t)) = (w.table.as_str(), w.field.as_str(), &w.value)
            else {
                continue;
            };
            let Ok(j) = serde_json::from_str::<Json>(t) else {
                continue;
            };
            match w.row.split_once(':') {
                Some(("ask", id)) => {
                    if let Some(a) = Ask::from_json(&j) {
                        inbox.asks.insert(id.into(), a);
                    }
                }
                Some(("rule", id)) => {
                    inbox.counts.insert(id.into(), j.as_i64().unwrap_or(0));
                }
                Some(("gate", id)) => {
                    inbox
                        .lifted
                        .insert(id.into(), j.as_str().unwrap_or("").into());
                }
                _ => {}
            }
        }
    }
}

/// Writes what differs from `kept`, and says how many rows.
pub fn save(log: &mut dyn OpLog, kept: &Inbox, now: &Inbox) -> Result<usize, String> {
    let mut writes = vec![];
    for a in now
        .asks
        .values()
        .filter(|a| kept.asks.get(&a.id) != Some(a))
    {
        writes.push(row(&format!("ask:{}", a.id), a.to_json()));
    }
    for (id, n) in now
        .counts
        .iter()
        .filter(|(id, n)| kept.counts.get(*id) != Some(n))
    {
        writes.push(row(&format!("rule:{id}"), json!(n)));
    }
    for (id, w) in now
        .lifted
        .iter()
        .filter(|(id, w)| kept.lifted.get(*id) != Some(w))
    {
        writes.push(row(&format!("gate:{id}"), json!(w)));
    }
    if !writes.is_empty() {
        log.write(&writes)?;
    }
    Ok(writes.len())
}

/// A rule's example as the table test and `dash inbox rules --check` run it: the ask and the facts it names.
pub fn example(rule: &Rule) -> Result<(Ask, Facts), String> {
    let ask = (rule.example.get("ask"))
        .and_then(Ask::from_json)
        .ok_or_else(|| format!("rule {}: its example names no ask with a what", rule.id))?;
    let facts = Facts::from_json(rule.example.get("facts").unwrap_or(&json!({})));
    Ok((ask, facts))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_core::MemoryLog as Log;
    use dash_meter::memory::TestBudget;

    /// A judge standing in for Jev and the agent: plain and only his unless told, its calls counted.
    #[derive(Default)]
    struct Fake {
        scores: Option<(f64, f64)>,
        verdict: Option<(bool, String, String)>,
        down: bool,
        jev_calls: usize,
        agent_calls: usize,
    }

    impl Judge for Fake {
        fn jev(&mut self, _: &Ask) -> Result<(f64, f64), String> {
            self.jev_calls += 1;
            match self.down {
                true => Err("no answer".into()),
                false => Ok(self.scores.unwrap_or((0.9, 0.1))),
            }
        }
        fn agent(&mut self, _: &Ask, _: &[Rule]) -> Result<(bool, String, String), String> {
            self.agent_calls += 1;
            self.verdict.clone().ok_or_else(|| "no answer".into())
        }
    }

    fn all() -> Vec<Rule> {
        rules(RULES).expect("the rules read")
    }

    fn ready(id: &str, what: &str) -> Ask {
        Ask {
            why: Some("wording".into()),
            pick: "approve".into(),
            link: format!("/drafts/{id}"),
            ..Ask::raised(id, "test", what, 1)
        }
    }

    fn facts() -> Facts {
        Facts {
            today: "2026-11-11".into(),
            ..Facts::default()
        }
    }

    /// Done-when 2: every rule is a row with its example and its verdict, one table test runs them all, and a rule that
    /// fires counts it.
    #[test]
    fn every_rule_row_gives_its_example_the_verdict_it_names_and_counts_its_firing() {
        let _budget = TestBudget::new();
        let rules = all();
        assert!(rules.len() >= 15, "{}", rules.len());
        for rule in &rules {
            let (ask, facts) = example(rule).expect("an example");
            let v = check(&ask, &rules, &facts);
            assert_eq!(v.state.name(), rule.expect, "rule {}: {v:?}", rule.id);
            assert_eq!(
                v.rule.as_deref(),
                Some(rule.id.as_str()),
                "rule {} is the one that fires",
                rule.id
            );
            let mut asks = BTreeMap::from([(ask.id.clone(), ask)]);
            let mut counts = Counts::new();
            pass(
                &mut asks,
                &rules,
                &facts,
                &mut counts,
                &mut Fake::default(),
                2,
            );
            assert_eq!(
                counts.get(&rule.id),
                Some(&1),
                "rule {} counts its firing: {counts:?}",
                rule.id
            );
        }
        // a row without its example, or naming a test no code defines, fails the read
        let no_example =
            r#"[{"id":"x","kind":"never","when":"kind","said":"s","expect":"dropped"}]"#;
        assert!(super::rules(no_example).unwrap_err().contains("no example"));
        let unknown = r#"[{"id":"x","kind":"never","when":"moon","said":"s","example":{},"expect":"dropped"}]"#;
        assert!(super::rules(unknown).is_err());
    }

    // Done-when 1: one test per move of section 3's table, each named for the state it reaches.

    /// raised: an agent's new ask starts raised, unchecked, and the check pass is what moves it out.
    #[test]
    fn move_raised_an_agent_s_new_ask_starts_raised_and_only_the_check_pass_moves_it() {
        let _budget = TestBudget::new();
        let a = ready("a", "Approve the reply to Laura");
        assert_eq!(
            (a.state, a.rule.as_deref(), a.jev.as_ref()),
            (State::Raised, None, None)
        );
        // only a shown ask is his: answering a raised one is refused
        assert!(answer(&mut a.clone(), "x", 2).is_err());
        let mut asks = BTreeMap::from([("a".to_string(), a)]);
        pass(
            &mut asks,
            &all(),
            &facts(),
            &mut Counts::new(),
            &mut Fake::default(),
            2,
        );
        assert_ne!(asks["a"].state, State::Raised);
    }

    /// checked: a raised ask the rules find his, past Jev, is checked; one past the page's room waits its turn checked.
    #[test]
    fn move_checked_a_raised_ask_found_his_is_checked_and_waits_its_turn_there() {
        let _budget = TestBudget::new();
        let rules = all();
        let v = check(&ready("a", "Approve the reply to Laura"), &rules, &facts());
        assert_eq!(
            (v.state, v.rule.as_deref()),
            (State::Checked, Some("his-wording"))
        );
        let mut many: BTreeMap<String, Ask> = (0..CAP + 2)
            .map(|i| (format!("m{i}"), ready(&format!("m{i}"), "Approve a reply")))
            .collect();
        pass(
            &mut many,
            &rules,
            &facts(),
            &mut Counts::new(),
            &mut Fake::default(),
            2,
        );
        assert_eq!(
            many.values().filter(|a| a.state == State::Checked).count(),
            2
        );
    }

    /// held: an ask behind a gate of his is held, never shown, and goes back to raised when the gate lifts.
    #[test]
    fn move_held_a_gated_ask_is_held_and_goes_back_to_raised_when_its_gate_lifts() {
        let _budget = TestBudget::new();
        let rules = all();
        let babel = ready(
            "babel",
            "Approve the contract question to Samir (Babel Coop)",
        );
        let mut asks = BTreeMap::from([("babel".to_string(), babel)]);
        pass(
            &mut asks,
            &rules,
            &facts(),
            &mut Counts::new(),
            &mut Fake::default(),
            2,
        );
        assert_eq!(
            (asks["babel"].state, asks["babel"].gate.as_deref()),
            (State::Held, Some("cape"))
        );
        let signed = Facts {
            lifted: ["cape".to_string()].into(),
            ..facts()
        };
        // the lift raises it, the same pass checks it afresh and the app shows it
        let mut judge = Fake::default();
        pass(
            &mut asks,
            &rules,
            &signed,
            &mut Counts::new(),
            &mut judge,
            3,
        );
        assert_eq!((asks["babel"].state, judge.jev_calls), (State::Shown, 1));
    }

    /// shown: the app shows checked asks up to CAP, nearest deadline first.
    #[test]
    fn move_shown_the_app_shows_checked_asks_up_to_cap_nearest_deadline_first() {
        let _budget = TestBudget::new();
        let mut many: BTreeMap<String, Ask> = (0..CAP + 2)
            .map(|i| (format!("m{i}"), ready(&format!("m{i}"), "Approve a reply")))
            .collect();
        let last = format!("m{}", CAP + 1);
        many.get_mut(&last).unwrap().deadline = Some("2026-11-12".into());
        pass(
            &mut many,
            &all(),
            &facts(),
            &mut Counts::new(),
            &mut Fake::default(),
            2,
        );
        let shown: Vec<&str> = (many.values())
            .filter(|a| a.state == State::Shown)
            .map(|a| a.id.as_str())
            .collect();
        assert_eq!(shown.len(), CAP);
        assert!(
            shown.contains(&last.as_str()),
            "the deadline jumps the line: {shown:?}"
        );
    }

    /// answered: he answers a shown ask, his words kept; answered is final and never asked twice.
    #[test]
    fn move_answered_his_answer_to_a_shown_ask_is_kept_and_final() {
        let _budget = TestBudget::new();
        let rules = all();
        let mut asks =
            BTreeMap::from([("a".to_string(), ready("a", "Approve the reply to Laura"))]);
        pass(
            &mut asks,
            &rules,
            &facts(),
            &mut Counts::new(),
            &mut Fake::default(),
            2,
        );
        assert_eq!(asks["a"].state, State::Shown);
        answer(asks.get_mut("a").unwrap(), "yes, send it", 3).expect("answers");
        assert_eq!(
            (asks["a"].state, asks["a"].answer.as_deref()),
            (State::Answered, Some("yes, send it"))
        );
        pass(
            &mut asks,
            &rules,
            &facts(),
            &mut Counts::new(),
            &mut Fake::default(),
            4,
        );
        assert_eq!(asks["a"].state, State::Answered);
        assert!(answer(asks.get_mut("a").unwrap(), "again", 5).is_err());
    }

    /// dropped: a raised ask that is never his, and a shown one a rule settles meanwhile, are dropped, each naming its rule.
    #[test]
    fn move_dropped_a_never_rule_drops_a_raised_ask_and_a_settled_rule_a_shown_one() {
        let _budget = TestBudget::new();
        let rules = all();
        let mut p = Ask::raised("p", "mail-poll", "Sponsor our podcast", 1);
        p.kind = "pitch".into();
        let v = check(&p, &rules, &facts());
        assert_eq!(
            (v.state, v.rule.as_deref()),
            (State::Dropped, Some("never-pitch"))
        );
        let mut b = ready("b", "Approve the follow-up to David");
        b.draft = Some("cs-relance".into());
        let mut asks = BTreeMap::from([("b".to_string(), b)]);
        pass(
            &mut asks,
            &rules,
            &facts(),
            &mut Counts::new(),
            &mut Fake::default(),
            2,
        );
        assert_eq!(asks["b"].state, State::Shown);
        let sent = Facts {
            drafts_settled: ["cs-relance".to_string()].into(),
            ..facts()
        };
        pass(
            &mut asks,
            &rules,
            &sent,
            &mut Counts::new(),
            &mut Fake::default(),
            3,
        );
        assert_eq!(
            (asks["b"].state, asks["b"].rule.as_deref()),
            (State::Dropped, Some("settled-draft"))
        );
    }

    /// moved: an ask the readiness bar refuses is moved back to its asker, still raised, naming what it lacks, its move
    /// stamped; it is never shown.
    #[test]
    fn move_back_to_its_asker_an_ask_the_bar_refuses_stays_raised_naming_what_it_lacks() {
        let _budget = TestBudget::new();
        let thin = Ask::raised("t", "pm-loop", "Datadog plugin sign-in", 1);
        let mut asks = BTreeMap::from([("t".to_string(), thin)]);
        pass(
            &mut asks,
            &all(),
            &facts(),
            &mut Counts::new(),
            &mut Fake::default(),
            7,
        );
        let t = &asks["t"];
        assert_eq!(
            (t.state, t.rule.as_deref(), t.asker.as_str(), t.moved_ms),
            (State::Raised, Some("bar"), "pm-loop", 7)
        );
        assert!(t.lacks.iter().any(|l| l.contains("link")), "{:?}", t.lacks);
    }

    /// Done-when 3: a gate holds its asks, prepared and never shown, and raises them again when it lifts, checked afresh.
    #[test]
    fn a_gate_holds_its_asks_and_raises_them_again_when_it_lifts() {
        let _budget = TestBudget::new();
        let rules = all();
        let mut c = Counts::new();
        let babel = ready(
            "babel",
            "Approve the contract question to Samir (Babel Coop)",
        );
        let mut graal = ready("graal", "Create graal's Developer ID certificate");
        graal.why = Some("his-hands".into());
        graal.not_before = Some("2027-01-04".into());
        let mut asks = BTreeMap::from([("babel".to_string(), babel), ("graal".to_string(), graal)]);
        pass(&mut asks, &rules, &facts(), &mut c, &mut Fake::default(), 2);
        assert_eq!(
            (asks["babel"].state, asks["babel"].gate.as_deref()),
            (State::Held, Some("cape"))
        );
        assert_eq!(
            (asks["graal"].state, asks["graal"].gate.as_deref()),
            (State::Held, Some("date"))
        );
        // still shut: held, never shown
        pass(&mut asks, &rules, &facts(), &mut c, &mut Fake::default(), 3);
        assert!(asks.values().all(|a| a.state == State::Held));
        // CAPE signed: back to raised, checked again on fresh facts, then shown
        let signed = Facts {
            lifted: ["cape".to_string()].into(),
            ..facts()
        };
        pass(&mut asks, &rules, &signed, &mut c, &mut Fake::default(), 4);
        assert_eq!(asks["babel"].state, State::Shown);
        assert_eq!(asks["graal"].state, State::Held);
        // the day comes
        let day = Facts {
            today: "2027-01-04".into(),
            ..signed
        };
        pass(&mut asks, &rules, &day, &mut c, &mut Fake::default(), 5);
        assert_eq!(asks["graal"].state, State::Shown);
        // a gate that lifts on an ask settled meanwhile drops it instead of showing it
        let mut late = ready("late", "Approve the Hostman reply");
        late.draft = Some("hostman-reply".into());
        let mut asks = BTreeMap::from([("late".to_string(), late)]);
        pass(&mut asks, &rules, &facts(), &mut c, &mut Fake::default(), 2);
        assert_eq!(asks["late"].state, State::Held);
        let settled = Facts {
            lifted: ["cape".to_string()].into(),
            drafts_settled: ["hostman-reply".to_string()].into(),
            ..facts()
        };
        pass(&mut asks, &rules, &settled, &mut c, &mut Fake::default(), 3);
        assert_eq!(asks["late"].state, State::Dropped);
    }

    /// Done-when 4: only an ask with the bar's five fields reaches checked; one without goes back naming each it lacks.
    #[test]
    fn only_an_ask_with_the_bar_s_five_fields_is_shown_and_one_without_goes_back_with_what_it_lacks()
     {
        let _budget = TestBudget::new();
        let rules = all();
        let mut c = Counts::new();
        let mut full = ready("full", "New opening line for Naresh (Tether): approve it?");
        full.approval = true;
        full.sent_by = "the LinkedIn tool".into();
        let mut asks = BTreeMap::from([("full".to_string(), full.clone())]);
        pass(&mut asks, &rules, &facts(), &mut c, &mut Fake::default(), 2);
        assert_eq!(asks["full"].state, State::Shown);
        let lacking = |f: &dyn Fn(&mut Ask), says: &str| {
            let mut a = full.clone();
            f(&mut a);
            let mut asks = BTreeMap::from([("full".to_string(), a)]);
            pass(
                &mut asks,
                &rules,
                &facts(),
                &mut Counts::new(),
                &mut Fake::default(),
                2,
            );
            let a = &asks["full"];
            assert_eq!(a.state, State::Raised, "{says}");
            assert!(
                a.lacks.iter().any(|l| l.starts_with(says)),
                "{says}: {:?}",
                a.lacks
            );
        };
        lacking(
            &|a| a.what = "tether-naresh-actu".into(),
            "what it is, in plain words",
        );
        lacking(
            &|a| a.what = "WARNING: step line over 200 characters".into(),
            "what it is, in plain words",
        );
        lacking(&|a| a.why = None, "why only him");
        lacking(&|a| a.why = Some("urgent".into()), "why only him");
        lacking(&|a| a.pick = String::new(), "the agent's pick");
        lacking(
            &|a| a.link = String::new(),
            "a link that opens the thing itself",
        );
        lacking(
            &|a| a.link = "http://100.72.158.115:7788/mail".into(),
            "a link that opens the thing itself, not the list page /mail",
        );
        lacking(&|a| a.sent_by = String::new(), "who sends it");
        // all five missing: each is named
        let bare = Ask::raised("bare", "x", "", 1);
        assert_eq!(
            lacks(&Ask {
                approval: true,
                ..bare
            })
            .len(),
            5
        );
    }

    /// The task's outcome (its title: « An ask for him is checked by an agent or Jev before it reaches /inbox »): no ask reaches
    /// checked, so none is shown, without Jev's score; a line Jev finds not plain goes back; one Jev doubts goes to the agent,
    /// whose verdict must cite a rule; the same words are never judged twice; a judge that does not answer leaves it raised.
    #[test]
    fn an_ask_reaches_his_inbox_only_through_jev_and_the_agent_s_check() {
        let _budget = TestBudget::new();
        let rules = all();
        let one = |a: Ask| BTreeMap::from([(a.id.clone(), a)]);
        // Jev agrees only he can: checked and shown, Jev's scores kept; a second pass on the same words asks Jev nothing
        let mut judge = Fake::default();
        let mut asks = one(ready("a", "Approve the reply to Laura"));
        pass(
            &mut asks,
            &rules,
            &facts(),
            &mut Counts::new(),
            &mut judge,
            2,
        );
        assert_eq!(asks["a"].state, State::Shown);
        assert_eq!(asks["a"].jev.as_ref().map(|j| j.plain), Some(0.9));
        pass(
            &mut asks,
            &rules,
            &facts(),
            &mut Counts::new(),
            &mut judge,
            3,
        );
        assert_eq!((judge.jev_calls, judge.agent_calls), (1, 0));
        // Jev finds it not plain: back, never shown, the score named
        let mut murky = Fake {
            scores: Some((0.2, 0.1)),
            ..Fake::default()
        };
        let mut asks = one(ready("b", "Approve the R12 relance per the 20b gate"));
        pass(
            &mut asks,
            &rules,
            &facts(),
            &mut Counts::new(),
            &mut murky,
            2,
        );
        assert_eq!(asks["b"].state, State::Raised);
        assert_eq!(
            asks["b"].lacks,
            ["what it is, in plain words: Jev scores it 0.20"]
        );
        // Jev doubts, the agent finds it not his by a never rule: dropped, the rule it cited counted
        let mut doubts = Fake {
            scores: Some((0.9, 0.8)),
            verdict: Some((
                false,
                "never-optional".into(),
                "nothing depends on it".into(),
            )),
            ..Fake::default()
        };
        let mut c = Counts::new();
        let mut asks = one(ready("c", "Sign in to the Datadog plugin"));
        pass(&mut asks, &rules, &facts(), &mut c, &mut doubts, 2);
        // never final on a model's word: back to its asker, the rule and its reason named, counted under the rule it cites
        assert_eq!(
            (asks["c"].state, asks["c"].rule.as_deref()),
            (State::Raised, Some("agent:never-optional"))
        );
        assert_eq!(
            asks["c"].lacks,
            ["not his, the agent's check finds (never-optional): nothing depends on it"]
        );
        assert_eq!(c.get("never-optional"), Some(&1));
        // the next pass judges nothing again and counts nothing again
        pass(&mut asks, &rules, &facts(), &mut c, &mut doubts, 3);
        assert_eq!(
            (
                doubts.jev_calls,
                doubts.agent_calls,
                c.get("never-optional")
            ),
            (1, 1, Some(&1))
        );
        // a new link is new words: judged again
        asks.get_mut("c").unwrap().link = "/drafts/other".into();
        pass(&mut asks, &rules, &facts(), &mut c, &mut doubts, 4);
        assert_eq!(doubts.jev_calls, 2);
        // a checked ask kept before the screen, never scored, is screened before its turn
        let mut murky = Fake {
            scores: Some((0.2, 0.1)),
            ..Fake::default()
        };
        let mut kept = ready("k", "Approve the R12 relance");
        kept.state = State::Checked;
        let mut asks = one(kept);
        pass(
            &mut asks,
            &rules,
            &facts(),
            &mut Counts::new(),
            &mut murky,
            2,
        );
        assert_eq!(asks["k"].state, State::Raised);
        // Jev doubts, the agent finds it his: checked, then shown
        let mut his = Fake {
            scores: Some((0.9, 0.8)),
            verdict: Some((true, "his-wording".into(), "new words in his voice".into())),
            ..Fake::default()
        };
        let mut asks = one(ready("d", "Approve the new opening line to Naresh"));
        pass(&mut asks, &rules, &facts(), &mut Counts::new(), &mut his, 2);
        assert_eq!(
            (asks["d"].state, asks["d"].rule.as_deref()),
            (State::Shown, Some("agent:his-wording"))
        );
        // a verdict citing no fitting rule (a held rule, or none) goes back and is never shown
        for cited in ["held-cape", "made-up"] {
            let mut odd = Fake {
                scores: Some((0.9, 0.8)),
                verdict: Some((true, cited.into(), "".into())),
                ..Fake::default()
            };
            let mut asks = one(ready("e", "Approve the reply to Laura"));
            pass(&mut asks, &rules, &facts(), &mut Counts::new(), &mut odd, 2);
            assert_eq!(asks["e"].state, State::Raised, "{cited}");
            assert!(
                asks["e"].lacks[0].starts_with("the agent's check cites no fitting rule"),
                "{cited}"
            );
        }
        // Jev down: raised, said, asked again next pass; new words are judged again
        let mut down = Fake {
            down: true,
            ..Fake::default()
        };
        let mut asks = one(ready("f", "Approve the reply to Laura"));
        pass(
            &mut asks,
            &rules,
            &facts(),
            &mut Counts::new(),
            &mut down,
            2,
        );
        assert_eq!(asks["f"].state, State::Raised);
        assert!(asks["f"].lacks[0].starts_with("Jev's check, not answered yet"));
        down.down = false;
        pass(
            &mut asks,
            &rules,
            &facts(),
            &mut Counts::new(),
            &mut down,
            3,
        );
        assert_eq!((asks["f"].state, down.jev_calls), (State::Shown, 2));
        // kept on the log and read back with its scores
        let mut log = Log::default();
        let inbox = Inbox {
            asks,
            ..Inbox::default()
        };
        save(&mut log, &Inbox::default(), &inbox).expect("saved");
        assert_eq!(
            load(&log).expect("loads").asks["f"].jev,
            inbox.asks["f"].jev
        );
    }

    #[test]
    fn the_inbox_is_kept_on_the_log_and_read_back_whole() {
        let _budget = TestBudget::new();
        let mut log = Log::default();
        let mut now = Inbox::default();
        now.asks
            .insert("a".into(), ready("a", "Approve the reply to Laura"));
        now.counts.insert("his-wording".into(), 3);
        now.lifted.insert("cape".into(), "signed, his word".into());
        assert_eq!(save(&mut log, &Inbox::default(), &now).unwrap(), 3);
        let back = load(&log).unwrap();
        assert_eq!(back, now);
        assert_eq!(
            save(&mut log, &back, &now).unwrap(),
            0,
            "nothing changed, nothing written"
        );
    }
}
