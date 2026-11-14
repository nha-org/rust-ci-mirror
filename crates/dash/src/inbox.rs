//! The binary's wiring of his inbox (dash_comments::asks; private/docs/rfc-inbox.html; task rust-inbox-triage): the asks kept
//! in `comments.sqlite` beside the comment queue, the older app's /inbox read in as raised asks (`dash handoff`, the one place
//! every older source writes today, so each of them reaches the check pass without an Elixir edit), the facts the rules test
//! read off the older app's drafts, GitHub and the task list, and /inbox's « Waiting on you » drawn from the shown asks alone.
//! `dash orch` runs a pass after each comment pass; `dash inbox pass --data <dir>` runs one.
#[cfg(test)]
use crate::home::drawn;
use crate::home::{Card, Read};
use crate::jev::{self, fnv, noul, paid_refused};
use crate::tasks::{now_ms, older_app, older_db};
use dash_comments::asks::{self, Ask, Facts, Inbox, State};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::time::Duration;

const CALL: Duration = Duration::from_secs(20);

/// Jev's two questions on an ask's words, each a 0 to 1 « noul » (crate::jev, the one client).
const JEV_QUESTIONS: &str = r#"{
  "plain_words": {"type": "noul",
    "instructions": "`ask` is the one line an agent puts in front of `reader` on his inbox; `link` opens the thing it is about (the draft, the form, the page). Is `ask` written in plain words he reads at a glance, with no task id, file path, code, internal acronym or jargon to decode? A short line is fine: what it is about is behind `link`.",
    "criteria": {"true": "Plain words: he knows what he is asked at a glance.", "false": "He would have to decode an id, a path, code, an acronym or jargon."}},
  "agent_can": {"type": "noul",
    "instructions": "`ask` is put to `reader` because, its agent says, only he can answer it (`why`). Could an AI agent with his tools, his stored credentials and his written rules do it or decide it without him?",
    "criteria": {"true": "An agent could do or decide it without him.", "false": "Only he can: it commits him to an outside party, needs new words in his own voice, needs his hands (a browser sign-in, sudo, a form with no stored credentials), or is an irreversible choice that is contested."}}
}"#;
/// One agent run at most.
const AGENT_CALL: Duration = Duration::from_secs(180);
/// The question pass's model and its run, at most, in US dollars: one dialog read, one JSON answered (task
/// rust-questions-orchestrator-first; @nha, 2026-11-12: « a sonnet pass on each with a specific brief »).
const QUESTION_MODEL: &str = "sonnet";
const QUESTION_BUDGET: f64 = 0.25;
/// The agent's run, at most, in US dollars: a few lines read, one JSON answered.
const AGENT_BUDGET: f64 = 0.25;

/// Paid calls in one pass, at most: the rest wait for the next pass, so a pass is bounded in time and money (/code-review).
const CALLS_PER_PASS: usize = 5;
/// Of those, the question pass's at most: his asks keep the others.
const QUESTIONS_PER_PASS: usize = 3;

/// The binary's judge: Jev over curl, its key from bin/secret in curl's environment, never its command line; the agent,
/// Claude Code in print mode with no tools (`DASH_CLAUDE` names another). CI and a test never pay for either. One a pass:
/// `left` its paid calls, `key` a failed key read remembered for the pass, `spent_micro` what its agent runs cost.
struct Checkers {
    left: usize,
    key: Option<String>,
    spent_micro: i64,
}

impl Checkers {
    fn new() -> Checkers {
        Checkers {
            left: CALLS_PER_PASS,
            key: None,
            spent_micro: 0,
        }
    }

    /// A paid call taken from the pass's allowance, or why none is.
    fn call(&mut self) -> Result<(), String> {
        if let Some(e) = &self.key {
            return Err(e.clone());
        }
        if self.left == 0 {
            return Err(format!(
                "{CALLS_PER_PASS} paid calls this pass: it waits for the next"
            ));
        }
        if let Some(no) = paid_refused() {
            return Err(no);
        }
        self.left -= 1;
        Ok(())
    }
}

/// The agent's prompt's version: raise it when `agent_prompt` changes, so its verdicts are asked again.
const AGENT_PROMPT_VERSION: &str = "1";

/// The agent's verdict in its words: one JSON object, maybe inside other text.
fn verdict(said: &str) -> Result<(bool, String, String), String> {
    let (start, end) = (said.find('{'), said.rfind('}'));
    let json = match (start, end) {
        (Some(a), Some(b)) if a < b => &said[a..=b],
        _ => {
            return Err(format!(
                "no JSON in the agent's answer: {}",
                said.chars().take(200).collect::<String>()
            ));
        }
    };
    let v: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("the agent's JSON: {e}"))?;
    let his = v["his"].as_bool().ok_or("the agent's JSON has no his")?;
    let rule = v["rule"].as_str().ok_or("the agent's JSON has no rule")?;
    Ok((
        his,
        rule.into(),
        v["why"].as_str().unwrap_or_default().into(),
    ))
}

/// The agent's prompt: the ask, then every rule it may cite.
fn agent_prompt(ask: &Ask, rules: &[asks::Rule]) -> String {
    let rows: Vec<String> = (rules.iter())
        .filter(|r| matches!(r.kind.as_str(), "settled" | "never" | "his"))
        .map(|r| format!("- {} ({}): {}", r.id, r.kind, r.said))
        .collect();
    format!(
        "You check an ask an agent wants to put in front of Nicolas on his inbox. Jev doubts that only he can answer it.\n\n\
         Ask: {}\nWhy only him, as its agent says: {}\nThe agent's pick: {}\nLink: {}\n\n\
         Decide whether only Nicolas can answer it. Cite exactly one rule id from this list: a his rule when it is his, a \
         settled or never rule when it is not.\n{}\n\n\
         Answer with one JSON object and nothing else: {{\"his\": true or false, \"rule\": \"<rule id>\", \"why\": \"<one sentence>\"}}",
        ask.what,
        ask.why.as_deref().unwrap_or(""),
        ask.pick,
        ask.link,
        rows.join("\n")
    )
}

impl asks::Judge for Checkers {
    /// A hash of all a judgement depends on beyond the ask: Jev's model and questions, the agent's prompt, the rules.
    fn version(&self) -> String {
        let all = format!(
            "{}\n{JEV_QUESTIONS}\n{AGENT_PROMPT_VERSION}\n{}",
            jev::MODEL,
            asks::RULES
        );
        format!("{:016x}", fnv(&all))
    }

    fn jev(&mut self, ask: &Ask) -> Result<(f64, f64), String> {
        self.call()?;
        let questions: serde_json::Value =
            serde_json::from_str(JEV_QUESTIONS).expect("the questions are JSON");
        let state = serde_json::json!({"ask": ask.what, "why": ask.why, "pick": ask.pick, "link": ask.link,
            "reader": "Nicolas, a freelance engineer, reading his inbox on his phone"});
        jev::key().inspect_err(|e| self.key = Some(e.clone()))?;
        let answer = jev::ask(&questions, state, "site_perso/inbox")?;
        Ok((noul(&answer, "plain_words")?, noul(&answer, "agent_can")?))
    }

    fn agent(&mut self, ask: &Ask, rules: &[asks::Rule]) -> Result<(bool, String, String), String> {
        self.call()?;
        let claude = std::env::var("DASH_CLAUDE").unwrap_or_else(|_| "claude".into());
        let args = dash_agent::headless(&agent_prompt(ask, rules), AGENT_BUDGET);
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let out = dash_process::run(&claude, &args, Some(&std::env::temp_dir()), AGENT_CALL)?;
        self.read_agent(&out)
    }
}

impl Checkers {
    /// The question pass's answer to `brief` (`dash_comments::panes::BRIEF` and one row's dialog): Sonnet with no tools, a paid
    /// call of the pass's allowance, its cost counted.
    fn question(&mut self, brief: &str) -> Result<String, String> {
        self.call()?;
        let claude = std::env::var("DASH_CLAUDE").unwrap_or_else(|_| "claude".into());
        let mut args = vec!["--model".to_string(), QUESTION_MODEL.into()];
        args.extend(dash_agent::headless(brief, QUESTION_BUDGET));
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let out = dash_process::run(&claude, &args, Some(&std::env::temp_dir()), AGENT_CALL)?;
        let (said, usd) = dash_agent::headless_outcome(&out)?;
        self.spent_micro += (usd * 1e6).round() as i64;
        Ok(said)
    }

    /// The agent's verdict read off its run's answer; its cost counted whatever its verdict reads as (/code-review):
    /// `dash inbox rules` says the total.
    fn read_agent(&mut self, out: &str) -> Result<(bool, String, String), String> {
        let (said, usd) = dash_agent::headless_outcome(out)?;
        self.spent_micro += (usd * 1e6).round() as i64;
        verdict(&said)
    }
}

fn rules() -> Vec<asks::Rule> {
    asks::rules(asks::RULES).expect("the tracked rules read: their table test holds it")
}

/// A stable id for an item of the older app's page: its part and its words, cut to letters, digits and dashes; past 80 they
/// are cut and the whole words' FNV-1a hash added, so two long items that share a start stay two asks (/code-review).
fn item_id(part: &str, card: &Card) -> String {
    let words = format!("{part} {} {}", card.title, card.line);
    let slug: String = (words.to_lowercase().chars())
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let slug: Vec<&str> = slug.split('-').filter(|w| !w.is_empty()).collect();
    let id = format!("handoff-{}", slug.join("-"));
    if id.len() <= 80 {
        return id;
    }
    format!("{}-{:016x}", &id[..80], fnv(&words))
}

/// Whether `s` is a day as the gates compare them, YYYY-MM-DD.
fn is_day(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10
        && b.iter().enumerate().all(|(i, c)| match i {
            4 | 7 => *c == b'-',
            _ => c.is_ascii_digit(),
        })
}

/// The day an item says it waits for, « (from 2027-01-04) »: the first « from » followed by a day.
fn from_day(line: &str) -> Option<String> {
    (line.match_indices("from ")).find_map(|(at, _)| {
        let day: String = line[at + 5..].chars().take(10).collect();
        is_day(&day).then_some(day)
    })
}

/// What sends an approved draft once he says yes, as the drafts page says Claude sends it (drafts::by_claude); none when no
/// tool does, so the bar sends the ask back naming it.
fn sender(draft: &serde_json::Value) -> String {
    if !dash_drafts::by_claude(draft) {
        return String::new();
    }
    match draft["channel"].as_str() {
        Some("linkedin") => "the LinkedIn tool".into(),
        Some("email") => "dash mail send, from nicolas@klar.im".into(),
        _ => "free-work's API, the CV shared in the same call".into(),
    }
}

/// The older app's /inbox as raised asks, each with what its part says of the bar's fields. `drafts`: the older app's drafts.
fn from_handoff(read: &Read, drafts: &serde_json::Value, now: i64) -> Vec<Ask> {
    let by_id = |id: &str| (drafts.as_array().into_iter().flatten()).find(|d| d["id"] == id);
    let mut out = vec![];
    for group in &read.groups {
        let part = group.title.split_whitespace().next().unwrap_or("");
        for card in &group.cards {
            let mut ask = Ask::raised(&item_id(part, card), "handoff", &card.title, now);
            ask.source = "handoff".into();
            ask.link = card.href.clone().unwrap_or_default();
            ask.not_before = from_day(&format!("{} {}", card.title, card.line));
            match part {
                "Drafts" => {
                    ask.what = format!("Approve the draft to {}", card.title);
                    ask.why = Some("wording".into());
                    ask.pick = "approve as written".into();
                    ask.approval = true;
                    ask.draft = Some(card.line.clone());
                    ask.sent_by = by_id(&card.line).map(sender).unwrap_or_default();
                }
                "Blocks" => {
                    ask.what = format!("Validate the new wording « {} »", card.title);
                    ask.why = Some("wording".into());
                    ask.pick = "approve".into();
                }
                // a sandbox change: only his tap widens what an agent may do
                "Sandbox" => {
                    ask.what = format!("Approve the sandbox change: {}", card.title);
                    ask.why = Some("his-hands".into());
                    ask.pick = "approve".into();
                }
                // a comment waiting on Claude is an agent's queue, never his (rfc-inbox section 6)
                "Comments" => ask.kind = "comment".into(),
                _ => {}
            }
            out.push(ask);
        }
    }
    out
}

/// Whether the older app says task `id` is done or dropped.
fn task_closed(id: &str) -> bool {
    let shown = dash_process::run(&older_app().bin, &["task", "show", id], None, CALL);
    shown.is_ok_and(|s| (s.lines()).any(|l| matches!(l.trim(), "status: done" | "status: dropped")))
}

/// Whether PR `n` is merged or closed.
fn pr_closed(n: u64) -> bool {
    let state = dash_process::run(
        "gh",
        &[
            "pr",
            "view",
            &n.to_string(),
            "--json",
            "state",
            "-q",
            ".state",
        ],
        None,
        CALL,
    );
    state.is_ok_and(|s| matches!(s.trim(), "MERGED" | "CLOSED"))
}

/// The facts for the open asks of `inbox`: only the PRs and tasks an open ask names are asked about.
fn facts(
    inbox: &Inbox,
    drafts: &serde_json::Value,
    listed: Option<BTreeSet<String>>,
    now: i64,
) -> Facts {
    let open =
        || (inbox.asks.values()).filter(|a| !matches!(a.state, State::Answered | State::Dropped));
    let settled =
        |d: &serde_json::Value| matches!(d["status"].as_str(), Some("sent" | "moot" | "rejected"));
    Facts {
        prs_closed: open()
            .filter_map(|a| a.pr)
            .filter(|n| pr_closed(*n))
            .collect(),
        tasks_done: (open().filter_map(|a| a.task.clone()))
            .filter(|t| task_closed(t))
            .collect(),
        drafts_settled: (drafts.as_array().into_iter().flatten())
            .filter(|d| settled(d))
            .filter_map(|d| d["id"].as_str().map(str::to_string))
            .collect(),
        superseded: BTreeSet::new(),
        listed,
        lifted: inbox.lifted.keys().cloned().collect(),
        today: dash_core::date(now.div_euclid(86_400_000)),
    }
}

/// How many asks in each state, as one line.
fn counted(inbox: &Inbox) -> String {
    let n = |s: State| inbox.asks.values().filter(|a| a.state == s).count();
    (State::ALL.iter())
        .map(|s| format!("{} {}", n(*s), s.name()))
        .collect::<Vec<_>>()
        .join(", ")
}

/// An item read from the older app's page into `inbox`: new, it lands raised; still raised (back to its asker), its fields
/// are read again, so a sender a failed drafts read missed is found on the next pass; dropped as gone from the page and back
/// on it, it lands again, so one short read never loses an item for good (/code-review). Any other is left to its state.
fn refreshed(inbox: &mut Inbox, mut ask: Ask) {
    match inbox.asks.get(&ask.id) {
        Some(old) if old.state == State::Raised => {
            (ask.raised_ms, ask.moved_ms) = (old.raised_ms, old.moved_ms);
            (ask.rule, ask.lacks) = (old.rule.clone(), old.lacks.clone());
            // its judgements kept: the same words are never paid for again (/code-review)
            (ask.jev, ask.agent) = (old.jev.clone(), old.agent.clone());
        }
        Some(old) if old.state == State::Dropped && old.rule.as_deref() == Some("settled-gone") => {
        }
        Some(_) => return,
        None => {}
    }
    inbox.asks.insert(ask.id.clone(), ask);
}

/// What the agent's runs cost, in millionths of a dollar, kept under `SPENT` beside the rules' counts.
const SPENT: &str = "spent:agent-micro-usd";

fn spent(inbox: &mut Inbox, judge: &Checkers) {
    if judge.spent_micro > 0 {
        *inbox.counts.entry(SPENT.into()).or_default() += judge.spent_micro;
    }
}

/// One pass: read the older app's /inbox in, check every ask, show what is ready, keep it. A line saying what it did.
pub fn pass(dir: &Path, now: i64) -> Result<String, String> {
    let mut log = crate::home::asks_log(dir)?;
    let kept = asks::load(&log)?;
    let mut inbox = kept.clone();
    let drafts = dash_store::older_drafts::drafts(&older_db(), None).unwrap_or_else(|e| {
        eprintln!("inbox: the older app's drafts: {e}");
        serde_json::Value::Null
    });
    // a page that did not answer lists nothing as gone: its asks stay as they are
    let listed = match dash_store::older_handoff::here(&crate::tasks::repo(), true).map(|r| r.text)
    {
        Ok(text) => {
            let read = crate::home::cards(&text, crate::home::handoff_link);
            let items = from_handoff(&read, &drafts, now);
            let ids = items.iter().map(|a| a.id.clone()).collect();
            for ask in items {
                refreshed(&mut inbox, ask);
            }
            Some(ids)
        }
        Err(e) => {
            eprintln!("inbox: the handoff's read: {e}");
            None
        }
    };
    let mut judge = Checkers::new();
    // the panes' questions first (task rust-questions-orchestrator-first): each ruled by the one brief, a reversible one picked
    // in its pane, a one-way one raised here and checked below. At most QUESTIONS_PER_PASS of the pass's paid calls, so his
    // asks keep the rest; a failure costs this part only, never his asks' checks (/code-review)
    let questions = crate::watch::rule_questions(
        dir,
        &mut inbox.asks,
        &mut |brief| judge.question(brief),
        QUESTIONS_PER_PASS,
        now,
    );
    let (ruled, settled) = questions.unwrap_or_else(|e| {
        eprintln!("inbox: the panes' questions: {e}");
        (0, BTreeSet::new())
    });
    let mut facts = facts(&inbox, &drafts, listed, now);
    facts.superseded.extend(settled);
    let moved = asks::pass(
        &mut inbox.asks,
        &rules(),
        &facts,
        &mut inbox.counts,
        &mut judge,
        now,
    );
    spent(&mut inbox, &judge);
    let written = asks::save(&mut log, &kept, &inbox)?;
    Ok(format!(
        "{}; {ruled} questions ruled, {moved} moved, {written} written",
        counted(&inbox)
    ))
}

fn said(r: Result<String, String>, mut out: File) {
    match r {
        Ok(line) => writeln!(out, "{line}").expect("stdout"),
        Err(e) => {
            eprintln!("inbox: {e}");
            std::process::exit(1);
        }
    }
}

/// The pass, its line or why it failed, waited for.
pub fn waited(dir: &Path) -> String {
    pass(dir, now_ms()).unwrap_or_else(|e| format!("failed: {e}"))
}

/// The pass on a thread of its own, one at a time, so the comment beat never waits on Jev or the agent (rust/CLAUDE.md
/// rule 5; /code-review). The line the last pass said, or that the first one runs.
pub fn in_background(dir: &Path) -> String {
    static INBOX: Background = Background::new(Duration::ZERO);
    on_own_thread(&INBOX, dir, waited)
}

/// A pass run on a thread of its own, one at a time, started at most once per `every`, and the line its last run said (the
/// live check's too).
pub struct Background {
    running: std::sync::atomic::AtomicBool,
    last: std::sync::Mutex<String>,
    every: Duration,
    /// When its last run started: stamped only by a start, so a run still going never pushes the next one back (/code-review).
    started: std::sync::Mutex<Option<std::time::Instant>>,
}

impl Background {
    pub const fn new(every: Duration) -> Background {
        Background {
            running: std::sync::atomic::AtomicBool::new(false),
            last: std::sync::Mutex::new(String::new()),
            every,
            started: std::sync::Mutex::new(None),
        }
    }
}

/// Starts `pass` on its own thread unless `b`'s last is still running; the line its last run said.
pub fn on_own_thread(b: &'static Background, dir: &Path, pass: fn(&Path) -> String) -> String {
    /// Lets the next pass start however this one ends, a panic included.
    struct Ends(&'static Background);
    impl Drop for Ends {
        fn drop(&mut self) {
            (self.0.running).store(false, std::sync::atomic::Ordering::SeqCst);
        }
    }
    let mut started = b.started.lock().unwrap_or_else(|p| p.into_inner());
    let due = started.is_none_or(|at| at.elapsed() >= b.every);
    if due && !b.running.swap(true, std::sync::atomic::Ordering::SeqCst) {
        *started = Some(std::time::Instant::now());
        let dir = dir.to_path_buf();
        std::thread::spawn(move || {
            let _ends = Ends(b);
            let line = pass(&dir);
            *b.last.lock().unwrap_or_else(|p| p.into_inner()) = line;
        });
    }
    drop(started);
    on_own_thread_said(b)
}

/// The line `b`'s last run said, or that its first one runs.
pub fn on_own_thread_said(b: &Background) -> String {
    let last = b.last.lock().unwrap_or_else(|p| p.into_inner()).clone();
    match last.is_empty() {
        true => "its first pass runs".into(),
        false => last,
    }
}

inventory::submit! { crate::usage::Command::usage("dash inbox pass|ls [<state>]|rules --data <dir>") }
pub fn once(dir: &Path, out: File) {
    said(pass(dir, now_ms()), out)
}

/// One ask as a line: its id, state, what, the rule that moved it last and, if it went back, what it lacks.
fn line(a: &Ask) -> String {
    let mut l = format!(
        "{}\t{}\t{}\t{}",
        a.id,
        a.state.name(),
        a.what,
        a.rule.as_deref().unwrap_or("-")
    );
    if let Some(g) = &a.gate {
        l.push_str(&format!("\theld by {g}"));
    }
    if !a.lacks.is_empty() {
        l.push_str(&format!("\tlacks: {}", a.lacks.join("; ")));
    }
    l
}

/// `dash inbox ls --data <dir> [<state>]`.
pub fn ls(dir: &Path, state: Option<&str>, out: File) {
    let r = (|| {
        let inbox = asks::load(&crate::home::asks_log(dir)?)?;
        let want = state
            .map(|s| State::parse(s).ok_or(format!("no state {s}")))
            .transpose()?;
        let lines: Vec<String> = (inbox.asks.values())
            .filter(|a| want.is_none_or(|w| a.state == w))
            .map(line)
            .collect();
        Ok(format!("{}\n{}", counted(&inbox), lines.join("\n")))
    })();
    said(r, out)
}

inventory::submit! { crate::usage::Command::usage("dash inbox rules [--data <dir>]") }
/// `dash inbox rules [--data <dir>]`: each rule, its kind, its firings and his words.
pub fn rules_listed(dir: Option<&Path>, out: File) {
    let r = (|| {
        let counts = match dir {
            Some(d) => asks::load(&crate::home::asks_log(d)?)?.counts,
            None => Default::default(),
        };
        let lines: Vec<String> = (rules().iter())
            .map(|r| {
                let n = counts.get(&r.id).copied().unwrap_or(0);
                format!("{}\t{}\t{n} fired\t{}", r.id, r.kind, r.said)
            })
            .collect();
        let usd = counts.get(SPENT).copied().unwrap_or(0) as f64 / 1e6;
        Ok(format!(
            "{}\nthe agent's check has cost ${usd:.4}",
            lines.join("\n")
        ))
    })();
    said(r, out)
}

inventory::submit! { crate::usage::Command::usage("dash inbox raise --data <dir> <id> --asker <who> --what <line> [--why <reason>] [--pick <pick>] [--link <url>] [--approval --sent-by <tool>] [--kind|--pr|--task|--draft|--supersedes|--not-before|--deadline <value>]") }
/// `dash inbox raise --data <dir> <id> --asker <who> --what <line> [--why <reason>] [--pick <pick>] [--link <url>]
/// [--approval --sent-by <tool>] [--kind <kind>] [--pr <n>] [--task <id>] [--draft <id>] [--supersedes <id>]
/// [--not-before <day>] [--deadline <day>]`: one ask raised, and the check's verdict on it at once, what it lacks named.
pub fn raise(dir: &Path, id: &str, flags: &[&str], out: File) {
    let r = (|| {
        let mut ask = Ask::raised(id, "", "", now_ms());
        let mut it = flags.iter();
        while let Some(flag) = it.next() {
            if *flag == "--approval" {
                ask.approval = true;
                continue;
            }
            let v = it
                .next()
                .ok_or(format!("{flag} takes a value"))?
                .to_string();
            match *flag {
                "--asker" => ask.asker = v,
                "--what" => ask.what = v,
                "--why" => ask.why = Some(v),
                "--pick" => ask.pick = v,
                "--link" => ask.link = v,
                "--sent-by" => ask.sent_by = v,
                "--kind" => ask.kind = v,
                "--pr" => ask.pr = Some(v.parse().map_err(|_| format!("--pr {v}: a number"))?),
                "--task" => ask.task = Some(v),
                "--draft" => ask.draft = Some(v),
                "--supersedes" => ask.supersedes = Some(v),
                "--not-before" | "--deadline" if !is_day(&v) => {
                    return Err(format!("{flag} {v}: a day, YYYY-MM-DD"));
                }
                "--not-before" => ask.not_before = Some(v),
                "--deadline" => ask.deadline = Some(v),
                f => return Err(format!("no flag {f}")),
            }
        }
        if ask.asker.is_empty() {
            return Err("an ask names its --asker".into());
        }
        let mut log = crate::home::asks_log(dir)?;
        let kept = asks::load(&log)?;
        if kept.asks.get(id).is_some_and(|a| a.state != State::Raised) {
            return Err(format!(
                "ask {id} is {} already",
                kept.asks[id].state.name()
            ));
        }
        let mut inbox = kept.clone();
        // raised again: its judgements kept while its words are the same
        if let Some(old) = kept.asks.get(id) {
            (ask.jev, ask.agent) = (old.jev.clone(), old.agent.clone());
        }
        // only this ask is checked and judged now, so a raise never waits on the others' calls (/code-review)
        let mut one = Inbox {
            asks: BTreeMap::from([(id.to_string(), ask)]),
            ..Inbox::default()
        };
        let facts = facts(&one, &serde_json::Value::Null, None, now_ms());
        let mut judge = Checkers::new();
        asks::pass(
            &mut one.asks,
            &rules(),
            &facts,
            &mut inbox.counts,
            &mut judge,
            now_ms(),
        );
        inbox.asks.extend(one.asks);
        asks::show(&mut inbox.asks, now_ms());
        spent(&mut inbox, &judge);
        asks::save(&mut log, &kept, &inbox)?;
        Ok(line(&inbox.asks[id]))
    })();
    said(r, out)
}

inventory::submit! { crate::usage::Command::usage("dash inbox answer --data <dir> <id> <words>") }
/// `dash inbox answer --data <dir> <id> <words>`: his answer to a shown ask, kept so it is never asked again.
pub fn answer(dir: &Path, id: &str, words: &str, out: File) {
    let r = (|| {
        let mut log = crate::home::asks_log(dir)?;
        let kept = asks::load(&log)?;
        let mut inbox = kept.clone();
        let ask = inbox.asks.get_mut(id).ok_or(format!("no ask {id}"))?;
        asks::answer(ask, words, now_ms())?;
        asks::show(&mut inbox.asks, now_ms());
        asks::save(&mut log, &kept, &inbox)?;
        Ok(line(&inbox.asks[id]))
    })();
    said(r, out)
}

inventory::submit! { crate::usage::Command::usage("dash inbox gate lift --data <dir> <gate> <his words>") }
/// `dash inbox gate lift --data <dir> <gate> <his words>`: a gate of his lifted (the CAPE contract signed, by his word until
/// the store has a field for it: rfc-inbox section 9); its asks go back to raised on the next pass.
pub fn lift(dir: &Path, gate: &str, words: &str, out: File) {
    let r = (|| {
        let mut log = crate::home::asks_log(dir)?;
        let kept = asks::load(&log)?;
        let mut inbox = kept.clone();
        inbox.lifted.insert(gate.into(), words.into());
        asks::save(&mut log, &kept, &inbox)?;
        Ok(format!(
            "gate {gate} lifted: its asks are checked again on the next pass"
        ))
    })();
    said(r, out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;
    use serde_json::json;

    /// Jev and the agent standing in: plain, only his.
    struct Agrees;
    impl asks::Judge for Agrees {
        fn jev(&mut self, _: &Ask) -> Result<(f64, f64), String> {
            Ok((0.9, 0.1))
        }
        fn agent(&mut self, _: &Ask, _: &[asks::Rule]) -> Result<(bool, String, String), String> {
            Err("not asked".into())
        }
    }

    /// Jev doubts, and the agent's check runs on a run's answer as `claude -p --output-format json` prints it: its verdict,
    /// read off its words by the code `Checkers::agent` runs, decides the ask in a whole pass, and its cost is counted.
    #[dash_test]
    fn the_agent_check_runs_in_a_pass_and_its_verdict_read_off_its_words_decides_the_ask() {
        struct Says(&'static str, Checkers, usize);
        impl asks::Judge for Says {
            fn jev(&mut self, _: &Ask) -> Result<(f64, f64), String> {
                Ok((0.9, 0.8))
            }
            fn agent(
                &mut self,
                _: &Ask,
                _: &[asks::Rule],
            ) -> Result<(bool, String, String), String> {
                self.2 += 1;
                self.1.read_agent(self.0)
            }
        }
        let ask = |id: &str| Ask {
            why: Some("wording".into()),
            pick: "approve".into(),
            link: format!("/drafts/{id}"),
            ..Ask::raised(id, "test", "Approve the new opening line to Naresh", 1)
        };
        let run = |out: &'static str| {
            let mut judge = Says(out, Checkers::new(), 0);
            let mut asks = BTreeMap::from([("d".to_string(), ask("d"))]);
            let mut counts = asks::Counts::new();
            asks::pass(
                &mut asks,
                &rules(),
                &Facts {
                    today: "2026-11-11".into(),
                    ..Facts::default()
                },
                &mut counts,
                &mut judge,
                2,
            );
            let d = asks.remove("d").unwrap();
            (
                d.state,
                d.rule,
                d.lacks,
                judge.2,
                judge.1.spent_micro,
                counts,
            )
        };
        // his, in prose around the JSON: shown, the rule it cites kept and counted, its cost counted
        let his = r#"{"type":"result","result":"Only he can. {\"his\": true, \"rule\": \"his-wording\", \"why\": \"new words in his voice\"}","total_cost_usd":0.0596}"#;
        let (state, rule, lacks, calls, spent, counts) = run(his);
        assert_eq!(
            (state, rule.as_deref(), lacks.len(), calls, spent),
            (State::Shown, Some("agent:his-wording"), 0, 1, 59600)
        );
        assert_eq!(counts.get("his-wording"), Some(&1));
        // not his: back to its asker with the rule and its reason, never dropped on a model's word
        let not = r#"[{"type":"system"},{"type":"result","result":"{\"his\": false, \"rule\": \"never-optional\", \"why\": \"nothing depends on it\"}","total_cost_usd":0.01}]"#;
        let (state, rule, lacks, ..) = run(not);
        assert_eq!(
            (state, rule.as_deref()),
            (State::Raised, Some("agent:never-optional"))
        );
        assert_eq!(
            lacks,
            ["not his, the agent's check finds (never-optional): nothing depends on it"]
        );
        // words with no verdict: never shown, the ask waits
        let (state, ..) =
            run(r#"{"type":"result","result":"I am not sure.","total_cost_usd":0.01}"#);
        assert_ne!(state, State::Shown);
    }

    /// The agent's answer is read off its words, a JSON object maybe wrapped in prose; its prompt names every rule it may
    /// cite; under nextest neither Jev nor the agent is paid.
    #[dash_test]
    fn the_agent_s_verdict_is_read_off_its_words_and_a_test_pays_nothing() {
        let said = "Here: {\"his\": false, \"rule\": \"never-optional\", \"why\": \"nothing depends on it\"}";
        assert_eq!(
            verdict(said),
            Ok((
                false,
                "never-optional".into(),
                "nothing depends on it".into()
            ))
        );
        assert!(verdict("no idea").is_err());
        assert!(verdict("{\"rule\": \"x\"}").is_err());
        let ask = Ask::raised("a", "t", "Sign in to the Datadog plugin", 1);
        let prompt = agent_prompt(&ask, &rules());
        assert!(prompt.contains("Ask: Sign in to the Datadog plugin"));
        assert!(
            prompt.contains("- never-optional (never): ")
                && prompt.contains("- his-wording (his): ")
        );
        assert!(
            !prompt.contains("held-cape"),
            "a held rule is never the agent's to cite"
        );
        use asks::Judge;
        assert!(
            Checkers::new()
                .jev(&ask)
                .unwrap_err()
                .contains("is set: a check never pays Jev or an agent there")
        );
        assert!(
            Checkers::new()
                .agent(&ask, &rules())
                .unwrap_err()
                .contains("is set: a check never pays Jev or an agent there")
        );
        // a pass's paid calls are bounded, and a failed key read is remembered for the pass, never read again (/code-review)
        let mut capped = Checkers {
            left: 0,
            ..Checkers::new()
        };
        assert!(capped.call().unwrap_err().contains("paid calls this pass"));
        let mut keyless = Checkers {
            key: Some("no TYPESAFE_API_KEY in bin/secret's typesafe".into()),
            ..Checkers::new()
        };
        assert!(
            keyless
                .call()
                .unwrap_err()
                .starts_with("no TYPESAFE_API_KEY")
        );
        // the judge's version is a short hash, never the questions' whole text kept on every ask
        assert_eq!(Checkers::new().version().len(), 16);
    }

    /// The older app's /inbox of 2026-11-11, cut: a draft with its sender, one with none, a Babel draft, a block on the
    /// review list, his Hostman step, a dated step and a warning.
    const HANDOFF: &str = "22 things wait on Nicolas. Page: http://100.72.158.115:7788/, first card. File: private/store/handoff.md
To read: Week 45, 2 November to 8 November. http://100.72.158.115:7788/insights#read

## His three steps (À toi) (1)
- Dis si Babel accepte le compte Hostman à son nom. [Le brouillon](http://100.72.158.115:7788/d/hostman-business-account-reply)

## Later (Plus tard) (1)
- Crée le Developer ID G2 de graal (Mac : .csr). [Le portail Apple](https://developer.apple.com/account/resources/certificates/list)  (from 2027-01-04)
WARNING: ready for him to send, on no step: mirakl-ai-intro (form, Mirakl (formulaire de candidature, poste Senior AI Engineer - Freelance))

## Drafts to review (3)
  Essayé : chaque brouillon est assemblé des blocs.
- tether-naresh-actu -> Naresh Ullal (Tether)
- caraz-laura-pistes -> Laura Pernot (Caraz)
- babel-samir-contrat-modele -> Samir Saidani (Babel Coop), samir.saidani@babel.coop

## Blocks to validate once (2)
- Qui signe pour Babel ?
- Quels assureur et montants je mets au contrat ?

## Comments waiting on Claude (1)
- /dashboard/tasks/rust-host-requests: « dash runs host request jobs itself »
";

    /// Done-when 5's page: of the older app's items, only a ready one is shown; the Babel ones held, the rest sent back
    /// naming what they lack, and the page's count is the shown asks alone.
    #[dash_test]
    fn the_older_app_s_inbox_comes_in_as_asks_and_only_ready_ones_are_shown() {
        let read = crate::home::cards(HANDOFF, crate::home::handoff_link);
        let drafts = json!([{"id": "tether-naresh-actu", "channel": "linkedin", "status": "pending"},
            {"id": "caraz-laura-pistes", "channel": "form", "where": "caraz.fr", "status": "pending"}]);
        let items = from_handoff(&read, &drafts, 1);
        let mut inbox = Inbox::default();
        let ids: BTreeSet<String> = items.iter().map(|a| a.id.clone()).collect();
        inbox
            .asks
            .extend(items.into_iter().map(|a| (a.id.clone(), a)));
        let facts = Facts {
            listed: Some(ids),
            today: "2026-11-11".into(),
            ..Facts::default()
        };
        asks::pass(
            &mut inbox.asks,
            &rules(),
            &facts,
            &mut inbox.counts,
            &mut Agrees,
            2,
        );
        fn state_of(inbox: &Inbox, what: &str) -> (State, Vec<String>) {
            let a = (inbox.asks.values())
                .find(|a| a.what.contains(what))
                .unwrap_or_else(|| panic!("{what}"));
            (a.state, a.lacks.clone())
        }
        let state = |what: &str| state_of(&inbox, what);
        assert_eq!(state("Naresh").0, State::Shown);
        // a form no tool sends goes back naming its sender
        let (s, lacks) = state("Laura");
        assert_eq!(s, State::Raised);
        assert!(
            lacks.iter().any(|l| l.starts_with("who sends it")),
            "{lacks:?}"
        );
        assert_eq!(state("Samir").0, State::Held);
        assert_eq!(state("Hostman").0, State::Held);
        assert_eq!(state("Developer ID").0, State::Held);
        assert_eq!(state("Qui signe").0, State::Held);
        assert_eq!(state("host request jobs").0, State::Dropped);
        // a block on the review list goes back: its link is a list page
        let (s, lacks) = state("assureur");
        assert_eq!(s, State::Raised);
        assert!(
            lacks
                .iter()
                .any(|l| l.contains("not the list page /review")),
            "{lacks:?}"
        );
        let drawn = drawn(&inbox);
        assert_eq!(
            drawn.lead,
            "1 wait on you, 4 held behind a gate of yours, 4 went back to their askers"
        );
        let card = &drawn.groups[0].cards[0];
        assert_eq!(card.title, "Approve the draft to Naresh Ullal (Tether)");
        assert_eq!(
            card.line,
            "his because: wording · pick: approve as written · sent by: the LinkedIn tool"
        );
        assert_eq!(card.href.as_deref(), Some("/drafts/tether-naresh-actu"));
        // the item leaves the older app's page: its ask is settled, dropped by its rule
        let gone = Facts {
            listed: Some(BTreeSet::new()),
            ..facts
        };
        asks::pass(
            &mut inbox.asks,
            &rules(),
            &gone,
            &mut inbox.counts,
            &mut Agrees,
            3,
        );
        assert_eq!(state_of(&inbox, "Naresh").0, State::Dropped);
        assert!(inbox.counts["settled-gone"] >= 1);
        // one short read dropped it; back on the page, it lands again (/code-review)
        let back = from_handoff(&read, &drafts, 4);
        for ask in back {
            refreshed(&mut inbox, ask);
        }
        assert_eq!(state_of(&inbox, "Naresh").0, State::Raised);
        // a raised ask's fields are read again: the sender a failed drafts read missed is found
        let missed = from_handoff(&read, &serde_json::Value::Null, 5);
        let mut fresh = Inbox::default();
        for ask in missed {
            refreshed(&mut fresh, ask);
        }
        for ask in from_handoff(&read, &drafts, 6) {
            refreshed(&mut fresh, ask);
        }
        let naresh = fresh
            .asks
            .values()
            .find(|a| a.what.contains("Naresh"))
            .expect("naresh");
        assert_eq!(naresh.sent_by, "the LinkedIn tool");
    }

    #[dash_test]
    fn an_item_s_id_and_its_day_are_read_off_its_words() {
        assert_eq!(from_day("(from 2027-01-04)").as_deref(), Some("2027-01-04"));
        assert_eq!(from_day("from tomorrow"), None);
        assert_eq!(
            from_day("Reply to the mail from Laura (from 2027-01-04)").as_deref(),
            Some("2027-01-04")
        );
        assert!(is_day("2027-01-04") && !is_day("2027-1-4") && !is_day("12/11/2026"));
        // two long items sharing their first 80 characters stay two asks
        let long = |line: &str| Card {
            title: "Samir Saidani (Babel Coop), samir.saidani@babel.coop".into(),
            line: line.into(),
            href: None,
        };
        let (a, b) = (
            item_id("Drafts", &long("babel-samir-contrat-modele")),
            item_id("Drafts", &long("babel-samir-contrat-modele-2")),
        );
        assert_ne!(a, b);
        assert!(a.len() <= 97, "{a}");
        let card = Card {
            title: "Laura Pernot (Caraz)".into(),
            line: "caraz-laura-pistes".into(),
            href: None,
        };
        assert_eq!(
            item_id("Drafts", &card),
            "handoff-drafts-laura-pernot-caraz-caraz-laura-pistes"
        );
    }
}
