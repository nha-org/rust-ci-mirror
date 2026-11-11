//! `dash task run --folded <at> mechanism|doc "<where>"`: one task lesson closed, a line of the task journal naming its kind and
//! where it now lives, answered by the front door before the older app (task rust-fold-port; @nha, 2026-11-10: « remember we
//! move to rust. no more elixir, so if something needs fixing port and then fix »). It retires `Dashboard.Orch.PM.fold/3` and the
//! part of `Dashboard.Orch.Folds` it read; `dash folds` stays the older app's until its own port, Rust adding each target's line.
//!
//! * A mechanism fold names what refuses the mistake again: a code path that exists, `dash <command>`, a module, a commit of the
//!   last 14 days that changed code, or a task (`task <id>`). A part that opens on a task's id, then a colon or words, names
//!   that task whatever its words say (« task rust-live-copy part 4: the brief lists… » read as a doc's sentence, 2026-11-10),
//!   and a done task counts: its merge is the mechanism (« task rust-port-parity: … » was refused once merged). A dropped one
//!   builds nothing.
//! * A doc fold is refused when a watcher's check could catch the lesson, and when an earlier lesson of another task, or one of
//!   his comments, is on the same subject: a lesson told twice is no longer one a sentence holds.
//! * A repeated lesson's mechanism fold raises the open site_perso task it names to priority 1, its lesson turns summed. Its
//!   epic is kept: the older app moved it to « speed », off its roadmap item, so the picker ranked it with nothing it serves.
//! * A fold whose task Rust's picker cannot start is refused, whatever its kind: 238 of 690 folds in 7 days went into 68 such
//!   tasks, and the fix never ran (task rust-fold-targets-live; @nha, 2026-11-11: « Think about the system and what would
//!   improve it for the future. Spike, measure. »). A done task stays live: its merge holds the lesson. `dash folds` ends with
//!   each target of the week's folds, live or dead, and its fold count.
use dash_store::older::Foldable;
use regex::Regex;
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

const KINDS: [&str; 2] = ["mechanism", "doc"];
/// A doc file never counts as a mechanism, even tracked.
const DOCS: [&str; 3] = ["md", "html", "txt"];
const USAGE: &str = "usage: private/dashboard/bin/dash task run --folded <at> mechanism|doc \"<where>\" (a mechanism names \
                     the code path, hook, test or command, or task <id>)";
/// What a commit names counts while this recent.
const COMMIT_DAYS: i64 = 14;

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("a fold's pattern")
}

// Any extension but a doc's (checked after the match: Rust's regex has no lookahead), lower case so a module is no path; the file
// must exist, so a word like example.com names nothing.
static CODE_PATH: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"(?:~/|\.{0,2}/)?[\w.-]+(?:/[\w.-]+)*\.[a-z][a-z0-9]*\b|(?:~/|\.{0,2}/)?(?:[\w.-]+/)*(?:bin|hooks|\.githooks|commands)/[\w.-]+",
    )
});
static MODULE: LazyLock<Regex> = LazyLock::new(|| re(r"\b(?:[A-Z]\w*\.)+[A-Z]\w*\b"));
static COMMAND: LazyLock<Regex> = LazyLock::new(|| re(r"\bdash ([a-z][\w-]*(?: [a-z-][\w-]*)*)"));
static TASK: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)\btasks? ([a-z0-9][a-z0-9-]*[a-z0-9])\b"));
/// A part that opens on a task's id, then a colon, words or nothing.
static LEADS: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?i)^\s*tasks? ([a-z0-9][a-z0-9-]*[a-z0-9])(?::|\s|$)"));
static DOC_MARKER: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"\.md\b|\bskills?\b|\bbrief\b|CLAUDE|\brules?\s+\d|\bquirks?\s+\d|\bpoint\s+\d|\bstanding\b|Orch\.Launch\b",
    )
});
/// A sha holds a letter and a digit (checked after the match): a date or a PR number is none.
static SHA: LazyLock<Regex> = LazyLock::new(|| re(r"\b[0-9a-f]{7,40}\b"));
/// The folder itself or a file in it, under .claude or leading, never the word in a sentence.
static MEMORY: LazyLock<Regex> =
    LazyLock::new(|| re(r"(^|\.claude[/\\](.*[/\\])?)memory([/\\]|$)|MEMORY\.md"));
static SPLIT: LazyLock<Regex> = LazyLock::new(|| re(r";|,\s+"));
static AND_TASK: LazyLock<Regex> = LazyLock::new(|| re(r"\s+and\s+(tasks?\s)"));
static WORD: LazyLock<Regex> = LazyLock::new(|| re(r"[a-z][a-z0-9_-]*[a-z0-9]"));
static SENTENCE_END: LazyLock<Regex> = LazyLock::new(|| re(r"[.;:!?]\s+"));
static UNWAKE: LazyLock<Regex> =
    LazyLock::new(|| re(r#"(?s)Lesson from task .*?(?:--folded \S+[^"\n]*"<where>"[^.]*\.|\z)"#));
static TURNS: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"(?i)\b(?:cost|costs|took|lost|wasted|spent)(?:\s+[a-z']+){0,2}\s+(\d+)\s+turns?\b|\b(\d+)\s+(?:more|extra|wasted)\s+turns?\b",
    )
});
/// A machine's load is two digits (« load hit 135 »), never a page's (« page load was 3 s »): the « page » before it is checked
/// apart, Rust's regex having no lookbehind.
static LOAD_HIT: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?i)\bload (?:average |avg )?(?:hit|over|was|went to|reached) \d{2,}\b"));
static LOAD: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"(?i)\bload average \d|\bloadavg\b|\bfree memory\b|memory pressure|(?-i:\bRAM\b)|\b\d+ ?GB (?:of )?(?:RAM|memory)\b|\bholds? (?:almost |nearly |about )?\d+ ?GB\b|\b\d+ (?:browser|chrome) tabs\b|too many (?:browser )?tabs|\btabs (?:are |were |left )?open|headless (?:chrome|chromium|browsers?)|\bidle (?:panes?|sessions?)|leftover (?:worktree )?(?:dashboards?|apps?)|\b(?:mac|machine|host|cpu|memory) (?:is |was )?overload|\boverload(?:ed)? (?:the )?(?:mac|machine|host)\b",
    )
});
static PANES: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"(?i)\bpanes? (?:stuck|unmoved|did not move|has not moved|never moved)|\bunmoved (?:for )?\d+ ?h",
    )
});
// ponytail: word overlap weighted by rarity, sentence against sentence, the older app's calibration on 2026-10-22's 81 lessons;
// an embedding model is the upgrade if it misses a paraphrase.
const MIN_SCORE: f64 = 0.4;
const MIN_SHARED: usize = 4;
/// English words that carry no subject.
const STOP: [&str; 58] = [
    "about", "after", "again", "also", "been", "before", "being", "both", "could", "does", "done",
    "each", "from", "have", "here", "into", "just", "like", "made", "make", "more", "most", "much",
    "must", "only", "other", "over", "same", "should", "some", "such", "than", "that", "their",
    "them", "then", "there", "these", "they", "this", "those", "through", "very", "were", "what",
    "when", "where", "which", "while", "will", "with", "would", "your", "yours", "ways", "part",
    "read", "first",
];

/// What a fold reads beyond the journal, and its one write besides it: the live one is `Live`, a test's a stand-in.
pub trait Look {
    /// Every task, every project's.
    fn tasks(&self) -> &[Foldable];
    /// His feedback before `before` (local time): its time, who said it, its words.
    fn comments(&self, before: &str) -> Vec<Lesson>;
    /// Every command `dash` answers, its literal words, and why some could not be read (the older app not answering).
    fn labels(&self) -> (Vec<String>, Option<String>);
    /// Whether code path `p` exists: a file, or the one tracked file its tail names.
    fn path(&self, p: &str) -> bool;
    /// Whether module `m` is defined.
    fn module(&self, m: &str) -> bool;
    /// Whether `sha` is a commit of the last 14 days that changed a file other than a doc.
    fn commit(&self, sha: &str) -> bool;
    /// Task `id` raised to priority 1, its lesson turns now `turns`; its epic, and so its roadmap item, as it was.
    fn raise(&self, id: &str, turns: i64) -> Result<(), String>;
}

/// A lesson or a comment of his: when, who (« task <id> », « his comment <ref> »), its words.
#[derive(Clone, Debug, PartialEq)]
pub struct Lesson {
    pub at: String,
    pub who: String,
    pub text: String,
}

/// An earlier lesson on the same subject: when, who, the sentence that matched, the words both share, the rarest first.
#[derive(Clone, Debug, PartialEq)]
pub struct Match {
    pub at: String,
    pub who: String,
    pub text: String,
    pub shared: Vec<String>,
}

fn describe(m: &Match) -> String {
    format!(
        "{} {}: « {} » (shared: {})",
        m.at,
        m.who,
        m.text.trim(),
        m.shared.join(", ")
    )
}

fn open(t: &Foldable) -> bool {
    !["done", "dropped", "closed", "archived"].contains(&t.stage.as_str())
}

/// Why Rust's picker cannot start `t`, or none: a done task's merge holds its lesson, a queued one will run, and a task merged
/// into another (`:duplicate-of`) is that one's, as its lessons now are.
pub fn dead(t: &Foldable, tasks: &[Foldable]) -> Option<String> {
    let end = merged_into(t, tasks);
    let why = held(end)?;
    Some(match end.id == t.id {
        true => why,
        false => format!("merged into {}, where {why}", end.id),
    })
}

/// The task `t`'s merges end on: itself when it was merged into none.
fn merged_into<'a>(t: &'a Foldable, tasks: &'a [Foldable]) -> &'a Foldable {
    let mut end = t;
    // ponytail: a merge chain read 8 deep at most, so a cycle ends
    for _ in 0..8 {
        let into = end
            .duplicate_of
            .as_deref()
            .filter(|_| end.stage == "dropped");
        match into.and_then(|id| tasks.iter().find(|x| x.id == id)) {
            Some(next) => end = next,
            None => break,
        }
    }
    end
}

/// The update that brings dead task `t` into Rust's queue, a flag for each thing that holds it, or none for a dropped task,
/// which builds nothing (/code-review: the one advice « --owner rust » left a backlog, put-off or dropped task dead).
fn revive(t: &Foldable) -> Option<String> {
    if ["dropped", "closed", "archived"].contains(&t.stage.as_str()) {
        return None;
    }
    let mut flags = vec![];
    if t.owner != "rust" {
        flags.push("--owner rust --layer <layer>");
    }
    if t.stage != "open" {
        flags.push("--status open");
    }
    flags.push("--later nil");
    let label = (t.owner == "rust" && t.stage == "open").then_some(", its Unplanned label off");
    Some(format!(
        "dash task update {} {}{}",
        t.id,
        flags.join(" "),
        label.unwrap_or_default()
    ))
}

fn held(t: &Foldable) -> Option<String> {
    if t.stage == "done" || t.queued {
        return None;
    }
    Some(match (t.stage.as_str(), t.owner.as_str()) {
        ("dropped" | "closed" | "archived", _) => format!("it is {}", t.stage),
        (_, "") => "it has no owner, so no picker starts it".into(),
        (_, "rust") if t.stage != "open" => format!("its stage is {}, not open", t.stage),
        (_, "rust") => "it is labelled Unplanned or set for later".into(),
        (_, owner) => format!("its owner is {owner}, not rust"),
    })
}

/// The task a fold into `place` names, any stage, as a raise reads it: the target `dash folds` counts.
fn target<'a>(place: &str, tasks: &'a [Foldable]) -> Option<&'a Foldable> {
    let ids: BTreeSet<&str> = tasks.iter().map(|t| t.id.as_str()).collect();
    let id = task_id(place, &ids)?;
    tasks.iter().find(|t| t.id == id)
}

fn site_perso(t: &Foldable) -> bool {
    t.project.is_empty() || t.project == "site_perso"
}

// ---------------------------------------------------------------- the command

/// The front door's answer to `task run --folded <rest>`: exit code, what it printed, its error. The older app is never asked.
/// The journal is opened and locked before it is read: a raise is never made on a journal that cannot take its line (the next
/// fold would count its turns again), and two folds of one lesson at once never both pass (/code-review of rust-fold-port).
pub fn answer(rest: &[String], own_usage: &str) -> (i32, String, String) {
    let [at, kind, words @ ..] = rest else {
        return (2, String::new(), format!("{USAGE}\n"));
    };
    if words.is_empty() {
        return (2, String::new(), format!("{USAGE}\n"));
    }
    let place = words.join(" ");
    let folded = plain(kind, &place).and_then(|()| {
        let journal = journal();
        let mut file = Locked::open(&journal)?;
        let lines = file.lines()?;
        let live = Live::new(own_usage)?;
        let (line, said) = fold(&lines, at, kind, &place, &live, &now())?;
        file.append(&line)?;
        Ok(said)
    });
    match folded {
        Ok(said) => {
            let said = said.map(|s| format!(": {s}")).unwrap_or_default();
            (
                0,
                format!("pm: folded {at} as {kind} into {place}{said}\n"),
                String::new(),
            )
        }
        Err(why) => (1, String::new(), format!("pm: {why}\n")),
    }
}

/// The task journal, the live folder's: the older app's `PM.journal_file/0`.
fn journal() -> PathBuf {
    crate::tasks::repo().join("private/live/task-journal.jsonl")
}

/// The journal open for appending, its lock held until dropped. Goal-loops' script appends without it: only folds wait on
/// each other.
struct Locked {
    file: std::fs::File,
    path: PathBuf,
}

impl Locked {
    fn open(path: &Path) -> Result<Locked, String> {
        use std::os::fd::AsRawFd;
        let named = |e: std::io::Error| format!("{}: {e}", path.display());
        let file = (std::fs::OpenOptions::new())
            .create(true)
            .read(true)
            .append(true)
            .open(path)
            .map_err(named)?;
        // SAFETY: flock only reads the descriptor, which `file` keeps open; the lock goes with it.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } != 0 {
            return Err(named(std::io::Error::last_os_error()));
        }
        Ok(Locked {
            file,
            path: path.into(),
        })
    }

    /// Its lines, each a JSON object; a line that reads as none is skipped, as the older app skipped it.
    fn lines(&mut self) -> Result<Vec<Value>, String> {
        use std::io::Read;
        let mut text = String::new();
        (self.file.read_to_string(&mut text))
            .map_err(|e| format!("{}: {e}", self.path.display()))?;
        Ok((text.lines())
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .filter(Value::is_object)
            .collect())
    }

    /// One line appended whole, as goal-loops' script appends: the older app reads the file again at its start and after
    /// goal-loops.
    fn append(&mut self, line: &Value) -> Result<(), String> {
        (self.file.write_all(format!("{line}\n").as_bytes()))
            .map_err(|e| format!("{}: {e}", self.path.display()))
    }
}

/// Now in UTC with its microseconds, as the older app's `DateTime.to_iso8601/1` wrote a fold's time.
fn now() -> String {
    iso(0)
}

/// `ago` seconds before now in UTC, a fold's time's form: a journal time compares with it as text.
fn iso(ago: u64) -> String {
    let since = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .saturating_sub(std::time::Duration::from_secs(ago));
    let secs = since.as_secs() as i64;
    let (days, s) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    format!(
        "{}T{:02}:{:02}:{:02}.{:06}Z",
        dash_core::date(days),
        s / 3600,
        s / 60 % 60,
        s % 60,
        since.subsec_micros()
    )
}

/// A journal time (UTC, « …Z ») as this Mac's local time, « 2026-10-22T09:00:00 »: the feedback table's and his day.
fn local(at: &str) -> String {
    let parsed = (|| {
        let days = dash_core::days(at.get(..10)?)?;
        let clock = at.get(11..19)?;
        let mut hms = clock.split(':').map(|n| n.parse::<i64>().ok());
        let (h, m, s) = (hms.next()??, hms.next()??, hms.next()??);
        (at.as_bytes().get(10) == Some(&b'T')).then_some(days * 86_400 + h * 3600 + m * 60 + s)
    })();
    parsed.map_or_else(|| at.to_string(), crate::pages::local_at)
}

// ---------------------------------------------------------------- dash folds

inventory::submit! { crate::usage::Command::usage("dash folds") }
/// `dash folds`' last part, from the live journal and the older app's list: each target of the last 7 days' folds.
pub fn targets_now() -> Result<String, String> {
    let path = journal();
    let text = match std::fs::read_to_string(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        read => read.map_err(|e| format!("{}: {e}", path.display()))?,
    };
    let lines: Vec<Value> = (text.lines())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    let tasks = dash_store::older::foldable(&crate::tasks::older_db())?;
    Ok(targets(&lines, &tasks, &iso(7 * 86_400)))
}

/// Each task the folds since `since` name (a journal time), live or dead with why, and its fold count: the dead first, then
/// the most folded. A fold that names no task is counted apart.
pub fn targets(lines: &[Value], tasks: &[Foldable], since: &str) -> String {
    let ids: BTreeSet<&str> = tasks.iter().map(|t| t.id.as_str()).collect();
    let by_id: BTreeMap<&str, &Foldable> = tasks.iter().map(|t| (t.id.as_str(), t)).collect();
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut none = 0;
    let recent = (lines.iter())
        .filter(|l| l["kind"] == "folded" && l["at"].as_str().is_some_and(|at| at >= since));
    for l in recent {
        match task_id(l["into"].as_str().unwrap_or_default(), &ids) {
            Some(id) => *counts.entry(id).or_default() += 1,
            None => none += 1,
        }
    }
    let mut rows: Vec<(Option<String>, usize, String)> = (counts.into_iter())
        .map(|(id, n)| (by_id.get(id.as_str()).and_then(|t| dead(t, tasks)), n, id))
        .collect();
    rows.sort_by(|a, b| (b.0.is_some(), b.1, &a.2).cmp(&(a.0.is_some(), a.1, &b.2)));
    let dead_n = rows.iter().filter(|r| r.0.is_some()).count();
    let mut out = format!(
        "fold targets since {}: {} live, {dead_n} dead; {none} folds named no task\n",
        since.get(..16).unwrap_or(since),
        rows.len() - dead_n
    );
    for (why, n, id) in rows {
        let (state, why) = why.map_or(("live", String::new()), |w| ("dead", format!(": {w}")));
        out += &format!("  {state} {n} {id}{why}\n");
    }
    out
}

// ---------------------------------------------------------------- the fold

/// Lesson `at` closed as `kind` into `place`: the journal line to append and what to say after « folded », or why not. A raise,
/// the one write besides the line, is made here, before the line, as the older app made it.
pub fn fold(
    lines: &[Value],
    at: &str,
    kind: &str,
    place: &str,
    look: &dyn Look,
    now: &str,
) -> Result<(Value, Option<String>), String> {
    plain(kind, place)?;
    if !open_feedback(lines).any(|l| l["at"].as_str() == Some(at)) {
        return Err(format!("no open feedback at {at}"));
    }
    if let Some(t) = target(place, look.tasks())
        && let Some(why) = dead(t, look.tasks())
    {
        let or = revive(merged_into(t, look.tasks()))
            .map(|update| format!(", or bring it into Rust's queue first ({update})"))
            .unwrap_or_default();
        return Err(format!(
            "task {}: Rust's picker cannot start it ({why}), so this lesson's fix would never run: fold it into a task Rust \
             starts{or}",
            t.id
        ));
    }
    let comments = |before: &str| look.comments(before);
    if kind == "mechanism" {
        mechanism(place, look)?;
        let repeat = repeat(lines, at, &comments);
        return Ok(folded(at, kind, place, repeat, look, now));
    }
    let text = lessons(lines)
        .into_iter()
        .find(|l| l.at == at)
        .map(|l| l.text)
        .unwrap_or_default();
    if let Some(w) = watcher(&text) {
        return Err(format!(
            "a doc fold of {at} is an orchestration lesson {w} could catch: fold it as mechanism into a check of {w} (or the \
             task that adds one, task <id>), a sentence only if no watcher can check it (@nha, 2026-10-23: « These lessons \
             about load should all go in (1) our automatic check/suggestion watcher »)"
        ));
    }
    if let Some(m) = matched(lines, at, &comments) {
        return Err(format!(
            "a doc fold of {at} repeats an earlier lesson: {}. A sentence did not hold it: fold it as mechanism, naming the \
             code path, hook, test or command that refuses it, or the task that builds one (task <id>)",
            describe(&m)
        ));
    }
    Ok(folded(at, kind, place, None, look, now))
}

/// The refusals that read nothing: a kind, an empty where, a memory file.
fn plain(kind: &str, place: &str) -> Result<(), String> {
    if !KINDS.contains(&kind) {
        return Err(format!(
            "{kind}: a fold is mechanism or doc: dash task run --folded <at> mechanism|doc \"<where>\" (the kind comes before \
             where since task fold-is-mechanism)"
        ));
    }
    // « "" » or blanks folded a lesson into nothing (task dash-equal-user)
    if place.trim().is_empty() {
        return Err(format!(
            "a fold names where its lesson went: dash task run --folded <at> {kind} \"<where>\""
        ));
    }
    // @nha, 2026-10-16: « Do not create memories without being prompted. 90% of the new ones are bad »
    if MEMORY.is_match(place) {
        return Err(format!(
            "{place}: a lesson never goes in a memory file; fold it into the mechanism that prevents it"
        ));
    }
    Ok(())
}

/// The entries that carry feedback no folded line has closed yet, oldest first.
fn open_feedback(lines: &[Value]) -> impl Iterator<Item = &Value> {
    let folded: BTreeSet<&str> = (lines.iter())
        .filter(|l| l["kind"] == "folded")
        .filter_map(|l| l["of"].as_str())
        .collect();
    let said = |v: &Value| !v.is_null() && *v != Value::Bool(false);
    lines.iter().filter(move |l| {
        said(&l["feedback"])
            && l["kind"].is_null()
            && !l["at"].as_str().is_some_and(|at| folded.contains(at))
    })
}

struct Repeat {
    of: String,
    who: String,
    /// The lessons of its chain no raise counted yet, this one included.
    pending: Vec<String>,
    turns: i64,
}

fn folded(
    at: &str,
    kind: &str,
    place: &str,
    repeat: Option<Repeat>,
    look: &dyn Look,
    now: &str,
) -> (Value, Option<String>) {
    let (mut line, said) = match repeat {
        Some(r) => escalate(&r, place, look),
        None => (Map::new(), None),
    };
    for (k, v) in [
        ("kind", "folded"),
        ("of", at),
        ("as", kind),
        ("into", place),
        ("at", now),
    ] {
        line.insert(k.into(), json!(v));
    }
    (Value::Object(line), said)
}

// A repeated lesson's fold carries the earlier one and the turns it adds, and raises the open site_perso task it names to
// priority 1, its lesson turns summed; a raise that fails counts nothing, so the next fold adds them (task lesson-cost-escalates).
// ponytail: the turns read then written whole; two folds of one task at once lose one sum (folds run from one orchestrator)
fn escalate(r: &Repeat, place: &str, look: &dyn Look) -> (Map<String, Value>, Option<String>) {
    let said = format!("repeats {} {}, {} turns", r.of, r.who, r.turns);
    let mut line = Map::new();
    line.insert("repeats".into(), json!(r.of));
    line.insert("turns".into(), json!(r.turns));
    let targets: Vec<&Foldable> = (look.tasks().iter())
        .filter(|t| open(t) && site_perso(t))
        .collect();
    let ids: BTreeSet<&str> = targets.iter().map(|t| t.id.as_str()).collect();
    let Some(id) = task_id(place, &ids) else {
        return (
            line,
            Some(format!("{said}, no open site_perso task named to raise")),
        );
    };
    let t = targets.iter().find(|t| t.id == id).expect("a target");
    let turns = t.lesson_turns + r.turns;
    match look.raise(&id, turns) {
        Ok(()) => {
            line.insert("raised".into(), json!(id));
            line.insert("counted".into(), json!(r.pending));
            let epic = t.epic.as_deref().unwrap_or("none");
            let said = format!(
                "{said}: task {id} raised to priority 1, {turns} lesson turns, its epic {epic} and roadmap item kept"
            );
            (line, Some(said))
        }
        Err(why) => (line, Some(format!("{said}: task {id} not raised: {why}"))),
    }
}

// ---------------------------------------------------------------- the kind

/// What a mechanism fold names, checked: the thing, or why `place` names nothing that exists. Each part of `place` (split on
/// `;`, a comma, an « and task ») that names a doc (a .md file, a skill, a rule, a quirk, the standing text) is doc whatever
/// code it mentions, unless it opens on a task's id.
pub fn mechanism(place: &str, look: &dyn Look) -> Result<String, String> {
    let all: BTreeSet<&str> = look.tasks().iter().map(|t| t.id.as_str()).collect();
    // a dropped task builds nothing; a done one is built: its merge is the mechanism
    let named: BTreeSet<&str> = (look.tasks().iter())
        .filter(|t| t.stage != "dropped")
        .map(|t| t.id.as_str())
        .collect();
    let parts = parts(place, &all, &named);
    let labels = std::cell::OnceCell::new();
    let found = (parts.iter())
        .find_map(|p| {
            code_path(p, look)
                // the labels are the older app's to tell: asked only of a part naming « dash <command> »
                .or_else(|| {
                    let named = COMMAND.is_match(p);
                    named.then(|| command(p, &labels.get_or_init(|| look.labels()).0))?
                })
                .or_else(|| module(p, look))
                .or_else(|| commit(p, look))
        })
        .or_else(|| parts.iter().find_map(|p| task(p, &named)));
    // a refusal says when the older app's commands went unread: the fold may name one, so try again (/code-review)
    let unread = (labels.get().and_then(|l| l.1.as_ref()))
        .map(|e| format!(" (the older app's commands were not read, try again: {e})"))
        .unwrap_or_default();
    found.ok_or_else(|| {
        format!(
            "{place}: names no code path, hook, test or command that exists, no recent commit that changed code, and no task \
             (task <id>) outside a doc's sentence: a mechanism fold names the thing that refuses the mistake again{unread}"
        )
    })
}

/// `place`'s parts that name no doc, `ids` the tasks whose ids are no doc words, `leading` those that may open a part (a dropped
/// task builds nothing, so its doc sentence stays one); `task_id` reads the same, so a task named in a doc's sentence is never
/// raised.
fn parts(place: &str, ids: &BTreeSet<&str>, leading: &BTreeSet<&str>) -> Vec<String> {
    let place = AND_TASK.replace_all(place, ";$1");
    (SPLIT.split(&place))
        .filter(|p| leads(p, leading) || !DOC_MARKER.is_match(&doc_words(p, ids)))
        .map(String::from)
        .collect()
}

/// Whether `part` opens on a task's id, then a colon or words: what follows says what it builds, a doc's words or not.
fn leads(part: &str, ids: &BTreeSet<&str>) -> bool {
    LEADS
        .captures(part)
        .is_some_and(|c| ids.contains(lower(&c[1]).as_str()))
}

/// A task id as the list keeps it: « Task Rust-Live-Copy » names rust-live-copy, as TASK matches any case.
fn lower(id: &str) -> String {
    id.to_ascii_lowercase()
}

/// A part without its code paths and its task ids: a code path's own words (.claude/skills/x/check.py) and a task id's (« task
/// review-brief-repo ») are no doc marker. A doc file's path stays, its « .md » the marker.
fn doc_words(part: &str, ids: &BTreeSet<&str>) -> String {
    let code = CODE_PATH.replace_all(part, |c: &regex::Captures| match doc_file(&c[0]) {
        true => c[0].to_string(),
        false => String::new(),
    });
    TASK.replace_all(&code, |c: &regex::Captures| {
        match ids.contains(lower(&c[1]).as_str()) {
            true => "task".to_string(),
            false => c[0].to_string(),
        }
    })
    .into_owned()
}

fn extension(p: &str) -> &str {
    let base = p.rsplit('/').next().unwrap_or(p);
    match base.rfind('.') {
        Some(i) if i > 0 => &base[i + 1..],
        _ => "",
    }
}

fn doc_file(p: &str) -> bool {
    DOCS.contains(&extension(p))
}

fn code_path(part: &str, look: &dyn Look) -> Option<String> {
    (CODE_PATH.find_iter(part))
        .map(|m| m.as_str())
        // the dash launcher runs every command: the command named decides
        .filter(|p| !doc_file(p) && p.rsplit('/').next() != Some("dash"))
        .find(|p| look.path(p))
        .map(String::from)
}

/// A whole label (`dash task run`), never its first word alone: `dash task <anything>` names no command.
fn command(part: &str, labels: &[String]) -> Option<String> {
    COMMAND.captures_iter(part).find_map(|c| {
        let rest = &c[1];
        (labels.iter())
            .find(|l| rest == l.as_str() || rest.starts_with(&format!("{l} ")))
            .map(|l| format!("dash {l}"))
    })
}

fn module(part: &str, look: &dyn Look) -> Option<String> {
    (MODULE.find_iter(part))
        .map(|m| m.as_str())
        .filter(|m| m.starts_with("Dashboard"))
        .find(|m| look.module(m))
        .map(String::from)
}

fn shas(part: &str) -> impl Iterator<Item = &str> {
    (SHA.find_iter(part)).map(|m| m.as_str()).filter(|s| {
        s.bytes().any(|b| b.is_ascii_digit()) && s.bytes().any(|b| b.is_ascii_lowercase())
    })
}

fn commit(part: &str, look: &dyn Look) -> Option<String> {
    shas(part)
        .find(|s| look.commit(s))
        .map(|s| format!("commit {s}"))
}

fn task(part: &str, ids: &BTreeSet<&str>) -> Option<String> {
    (TASK.captures_iter(part))
        .find(|c| ids.contains(lower(&c[1]).as_str()))
        .map(|c| c[0].to_string())
}

/// The task `place` names among `ids`, whatever it names first, or none.
fn task_id(place: &str, ids: &BTreeSet<&str>) -> Option<String> {
    let found = parts(place, ids, ids).iter().find_map(|p| task(p, ids))?;
    TASK.captures(&found).map(|c| c[1].to_ascii_lowercase())
}

/// Literal words of each label: `task run [flags]` is `task run`, `serve|mcp --data` both `serve` and `mcp`. `text` holds one
/// command a line, as `dash help all` and the binary's usage print them; a label ends at two blanks, where its line's
/// description starts.
pub fn labels_of(text: &str) -> Vec<String> {
    let word = |w: &str| {
        let mut c = w.chars();
        c.next().is_some_and(|f| f.is_ascii_lowercase())
            && c.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    };
    let mut out = BTreeSet::new();
    for line in text.lines() {
        let line = line.trim_start_matches("usage:").trim_start();
        let line = line.strip_prefix("dash ").unwrap_or(line);
        let head = line.split("  ").next().unwrap_or_default();
        let mut each: Vec<String> = vec![String::new()];
        for w in head.split_whitespace() {
            let alts: Vec<&str> = w.split('|').collect();
            if !alts.iter().all(|a| word(a)) {
                break;
            }
            each = (each.iter())
                .flat_map(|e| {
                    alts.iter()
                        .map(move |a| format!("{e} {a}").trim().to_string())
                })
                .collect();
        }
        out.extend(each.into_iter().filter(|e| !e.is_empty()));
    }
    out.into_iter().collect()
}

// ---------------------------------------------------------------- a watcher's check

/// The watcher whose check could catch the lesson `text`, or none: its doc fold is refused (@nha, 2026-10-23: « These lessons
/// about load should all go in (1) our automatic check/suggestion watcher for you and (2) the skill only if (1) is not
/// possible »).
fn watcher(text: &str) -> Option<&'static str> {
    let page = |at: usize| text[..at].to_lowercase().ends_with("page ");
    let load = LOAD_HIT.find_iter(text).any(|m| !page(m.start())) || LOAD.is_match(text);
    if load {
        return Some("Dashboard.Watch.Load");
    }
    PANES.is_match(text).then_some("Dashboard.Watch.Panes")
}

// ---------------------------------------------------------------- the subject

fn lessons(lines: &[Value]) -> Vec<Lesson> {
    (lines.iter())
        .filter(|l| l["kind"].is_null())
        .filter_map(|l| {
            Some(Lesson {
                at: l["at"].as_str()?.into(),
                who: format!("task {}", l["task"].as_str()?),
                text: l["feedback"].as_str()?.into(),
            })
        })
        .collect()
}

fn base(who: &str) -> &str {
    let who = who.strip_prefix("task ").unwrap_or(who);
    who.strip_prefix("review-").unwrap_or(who)
}

/// A wake's lesson runs to its own `--folded <at> ... "<where>".`, or to the end of a cut paste: his words are the rest.
pub fn unwake(text: &str) -> String {
    UNWAKE.replace_all(text, "").into_owned()
}

/// The closest earlier lesson of another task, or comment of his, on the same subject as the lesson at `at`, or none. Earlier
/// means before the lesson's own time; the same task's own lessons (its resends, its review's) are no repeat.
fn matched(lines: &[Value], at: &str, comments: &dyn Fn(&str) -> Vec<Lesson>) -> Option<Match> {
    let all = lessons(lines);
    let lesson = all.iter().find(|l| l.at == at)?;
    let own = base(&lesson.who);
    let mut earlier: Vec<Lesson> = (all.iter())
        .filter(|l| l.at.as_str() < at && base(&l.who) != own)
        .cloned()
        .collect();
    earlier.extend(comments(&local(at)));
    closest(lesson, &earlier)
}

fn words(text: &str) -> BTreeSet<String> {
    let lower = text.to_lowercase();
    (WORD.find_iter(&lower))
        .map(|m| m.as_str())
        .filter(|w| w.chars().count() > 3 && !STOP.contains(w))
        .map(String::from)
        .collect()
}

/// Each sentence and its words; one with fewer than three says nothing to match.
fn sentences(text: &str) -> Vec<(&str, BTreeSet<String>)> {
    let mut out = vec![];
    let mut start = 0;
    let ends = SENTENCE_END
        .find_iter(text)
        .map(|m| (m.start() + 1, m.end()));
    for (cut, next) in ends.chain([(text.len(), text.len())]) {
        let s = &text[start..cut];
        start = next;
        let ws = words(s);
        if ws.len() >= 3 {
            out.push((s, ws));
        }
    }
    out
}

/// Each word's weight, squared: its rarity over the texts, smoothed so a word every text holds still weighs a little; none for a
/// word one text alone holds.
fn weights(texts: &[&str]) -> BTreeMap<String, f64> {
    let mut df: BTreeMap<String, usize> = BTreeMap::new();
    for t in texts {
        for w in words(t) {
            *df.entry(w).or_default() += 1;
        }
    }
    let n = texts.len() as f64;
    (df.into_iter())
        .filter(|(_, d)| *d > 1)
        .map(|(w, d)| (w, (1.0 + n / d as f64).ln().powi(2)))
        .collect()
}

fn closest(lesson: &Lesson, earlier: &[Lesson]) -> Option<Match> {
    let texts: Vec<&str> = std::iter::once(lesson)
        .chain(earlier)
        .map(|l| l.text.as_str())
        .collect();
    let w2 = weights(&texts);
    let sum = |set: &BTreeSet<String>| set.iter().map(|w| w2[w]).sum::<f64>();
    let weigh = |text: &'_ str| -> Vec<(String, BTreeSet<String>, f64)> {
        (sentences(text).into_iter())
            .map(|(s, ws)| {
                let ws: BTreeSet<String> = ws.into_iter().filter(|w| w2.contains_key(w)).collect();
                let total = sum(&ws);
                (s.to_string(), ws, total)
            })
            .collect()
    };
    let mine = weigh(&lesson.text);
    let mut best: Option<(f64, BTreeSet<String>, &Lesson, String)> = None;
    for e in earlier {
        for (text, b, nb) in weigh(&e.text) {
            for (_, a, na) in &mine {
                let shared: BTreeSet<String> = a.intersection(&b).cloned().collect();
                if shared.len() < MIN_SHARED {
                    continue;
                }
                let score = sum(&shared) / (na * nb).sqrt();
                // the first of the best, as the older app's max_by
                if score >= MIN_SCORE && best.as_ref().is_none_or(|b| score > b.0) {
                    best = Some((score, shared, e, text.clone()));
                }
            }
        }
    }
    best.map(|(_, shared, e, text)| {
        let mut shared: Vec<String> = shared.into_iter().collect();
        shared.sort_by(|a, b| w2[b].total_cmp(&w2[a]));
        Match {
            at: e.at.clone(),
            who: e.who.clone(),
            text,
            shared,
        }
    })
}

// ---------------------------------------------------------------- the cost

/// The turns a lesson's text says it cost: the largest « <n> turns » after cost, took, lost, wasted or spent, or « <n>
/// more|extra|wasted turns », else 1, a lesson told at all cost one. A session's count or a cap is no cost.
fn turns(text: &str) -> i64 {
    (TURNS.captures_iter(text))
        .filter_map(|c| c.get(1).or(c.get(2))?.as_str().parse().ok())
        .max()
        .unwrap_or(1)
}

/// A lesson that repeats an earlier one, with the lessons of its chain (linked by folds of repeats) whose turns no raise counted
/// yet, and their sum. His comment adds none.
fn repeat(lines: &[Value], at: &str, comments: &dyn Fn(&str) -> Vec<Lesson>) -> Option<Repeat> {
    let m = matched(lines, at, comments)?;
    let counted: BTreeSet<&str> = (lines.iter())
        .filter(|l| l["kind"] == "folded")
        .filter_map(|l| l["counted"].as_array())
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let edges: Vec<(&str, &str)> = (lines.iter())
        .filter(|l| l["kind"] == "folded")
        .filter_map(|l| Some((l["of"].as_str()?, l["repeats"].as_str()?)))
        .collect();
    let pending: Vec<String> = (chain(&edges, &[at, &m.at]).into_iter())
        .filter(|a| !counted.contains(a.as_str()))
        .collect();
    let texts: BTreeMap<String, String> = (lessons(lines).into_iter())
        .map(|l| (l.at, l.text))
        .collect();
    let turns = (pending.iter())
        .map(|a| texts.get(a).map_or(0, |t| turns(t)))
        .sum();
    Some(Repeat {
        of: m.at,
        who: m.who,
        pending,
        turns,
    })
}

/// The lessons linked to `ats` through `edges`, `ats` included, oldest first: his comments' times are local, the lessons' UTC,
/// so compared in local time.
fn chain(edges: &[(&str, &str)], ats: &[&str]) -> Vec<String> {
    let mut set: BTreeSet<String> = ats.iter().map(|a| a.to_string()).collect();
    loop {
        let before = set.len();
        for (a, b) in edges {
            if set.contains(*a) || set.contains(*b) {
                set.insert(a.to_string());
                set.insert(b.to_string());
            }
        }
        if set.len() == before {
            break;
        }
    }
    let mut out: Vec<String> = set.into_iter().collect();
    out.sort_by_cached_key(|a| {
        if a.ends_with('Z') {
            local(a)
        } else {
            a.clone()
        }
    });
    out
}

// ---------------------------------------------------------------- the live reads

/// What the live fold reads: the older app's database, read-only, and this checkout's files and git.
struct Live {
    repo: PathBuf,
    db: PathBuf,
    tasks: Vec<Foldable>,
    own_usage: String,
}

impl Live {
    fn new(own_usage: &str) -> Result<Live, String> {
        let db = crate::tasks::older_db();
        Ok(Live {
            repo: crate::tasks::repo(),
            tasks: dash_store::older::foldable(&db)?,
            db,
            own_usage: own_usage.into(),
        })
    }

    fn git(&self, args: &[&str]) -> Option<String> {
        let out = std::process::Command::new("git")
            .args(["-c", "core.fsmonitor=false", "-C"])
            .arg(&self.repo)
            .args(args)
            .output()
            .ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
    }
}

impl Look for Live {
    fn tasks(&self) -> &[Foldable] {
        &self.tasks
    }

    fn comments(&self, before: &str) -> Vec<Lesson> {
        let rows = dash_store::older::his_feedback(&self.db, before).unwrap_or_default();
        (rows.into_iter())
            .map(|[at, source, r, text]| Lesson {
                at,
                who: format!("his {source} {r}"),
                text: unwake(&text),
            })
            .collect()
    }

    fn labels(&self) -> (Vec<String>, Option<String>) {
        let (older, unread) = match crate::tasks::older_app().help_all() {
            Ok(older) => (older, None),
            Err(e) => (String::new(), Some(e)),
        };
        (labels_of(&format!("{}\n{older}", self.own_usage)), unread)
    }

    fn path(&self, p: &str) -> bool {
        let file = match p.strip_prefix("~/") {
            Some(rest) => std::env::var("HOME")
                .map_or_else(|_| PathBuf::from(p), |h| Path::new(&h).join(rest)),
            None => self.repo.join(p),
        };
        // a name or a path's tail (launch.ex, crates/gate/tests/x.rs) names the one tracked file ending so: lib.rs names none
        file.exists()
            || (self.git(&["ls-files", "--", p, &format!("*/{p}")]))
                .is_some_and(|out| out.lines().filter(|l| !l.is_empty()).count() == 1)
    }

    fn module(&self, m: &str) -> bool {
        let defined = format!("defmodule {m} do");
        self.git(&[
            "grep",
            "-q",
            "-F",
            "-e",
            &defined,
            "--",
            "private/dashboard/lib",
        ])
        .is_some()
    }

    fn commit(&self, sha: &str) -> bool {
        // ^{commit}: a tag, tree or blob is no commit
        let Some(out) = self.git(&[
            "show",
            // a merge lists no file by default: its changes against the branch it landed on
            "--diff-merges=first-parent",
            "--name-only",
            "--format=%ct",
            &format!("{sha}^{{commit}}"),
        ]) else {
            return false;
        };
        let mut lines = out.lines().filter(|l| !l.is_empty());
        let since = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64)
            - COMMIT_DAYS * 86_400;
        let recent = (lines.next())
            .and_then(|t| t.trim().parse::<i64>().ok())
            .is_some_and(|t| t >= since);
        recent && lines.any(|f| !doc_file(f))
    }

    fn raise(&self, id: &str, turns: i64) -> Result<(), String> {
        let turns = turns.to_string();
        let flags = [("--priority", "1"), ("--lesson-turns", turns.as_str())];
        crate::tasks::older_app().update(id, &flags).map(drop)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::memory::TestBudget;
    use std::cell::RefCell;

    /// A stand-in: tasks and labels as given, the paths, modules and commits listed exist, every raise noted.
    #[derive(Default)]
    struct Stand {
        tasks: Vec<Foldable>,
        comments: Vec<Lesson>,
        labels: Vec<String>,
        unread: Option<String>,
        paths: Vec<&'static str>,
        raised: RefCell<Vec<(String, i64)>>,
    }

    impl Look for Stand {
        fn tasks(&self) -> &[Foldable] {
            &self.tasks
        }
        fn comments(&self, before: &str) -> Vec<Lesson> {
            let earlier = self.comments.iter().filter(|c| c.at.as_str() < before);
            earlier.cloned().collect()
        }
        fn labels(&self) -> (Vec<String>, Option<String>) {
            (self.labels.clone(), self.unread.clone())
        }
        fn path(&self, p: &str) -> bool {
            self.paths.contains(&p)
        }
        fn module(&self, m: &str) -> bool {
            m == "Dashboard.Orch.PM"
        }
        fn commit(&self, sha: &str) -> bool {
            sha == "abc1234"
        }
        fn raise(&self, id: &str, turns: i64) -> Result<(), String> {
            self.raised.borrow_mut().push((id.into(), turns));
            Ok(())
        }
    }

    fn task(id: &str, stage: &str, epic: &str) -> Foldable {
        Foldable {
            id: id.into(),
            project: "site_perso".into(),
            stage: stage.into(),
            epic: Some(epic.into()),
            lesson_turns: 2,
            owner: "rust".into(),
            queued: stage == "open",
            duplicate_of: None,
        }
    }

    /// A task Rust's picker cannot start: owned by `owner`, open, out of Rust's queue.
    fn stuck(id: &str, owner: &str) -> Foldable {
        Foldable {
            owner: owner.into(),
            queued: false,
            ..task(id, "open", "speed")
        }
    }

    fn stand() -> Stand {
        Stand {
            tasks: vec![
                task("rust-live-copy", "open", "rust-poc"),
                task("rust-port-parity", "done", "rust-poc"),
                task("old-idea", "dropped", "rust-poc"),
            ],
            labels: labels_of("task run [--ask]\nfolds [--recheck <day>]"),
            paths: vec!["rust/crates/dash/src/folds.rs"],
            ..Stand::default()
        }
    }

    fn lesson(at: &str, task: &str, text: &str) -> Value {
        json!({"at": at, "task": task, "feedback": text, "verdict": "met"})
    }

    const NOW: &str = "2026-11-11T08:00:00.000000Z";

    fn folded(
        lines: &[Value],
        kind: &str,
        place: &str,
        look: &Stand,
    ) -> Result<(Value, Option<String>), String> {
        fold(lines, "2026-11-10T23:00:00Z", kind, place, look, NOW)
    }

    /// Done-when 1 of rust-fold-port, the colon form: « task <id>: … » names that task, its words a doc's or not.
    #[test]
    fn a_task_id_then_a_colon_names_the_task() {
        let _budget = TestBudget::new();
        let lines = [lesson(
            "2026-11-10T23:00:00Z",
            "a",
            "The session lost a turn on a guard.",
        )];
        let look = stand();
        let place =
            "task rust-live-copy: the brief lists the session's writable folders from its profile";
        let (line, said) = folded(&lines, "mechanism", place, &look).expect("folded");
        assert_eq!(line["into"], place);
        assert_eq!(
            (line["as"].as_str(), line["of"].as_str()),
            (Some("mechanism"), Some("2026-11-10T23:00:00Z"))
        );
        assert_eq!(said, None);
        // a done task is built: its merge is the mechanism (the older app refused it once merged)
        let done =
            "task rust-port-parity: dash parity prints both apps' computed styles side by side";
        assert!(folded(&lines, "mechanism", done, &look).is_ok());
    }

    /// Done-when 1 of rust-fold-targets-live, owner not rust: a fold into a task only the frozen older app would start is
    /// refused, naming why, whatever its kind; the same fold into a Rust task passes.
    #[test]
    fn a_fold_into_a_task_not_rust_s_is_refused() {
        let _budget = TestBudget::new();
        let lines = [lesson(
            "2026-11-10T23:00:00Z",
            "a",
            "The fold went nowhere.",
        )];
        let mut look = stand();
        look.tasks.push(stuck("fold-backlog", "claude"));
        for kind in KINDS {
            let why = folded(&lines, kind, "task fold-backlog: count the folds", &look)
                .expect_err("a dead target");
            assert!(
                why.contains("task fold-backlog: Rust's picker cannot start it (its owner is claude, not rust"),
                "{why}"
            );
        }
        let why = folded(&lines, "mechanism", "task fold-backlog", &look).expect_err("dead");
        assert!(
            why.ends_with(
                "(dash task update fold-backlog --owner rust --layer <layer> --later nil)"
            ),
            "{why}"
        );
        // no owner at all is as dead
        look.tasks.push(stuck("orphan", ""));
        let why = folded(&lines, "mechanism", "task orphan", &look).expect_err("no owner");
        assert!(
            why.contains("(it has no owner, so no picker starts it)"),
            "{why}"
        );
        // a code path beside it does not carry the dead task: dash folds would still count the fold there
        let both = "rust/crates/dash/src/folds.rs; task fold-backlog";
        assert!(folded(&lines, "mechanism", both, &look).is_err());
        assert!(folded(&lines, "mechanism", "task rust-live-copy", &look).is_ok());
    }

    /// Done-when 1 of rust-fold-targets-live, not in its queue: a Rust task labelled Unplanned, put off, out of open, or
    /// dropped is refused, naming why; a done one is live, its merge holding the lesson.
    #[test]
    fn a_fold_into_a_rust_task_out_of_its_queue_is_refused() {
        let _budget = TestBudget::new();
        let lines = [lesson(
            "2026-11-10T23:00:00Z",
            "a",
            "The fold went nowhere.",
        )];
        let mut look = stand();
        look.tasks.push(stuck("rust-later", "rust"));
        look.tasks.push(Foldable {
            stage: "backlog".into(),
            ..stuck("rust-backlog", "rust")
        });
        let refused =
            |look: &Stand, place: &str| folded(&lines, "mechanism", place, look).expect_err(place);
        // each says what brings it into the queue, never an update that leaves it dead (/code-review)
        let why = refused(&look, "task rust-later");
        assert!(
            why.contains("(it is labelled Unplanned or set for later)"),
            "{why}"
        );
        assert!(
            why.ends_with("(dash task update rust-later --later nil, its Unplanned label off)"),
            "{why}"
        );
        let why = refused(&look, "task rust-backlog");
        assert!(why.contains("(its stage is backlog, not open)"), "{why}");
        assert!(
            why.ends_with("(dash task update rust-backlog --status open --later nil)"),
            "{why}"
        );
        let why = refused(&look, "task old-idea: an idea dropped");
        assert!(why.contains("(it is dropped)"), "{why}");
        assert!(why.ends_with("fold it into a task Rust starts"), "{why}");
        assert!(folded(&lines, "mechanism", "task rust-port-parity", &look).is_ok());
        // merged into a Rust task (done-when 2), its past folds are that task's: live while it is. A new fold names the task it
        // was merged into: a dropped task builds nothing (rust-fold-port).
        let merged = |id: &str, into: &str| Foldable {
            stage: "dropped".into(),
            duplicate_of: Some(into.into()),
            ..stuck(id, "claude")
        };
        let (live, later) = (
            merged("fold-backlog", "rust-live-copy"),
            merged("into-later", "rust-later"),
        );
        assert_eq!(dead(&live, &look.tasks), None);
        assert_eq!(
            dead(&later, &look.tasks).as_deref(),
            Some("merged into rust-later, where it is labelled Unplanned or set for later")
        );
    }

    /// Done-when 3 of rust-fold-targets-live: `dash folds` prints each target of the week's folds, live or dead with why, and
    /// its fold count, the dead first; an older fold and one naming no task are left out of the targets.
    #[test]
    fn dash_folds_prints_each_target_live_or_dead_with_its_count() {
        let _budget = TestBudget::new();
        let fold = |at: &str, into: &str| json!({"kind": "folded", "at": at, "into": into});
        let lines = [
            fold("2026-11-10T09:00:00Z", "task fold-backlog: count"),
            fold("2026-11-10T10:00:00Z", "task fold-backlog"),
            fold("2026-11-10T11:00:00Z", "task rust-live-copy"),
            fold("2026-11-10T12:00:00Z", "rust/crates/dash/src/folds.rs"),
            fold("2026-11-01T09:00:00Z", "task rust-live-copy"),
            lesson("2026-11-10T13:00:00Z", "a", "task fold-backlog"),
        ];
        let mut tasks = stand().tasks;
        tasks.push(stuck("fold-backlog", "claude"));
        let said = targets(&lines, &tasks, "2026-11-04T08:00:00.000000Z");
        assert_eq!(
            said,
            "fold targets since 2026-11-04T08:00: 1 live, 1 dead; 1 folds named no task\n\
             \x20 dead 2 fold-backlog: its owner is claude, not rust\n\
             \x20 live 1 rust-live-copy\n"
        );
    }

    /// Done-when 1 of rust-fold-port, the words form: « task <id> part 4 the brief … » names that task too.
    #[test]
    fn a_task_id_then_words_names_the_task() {
        let _budget = TestBudget::new();
        let lines = [lesson(
            "2026-11-10T23:00:00Z",
            "a",
            "The session lost a turn on a guard.",
        )];
        let look = stand();
        let place = "task rust-live-copy part 4 the brief lists the session's writable folders";
        assert_eq!(mechanism(place, &look), Ok("task rust-live-copy".into()));
        assert!(folded(&lines, "mechanism", place, &look).is_ok());
        // a task named inside a doc's sentence is still a doc, and a dropped task builds nothing
        let inside = "the brief now says to read task rust-live-copy first";
        assert!(mechanism(inside, &look).is_err(), "{inside}");
        assert!(mechanism("task old-idea: it builds the check", &look).is_err());
        // an id in capitals names the same task (/code-review: leads matched it, the lookup did not)
        let capitals = "Task Rust-Live-Copy: the brief lists the writable folders";
        assert_eq!(mechanism(capitals, &look), Ok("Task Rust-Live-Copy".into()));
        // a dropped task opening a doc's sentence leaves it a doc: the path inside it names nothing
        let dropped = "task old-idea: CLAUDE.md quirk 2 now cites rust/crates/dash/src/folds.rs";
        assert!(mechanism(dropped, &look).is_err(), "{dropped}");
    }

    /// Done-when 2 of rust-fold-port: a name that does not exist is refused, whatever it looks like.
    #[test]
    fn a_name_that_does_not_exist_is_refused() {
        let _budget = TestBudget::new();
        let lines = [lesson(
            "2026-11-10T23:00:00Z",
            "a",
            "The session lost a turn on a guard.",
        )];
        let look = stand();
        for place in [
            "task no-such-task: it builds the check",
            "rust/crates/dash/src/nowhere.rs refuses it",
            "dash task frobnicate",
            "Dashboard.Orch.Nowhere",
            "commit 1234abcd",
        ] {
            let why = folded(&lines, "mechanism", place, &look).expect_err(place);
            assert!(
                why.contains("names no code path, hook, test or command that exists"),
                "{why}"
            );
        }
        // what exists passes: a path, a whole label, a module, a recent commit
        for (place, thing) in [
            (
                "rust/crates/dash/src/folds.rs refuses it",
                "rust/crates/dash/src/folds.rs",
            ),
            ("dash task run --folded refuses it", "dash task run"),
            ("Dashboard.Orch.PM's check", "Dashboard.Orch.PM"),
            ("commit abc1234", "commit abc1234"),
        ] {
            assert_eq!(mechanism(place, &look), Ok(thing.into()), "{place}");
        }
        // the older app's commands unread: the refusal says so, never only « exists »
        let look = Stand {
            unread: Some("dash help all: exit 3".into()),
            ..stand()
        };
        let why = mechanism("dash queue done refuses it", &look).expect_err("unread");
        assert!(
            why.ends_with(
                "(the older app's commands were not read, try again: dash help all: exit 3)"
            ),
            "{why}"
        );
    }

    /// Done-when 2 of rust-fold-port: a doc fold of a lesson an earlier task already told is refused, the match quoted.
    #[test]
    fn a_doc_fold_of_a_repeated_lesson_is_refused() {
        let _budget = TestBudget::new();
        let lines = [
            lesson(
                "2026-11-09T10:00:00Z",
                "earlier-task",
                "The heredoc guard refused the python script that cited rm in its text, so the edit ran twice.",
            ),
            lesson(
                "2026-11-10T23:00:00Z",
                "later-task",
                "The heredoc guard refused my python script citing rm in its text and the edit ran again.",
            ),
        ];
        let look = stand();
        let why = folded(&lines, "doc", "CLAUDE.md quirk 2", &look).expect_err("a repeat");
        assert!(
            why.starts_with("a doc fold of 2026-11-10T23:00:00Z repeats an earlier lesson: 2026-11-09T10:00:00Z task earlier-task: «"),
            "{why}"
        );
        assert!(
            why.contains("(shared: ") && why.contains("heredoc"),
            "{why}"
        );
        // the same task's own earlier lesson is no repeat
        let own = [
            lines[0].clone(),
            lesson(
                "2026-11-10T23:00:00Z",
                "review-earlier-task",
                lines[1]["feedback"].as_str().unwrap(),
            ),
        ];
        assert!(folded(&own, "doc", "CLAUDE.md quirk 2", &look).is_ok());
    }

    /// The older app's other refusals, kept: a kind, an empty where, a memory file, a lesson not open, a watcher's lesson.
    #[test]
    fn the_older_app_s_other_refusals_hold() {
        let _budget = TestBudget::new();
        let at = "2026-11-10T23:00:00Z";
        let lines = [lesson(at, "a", "The load hit 135 while four suites ran.")];
        let look = stand();
        let refused = |kind: &str, place: &str, lines: &[Value]| {
            folded(lines, kind, place, &look).expect_err(place)
        };
        assert!(refused("rule", "x", &lines).starts_with("rule: a fold is mechanism or doc"));
        assert!(refused("doc", "  ", &lines).starts_with("a fold names where its lesson went"));
        let memory = refused("doc", "~/.claude/projects/x/memory/a.md", &lines);
        assert!(
            memory.contains("a lesson never goes in a memory file"),
            "{memory}"
        );
        assert!(
            refused("doc", "Dashboard.Watch.Load", &lines)
                .contains("Dashboard.Watch.Load could catch")
        );
        let closed = [
            lines[0].clone(),
            json!({"kind": "folded", "of": at, "as": "doc", "into": "x"}),
        ];
        assert_eq!(
            refused("doc", "x", &closed),
            format!("no open feedback at {at}")
        );
        // a page's load is no machine's
        let page = [lesson(at, "a", "The page load was 30 s on the board.")];
        assert!(folded(&page, "doc", "a skill sentence", &look).is_ok());
    }

    /// Done-when 4 of rust-fold-port: raising a repeated lesson's task writes its priority and turns, never its epic, so it
    /// keeps its roadmap item; the line says so and names what it counted.
    #[test]
    fn raising_a_repeated_lesson_s_task_keeps_its_epic() {
        let _budget = TestBudget::new();
        let lines = [
            lesson(
                "2026-11-09T10:00:00Z",
                "earlier-task",
                "The heredoc guard refused the python script that cited rm in its text, it cost 3 turns.",
            ),
            lesson(
                "2026-11-10T23:00:00Z",
                "later-task",
                "The heredoc guard refused my python script citing rm in its text and the edit ran again.",
            ),
        ];
        let look = stand();
        let (line, said) = folded(
            &lines,
            "mechanism",
            "task rust-live-copy: the guard hints the Edit tool",
            &look,
        )
        .expect("folded");
        assert_eq!(
            *look.raised.borrow(),
            [("rust-live-copy".to_string(), 2 + 3 + 1)]
        );
        assert_eq!(
            said.as_deref(),
            Some(
                "repeats 2026-11-09T10:00:00Z task earlier-task, 4 turns: task rust-live-copy raised to priority 1, 6 lesson \
                 turns, its epic rust-poc and roadmap item kept"
            )
        );
        assert_eq!(line["raised"], "rust-live-copy");
        assert_eq!(line["repeats"], "2026-11-09T10:00:00Z");
        assert_eq!(
            line["counted"],
            json!(["2026-11-09T10:00:00Z", "2026-11-10T23:00:00Z"])
        );
        assert!(line.get("epic-was").is_none(), "no epic changed: {line}");
        // a done task is named, never raised
        let look = stand();
        let (_, said) = folded(
            &lines,
            "mechanism",
            "task rust-port-parity: dash parity",
            &look,
        )
        .expect("folded");
        assert!(look.raised.borrow().is_empty());
        assert!(said.is_some_and(|s| s.ends_with("no open site_perso task named to raise")));
    }

    #[test]
    fn labels_are_whole_commands_from_both_apps_lists() {
        let _budget = TestBudget::new();
        let usage = "usage: dash serve|mcp --data <dir>\n       dash task show|run --data <dir> <id>\n       dash parity <route>";
        let help =
            "agents                      /agents as text\nab [assign <exp>]the A/B tests' report";
        let labels = labels_of(&format!("{usage}\n{help}"));
        for l in [
            "serve",
            "mcp",
            "task show",
            "task run",
            "parity",
            "agents",
            "ab",
        ] {
            assert!(labels.contains(&l.to_string()), "{l}: {labels:?}");
        }
        assert!(
            !labels.iter().any(|l| l.contains("as text") || l == "task"),
            "{labels:?}"
        );
    }

    #[test]
    fn a_lesson_s_cost_is_its_largest_count_of_turns() {
        let _budget = TestBudget::new();
        assert_eq!(turns("it cost 3 turns, then took them 5 turns"), 5);
        assert_eq!(turns("2 extra turns"), 2);
        assert_eq!(turns("cost nothing, the session ran 60 turns"), 1);
        assert_eq!(
            unwake(
                "mine. Lesson from task x: y. dash task run --folded 1 mechanism \"<where>\". his"
            ),
            "mine.  his"
        );
    }
}
