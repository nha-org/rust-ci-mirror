//! A merged task counts as done only once its outcome holds on the live app (task done-checked-live; @nha, 2026-11-02: « jev
//! not judged despite the task marking this automatically calling it done », « are you watching tasks done ? If not why
//! not ? »): voice-clone-local was closed done while its listening page's samples answered 404. Rust's port of the older app's
//! `Dashboard.Orch.LiveCheck`, and of the after-merge step the orchestrator ran by hand (rfc-rust-unified#orchestration).
//! Each pass, on its own thread beside the comment beat:
//! 1. **Proofs**: each task merged on main in the last 14 days, done, its merge past the live app's swap, is checked once per
//!    merge: every live-app page its done-when parts or its report's checks name (a `:7788` link or a `private/` page, served
//!    at `/v/`) must answer 200, and each same-host media src of it 2xx to a HEAD. A failed part reopens the task, naming it.
//! 2. **After merge**: each open « After merge checks of task » request in to_process, once its PRs merged. Jev first says
//!    whether a page or command the lines name decides them; that runs, else an agent with read-only tools checks them. Not
//!    met reopens the task with what it printed; met closes the request.
//!
//! Nothing here acts on what it checks: a page is a GET, a command passes `read_only`, the agent runs `--restricted` with
//! `dontAsk` and a read-only allow-list, so a line saying « send the reply » is checked as sent or not, never sent.
use crate::jev;
use regex::Regex;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::Duration;

/// A merge older than this is not checked: the first pass spreads the window over several.
const WINDOW_S: i64 = 14 * 86_400;
/// The live app swaps to a newer main within 30 minutes (`bin/live`): a merge is checked past this.
const SETTLE_S: i64 = 35 * 60;
/// Tasks proved per pass, and after-merge requests looked at: a pass stays bounded in time and money.
const PROOFS_PER_PASS: usize = 5;
const REQUESTS_PER_PASS: usize = 4;
/// A page that timed out or answered 5xx is asked again; on this pass it counts as failed.
const TRIES: u64 = 3;
/// Keys kept in the state, newest first.
const KEPT: usize = 500;
/// At most one pass per this, however often the beat asks.
const EVERY: Duration = Duration::from_secs(10 * 60);
const AGENT_MODEL: &str = "sonnet";
const AGENT_BUDGET: &str = "0.50";
const AGENT_CALL: Duration = Duration::from_secs(300);
const COMMAND_CALL: Duration = Duration::from_secs(60);
const LIVE: &str = "http://127.0.0.1:7788";
const CHECKS_TITLE: &str = "# After merge checks of task ";

static LINK: LazyLock<Regex> = LazyLock::new(|| {
    // the path ends on no sentence punctuation: « at http://127.0.0.1:7788/projects/x. »
    Regex::new(r#"https?://(?:127\.0\.0\.1|localhost|100\.72\.158\.115):7788(/[^\s"'<>)\]`,;]*[^\s"'<>)\]`,;.:!?*])"#)
        .expect("regex")
});
static PAGE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"private/[\w./-]+\.(?:html|md)").expect("regex"));
static MEDIA: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?i)<(?:audio|video|source|img|track|embed)\b[^>]*?\ssrc\s*=\s*(?:"([^"]+)"|'([^']+)')"#,
    )
    .expect("regex")
});
/// A link straight to a file no page wraps: asked by a HEAD, never downloaded.
static FILE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\.(?:m4a|mp3|wav|ogg|webm|mp4|mov|png|jpe?g|gif|webp|svg|pdf|zip)(?:\?|$)")
        .expect("regex")
});
/// A part asking for a page to be gone proves nothing by a GET (« 404 » alone: `gone` reads « no 404 » as an answer asked).
static GONE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:removed|deleted|retired|gone|no longer|redirect\w*)\b").expect("regex")
});
/// A request that writes: -X POST, --request PUT, -d, -sd, --data*, --json, -F, --form.
static WRITES: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\s(?:-[a-zA-Z]*X\s*(?:POST|PUT|PATCH|DELETE)|--request[\s=]+(?:POST|PUT|PATCH|DELETE)|-[a-zA-Z]*[dF]|--data\S*|--json|--form)(?:\s|$)")
        .expect("regex")
});
static SKIP: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:/dashboard/push/open|/dashboard/pane|/live$)").expect("regex")
});
static PR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"https://github\.com/[\w.-]+/[\w.-]+/pull/\d+").expect("regex"));

/// One answer of the live app.
#[derive(Clone, Debug, PartialEq)]
pub enum Got {
    /// Its status and body (empty for a HEAD).
    Status(u16, String),
    /// It refused the connection: nothing is decided this pass.
    Down,
    /// A timeout: asked again.
    Slow,
    /// Any other error, the part's failure.
    Failed(String),
}

/// A task as the list holds it: its stage, done-when and newest report's checks.
pub struct Task {
    pub stage: String,
    pub done_when: String,
    pub checks: Vec<Value>,
}

/// What a pass reads and writes outside itself; the binary's is `Real`, a test's a fake.
pub trait Live {
    fn get(&mut self, path: &str, head: bool) -> Got;
    fn task(&mut self, id: &str) -> Result<Option<Task>, String>;
    /// « MERGED », « OPEN » or « CLOSED ».
    fn pr_state(&mut self, url: &str) -> Result<String, String>;
    /// Jev's score, 0 to 1, that a page or command the lines name decides them alone.
    fn jev(&mut self, task: &str, lines: &str) -> Result<f64, String>;
    /// A read-only command's exit, success or not, and what it printed.
    fn run(&mut self, argv: &[String]) -> Result<(bool, String), String>;
    /// The agent's last words.
    fn agent(&mut self, prompt: &str) -> Result<String, String>;
    /// Opens task `id` again if done, its `live-failed` saying `why`.
    fn reopen(&mut self, id: &str, why: &str) -> Result<(), String>;
    /// Whether `file` was at merge `sha` but a later commit on main moved or deleted it: no proof then (the older app's
    /// `moved?`, /code-review of #942).
    fn moved(&mut self, _sha: &str, _file: &str) -> bool {
        false
    }
    /// Whether the build the live app serves holds `sha`; none when no build is named, and the merge's age decides.
    fn serves(&mut self, _sha: &str) -> Option<bool> {
        None
    }
}

/// The numbers a check's `part` names: 3, "3", "1, 3", "part 2".
fn part_numbers(part: &Value) -> Vec<usize> {
    match part {
        Value::Number(n) => n.as_u64().map(|n| n as usize).into_iter().collect(),
        Value::String(s) => (s.split(|c: char| !c.is_ascii_digit()))
            .filter_map(|d| d.parse().ok())
            .collect(),
        _ => vec![],
    }
}

fn gone(part: &str) -> bool {
    let lower = part.to_lowercase();
    GONE.is_match(part) || (lower.match_indices("404")).any(|(at, _)| !lower[..at].ends_with("no "))
}

/// The live-app paths `text` names: a `:7788` link's path, or a `private/` page as `/v/` serves it (bare, or under the repo's
/// own folder: « ~/repo/site_perso/private/docs/x.html »).
pub fn paths(text: &str) -> Vec<String> {
    let mut out: Vec<String> = (LINK.captures_iter(text))
        .map(|c| c[1].to_string())
        .collect();
    let word = |c: char| c.is_alphanumeric() || c == '_';
    for m in PAGE.find_iter(text) {
        let before = &text[..m.start()];
        let bare = (before.chars().last()).is_none_or(|c| !(word(c) || "/.-".contains(c)));
        let ends = (text[m.end()..].chars().next()).is_none_or(|c| !(word(c) || "/-".contains(c)));
        if (bare || before.ends_with("site_perso/")) && ends {
            out.push(format!("/v/{}", m.as_str()));
        }
    }
    out
}

/// Each `(part number, path)` to fetch for a task: what each part, or a read-only check its report names for it, links to.
pub fn proofs(done_when: &str, checks: &[Value]) -> Vec<(usize, String)> {
    let mut out: Vec<(usize, String)> = vec![];
    for (i, part) in crate::reports::parts(done_when).iter().enumerate() {
        let n = i + 1;
        if gone(part) {
            continue;
        }
        let commands = (checks.iter())
            .filter(|c| part_numbers(&c["part"]).contains(&n))
            .filter_map(|c| c["command"].as_str())
            .filter(|c| !WRITES.is_match(&format!(" {c} ")));
        for text in std::iter::once(part.as_str()).chain(commands) {
            for p in paths(text) {
                let skip = SKIP.is_match(p.split('?').next().unwrap_or_default());
                if !skip && !out.contains(&(n, p.clone())) {
                    out.push((n, p));
                }
            }
        }
    }
    out
}

/// The same-host media srcs of the page at `path`, as paths.
fn media(path: &str, body: &str) -> Vec<String> {
    let dir = &path[..path.rfind('/').map_or(0, |at| at + 1)];
    let mut out = vec![];
    for c in MEDIA.captures_iter(body) {
        let src = c
            .get(1)
            .or(c.get(2))
            .map_or("", |m| m.as_str())
            .replace("&amp;", "&");
        let own = if let Some(rest) = src.strip_prefix("http://").or(src.strip_prefix("https://")) {
            let (host, p) = rest.split_once('/').unwrap_or((rest, ""));
            ["127.0.0.1:7788", "localhost:7788", "100.72.158.115:7788"]
                .contains(&host)
                .then(|| format!("/{p}"))
        } else if src.starts_with("//") || src.starts_with("data:") || src.contains(':') {
            None
        } else if src.starts_with('/') {
            Some(src.clone())
        } else {
            Some(normal(&format!("{dir}{src}")))
        };
        if let Some(p) = own.filter(|p| !out.contains(p)) {
            out.push(p);
        }
    }
    out
}

/// `/a/b/../c/./d` as `/a/c/d`.
fn normal(path: &str) -> String {
    let mut kept: Vec<&str> = vec![];
    for seg in path.split('/').skip(1) {
        match seg {
            "." => {}
            ".." => drop(kept.pop()),
            s => kept.push(s),
        }
    }
    format!("/{}", kept.join("/"))
}

/// What the proofs say: each held, some failed (« part N: … »), or nothing decided this pass and why.
#[derive(Debug, PartialEq)]
pub enum Proof {
    Held,
    Failed(Vec<String>),
    /// A timeout or a 5xx: asked again, counted toward `TRIES`.
    Again(String),
    /// The app refused the connection: nothing decided, nothing counted.
    Down,
}

fn answered(got: &Got) -> String {
    match got {
        Got::Status(s, _) => format!("answered {s}"),
        Got::Down => "did not answer (refused)".into(),
        Got::Slow => "did not answer in time".into(),
        Got::Failed(e) => format!(
            "did not answer ({})",
            e.chars().take(80).collect::<String>()
        ),
    }
}

/// The proofs fetched on the live app. `last_try`: a timeout or a 5xx counts as failed, else it asks again next pass.
pub fn check(proofs: &[(usize, String)], live: &mut impl Live, last_try: bool) -> Proof {
    let (mut failed, mut slow) = (vec![], vec![]);
    let mut judged = |n: usize, what: String, got: &Got| -> Option<Proof> {
        let why = format!("part {n}: {what} {}", answered(got));
        match got {
            Got::Down => return Some(Proof::Down),
            Got::Slow | Got::Status(500.., _) if !last_try => slow.push(why),
            Got::Status(200..=299, _) => {}
            _ => failed.push(why),
        }
        None
    };
    for (n, path) in proofs {
        if FILE.is_match(path) {
            let got = live.get(path, true);
            if let Some(again) = judged(*n, path.clone(), &got) {
                return again;
            }
            continue;
        }
        let got = live.get(path, false);
        let Got::Status(200, body) = &got else {
            // a page that answered 2xx but not 200 has no body to read: its own answer is the proof
            if let Some(again) = judged(*n, path.clone(), &got) {
                return again;
            }
            continue;
        };
        for src in media(path, body) {
            let got = live.get(&src, true);
            if let Some(again) = judged(*n, format!("{path}: {src}"), &got) {
                return again;
            }
        }
    }
    match (failed.is_empty(), slow.is_empty()) {
        (true, true) => Proof::Held,
        (_, false) => Proof::Again(format!("asked again next pass: {}", slow.join("; "))),
        (false, true) => Proof::Failed(failed),
    }
}

/// What a pass remembers: each `task:sha` and request decided, and how often each was asked again.
#[derive(Default)]
pub struct State {
    checked: Vec<String>,
    tries: BTreeMap<String, u64>,
}

impl State {
    fn read(dir: &Path) -> State {
        let text = std::fs::read_to_string(dir.join("live-check.json")).unwrap_or_default();
        let v: Value = serde_json::from_str(&text).unwrap_or_default();
        let checked = v["checked"].as_array().into_iter().flatten();
        let tries = v["tries"].as_object().into_iter().flatten();
        State {
            checked: checked
                .filter_map(|k| k.as_str().map(String::from))
                .collect(),
            tries: tries
                .filter_map(|(k, n)| Some((k.clone(), n.as_u64()?)))
                .collect(),
        }
    }

    fn write(&self, dir: &Path) -> Result<(), String> {
        let file = dir.join("live-check.json");
        let tmp = dir.join("live-check.json.tmp");
        let text = json!({"checked": self.checked, "tries": self.tries}).to_string();
        std::fs::write(&tmp, text).map_err(|e| format!("{}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, &file).map_err(|e| format!("{}: {e}", file.display()))
    }

    fn decided(&mut self, key: &str) {
        self.tries.remove(key);
        self.checked.insert(0, key.to_string());
        self.checked.truncate(KEPT);
    }

    fn again(&mut self, key: &str) {
        *self.tries.entry(key.to_string()).or_default() += 1;
    }
}

/// The proof pass over `merges` (task id to its newest merge's sha and time): lines saying what it did.
pub fn prove(
    merges: &BTreeMap<String, (String, i64)>,
    live: &mut impl Live,
    state: &mut State,
    now_s: i64,
) -> Vec<String> {
    let mut due: Vec<(&String, &String, String)> = (merges.iter())
        .filter(|(_, (_, at))| now_s - at <= WINDOW_S)
        .map(|(id, (sha, at))| (id, sha, *at, format!("{id}:{sha}")))
        .filter(|(_, _, _, key)| !state.checked.contains(key))
        // live once the served build holds it, else once past the swap (/code-review: a swap that failed served the old build)
        .filter(|(_, sha, at, _)| live.serves(sha).unwrap_or(now_s - at >= SETTLE_S))
        .map(|(id, sha, _, key)| (id, sha, key))
        .collect();
    // a task asked again least often first: one that keeps timing out never starves the rest
    due.sort_by_key(|(_, _, key)| state.tries.get(key).copied().unwrap_or(0));
    let mut lines = vec![];
    let mut taken = 0;
    for (id, sha, key) in due {
        if taken == PROOFS_PER_PASS {
            break;
        }
        let task = match live.task(id) {
            Ok(Some(t)) if t.stage == "done" => t,
            // gone or dropped: nothing to prove; open or in review: asked again once it is done (/code-review)
            Ok(None) => {
                state.decided(&key);
                continue;
            }
            Ok(Some(t)) if t.stage == "dropped" => {
                state.decided(&key);
                continue;
            }
            Ok(Some(_)) => continue,
            Err(e) => {
                lines.push(format!("{id}: not read: {e}"));
                continue;
            }
        };
        taken += 1;
        let last_try = state.tries.get(&key).copied().unwrap_or(0) + 1 >= TRIES;
        let proofs: Vec<(usize, String)> = (proofs(&task.done_when, &task.checks).into_iter())
            .filter(|(_, p)| !p.strip_prefix("/v/").is_some_and(|f| live.moved(sha, f)))
            .collect();
        match check(&proofs, live, last_try) {
            Proof::Held => state.decided(&key),
            Proof::Again(why) => {
                state.again(&key);
                lines.push(format!("{id}: {why}"));
            }
            Proof::Down => lines.push(format!(
                "{id}: not checked, the live app refused the connection"
            )),
            Proof::Failed(failed) => {
                let why = failed.join("; ");
                match live.reopen(id, &why) {
                    Ok(()) => {
                        state.decided(&key);
                        lines.push(format!("{id}: open again, {why}"));
                    }
                    Err(e) => lines.push(format!("{id}: {why}; not reopened: {e}")),
                }
            }
        }
    }
    lines
}

/// An open after-merge request of to_process.
#[derive(Debug, PartialEq)]
pub struct Request {
    pub file: PathBuf,
    pub task: String,
    pub prs: Vec<String>,
    /// Each line to check: its « Part N » and its report's « After merge ».
    pub lines: Vec<String>,
}

/// The request in `text`, none when it is no after-merge request or is closed (a Done line) or waits on him (a Waiting line).
pub fn request(file: &Path, text: &str) -> Option<Request> {
    let task = text
        .strip_prefix(CHECKS_TITLE)?
        .split([':', '\n'])
        .next()?
        .trim();
    if text
        .lines()
        .any(|l| l.starts_with("**Done ") || l.starts_with("**Waiting "))
    {
        return None;
    }
    let prs = (text.lines().filter(|l| l.starts_with("- **PR:**")))
        .flat_map(|l| PR.find_iter(l).map(|m| m.as_str().to_string()))
        .collect();
    let lines = (text.lines())
        .filter(|l| l.starts_with("- **Part ") || l.starts_with("- **Its report's After merge:**"))
        .filter_map(|l| l.split_once(":** ").map(|(_, t)| t.trim().to_string()))
        .collect();
    Some(Request {
        file: file.to_path_buf(),
        task: task.to_string(),
        prs,
        lines,
    })
}

/// The open after-merge requests in `folder`, by name.
pub fn requests(folder: &Path) -> Vec<Request> {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return vec![];
    };
    let mut files: Vec<PathBuf> = (entries.flatten())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "md"))
        .collect();
    files.sort();
    (files.iter())
        .filter_map(|f| request(f, &std::fs::read_to_string(f).ok()?))
        .collect()
}

/// The live app's hosts, as a URL starts.
const LIVE_URLS: [&str; 3] = [
    "http://127.0.0.1:7788/",
    "http://localhost:7788/",
    "http://100.72.158.115:7788/",
];

/// `words` as a read-only command to run, or none: one of the readers below with no option that writes, sends, uploads,
/// follows or opens a browser. curl reads the live app alone and gets `-f`, so a page answering 404 exits non-zero (/code-review:
/// a curl prefix rule let `-X POST`, `-o <file>` and `-T` through, and curl exits 0 on a 404). Words go to the program as they
/// are, never through a shell. The one door: the Jev path runs it, and the agent's only command is `dash live-check read`.
pub fn read_only(words: &[String]) -> Option<Vec<String>> {
    let w: Vec<&str> = words.iter().map(String::as_str).collect();
    let none_of = |bad: &[&str]| !w.iter().any(|a| bad.iter().any(|b| a.starts_with(b)));
    match w[..] {
        ["curl", ref rest @ ..] => {
            let (flags, urls): (Vec<&str>, Vec<&str>) =
                rest.iter().partition(|a| a.starts_with('-'));
            let flags_ok = flags
                .iter()
                .all(|f| ["-s", "-S", "-sS", "-I", "-sI", "-i", "-si"].contains(f));
            let url_ok = matches!(urls[..], [u] if LIVE_URLS.iter().any(|l| u.starts_with(l)));
            (flags_ok && url_ok).then(|| {
                let mut argv = vec![
                    "curl".to_string(),
                    "-f".into(),
                    "--max-time".into(),
                    "15".into(),
                ];
                argv.extend(rest.iter().map(|a| a.to_string()));
                argv
            })
        }
        ["git", "log" | "show" | "cat-file" | "merge-base", ..]
            if none_of(&["--output", "--ext-diff", "-o"]) =>
        {
            Some(words.to_vec())
        }
        ["gh", "pr", "view" | "checks", ..] | ["gh", "run", "view" | "list", ..]
            if none_of(&["--web", "-w"]) =>
        {
            Some(words.to_vec())
        }
        ["tail", ..] if none_of(&["-f", "-F", "--follow"]) => Some(words.to_vec()),
        ["pgrep", ..] | ["grep", ..] => Some(words.to_vec()),
        ["dash", "task" | "draft" | "contact" | "block", "show", ..]
        | ["dash", "store", "text" | "files", ..]
        | ["dash", "orch", "next"] => Some(words.to_vec()),
        _ => None,
    }
}

/// The first read-only command in backquotes in `text`; one with a quote or shell syntax is none, its words unknown.
fn command_of(text: &str) -> Option<Vec<String>> {
    let plain = |c: &&str| !c.contains([';', '|', '&', '<', '>', '$', '\'', '"', '\n', '\\']);
    (text.split('`').skip(1).step_by(2))
        .filter(plain)
        .find_map(|c| read_only(&c.split_whitespace().map(String::from).collect::<Vec<_>>()))
}

/// A read-only command run from `repo`, `dash` for its `dash`: whether it exited 0, and what it printed.
fn run_read(argv: &[String], repo: &Path, dash: &str) -> Result<(bool, String), String> {
    let bin = match argv[0].as_str() {
        "dash" => dash,
        other => other,
    };
    let args: Vec<&str> = argv[1..].iter().map(String::as_str).collect();
    let (status, out, err) =
        dash_process::answer(bin, &args, Some(repo), &[], COMMAND_CALL).map_err(String::from)?;
    Ok((status.success(), format!("{out}{err}")))
}

/// An after-merge request's verdict.
#[derive(Debug, PartialEq)]
pub enum Verdict {
    /// The outcome holds: what decided it.
    Met(String),
    /// It does not: what was printed.
    NotMet(String),
    /// Nothing to check any more, and why: the request closes, the task as it is.
    Moot(String),
    /// Not yet: why.
    Wait(String),
}

/// The agent's verdict in its words: one JSON object `{met, printed}`, maybe inside other text.
fn agent_verdict(said: &str) -> Result<Verdict, String> {
    let (start, end) = (said.find('{'), said.rfind('}'));
    let (Some(a), Some(b)) = (start, end) else {
        return Err(format!(
            "no JSON in the agent's answer: {}",
            said.chars().take(200).collect::<String>()
        ));
    };
    let v: Value =
        serde_json::from_str(&said[a..=b]).map_err(|e| format!("the agent's JSON: {e}"))?;
    let printed = v["printed"].as_str().unwrap_or_default().to_string();
    // null: it could not read what decides it, which is no verdict: asked again, then Waiting, never a reopen (part 5's run
    // reopened flare-memory-cap on « Bash was denied »)
    match v["met"].as_bool() {
        Some(true) => Ok(Verdict::Met(printed)),
        Some(false) => Ok(Verdict::NotMet(printed)),
        None if v["met"].is_null() && !printed.is_empty() => {
            Err(format!("the agent could not check: {printed}"))
        }
        None => Err("the agent's JSON has no met".into()),
    }
}

/// The agent's prompt for request `r` of a task whose done-when is `done_when`; `dash` is the dash it may read with.
fn agent_prompt(r: &Request, done_when: &str, dash: &str) -> String {
    format!(
        "You check whether an after-merge outcome of task {task} holds now on Nicolas's Mac and his live app ({LIVE}). Its PRs \
         merged: {prs}. Read only: never send, write, reload, restart, install or ask anyone; a step the lines ask that nobody \
         took means the outcome does not hold. Read files with your tools; run a command only as `{dash} live-check read \
         <command>`, which runs a reader and refuses the rest: curl -s on {LIVE}/..., git log or show, gh pr view or checks, \
         gh run list or view, pgrep, grep, tail, and dash task show, store text, draft show or orch next.\n\nThe task's done-when: {done_when}\n\n\
         The lines to check:\n{lines}\n\nAnswer with one JSON object and nothing else: {{\"met\": true or false, \"printed\": \
         \"<the command or file you read and what it printed that decides it, at most 600 characters>\"}}. If you could not \
         read what decides it (a command refused, a file out of reach), answer {{\"met\": null, \"printed\": \"<what you \
         could not read, and why>\"}}: never false for want of evidence.",
        task = r.task,
        prs = if r.prs.is_empty() {
            "none named".into()
        } else {
            r.prs.join(", ")
        },
        lines = r
            .lines
            .iter()
            .map(|l| format!("- {l}"))
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

/// What starts an agent's answer that no login had room: the request waits, its tries untouched.
pub const NO_ROOM: &str = "no login with room";

/// `r` as its task's own: a reviewer's request (`review-<id>`), whose run left no task on the list, is checked as `<id>`'s (8 of
/// the 26 open on 2026-11-12 were).
fn parent(r: &Request, live: &mut impl Live) -> Request {
    let own = |id: &str| Request {
        file: r.file.clone(),
        task: id.to_string(),
        prs: r.prs.clone(),
        lines: r.lines.clone(),
    };
    match r.task.strip_prefix("review-") {
        Some(id) if matches!(live.task(&r.task), Ok(None)) => own(id),
        _ => own(&r.task),
    }
}

/// Request `r` judged: its PRs merged, its task still listed, then Jev first, then a page, a command or the agent.
pub fn judge(
    r: &Request,
    live: &mut impl Live,
    dash: &str,
    last_try: bool,
) -> Result<Verdict, String> {
    let mut merged = false;
    for url in &r.prs {
        match live.pr_state(url)?.as_str() {
            "OPEN" => return Ok(Verdict::Wait(format!("{url} is still open"))),
            "MERGED" => merged = true,
            _ => {}
        }
    }
    if !r.prs.is_empty() && !merged {
        return Ok(Verdict::Moot(format!(
            "its PRs closed unmerged ({}): nothing merged to check",
            r.prs.join(", ")
        )));
    }
    let task = match live.task(&r.task)? {
        None => return Ok(Verdict::Moot(format!("no task {} on the list", r.task))),
        Some(t) if t.stage == "dropped" => {
            return Ok(Verdict::Moot(format!("task {} was dropped", r.task)));
        }
        // no PR, and the task open again: its next run's report names its after-merge parts afresh (rust-queue-fill, 2026-11-12)
        Some(t) if r.prs.is_empty() && t.stage != "done" => {
            return Ok(Verdict::Moot(format!(
                "no PR named and task {} is {} again: its next report names its after-merge parts",
                r.task, t.stage
            )));
        }
        Some(t) => t,
    };
    let text = r.lines.join("\n");
    if r.lines.is_empty() {
        return Ok(Verdict::Moot("no line to check".into()));
    }
    if live.jev(&r.task, &text)? >= 0.5 {
        let pages: Vec<(usize, String)> = paths(&text).into_iter().map(|p| (1, p)).collect();
        if !pages.is_empty() {
            return Ok(match check(&pages, live, last_try) {
                Proof::Held => Verdict::Met(format!(
                    "each page answered: {}",
                    pages
                        .iter()
                        .map(|p| p.1.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )),
                Proof::Failed(f) => Verdict::NotMet(f.join("; ").replace("part 1: ", "")),
                Proof::Again(why) => Verdict::Wait(why),
                Proof::Down => Verdict::Wait("the live app refused the connection".into()),
            });
        }
        if let Some(argv) = command_of(&text) {
            let (ok, printed) = live.run(&argv)?;
            let said = format!(
                "{}: {}",
                argv.join(" "),
                printed.trim().chars().take(600).collect::<String>()
            );
            return Ok(if ok {
                Verdict::Met(said)
            } else {
                Verdict::NotMet(said)
            });
        }
    }
    agent_verdict(&live.agent(&agent_prompt(r, &task.done_when, dash))?)
}

/// A request's Done line, on one line.
fn done_line(day: &str, note: &str) -> String {
    let note = note.replace('\n', " ");
    format!("**Done {day} by Rust's after-merge check (dash live-check):** {note}")
}

/// `text` with `line` under its title.
fn under_title(text: &str, line: &str) -> String {
    match text.split_once("\n\n") {
        Some((title, rest)) => format!("{title}\n\n{line}\n\n{rest}"),
        None => format!("{}\n\n{line}\n", text.trim_end()),
    }
}

fn close(file: &Path, day: &str, note: &str) -> Result<(), String> {
    marked(file, &done_line(day, note))
}

/// Request `file` with `line` under its title, written whole.
fn marked(file: &Path, line: &str) -> Result<(), String> {
    let text = std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
    let tmp = file.with_extension("md.tmp");
    std::fs::write(&tmp, under_title(&text, &line.replace('\n', " ")))
        .map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, file).map_err(|e| format!("{}: {e}", file.display()))
}

fn name(file: &Path) -> String {
    file.file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
}

/// The after-merge pass over `folder`'s open requests, least asked first: lines saying what it did.
pub fn after_merge(
    folder: &Path,
    live: &mut impl Live,
    state: &mut State,
    day: &str,
    dash: &str,
) -> Vec<String> {
    let mut open = requests(folder);
    open.sort_by_key(|r| state.tries.get(&name(&r.file)).copied().unwrap_or(0));
    let mut lines = vec![];
    let mut judged = 0;
    for r in &open {
        // a request waiting on no room takes none of the pass's turns, so one never holds the others off
        if judged == REQUESTS_PER_PASS {
            break;
        }
        judged += 1;
        let key = name(&r.file);
        let r = &parent(r, live);
        let tried = state.tries.get(&key).copied().unwrap_or(0);
        let verdict = match judge(r, live, dash, tried + 1 >= TRIES) {
            Ok(v) => v,
            // no login with room is no failed check: asked again, never counted (the pass of 2026-11-12 hit a weekly limit)
            Err(e) if e.starts_with(NO_ROOM) => {
                judged -= 1;
                lines.push(format!("{key}: waits: {e}"));
                continue;
            }
            // a paid call failing every pass is no longer asked: it waits on whoever reads to_process (/code-review)
            Err(e) if tried + 1 >= TRIES => {
                let why = format!("Rust's after-merge check failed {TRIES} times, the last: {e}");
                match marked(&r.file, &format!("**Waiting {day}:** {why}")) {
                    Ok(()) => state.decided(&key),
                    Err(w) => lines.push(format!("{key}: not marked waiting: {w}")),
                }
                lines.push(format!("{key}: waits: {why}"));
                continue;
            }
            Err(e) => {
                state.again(&key);
                lines.push(format!("{key}: not judged: {e}"));
                continue;
            }
        };
        let note = match &verdict {
            Verdict::Wait(why) => {
                state.again(&key);
                lines.push(format!("{key}: waits: {why}"));
                continue;
            }
            Verdict::Met(said) => format!("met: {said}"),
            Verdict::Moot(why) => why.clone(),
            Verdict::NotMet(said) => {
                let why = format!("after merge not met: {said}");
                if let Err(e) = live.reopen(&r.task, &why) {
                    state.again(&key);
                    lines.push(format!("{key}: not met, {} not reopened: {e}", r.task));
                    continue;
                }
                format!("not met, task {} reopened: {said}", r.task)
            }
        };
        match close(&r.file, day, &note) {
            Ok(()) => {
                state.decided(&key);
                lines.push(format!("{key}: {note}"));
            }
            Err(e) => lines.push(format!("{key}: {note}; not closed: {e}")),
        }
    }
    lines
}

/// The newest merge of each task on `rev`'s first parents in the window, `id -> (sha, time)`; a revert never counts.
fn merges(repo: &Path, rev: &str, now_s: i64) -> Result<BTreeMap<String, (String, i64)>, String> {
    let since = format!("--since={}", now_s - WINDOW_S);
    let args = [
        "-C",
        &repo.display().to_string(),
        "log",
        "--first-parent",
        &since,
        "--format=%H%x09%ct%x09%s",
        rev,
    ]
    .map(String::from);
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let out = dash_process::run("git", &args, None, COMMAND_CALL)?;
    let mut by_task = BTreeMap::new();
    for line in out.lines() {
        let mut it = line.splitn(3, '\t');
        let (Some(sha), Some(at), Some(subject)) = (it.next(), it.next(), it.next()) else {
            continue;
        };
        if subject.starts_with("Revert ") {
            continue;
        }
        if let (Some(id), Ok(at)) = (crate::tasks::task_id_from_title(subject), at.parse()) {
            by_task.entry(id).or_insert((sha.to_string(), at));
        }
    }
    Ok(by_task)
}

/// The binary's ports: the live app over curl, the older app's list, gh, Jev and Claude Code. CI and a test never pay.
struct Real {
    repo: PathBuf,
    db: PathBuf,
    dash: String,
    /// This binary, whose `live-check read` is the agent's one command.
    me: String,
    /// His home, where each login's folder and limit readings are.
    home: PathBuf,
    /// The logins that answered their limit this pass: never asked again in it.
    full: std::collections::BTreeSet<String>,
}

/// The logins an agent may run on, in the order it tries them: those with room by their week's reset, first first, then the
/// rest, since a kept reading can be stale (2026-11-12: the default login read as having room and answered its weekly limit).
/// Each is its name and config folder, none for the default login.
fn logins(home: &Path, now: u64) -> Vec<(String, Option<PathBuf>)> {
    let claude = home.join(".claude");
    let limits = crate::tasks::claude_code(&home.display().to_string())
        .limits_at(now, 200)
        .unwrap_or_default();
    let room = |name: &str| {
        (limits.iter())
            .find(|l| !l.key && l.account == name)
            .map(|l| (!l.room(), l.week_resets))
    };
    let mut names = dash_agent::logins(&claude);
    names.sort_by_key(|n| room(n).unwrap_or((false, u64::MAX)));
    (names.into_iter())
        .map(|n| {
            let dir = (n != "default").then(|| claude.join("accounts").join(&n));
            (n, dir)
        })
        .collect()
}

/// Whether a headless run's `out` is its login's limit answer (429, usage_limit_reached), and its words.
fn limit_reached(out: &str) -> Option<String> {
    let answer: Value = serde_json::from_str(out.trim()).unwrap_or_default();
    let result = (answer.as_array().into_iter().flatten())
        .rfind(|e| e["type"] == "result")
        .unwrap_or(&answer);
    (result["api_error"] == "usage_limit_reached" || result["api_error_status"] == 429)
        .then(|| result["result"].as_str().unwrap_or_default().to_string())
}

impl Live for Real {
    fn get(&mut self, path: &str, head: bool) -> Got {
        let url = format!("{LIVE}{path}");
        let mut args = vec!["-sS", "--max-time", "15", "-w", "\n%{http_code}"];
        if head {
            args.extend(["-I", "-o", "/dev/null"]);
        }
        args.push(&url);
        match dash_process::answer("curl", &args, None, &[], Duration::from_secs(20)) {
            Ok((status, text, _)) if status.success() => {
                let (body, code) = text.rsplit_once('\n').unwrap_or(("", text.trim()));
                match code.trim().parse() {
                    Ok(code) => Got::Status(code, body.to_string()),
                    Err(_) => Got::Failed(format!("no status in curl's answer: {code}")),
                }
            }
            Ok((status, _, reason)) => match status.code() {
                Some(7) => Got::Down,
                Some(28) => Got::Slow,
                _ => Got::Failed(reason.trim().to_string()),
            },
            Err(e) => Got::Failed(e.why),
        }
    }

    fn task(&mut self, id: &str) -> Result<Option<Task>, String> {
        let Some((stage, _)) = dash_store::older::stage(&self.db, id)? else {
            return Ok(None);
        };
        let done_when = (dash_store::older::brief(&self.db, id)?)
            .map(|f| f.done_when)
            .unwrap_or_default();
        let report = dash_store::older::last_report(&self.db, id)?;
        let record: Value = report
            .and_then(|(_, r)| serde_json::from_str(&r).ok())
            .unwrap_or_default();
        let checks = record["checks"].as_array().cloned().unwrap_or_default();
        Ok(Some(Task {
            stage,
            done_when,
            checks,
        }))
    }

    fn pr_state(&mut self, url: &str) -> Result<String, String> {
        let out = dash_process::run(
            "gh",
            &["pr", "view", url, "--json", "state", "-q", ".state"],
            None,
            COMMAND_CALL,
        )?;
        Ok(out.trim().to_string())
    }

    fn jev(&mut self, task: &str, lines: &str) -> Result<f64, String> {
        let questions: Value = serde_json::from_str(JEV_QUESTIONS).expect("the questions are JSON");
        let answer = jev::ask(
            &questions,
            json!({"task": task, "lines": lines}),
            "dash-live-check",
        )?;
        jev::noul(&answer, "by_command")
    }

    fn run(&mut self, argv: &[String]) -> Result<(bool, String), String> {
        run_read(argv, &self.repo, &self.dash)
    }

    fn agent(&mut self, prompt: &str) -> Result<String, String> {
        if let Some(no) = jev::paid_refused() {
            return Err(no);
        }
        let claude = std::env::var("DASH_CLAUDE").unwrap_or_else(|_| "claude".into());
        let allowed = agent_allowed(&self.me);
        let mut args: Vec<&str> = vec![
            "--model",
            AGENT_MODEL,
            "--restricted",
            "--tools",
            "Read,Grep,Glob,Bash",
        ];
        args.extend(["--permission-mode", "dontAsk", "--allowedTools"]);
        args.extend(allowed.iter().map(String::as_str));
        args.extend([
            "--max-budget-usd",
            AGENT_BUDGET,
            "--output-format",
            "json",
            "-p",
            prompt,
        ]);
        // each login in turn, as a task's start picks (rfc-rust-unified 20h), till one answers; one at its limit is skipped for
        // the pass. Through env as `dash_agent::usage` runs, `-u` for the default login, whose keychain item a set
        // CLAUDE_CONFIG_DIR misses
        let mut said = vec![];
        for (name, dir) in logins(&self.home, crate::tasks::now_ms() as u64 / 1000) {
            if self.full.contains(&name) {
                continue;
            }
            let set = dir.map(|d| format!("CLAUDE_CONFIG_DIR={}", d.display()));
            let mut words: Vec<&str> = match &set {
                Some(set) => vec![set],
                None => vec!["-u", "CLAUDE_CONFIG_DIR"],
            };
            words.push(&claude);
            words.extend(&args);
            let (_, out, reason) =
                dash_process::answer("env", &words, Some(&self.repo), &[], AGENT_CALL)
                    .map_err(String::from)?;
            if let Some(words) = limit_reached(&out) {
                said.push(format!("{name}: {words}"));
                self.full.insert(name);
                continue;
            }
            return dash_agent::headless_outcome(&out)
                .map(|(said, _)| said)
                .map_err(|e| format!("{e}; {}", reason.trim()));
        }
        Err(format!("{NO_ROOM}: {}", said.join("; ")))
    }

    fn reopen(&mut self, id: &str, why: &str) -> Result<(), String> {
        let app = crate::tasks::older_app();
        if dash_store::older::stage(&self.db, id)?.is_some_and(|(s, _)| s == "done") {
            // `task reopen` takes a Rust task alone; the older app's own is listed again by its status, its next run the
            // picker's, never `task restart`, which starts one at once
            // ponytail: told apart by the app's refusal; an owner reader in dash_store::older when a second caller needs it
            match app.reopen(id) {
                Err(e) if e.contains("its owner is not rust") => app.reopen_older(id)?,
                other => other?,
            }
        }
        app.live_failed(id, why)
    }

    fn moved(&mut self, sha: &str, file: &str) -> bool {
        let at = |rev: &str| {
            self.git(&["cat-file", "-e", &format!("{rev}:{file}")])
                .is_ok()
        };
        at(sha) && !at("origin/main")
    }

    fn serves(&mut self, sha: &str) -> Option<bool> {
        let current = std::fs::read_to_string(self.repo.join(".claude/run/live/current")).ok()?;
        let current = current.trim();
        let named = current.len() == 40 && current.bytes().all(|b| b.is_ascii_hexdigit());
        named.then(|| {
            self.git(&["merge-base", "--is-ancestor", sha, current])
                .is_ok()
        })
    }
}

impl Real {
    fn git(&self, args: &[&str]) -> Result<String, String> {
        let repo = self.repo.display().to_string();
        let args: Vec<&str> = ["-C", repo.as_str()]
            .into_iter()
            .chain(args.iter().copied())
            .collect();
        dash_process::run("git", &args, None, COMMAND_CALL)
    }
}

/// What the agent may use: the file readers, and one command, this binary's `live-check read`, which runs `read_only`'s readers
/// and refuses the rest. Its tools are `--restricted`, so his settings' allow rules never widen this (/code-review: a prefix
/// rule such as `Bash(curl -s:*)` let a POST through).
fn agent_allowed(me: &str) -> Vec<String> {
    let mut allowed: Vec<String> = ["Read", "Grep", "Glob"].map(String::from).to_vec();
    allowed.push(format!("Bash({me} live-check read:*)"));
    allowed
}

/// Jev's question on an after-merge request's lines, a 0 to 1 « noul » (crate::jev, the one client).
const JEV_QUESTIONS: &str = r#"{
  "by_command": {"type": "noul",
    "instructions": "`lines` say what should hold on Nicolas's live app once task `task` merged. Does a live-app link (http://127.0.0.1:7788/...) or a command in backquotes that `lines` name decide it alone: its page answering, or the command exiting 0, means it holds, with no other reading or judgement?",
    "criteria": {"true": "Fetching the named page or running the named command alone decides it.", "false": "Deciding it needs reading something else, judging what is printed, or waiting for an event or someone's action."}}
}"#;

/// One pass: proofs of the merges on origin/main, then the after-merge requests. A line saying what it did.
pub fn pass(dir: &Path) -> Result<String, String> {
    let repo = crate::tasks::repo();
    let now_s = crate::tasks::now_ms() / 1000;
    let dash = repo
        .join("private/dashboard/bin/dash")
        .display()
        .to_string();
    let me = std::env::current_exe().map_err(|e| format!("this binary: {e}"))?;
    let mut live = Real {
        repo: repo.clone(),
        db: crate::tasks::older_db(),
        dash: dash.clone(),
        me: me.display().to_string(),
        home: PathBuf::from(std::env::var("HOME").map_err(|e| format!("HOME: {e}"))?),
        full: Default::default(),
    };
    let mut state = State::read(dir);
    let mut lines = match merges(&repo, "origin/main", now_s) {
        Ok(m) => prove(&m, &mut live, &mut state, now_s),
        Err(e) => vec![format!("main's history not read: {e}")],
    };
    let day = crate::audit::today(now_s * 1000);
    let me = live.me.clone();
    lines.extend(after_merge(
        &repo.join("to_process"),
        &mut live,
        &mut state,
        &day,
        &me,
    ));
    state.write(dir)?;
    Ok(match lines.is_empty() {
        true => "nothing to check".into(),
        false => lines.join("; "),
    })
}

/// The pass on a thread of its own, at most once per `EVERY`, so the comment beat never waits on the live app, Jev or the agent.
pub fn in_background(dir: &Path) -> String {
    static LIVE_PASS: crate::inbox::Background = crate::inbox::Background::new(EVERY);
    crate::inbox::on_own_thread(&LIVE_PASS, dir, |d| {
        pass(d).unwrap_or_else(|e| format!("failed: {e}"))
    })
}

inventory::submit! { crate::usage::Command::usage("dash live-check read <command>...") }
/// `dash live-check read <command>...`: the command if `read_only` passes it, its output and exit; else refused, exit 2.
pub fn read(words: &[&str], mut out: std::fs::File) {
    use std::io::Write;
    let words: Vec<String> = words.iter().map(|w| w.to_string()).collect();
    let Some(argv) = read_only(&words) else {
        eprintln!(
            "live-check read: refused, not a reader: {}",
            words.join(" ")
        );
        std::process::exit(2);
    };
    let repo = crate::tasks::repo();
    let dash = repo
        .join("private/dashboard/bin/dash")
        .display()
        .to_string();
    match run_read(&argv, &repo, &dash) {
        Ok((ok, printed)) => {
            write!(out, "{printed}").expect("stdout");
            std::process::exit(if ok { 0 } else { 1 });
        }
        Err(e) => {
            eprintln!("live-check read: {e}");
            std::process::exit(1);
        }
    }
}

inventory::submit! { crate::usage::Command::usage("dash live-check pass --data <dir>") }
pub fn once(dir: &Path, mut out: std::fs::File) {
    use std::io::Write;
    match pass(dir) {
        Ok(line) => writeln!(out, "{line}").expect("stdout"),
        Err(e) => {
            eprintln!("live-check: {e}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;

    /// A live app, list, forge, Jev and agent from tables; what it was asked to reopen, kept.
    #[derive(Default)]
    struct Fake {
        pages: BTreeMap<String, Got>,
        tasks: BTreeMap<String, (String, String, Vec<Value>)>,
        prs: BTreeMap<String, String>,
        jev: f64,
        command: Option<(bool, String)>,
        /// The agent's answers, one per call, in order.
        agent: Vec<String>,
        ran: Vec<Vec<String>>,
        asked_agent: Vec<String>,
        reopened: Vec<(String, String)>,
        /// Files a later commit moved, and the build served: `None` when none is named.
        moved: Vec<String>,
        served: Option<bool>,
    }

    impl Live for Fake {
        fn get(&mut self, path: &str, _head: bool) -> Got {
            self.pages
                .get(path)
                .cloned()
                .unwrap_or(Got::Status(404, String::new()))
        }
        fn task(&mut self, id: &str) -> Result<Option<Task>, String> {
            Ok(self.tasks.get(id).map(|(s, d, c)| Task {
                stage: s.clone(),
                done_when: d.clone(),
                checks: c.clone(),
            }))
        }
        fn pr_state(&mut self, url: &str) -> Result<String, String> {
            self.prs.get(url).cloned().ok_or(format!("{url}: unknown"))
        }
        fn jev(&mut self, _task: &str, _lines: &str) -> Result<f64, String> {
            Ok(self.jev)
        }
        fn run(&mut self, argv: &[String]) -> Result<(bool, String), String> {
            self.ran.push(argv.to_vec());
            self.command.clone().ok_or("no command expected".into())
        }
        fn agent(&mut self, prompt: &str) -> Result<String, String> {
            self.asked_agent.push(prompt.to_string());
            match self.agent.is_empty() {
                true => Err("no agent expected".into()),
                false => Ok(self.agent.remove(0)),
            }
        }
        fn reopen(&mut self, id: &str, why: &str) -> Result<(), String> {
            self.reopened.push((id.into(), why.into()));
            if let Some(t) = self.tasks.get_mut(id) {
                t.0 = "open".into();
            }
            Ok(())
        }
        fn moved(&mut self, _sha: &str, file: &str) -> bool {
            self.moved.iter().any(|m| m == file)
        }
        fn serves(&mut self, _sha: &str) -> Option<bool> {
            self.served
        }
    }

    fn ok(body: &str) -> Got {
        Got::Status(200, body.into())
    }

    fn merged(id: &str, sha: &str, at: i64) -> BTreeMap<String, (String, i64)> {
        BTreeMap::from([(id.to_string(), (sha.to_string(), at))])
    }

    const NOW: i64 = 1_800_000_000;

    /// Done-when 1: after a merge, the pass fetches each done-when part's proof on the live app; a failed one reopens the task,
    /// naming the part; one that holds is decided and never asked again.
    #[dash_test]
    fn a_merged_task_s_failed_part_reopens_it_naming_the_part() {
        let mut live = Fake::default();
        let done_when = "1. dash task show names it 2. The page http://127.0.0.1:7788/projects/x answers 3. Report at private/docs/x.html";
        live.tasks
            .insert("t".into(), ("done".into(), done_when.into(), vec![]));
        live.pages.insert("/projects/x".into(), ok("fine"));
        let mut state = State::default();
        let lines = prove(&merged("t", "abc", NOW - 3600), &mut live, &mut state, NOW);
        assert_eq!(
            live.reopened,
            vec![(
                "t".to_string(),
                "part 3: /v/private/docs/x.html answered 404".to_string()
            )],
            "part 3's page answered 404, part 2's held"
        );
        assert_eq!(
            lines,
            vec!["t: open again, part 3: /v/private/docs/x.html answered 404"]
        );
        assert_eq!(state.checked, vec!["t:abc"], "decided once per merge");
        // the same merge is never asked again; a merge too fresh for the live app's swap waits
        live.reopened.clear();
        prove(&merged("t", "abc", NOW - 3600), &mut live, &mut state, NOW);
        prove(&merged("t", "def", NOW - 60), &mut live, &mut state, NOW);
        assert!(live.reopened.is_empty());
        // all held: decided, nothing reopened
        live.tasks.insert(
            "u".into(),
            (
                "done".into(),
                "1. http://127.0.0.1:7788/projects/x answers".into(),
                vec![],
            ),
        );
        prove(&merged("u", "0e1", NOW - 3600), &mut live, &mut state, NOW);
        assert!(live.reopened.is_empty() && state.checked[0] == "u:0e1");
    }

    /// Done-when 2: voice-clone-local's case, its listening page answering 200 while each audio src answers 404, reopens it naming
    /// part 3 and the sample; the report's check command names the page too.
    #[dash_test]
    fn voice_clone_local_s_404_samples_reopen_it() {
        let mut live = Fake::default();
        let done_when = "1. Pocket TTS runs on this Mac. 2. His voice cloned from a sample. 3. The same 4 sentences in French and English, audio files and a listening page. 4. Compared with Kitten TTS.";
        let checks = vec![
            json!({"part": "3", "command": "open private/docs/voice-clone-local.html", "output": "16 players"}),
        ];
        live.tasks.insert(
            "voice-clone-local".into(),
            ("done".into(), done_when.into(), checks),
        );
        let page = include_str!("../tests/fixtures/voice-clone-local.html");
        live.pages
            .insert("/v/private/docs/voice-clone-local.html".into(), ok(page));
        let mut state = State::default();
        prove(
            &merged("voice-clone-local", "248", NOW - 3600),
            &mut live,
            &mut state,
            NOW,
        );
        let (id, why) = &live.reopened[0];
        assert_eq!(id, "voice-clone-local");
        assert_eq!(
            why,
            "part 3: /v/private/docs/voice-clone-local.html: /f/private/interview/video/voice/out/pocket-fr-1.m4a answered 404; \
             part 3: /v/private/docs/voice-clone-local.html: /v/private/interview/video/voice/out/kitten-fr-1.m4a answered 404"
        );
    }

    /// A refused connection decides nothing; a timeout is asked again, and fails on its third pass.
    #[dash_test]
    fn a_down_app_decides_nothing_and_a_slow_page_fails_on_its_third_try() {
        let mut live = Fake::default();
        live.tasks.insert(
            "t".into(),
            (
                "done".into(),
                "1. http://127.0.0.1:7788/slow answers".into(),
                vec![],
            ),
        );
        live.pages.insert("/slow".into(), Got::Down);
        let mut state = State::default();
        prove(&merged("t", "a", NOW - 3600), &mut live, &mut state, NOW);
        assert!(live.reopened.is_empty() && state.checked.is_empty());
        live.pages.insert("/slow".into(), Got::Slow);
        for _ in 0..2 {
            prove(&merged("t", "a", NOW - 3600), &mut live, &mut state, NOW);
            assert!(live.reopened.is_empty());
        }
        prove(&merged("t", "a", NOW - 3600), &mut live, &mut state, NOW);
        assert_eq!(live.reopened[0].1, "part 1: /slow did not answer in time");
    }

    #[dash_test]
    fn proofs_skip_a_part_asking_for_a_page_gone_and_a_check_that_writes() {
        let done_when = "1. /x is removed: http://127.0.0.1:7788/x answers 404 2. http://localhost:7788/y. answers, no 404";
        let checks = vec![
            json!({"part": 2, "command": "curl -X POST http://127.0.0.1:7788/write"}),
            json!({"part": "parts 1, 2", "command": "curl -s http://127.0.0.1:7788/z"}),
        ];
        assert_eq!(
            proofs(done_when, &checks),
            vec![(2, "/y".into()), (2, "/z".into())]
        );
        assert_eq!(
            paths("see ~/repo/site_perso/private/docs/a.html and x/private/b.md"),
            vec!["/v/private/docs/a.html"]
        );
    }

    fn request_text(prs: &str, part: &str) -> String {
        format!(
            "# After merge checks of task t: part 3\n\nFrom the dashboard's PM check: task t is met on every part but these.\n\n\
             - **PR:** {prs}\n- **Part 3:** {part}\n- **How:** the orchestrator skill.\n"
        )
    }

    fn folder(files: &[(&str, &str)]) -> dash_meter::TestDir {
        let dir = dash_meter::TestDir::new("after-merge");
        for (name, text) in files {
            std::fs::write(dir.0.join(name), text).expect("write");
        }
        dir
    }

    const PR1: &str = "https://github.com/nha-org/site_perso/pull/1";

    #[dash_test]
    fn an_after_merge_request_reads_its_task_prs_and_lines_and_closed_ones_are_skipped() {
        let text = request_text(
            &format!("{PR1} ready for review; https://github.com/nha/x/pull/2 ready"),
            "After merge: it holds",
        );
        let text = text.replace(
            "- **How:**",
            "- **Its report's After merge:** 3. the page shows it\n- **How:**",
        );
        let r = request(Path::new("f.md"), &text).expect("a request");
        assert_eq!(r.task, "t");
        assert_eq!(
            r.prs,
            vec![PR1.to_string(), "https://github.com/nha/x/pull/2".into()]
        );
        assert_eq!(
            r.lines,
            vec!["After merge: it holds", "3. the page shows it"]
        );
        assert!(
            request(
                Path::new("f.md"),
                &under_title(&text, &done_line("2026-11-12", "met"))
            )
            .is_none()
        );
        assert!(
            request(
                Path::new("f.md"),
                &text.replace("\n\nFrom", "\n\n**Waiting 2026-11-12:** him\n\nFrom")
            )
            .is_none()
        );
        assert!(request(Path::new("f.md"), "# Answer task t\n").is_none());
    }

    /// Done-when 3: Jev first; a line Jev says a named command decides runs that command, a page is fetched, the rest goes to
    /// the agent, whose read-only verdict is read.
    #[dash_test]
    fn jev_first_then_a_command_a_page_or_the_agent() {
        let dash = "/repo/private/dashboard/bin/dash";
        let mut live = Fake {
            jev: 0.9,
            command: Some((true, "id: t\nstatus: done".into())),
            ..Fake::default()
        };
        live.prs.insert(PR1.into(), "MERGED".into());
        live.tasks
            .insert("t".into(), ("done".into(), "1. a 2. b 3. c".into(), vec![]));
        let r = request(
            Path::new("f.md"),
            &request_text(PR1, "After merge: `dash task show t` prints status: done"),
        )
        .expect("r");
        assert_eq!(
            judge(&r, &mut live, dash, false),
            Ok(Verdict::Met("dash task show t: id: t\nstatus: done".into()))
        );
        assert_eq!(
            live.ran,
            vec![vec![
                "dash".to_string(),
                "task".into(),
                "show".into(),
                "t".into()
            ]]
        );
        // a page Jev says decides it is fetched
        let r = request(
            Path::new("f.md"),
            &request_text(PR1, "After merge: http://127.0.0.1:7788/plan lists it"),
        )
        .expect("r");
        assert_eq!(
            judge(&r, &mut live, dash, false),
            Ok(Verdict::NotMet("/plan answered 404".into()))
        );
        // Jev doubting: the agent, its prompt saying read only, its verdict read
        live.jev = 0.1;
        let met = "Checked.\n{\"met\": true, \"printed\": \"held.log: resumed once\"}";
        live.agent = vec![met.into(), met.into()];
        assert_eq!(
            judge(&r, &mut live, dash, false),
            Ok(Verdict::Met("held.log: resumed once".into()))
        );
        assert!(
            live.asked_agent[0].contains("Read only: never send")
                && live.asked_agent[0].contains("- After merge: http")
        );
        // a command that writes is never run: the agent decides
        live.jev = 0.9;
        let r = request(
            Path::new("f.md"),
            &request_text(PR1, "`dash draft set x --status :sent` then sent"),
        )
        .expect("r");
        judge(&r, &mut live, dash, false).expect("judged");
        assert_eq!(live.ran.len(), 1, "no second command ran");
        assert_eq!(live.asked_agent.len(), 2);
    }

    #[dash_test]
    fn an_open_pr_waits_a_closed_one_or_a_dropped_task_makes_it_moot() {
        let mut live = Fake::default();
        live.tasks
            .insert("t".into(), ("done".into(), String::new(), vec![]));
        let r = request(Path::new("f.md"), &request_text(PR1, "it holds")).expect("r");
        live.prs.insert(PR1.into(), "OPEN".into());
        assert!(matches!(
            judge(&r, &mut live, "", false),
            Ok(Verdict::Wait(_))
        ));
        live.prs.insert(PR1.into(), "CLOSED".into());
        assert!(matches!(
            judge(&r, &mut live, "", false),
            Ok(Verdict::Moot(_))
        ));
        live.prs.insert(PR1.into(), "MERGED".into());
        live.tasks
            .insert("t".into(), ("dropped".into(), String::new(), vec![]));
        assert_eq!(
            judge(&r, &mut live, "", false),
            Ok(Verdict::Moot("task t was dropped".into()))
        );
    }

    /// Done-when 4: a failed check reopens its task with what it printed and closes the request saying so; a met one closes
    /// its request, the task left done; a request whose check could not run stays open.
    #[dash_test]
    fn a_failed_check_reopens_its_task_with_what_it_printed_and_a_met_one_closes_it() {
        let failed = request_text(PR1, "After merge: the reply is sent");
        let met = failed.replace("task t:", "task m:");
        let dir = folder(&[
            ("a.md", &failed),
            ("b.md", &met),
            ("c.md", &failed.replace("task t:", "task gone:")),
        ]);
        let mut live = Fake {
            jev: 0.0,
            ..Fake::default()
        };
        live.prs.insert(PR1.into(), "MERGED".into());
        live.tasks
            .insert("t".into(), ("done".into(), String::new(), vec![]));
        live.tasks
            .insert("m".into(), ("done".into(), String::new(), vec![]));
        live.agent = vec![
            "{\"met\": false, \"printed\": \"dash draft show r: status approved, not sent\"}"
                .into(),
            "{\"met\": true, \"printed\": \"get_conversation shows the reply\"}".into(),
        ];
        let mut state = State::default();
        let lines = after_merge(&dir.0, &mut live, &mut state, "2026-11-12", "dash");
        assert_eq!(
            lines,
            vec![
                "a.md: not met, task t reopened: dash draft show r: status approved, not sent",
                "b.md: met: get_conversation shows the reply",
                "c.md: no task gone on the list",
            ]
        );
        assert_eq!(
            live.reopened,
            vec![(
                "t".into(),
                "after merge not met: dash draft show r: status approved, not sent".into()
            )]
        );
        let a = std::fs::read_to_string(dir.0.join("a.md")).expect("a");
        assert!(a.starts_with(
            "# After merge checks of task t: part 3\n\n**Done 2026-11-12 by Rust's after-merge check (dash live-check):** not \
             met, task t reopened: dash draft show r: status approved, not sent\n\nFrom"
        ));
        assert_eq!(live.tasks["m"].0, "done", "a met one leaves its task done");
        assert!(requests(&dir.0).is_empty(), "each closed");
        // an agent that fails decides nothing: the request stays open and is asked again
        std::fs::write(dir.0.join("d.md"), &failed).expect("d");
        let lines = after_merge(&dir.0, &mut live, &mut state, "2026-11-12", "dash");
        assert_eq!(lines, vec!["d.md: not judged: no agent expected"]);
        assert_eq!(requests(&dir.0).len(), 1);
    }

    /// Part 5's run, 2026-11-12: a reviewer's request is checked, and reopened, as its task's; a weekly limit on every login is
    /// no failed check, so it never counts toward the Waiting mark.
    #[dash_test]
    fn a_reviewer_s_request_is_its_task_s_and_no_room_counts_no_try() {
        let text = request_text(PR1, "After merge: the reply is sent")
            .replace("task t:", "task review-t:");
        let names = ["a.md", "b.md", "c.md", "d.md", "e.md"];
        let dir = folder(&names.map(|n| (n, text.as_str())));
        let mut live = Fake::default();
        live.prs.insert(PR1.into(), "MERGED".into());
        live.tasks
            .insert("t".into(), ("done".into(), String::new(), vec![]));
        let mut state = State::default();
        // every login at its limit, as Real's agent answers then
        struct Full(Fake);
        impl Live for Full {
            fn get(&mut self, p: &str, h: bool) -> Got {
                self.0.get(p, h)
            }
            fn task(&mut self, id: &str) -> Result<Option<Task>, String> {
                self.0.task(id)
            }
            fn pr_state(&mut self, u: &str) -> Result<String, String> {
                self.0.pr_state(u)
            }
            fn jev(&mut self, t: &str, l: &str) -> Result<f64, String> {
                self.0.jev(t, l)
            }
            fn run(&mut self, a: &[String]) -> Result<(bool, String), String> {
                self.0.run(a)
            }
            fn agent(&mut self, _: &str) -> Result<String, String> {
                Err(format!("{NO_ROOM}: every login is at its limit"))
            }
            fn reopen(&mut self, id: &str, why: &str) -> Result<(), String> {
                self.0.reopen(id, why)
            }
        }
        let mut full = Full(live);
        for _ in 0..4 {
            let lines = after_merge(&dir.0, &mut full, &mut state, "2026-11-12", "dash");
            let waits = names
                .map(|n| format!("{n}: waits: no login with room: every login is at its limit"));
            assert_eq!(
                lines, waits,
                "each of the 5 asked: a wait on no room takes none of the pass's 4 turns"
            );
        }
        assert!(state.tries.is_empty(), "no try counted");
        let mut live = full.0;
        live.agent = vec!["{\"met\": false, \"printed\": \"not sent\"}".into()];
        let lines = after_merge(&dir.0, &mut live, &mut state, "2026-11-12", "dash");
        assert_eq!(lines[0], "a.md: not met, task t reopened: not sent");
        assert_eq!(live.reopened[0].0, "t", "the reviewed task, never review-t");
    }

    /// Part 5's run: an agent that could not read its evidence gives no verdict, so nothing is reopened on it; a request with no
    /// PR whose task is open again closes, its next report naming its parts.
    #[dash_test]
    fn could_not_check_is_no_verdict_and_an_open_pr_less_task_s_request_closes() {
        let said = "{\"met\": null, \"printed\": \"Bash was denied\"}";
        assert_eq!(
            agent_verdict(said),
            Err("the agent could not check: Bash was denied".into())
        );
        let mut live = Fake::default();
        live.tasks
            .insert("t".into(), ("open".into(), String::new(), vec![]));
        let r = request(
            Path::new("f.md"),
            &request_text("none", "After merge: :7799 lists them"),
        )
        .expect("r");
        assert!(r.prs.is_empty());
        assert_eq!(
            judge(&r, &mut live, "", false),
            Ok(Verdict::Moot(
                "no PR named and task t is open again: its next report names its after-merge parts"
                    .into()
            ))
        );
        assert!(live.reopened.is_empty() && live.asked_agent.is_empty());
    }

    /// Part 5's run: the answer of a login at its weekly limit, as Claude Code printed it, reads as one; a real answer does not.
    #[dash_test]
    fn a_weekly_limit_answer_reads_as_no_room() {
        let full = r#"[{"type":"system","subtype":"init"},{"type":"result","subtype":"success","is_error":true,"api_error_status":429,"api_error":"usage_limit_reached","result":"You've hit your weekly limit · resets Nov 15 at 6am (Europe/Paris)"}]"#;
        assert_eq!(
            limit_reached(full).as_deref(),
            Some("You've hit your weekly limit · resets Nov 15 at 6am (Europe/Paris)")
        );
        let said = r#"{"type":"result","subtype":"success","is_error":false,"result":"{\"met\": true}","total_cost_usd":0.1}"#;
        assert_eq!(limit_reached(said), None);
    }

    /// /code-review: only readers pass, curl reads the live app alone and gets -f, and the agent's one command is this check.
    #[dash_test]
    fn a_read_only_command_is_a_reader_with_no_write_option() {
        let words = |c: &str| c.split_whitespace().map(String::from).collect::<Vec<_>>();
        assert!(read_only(&words("dash task show t")).is_some());
        assert_eq!(
            read_only(&words("curl -s http://127.0.0.1:7788/x")),
            Some(words("curl -f --max-time 15 -s http://127.0.0.1:7788/x")),
            "a 404 exits non-zero"
        );
        let refused = [
            "dash task update t --x y",
            "curl -s -X POST http://127.0.0.1:7788/send",
            "curl -s -d a=b http://127.0.0.1:7788/x",
            "curl -s -o private/x.html http://127.0.0.1:7788/x",
            "curl -T f http://127.0.0.1:7788/x",
            "curl -s https://example.com/x",
            "git log --output=README.md",
            "git push",
            "gh pr view 1 --web",
            "tail -f log",
            "rm -f x",
        ];
        for no in refused {
            assert!(read_only(&words(no)).is_none(), "{no}");
        }
        assert!(
            command_of("`grep 'a b' f`").is_none(),
            "a quote: its words are unknown"
        );
        assert_eq!(
            agent_allowed("/r/dash"),
            ["Read", "Grep", "Glob", "Bash(/r/dash live-check read:*)"]
        );
    }

    /// /code-review: a merge not yet done waits undecided; a page a later commit moved is no proof; a merge the served build
    /// lacks waits, whatever its age.
    #[dash_test]
    fn a_task_not_yet_done_a_moved_page_and_an_unserved_merge_wait() {
        let mut live = Fake::default();
        let done_when = "1. private/docs/old.html shows it";
        live.tasks
            .insert("t".into(), ("review".into(), done_when.into(), vec![]));
        let mut state = State::default();
        prove(&merged("t", "a", NOW - 3600), &mut live, &mut state, NOW);
        assert!(state.checked.is_empty(), "asked again once done");
        live.tasks.get_mut("t").expect("t").0 = "done".into();
        live.served = Some(false);
        prove(&merged("t", "a", NOW - 3600), &mut live, &mut state, NOW);
        assert!(
            state.checked.is_empty() && live.reopened.is_empty(),
            "the live build lacks it"
        );
        live.served = Some(true);
        live.moved = vec!["private/docs/old.html".into()];
        prove(&merged("t", "a", NOW - 60), &mut live, &mut state, NOW);
        assert!(live.reopened.is_empty(), "its page moved since: no proof");
        assert_eq!(state.checked, vec!["t:a"], "served: checked however fresh");
    }

    /// /code-review: a request whose check fails three passes is marked waiting and never asked again; a slow page waits.
    #[dash_test]
    fn a_check_failing_three_passes_waits_and_a_slow_page_is_asked_again() {
        let dir = folder(&[("a.md", &request_text(PR1, "After merge: it holds"))]);
        let mut live = Fake::default();
        live.prs.insert(PR1.into(), "MERGED".into());
        live.tasks
            .insert("t".into(), ("done".into(), String::new(), vec![]));
        let mut state = State::default();
        for _ in 0..2 {
            let lines = after_merge(&dir.0, &mut live, &mut state, "2026-11-12", "dash");
            assert_eq!(lines, vec!["a.md: not judged: no agent expected"]);
        }
        let lines = after_merge(&dir.0, &mut live, &mut state, "2026-11-12", "dash");
        assert_eq!(
            lines,
            vec![
                "a.md: waits: Rust's after-merge check failed 3 times, the last: no agent expected"
            ]
        );
        assert!(requests(&dir.0).is_empty(), "its Waiting line holds it off");
        let a = std::fs::read_to_string(dir.0.join("a.md")).expect("a");
        assert!(a.contains("\n\n**Waiting 2026-11-12:** Rust's after-merge check failed 3 times"));
        live.jev = 0.9;
        live.pages.insert("/plan".into(), Got::Slow);
        let r = request(
            Path::new("f.md"),
            &request_text(PR1, "http://127.0.0.1:7788/plan lists it"),
        )
        .expect("r");
        assert!(matches!(
            judge(&r, &mut live, "", false),
            Ok(Verdict::Wait(_))
        ));
        assert_eq!(
            judge(&r, &mut live, "", true),
            Ok(Verdict::NotMet("/plan did not answer in time".into()))
        );
    }
}
