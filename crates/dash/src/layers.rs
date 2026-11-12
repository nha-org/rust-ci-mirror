//! `dash task add` for a Rust task names the layer it changes, checked at the front door before the older app files it (task
//! rust-task-names-layer; @nha, 2026-11-09: « We don't do one-offs. We must prevent this systematically. », « HTML is an impl
//! detail of our components that are reusable. Components are impl. details of our pages »). A layer is `design:<name>` (a
//! component or view /design shows), `model:<name>` or `rfc:<section id>` (of rfc-rust-unified or rfc-rust-plugins-ui). The
//! older app knows no `--layer`: the front door takes it out, and Rust's store keeps it for the picker and the brief.
//!
//! Every new task belongs to an epic or is urgent with a deadline, a `--deadline` kept the same way (RFC 20e rule 2; task
//! rust-orch-pick-dynamic; @nha, 2026-11-09: « Tasks must belong to an epic OR be urgent », « Urgent tasks are time-bound
//! usually. Ex. A deadline to prepare a meeting. »).
//!
//! A layer's open tasks run one at a time: an add says which share its layer and the closest other sections, and past
//! `CROWDED` it is refused unless `--same-layer <why>` (task rust-add-layer-load; @nha, 2026-11-10: « this then finally
//! unblocks adding parallelism and unblocks velocity development »: 13 tasks filed under rfc:architecture queued one behind
//! another). A done-when citing a file:line is refused: lines move under a task.
use dash_core::tasks::Queued;
use dash_store::older::Foldable;
use std::path::Path;

/// The data shapes Rust holds, a task naming one when it changes the shape itself: the stores' records and the document model
/// every page and message is drawn from (rust/CLAUDE.md rule 7, canvist_model). ponytail: kept by hand; read from the stores'
/// schemas once one is added without a line here.
pub const MODELS: [&str; 9] = [
    "document", "task", "comment", "draft", "mail", "key", "fact", "agent", "access",
];

/// The RFCs whose sections a Rust task may name (rust/CLAUDE.md rule 8).
pub(crate) const RFCS: [&str; 2] = [
    "private/docs/rfc-rust-unified.html",
    "private/docs/rfc-rust-plugins-ui.html",
];

/// What the front door does with a command.
#[derive(Debug, PartialEq)]
pub enum Filing {
    /// Not a task filing this check reads: it goes on as it came.
    Pass,
    /// It names a `layer` or a `deadline`, each kept for `id` unless `dry`; `args` are its words with `--layer <layer>` and
    /// `--deadline <day>` taken out. `update`: a `task update`, whose layer or deadline alone is no command for the older app.
    /// `owned`: a new task naming a layer and no owner, `--owner rust` added to `args`, so the Rust picker sees it (task
    /// rust-layer-means-owner; @nha, 2026-11-10: « do not pick work, orchestrate »).
    Keep {
        id: String,
        layer: Option<String>,
        deadline: Option<String>,
        dry: bool,
        update: bool,
        owned: bool,
        /// Why it joins a layer holding `CROWDED` open tasks already, as `--same-layer` gave it.
        same_layer: Option<String>,
        /// `--into`: its checks join a task filed already, so no task `id` is made: nothing is kept for it, nor refused for a
        /// crowd (task rust-front-door-known-id).
        into: bool,
        args: Vec<String>,
    },
    /// Refused, nothing filed: why, the closest layer named.
    Refused(String),
}

/// Each section id the RFCs at `repo` hold (`documents::section_ids`): an RFC that cannot be read holds none.
pub fn rfc_sections(repo: &Path) -> Vec<String> {
    let mut ids: Vec<String> = vec![];
    for rfc in RFCS {
        let html = std::fs::read_to_string(repo.join(rfc)).unwrap_or_default();
        for id in dash_surface::documents::section_ids(&html) {
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
    }
    ids
}

/// Every layer a task may name now, each as `kind:name`.
fn known(rfc: &[String]) -> Vec<String> {
    let design = dash_surface::design::names()
        .into_iter()
        .map(|n| format!("design:{n}"));
    let models = MODELS.iter().map(|m| format!("model:{m}"));
    let sections = rfc.iter().map(|s| format!("rfc:{s}"));
    design.chain(models).chain(sections).collect()
}

/// The edits from `a` to `b`, a character each.
fn distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut last = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let next = (last + usize::from(ca != *cb))
                .min(row[j] + 1)
                .min(row[j + 1] + 1);
            last = row[j + 1];
            row[j + 1] = next;
        }
    }
    row[b.len()]
}

/// The value of `flag`, the first time it comes: the word after it, or after its `=`.
fn value<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    let joined = |a: &'a String| a.strip_prefix(flag)?.strip_prefix('=');
    let at = args.iter().position(|a| a == flag || joined(a).is_some())?;
    joined(&args[at]).or_else(|| args.get(at + 1).map(String::as_str))
}

/// The first check of its done-when: its first `--done-when`, cut before a « 2. » after a blank or a line break when one text
/// numbers them all.
fn first_check(args: &[String]) -> &str {
    let checks = value(args, "--done-when").unwrap_or_default();
    let second = checks
        .match_indices("2. ")
        .find(|(at, _)| checks[..*at].ends_with(char::is_whitespace));
    second.map_or(checks, |(at, _)| &checks[..at])
}

/// The layer its words come closest to: the longest layer name its title or why holds as a word.
fn guessed<'a>(args: &[String], known: &'a [String]) -> Option<&'a String> {
    let text = [value(args, "--title"), value(args, "--why")];
    let text = text
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    let words: Vec<&str> = text
        .split(|c: char| !c.is_alphanumeric() && c != '-')
        .collect();
    let named = |l: &&String| l.split_once(':').is_some_and(|(_, n)| words.contains(&n));
    known.iter().filter(named).max_by_key(|l| l.len())
}

const HOW: &str = "--layer design:NAME (a component or view /design shows) | model:NAME | rfc:SECTION (a section id of \
                   rfc-rust-unified or rfc-rust-plugins-ui)";

const EPIC_OR_URGENT: &str = "a task belongs to an epic or is urgent with a deadline: --epic ID (dash focus roadmap \
                              lists them) | --priority 1 --deadline DAY (written 2026-11-15)";

/// `task add <id>` or `task update <id>` checked: a new task with no epic, unless urgent with a deadline, is refused; so is a
/// Rust task (`--owner rust`, or a `rust-` id) added with no layer, and any layer no list holds, the closest named. A /design
/// name not on /design yet passes only when the first check adds it there. `rfc`: the sections `rfc_sections` reads, asked
/// only when a layer is checked or named.
pub fn filing(args: &[String], rfc: &dyn Fn() -> Vec<String>) -> Filing {
    let (Some("task"), Some(verb @ ("add" | "update")), Some(id)) = (
        args.first().map(String::as_str),
        args.get(1).map(String::as_str),
        args.get(2),
    ) else {
        return Filing::Pass;
    };
    checked(args, id, verb == "update", rfc)
        .unwrap_or_else(|why| Filing::Refused(format!("{id}: refused, {why}")))
}

fn checked(
    args: &[String],
    id: &str,
    update: bool,
    rfc: &dyn Fn() -> Vec<String>,
) -> Result<Filing, String> {
    if let Some(cited) = cited_line(args) {
        return Err(format!(
            "its done-when cites {cited}, a line that moves under a task: name the test or the function instead"
        ));
    }
    if let Some(why) = done_whens(args).flat_map(numbered).find_map(unmeetable) {
        return Err(why);
    }
    let rust = id.starts_with("rust-")
        || value(args, "--owner") == Some("rust")
        || value(args, "--layer").is_some();
    if let Some((check, word)) = unrouted(args).filter(|_| rust) {
        return Err(format!(
            "its check « {check} » adds interaction ({word}) and names no route: name its Dioxus handler, run in the \
             window and through liveview, never a script of ours (rfc-rust-unified#ui-lib)"
        ));
    }
    for flag in ["--layer", "--deadline", "--same-layer"] {
        let joined = format!("{flag}=");
        if let Some(arg) = args.iter().find(|a| a.starts_with(&joined)) {
            let value = &arg[joined.len()..];
            return Err(format!("write {flag} {value}, a word apart"));
        }
    }
    let layer = layer_at(args, id, update, rfc)?;
    let day_at = args.iter().position(|a| a == "--deadline");
    let day = match day_at {
        None => None,
        Some(at) => Some(
            (args.get(at + 1))
                .filter(|d| d.len() == 10 && dash_core::days(d).is_some())
                .ok_or("--deadline takes a day, written 2026-11-15")?,
        ),
    };
    // `--into` adds its checks to a task filed already: no new task
    let into = !update && value(args, "--into").is_some();
    let new = !update && !into;
    if new {
        let named = |e: &str| !e.trim().is_empty() && e != "nil" && !e.starts_with("--");
        let epic = value(args, "--epic").is_some_and(named);
        let urgent = value(args, "--priority") == Some("1") && day.is_some();
        if !epic && !urgent {
            return Err(EPIC_OR_URGENT.into());
        }
    }
    let same_at = args.iter().position(|a| a == "--same-layer");
    let same_layer = match same_at {
        None => None,
        Some(at) => Some(
            (args.get(at + 1))
                .filter(|w| !w.trim().is_empty() && !w.starts_with("--"))
                .ok_or("--same-layer takes why it joins a crowded layer")?,
        ),
    };
    if same_at.is_some() && layer.is_none() {
        return Err("--same-layer comes with the --layer it joins".into());
    }
    let mut taken: Vec<usize> = layer.into_iter().chain(day_at).chain(same_at).collect();
    if taken.is_empty() {
        return Ok(Filing::Pass);
    }
    taken.sort_unstable_by(|a, b| b.cmp(a));
    let mut kept = args.to_vec();
    for at in taken {
        kept.drain(at..=at + 1);
    }
    // a layer is Rust's: a new task naming one and no owner is the Rust picker's
    // any --owner, even one naming none: the older app answers a bare one, never a second added here (/code-review)
    let given = args
        .iter()
        .any(|a| a == "--owner" || a.starts_with("--owner="));
    let owned = new && layer.is_some() && !given;
    if owned {
        kept.extend(["--owner".into(), "rust".into()]);
    }
    Ok(Filing::Keep {
        id: id.into(),
        layer: layer.map(|at| args[at + 1].clone()),
        deadline: day.cloned(),
        dry: args.iter().any(|a| a == "--dry-run"),
        update,
        owned,
        same_layer: same_layer.cloned(),
        into,
        args: kept,
    })
}

/// The prompt the older app's add files when none is given (`Records.@start_prompt`): an update filling a run gives it too.
pub const START_PROMPT: &str =
    "Build this task's :done-when. One PR, a test, /code-review before TASK_DONE.";

/// `task add|update <id>` made startable when filed, never at its start (task run-fields-at-add; @nha, 2026-11-02: « dispatch
/// one agent on graal and one on nanolathe as soon as we can »: nanolathe-local-ai failed two starts, one missing field each):
/// the words the older app gets and a line said after its answer, or why it is refused. `stored`: the task as filed, asked
/// only when an update needs it, none for an add.
/// * `--project <repo>`, or an update's `--start`, on a task with no run prompt adds `--prompt` with `START_PROMPT`: the older
///   app's `put_flag` then makes its run on that project (a prompt given lifts `:later`, its own rule: the plan is made).
/// * An update's `--start <why>` clears `:later`; an empty `--later` or `--duplicate-of` clears it, as `nil` does
///   (rust-scripted-edit-fmt stuck later, 2026-11-11).
/// * `--owner rust` is refused for a task with no layer or no run prompt, each named: Rust's start reads both. An add's layer is
///   `filing`'s, which names the closest.
pub fn runnable(
    args: &[String],
    stored: &dyn Fn() -> Result<Option<Foldable>, String>,
) -> Result<(Vec<String>, String), String> {
    let (Some("task"), Some(verb @ ("add" | "update")), Some(id)) = (
        args.first().map(String::as_str),
        args.get(1).map(String::as_str),
        args.get(2),
    ) else {
        return Ok((args.to_vec(), String::new()));
    };
    // several ids in one update: each is the older app's own write
    if args.get(3).is_some_and(|a| !a.starts_with("--")) {
        return Ok((args.to_vec(), String::new()));
    }
    let update = verb == "update";
    let refused = |why: String| format!("{id}: refused, {why}");
    let mut out = args.to_vec();
    let mut said = String::new();
    let start = update
        .then(|| out.iter().position(|a| a == "--start"))
        .flatten();
    if let Some(at) = start {
        let why = (out.get(at + 1))
            .filter(|w| !w.trim().is_empty() && !w.starts_with("--"))
            .ok_or_else(|| refused("--start takes why it starts now, --start <why>".into()))?
            .clone();
        if value(&out, "--later").is_some() {
            return Err(refused(
                "--start clears :later, so it comes without --later".into(),
            ));
        }
        out.drain(at..=at + 1);
        out.extend(["--later".into(), "nil".into()]);
        said += &format!(
            "{id}: its :later cleared, so the next pass may start it (why: {})\n",
            why.trim()
        );
    }
    if update {
        for flag in ["--later", "--duplicate-of"] {
            let at = out.iter().position(|a| a == flag);
            if let Some(v) = at.and_then(|at| out.get_mut(at + 1))
                && v.trim().is_empty()
            {
                *v = "nil".into();
            }
        }
    }
    let given = |flag: &str| {
        let v = value(&out, flag).filter(|v| !v.trim().is_empty() && *v != "nil");
        v.map(String::from)
    };
    let (prompt_given, project, layer) = (
        value(&out, "--prompt").is_some(),
        given("--project"),
        given("--layer"),
    );
    let owner_given = value(&out, "--owner");
    // an add naming a layer and no owner is filed `--owner rust` (`filing`'s `owned`): its prompt is checked too (/code-review)
    let rust = owner_given == Some("rust") || (!update && owner_given.is_none() && layer.is_some());
    let asks_run = start.is_some() || project.is_some();
    // an update reads the task as filed; one the list does not hold is refused as such, never for a field (/code-review)
    let row = match update && (rust || asks_run) {
        true => Some(
            stored()
                .map_err(|e| refused(format!("its run fields could not be read: {e}")))?
                .ok_or_else(|| refused("the list holds no such task".into()))?,
        ),
        false => None,
    };
    // a task he owns starts no session: no run is made for it (/code-review)
    let owner = match owner_given {
        Some("nil") => String::new(),
        Some(o) => o.to_string(),
        None => row.as_ref().map(|r| r.owner.clone()).unwrap_or_default(),
    };
    let agent = ["", "claude", "vibe", "rust"].contains(&owner.as_str());
    let prompt = given("--prompt").or_else(|| match prompt_given {
        // `--prompt nil` clears it
        true => None,
        false => row
            .as_ref()
            .map(|r| r.prompt.clone())
            .filter(|p| !p.trim().is_empty()),
    });
    let fills = agent && !prompt_given && prompt.is_none() && asks_run;
    if fills {
        out.extend(["--prompt".into(), START_PROMPT.into()]);
        said += &format!("{id}: its run gets the default prompt, « {START_PROMPT} »\n");
    }
    // a start reads its run's project: a blank one is filled from the task's own, as `Records.start/1` did (/code-review)
    let blank_project = row
        .as_ref()
        .is_some_and(|r| r.run_project.trim().is_empty());
    if start.is_some() && project.is_none() && agent && owner != "rust" && blank_project {
        let own = row
            .as_ref()
            .map(|r| r.project.clone())
            .filter(|p| !p.trim().is_empty());
        let own = own.unwrap_or_else(|| "site_perso".into());
        said += &format!("{id}: its run's project is its own, {own}\n");
        out.extend(["--project".into(), own]);
    }
    if rust {
        // an add with no --prompt gets the older app's default one
        let prompt =
            prompt.or_else(|| (fills || (!update && !prompt_given)).then(|| START_PROMPT.into()));
        let layer = layer.or_else(|| row.as_ref().and_then(|r| r.layer.clone()));
        let after = Foldable {
            owner: "rust".into(),
            layer,
            prompt: prompt.unwrap_or_default(),
            ..Foldable::default()
        };
        let lacks: Vec<&str> = after
            .missing()
            .into_iter()
            .filter(|l| update || *l != "layer")
            .collect();
        if !lacks.is_empty() {
            return Err(refused(format!(
                "--owner rust: Rust's start reads its layer and its run prompt, and it has no {} (--layer {HOW_SHORT}, \
                 --prompt <what to build>)",
                lacks.join(", no ")
            )));
        }
    }
    Ok((out, said))
}

const HOW_SHORT: &str = "design:NAME|model:NAME|rfc:SECTION";

/// The first `file:line` any `--done-when` cites (`main.rs:2001`, `src/a.rs:12:5`, `bin/dash:40`, `Makefile:12`): a path
/// whose extension starts with a letter, or one with a `/`, then a `:` and a digit. A URL's port is none, nor a host's
/// (`nicolas-ha.com:443`, `x.ts.net:7788/tasks`): a top-level domain, or a `/` after its digits. The text read from stdin is
/// in `args` already, the front door put it there.
fn cited_line(args: &[String]) -> Option<String> {
    let checks =
        args.iter()
            .enumerate()
            .filter_map(|(at, a)| match a.strip_prefix("--done-when") {
                Some("") => args.get(at + 1),
                Some(joined) if joined.starts_with('=') => Some(a),
                _ => None,
            });
    let line = |word: &&str| {
        word.match_indices(':').any(|(at, _)| {
            let (path, rest) = (&word[..at], &word[at + 1..]);
            let name = path.rsplit('/').next().unwrap_or_default();
            let ext = name.rsplit_once('.').map(|(_, e)| e);
            let digits = rest.trim_start_matches(|c: char| c.is_ascii_digit());
            let file = match ext {
                Some(e) => {
                    e.starts_with(|c: char| c.is_ascii_alphabetic())
                        && e.chars().all(char::is_alphanumeric)
                        && !TLDS.contains(&e)
                }
                None => {
                    path.contains('/') || ["Makefile", "Dockerfile", "Justfile"].contains(&name)
                }
            };
            rest.starts_with(|c: char| c.is_ascii_digit())
                && !digits.starts_with('/')
                && !path.contains("://")
                && name
                    .chars()
                    .all(|c| c.is_alphanumeric() || "._-".contains(c))
                && file
        })
    };
    let trim = |w: &str| {
        w.trim_matches(|c: char| "`'\"()[]{},;<>«»".contains(c))
            .to_string()
    };
    checks
        .flat_map(|c| c.split_whitespace())
        .map(trim)
        .find(|w| line(&w.as_str()))
}

/// What `dash task style` adds for the checks no gate lets pass (`unmeetable`): each open task's, every project's, a line
/// each, then their count; `only`, that one task's lines alone.
pub fn unmeetable_lines(open: &[dash_store::similar::Text], only: Option<&str>) -> String {
    let lines: Vec<String> = (open.iter())
        .filter(|t| only.is_none_or(|id| id == t.id))
        .flat_map(|t| {
            (numbered(&t.done_when).into_iter())
                .filter_map(unmeetable)
                .map(|why| format!("{}: {why}\n", t.id))
        })
        .collect();
    match only {
        Some(_) => lines.concat(),
        None => format!(
            "{}{} check(s) no gate lets pass\n",
            lines.concat(),
            lines.len()
        ),
    }
}

/// Words that name interaction in product words, the kind the back-test's briefs used with no route (a menu, a button, a drag).
const INTERACTION: [&str; 20] = [
    "click",
    "clicks",
    "clicked",
    "clicking",
    "menu",
    "menus",
    "button",
    "buttons",
    "drag",
    "drags",
    "dragged",
    "dragging",
    "dropdown",
    "toggle",
    "toggles",
    "submit",
    "hover",
    "keypress",
    "shortcut",
    "shortcuts",
];

/// Words that name a route for it: a Dioxus handler, or liveview carrying it to the web page.
const ROUTE: [&str; 5] = ["handler", "handlers", "liveview", "onclick", "oninput"];

/// The first check of a `--done-when` that names interaction and no route, with the word: a check that adds a behaviour says
/// how it runs, or the web half gets a script of ours again (rfc-rust-unified#conformance's back-test: 4 of 10 briefs named a
/// menu, a form or a refresh and never its route; task rust-task-rfc-judge).
fn unrouted(args: &[String]) -> Option<(String, &'static str)> {
    for check in done_whens(args).flat_map(numbered) {
        let words = lower_words(check);
        let named =
            |list: &[&'static str]| list.iter().find(|w| words.iter().any(|x| x == *w)).copied();
        if let (Some(word), None) = (named(&INTERACTION), named(&ROUTE)) {
            return Some((check.to_string(), word));
        }
    }
    None
}

/// Each `--done-when` text of `args`, given apart or joined by `=`.
fn done_whens(args: &[String]) -> impl Iterator<Item = &str> {
    args.iter()
        .enumerate()
        .filter_map(|(at, a)| match a.strip_prefix("--done-when") {
            Some("") => args.get(at + 1).map(String::as_str),
            Some(joined) => joined.strip_prefix('='),
            None => None,
        })
}

/// Each numbered check of a done-when `text`: a « N. » at its start or after a blank begins one.
pub fn numbered(text: &str) -> Vec<&str> {
    let starts = text.match_indices(". ").filter_map(|(at, _)| {
        let head = text[..at].trim_end_matches(|c: char| c.is_ascii_digit());
        (head.len() < at && (head.is_empty() || head.ends_with(char::is_whitespace)))
            .then_some(head.len())
    });
    let mut cuts: Vec<usize> = starts.collect();
    cuts.push(text.len());
    let firsts = std::iter::once(0).chain(cuts.iter().copied());
    (firsts.zip(cuts.iter().copied()).filter(|(f, t)| f < t))
        .map(|(from, to)| text[from..to].trim())
        .filter(|c| !c.is_empty())
        .collect()
}

fn lower_words(text: &str) -> Vec<String> {
    (text.split(|c: char| !c.is_alphanumeric()))
        .map(str::to_lowercase)
        .collect()
}

/// The extensions a check's file names end on. ponytail: the ones the repos hold; add one when a deletion of another kind
/// goes unseen.
const FILE_EXTS: [&str; 18] = [
    "ex", "exs", "heex", "py", "sh", "rs", "js", "ts", "clj", "bb", "edn", "md", "html", "css",
    "json", "toml", "yml", "sql",
];

/// Why `check` cannot be met as written, or none: it names an .ex or .exs file with any verb but delete, which the
/// pre-commit's no-elixir step refuses (@nha, 2026-11-10: « no diffs on ex or exs at all, rust HAS to take over »); or it
/// deletes a whole file and names none of its callers (rust-hooks-rest-port's « the .py deleted » met outbound_gate.py,
/// which the frozen Elixir mail send still calls, 2026-11-12). Task rust-style-no-elixir-edit.
pub fn unmeetable(check: &str) -> Option<String> {
    // each word as a path reads: quotes, a possessive and a `#L40` or `:40` after it cut
    let words: Vec<&str> = (check.split_whitespace())
        .map(|w| w.trim_matches(|c: char| "`'\"()[]{},;:<>«».!?".contains(c)))
        .map(|w| w.trim_end_matches("'s").trim_end_matches("’s"))
        .map(|w| w.split(['#', ':']).next().unwrap_or_default())
        .collect();
    let lower: Vec<String> = words.iter().map(|w| w.to_lowercase()).collect();
    let is = |at: Option<usize>, list: &[&str]| {
        at.and_then(|a| lower.get(a))
            .is_some_and(|w| list.contains(&w.as_str()))
    };
    const VERB: [&str; 3] = ["delete", "deletes", "deleting"];
    // the delete word beside the file, never anywhere in the check (/code-review: « older.rs skips deleted rows » deletes
    // no file, « lint.ex deleted, the mix.exs aliases gone » edits mix.exs)
    let deleted = |at: usize| {
        let (before, before2) = (at.checked_sub(1), at.checked_sub(2));
        is(Some(at + 1), &["deleted"])
            || (is(Some(at + 1), &["is", "was", "gets", "goes"]) && is(Some(at + 2), &["deleted"]))
            || is(before, &VERB)
            || (is(before, &["the", "whole"]) && is(before2, &VERB))
            || (is(before, &["of"]) && is(before2, &["deletion"]))
    };
    let file = |w: &str| {
        w.rsplit_once('.').is_some_and(|(stem, ext)| {
            FILE_EXTS.contains(&ext) && !stem.is_empty() && !stem.ends_with('/')
        })
    };
    // the gate's own list: an .ex, .exs or .heex anywhere, or a named file of the app's folders; a kind alone (« .ex »), a
    // folder or a tool run from it (private/dashboard/test/scripts/shot) is no file edited
    let elixir =
        |w: &str| !w.starts_with(['.', '*']) && file(w) && dash_gate::hook::no_elixir::elixir(w);
    if let Some(at) = (0..words.len()).find(|&at| elixir(words[at]) && !deleted(at)) {
        return Some(format!(
            "its check « {check} » names {} with a verb but delete: no check may edit an Elixir file, the pre-commit's \
             no-elixir step refuses it; only a whole-file deletion passes (CLAUDE.md, Coordination 5)",
            words[at]
        ));
    }
    let deletes = (0..words.len()).any(|at| file(words[at]) && deleted(at));
    let callers =
        lower.iter().any(|w| w.starts_with("caller")) || check.to_lowercase().contains("called by");
    // the check quoted, never a file of it: the first one named may be one it leaves untouched (rust-hooks-rest-port's)
    (deletes && !callers).then(|| {
        format!(
            "its check « {check} » deletes a whole file and names none of its callers: name each, read with \
             `git -c core.fsmonitor=false grep`, or write « no callers » (task-writing)"
        )
    })
}

/// The top-level domains a host in a check ends on, never a file's extension. ponytail: the ones his checks name; add one
/// when a host is refused as a file:line.
const TLDS: [&str; 10] = [
    "com", "net", "org", "io", "dev", "app", "im", "eu", "fr", "uk",
];

/// Past this many open tasks on one layer, another is refused unless `--same-layer` says why.
pub const CROWDED: usize = 5;

/// What `task add` or `update` says of `layer`'s load: the open tasks of `open` sharing it, which run one at a time, and the
/// three sections of its kind closest to it, each with its open tasks; refused when `CROWDED` share it and no `same_layer`.
pub fn load(
    id: &str,
    layer: &str,
    open: &[Queued],
    rfc: &[String],
    same_layer: Option<&str>,
) -> Result<String, String> {
    let on = |l: &str| -> Vec<&str> {
        (open.iter())
            .filter(|t| t.id != id && t.layer.as_deref() == Some(l))
            .map(|t| t.id.as_str())
            .collect()
    };
    let sharing = on(layer);
    let kind = layer.split_once(':').map_or("", |(k, _)| k);
    let mut others: Vec<String> = (known(rfc).into_iter())
        .filter(|k| k != layer && k.split_once(':').is_some_and(|(o, _)| o == kind))
        .collect();
    others.sort_by_key(|k| distance(layer, k));
    let closest: Vec<String> = (others.iter().take(3))
        .map(|k| format!("{k} ({} open)", on(k).len()))
        .collect();
    let closest = match closest.is_empty() {
        true => String::new(),
        false => format!("; closest other sections: {}", closest.join(", ")),
    };
    // a task on it already adds no load: an update naming its own layer again passes
    let already = (open.iter()).any(|t| t.id == id && t.layer.as_deref() == Some(layer));
    if sharing.len() >= CROWDED && same_layer.is_none() && !already {
        return Err(format!(
            "{id}: refused, {layer} holds {} open tasks, which run one at a time ({}): name a closer section, or \
             --same-layer WHY{closest}",
            sharing.len(),
            sharing.join(", ")
        ));
    }
    let said = match sharing.len() {
        0 => "no other open task shares it".to_string(),
        n => format!(
            "{n} open task(s) share it, and its tasks run one at a time: {}",
            sharing.join(", ")
        ),
    };
    // the why that let it past CROWDED, said where it is read
    let why = match same_layer {
        Some(why) if sharing.len() >= CROWDED => format!(", let in by --same-layer: {why}"),
        _ => String::new(),
    };
    Ok(format!("{id}: on {layer}, {said}{why}{closest}\n"))
}

/// The line `task add` says after its answer when the new task joins an epic and waits on nothing: /tasks?layout=graph draws
/// it with no arrow, its order in the epic unsaid (task rust-graph-why-these, done-when 5). A warning, never a refusal: some
/// tasks wait on nothing. `None` for any other command.
pub fn warning(args: &[String]) -> Option<String> {
    let (Some("task"), Some("add"), Some(id)) = (
        args.first().map(String::as_str),
        args.get(1).map(String::as_str),
        args.get(2),
    ) else {
        return None;
    };
    let named = |v: &&str| !v.trim().is_empty() && *v != "nil" && !v.starts_with("--");
    let epic = value(args, "--epic").filter(named)?;
    if value(args, "--into").is_some() || value(args, "--blocked-by").is_some_and(|w| named(&w)) {
        return None;
    }
    Some(format!(
        "{id}: warning, it joins epic {epic} and waits on nothing, so /tasks?layout=graph draws it with no arrow: name what it \
         waits on with dash task update {id} --blocked-by ID[,ID]\n"
    ))
}

/// Where its `--layer` stands, once its layer is checked; `None` when it names none and need not.
fn layer_at(
    args: &[String],
    id: &str,
    update: bool,
    rfc: &dyn Fn() -> Vec<String>,
) -> Result<Option<usize>, String> {
    let rust = id.starts_with("rust-") || value(args, "--owner") == Some("rust");
    let at = args.iter().position(|a| a == "--layer");
    if at.is_none() && (update || !rust) {
        return Ok(None);
    }
    let known = known(&rfc());
    let Some(at) = at else {
        let closest =
            guessed(args, &known).map_or(String::new(), |l| format!("; closest to its title: {l}"));
        return Err(format!(
            "a Rust task names the layer it changes: {HOW}{closest}"
        ));
    };
    let Some(layer) = args.get(at + 1).filter(|l| !l.starts_with("--")) else {
        return Err(format!("--layer names no layer: {HOW}"));
    };
    let first = first_check(args);
    let new_on_design = layer.strip_prefix("design:").is_some_and(|name| {
        !name.is_empty()
            && first.contains("/design")
            && first
                .split(|c: char| !c.is_alphanumeric() && c != '-')
                .any(|w| w == name)
    });
    if !known.contains(layer) && !new_on_design {
        let closest = known.iter().min_by_key(|k| distance(layer, k));
        let closest = closest.map_or(String::new(), |c| format!("; closest: {c}"));
        let new = match layer.starts_with("design:") {
            true => ", and its first check does not add it to /design",
            false => "",
        };
        return Err(format!("{layer} is no layer{new}: {HOW}{closest}"));
    }
    Ok(Some(at))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::memory::TestBudget;

    fn words(line: &[&str]) -> Vec<String> {
        line.iter().map(|w| w.to_string()).collect()
    }

    /// Done-when 5 of rust-graph-why-these: a new task in an epic with no wait is warned, never refused; one with a wait, an
    /// update, an --into or a task in no epic is not.
    #[test]
    fn task_add_warns_on_an_epic_task_with_no_wait() {
        let _budget = TestBudget::new();
        let alone = warning(&words(&[
            "task", "add", "x", "--epic", "rust-poc", "--title", "t",
        ]));
        let alone = alone.expect("warned");
        assert!(
            alone.starts_with("x: warning, it joins epic rust-poc and waits on nothing"),
            "{alone}"
        );
        assert!(alone.contains("dash task update x --blocked-by"), "{alone}");
        assert!(warning(&words(&["task", "add", "x", "--epic=rust-poc"])).is_some());
        for line in [
            &[
                "task",
                "add",
                "x",
                "--epic",
                "rust-poc",
                "--blocked-by",
                "y",
            ][..],
            &["task", "add", "x", "--epic", "rust-poc", "--blocked-by=y,z"],
            &["task", "add", "x", "--epic", "rust-poc", "--into", "y"],
            &["task", "update", "x", "--epic", "rust-poc"],
            &[
                "task",
                "add",
                "x",
                "--priority",
                "1",
                "--deadline",
                "2026-11-15",
            ],
            &["task", "add", "x", "--epic", "nil"],
            &["focus", "add", "x", "--epic", "rust-poc"],
        ] {
            assert_eq!(warning(&words(line)), None, "{line:?}");
        }
        // a warning only: the same line still files
        let filed = filing(
            &words(&["task", "add", "x", "--epic", "rust-poc"]),
            &Vec::new,
        );
        assert_eq!(filed, Filing::Pass);
    }

    fn refused(line: &[&str]) -> String {
        match filing(&words(line), &|| vec!["dataflow".into()]) {
            Filing::Refused(why) => why,
            other => panic!("{other:?}"),
        }
    }

    /// Done-when 1 of rust-task-names-layer: a Rust task with no layer, or one no list holds, is refused, the closest named.
    #[test]
    fn a_rust_task_with_no_layer_or_an_unknown_one_is_refused_the_closest_named() {
        let _budget = TestBudget::new();
        let none = refused(&[
            "task",
            "add",
            "rust-x",
            "--title",
            "The board shows agents",
            "--owner",
            "rust",
        ]);
        assert!(
            none.starts_with(
                "rust-x: refused, a Rust task names the layer it changes: --layer design:"
            ),
            "{none}"
        );
        assert!(
            none.ends_with("; closest to its title: design:agents"),
            "{none}"
        );
        // an owner-rust task with any id, or a rust- id with no owner
        let owned = refused(&["task", "add", "x", "--owner", "rust"]);
        assert!(
            owned.starts_with("x: refused, a Rust task names"),
            "{owned}"
        );
        let wrong = refused(&["task", "add", "rust-x", "--layer", "design:bord"]);
        assert!(
            wrong.contains(
                "design:bord is no layer, and its first check does not add it to /design"
            ),
            "{wrong}"
        );
        assert!(wrong.ends_with("; closest: design:board"), "{wrong}");
        let rfc = refused(&["task", "add", "rust-x", "--layer", "rfc:dataflw"]);
        assert!(rfc.ends_with("; closest: rfc:dataflow"), "{rfc}");
        let model = refused(&["task", "add", "rust-x", "--layer", "model:tasks"]);
        assert!(model.ends_with("; closest: model:task"), "{model}");
        let bare = refused(&["task", "add", "rust-x", "--layer", "--title", "t"]);
        assert!(bare.contains("--layer names no layer"), "{bare}");
        // not a Rust task, not a filing, an update naming none: as they came
        for line in [
            &[
                "task",
                "add",
                "klar-x",
                "--owner",
                "klar",
                "--epic",
                "klar-plus-beta",
            ][..],
            &["task", "update", "rust-x", "--title", "t"],
            &["task", "show", "rust-x"],
            &["outbox"],
        ] {
            let unread = || panic!("{line:?}: no RFC is read for a command naming no layer");
            assert_eq!(filing(&words(line), &unread), Filing::Pass, "{line:?}");
        }
        // the flags written with an =: an owner read, a layer asked as two words
        let owned = refused(&["task", "add", "x", "--owner=rust"]);
        assert!(
            owned.starts_with("x: refused, a Rust task names"),
            "{owned}"
        );
        let joined = refused(&["task", "add", "rust-x", "--layer=design:board"]);
        assert_eq!(
            joined,
            "rust-x: refused, write --layer design:board, a word apart"
        );
        // an update naming a layer and nothing else is the front door's to answer; an add never is
        let update = words(&["task", "update", "rust-x", "--layer", "design:board"]);
        let Filing::Keep {
            update: true, args, ..
        } = filing(&update, &Vec::new)
        else {
            panic!("{:?}", filing(&update, &Vec::new));
        };
        assert_eq!(args, words(&["task", "update", "rust-x"]));
    }

    /// Done-when 2 of rust-task-names-layer: a /design name not shown there yet passes only when the first check adds it there.
    #[test]
    fn a_layer_not_on_design_yet_passes_only_when_the_first_check_adds_it() {
        let _budget = TestBudget::new();
        let add = |checks: &[&str]| {
            let mut line = words(&[
                "task",
                "add",
                "rust-x",
                "--layer",
                "design:timeline-card",
                "--dry-run",
                "--epic",
                "rust-poc",
            ]);
            for c in checks {
                line.extend(words(&["--done-when", c]));
            }
            filing(&line, &Vec::new)
        };
        let shown =
            "1. /design shows timeline-card, its HTML and text: a test 2. the board uses it";
        let Filing::Keep {
            id,
            layer,
            deadline: None,
            dry,
            update: false,
            owned: true,
            same_layer: None,
            into: false,
            args,
        } = add(&[shown])
        else {
            panic!("{:?}", add(&[shown]));
        };
        assert_eq!(
            (id.as_str(), layer.as_deref(), dry),
            ("rust-x", Some("design:timeline-card"), true)
        );
        assert_eq!(
            args,
            words(&[
                "task",
                "add",
                "rust-x",
                "--dry-run",
                "--epic",
                "rust-poc",
                "--done-when",
                shown,
                "--owner",
                "rust"
            ])
        );
        // added in a later check only, or never: refused
        for checks in [
            &["1. the board uses it 2. /design shows timeline-card"][..],
            &["1. the board uses it\n2. /design shows timeline-card"],
            &["the board uses it", "/design shows timeline-card"],
            &["/design shows timeline-cards"],
            &[],
        ] {
            let Filing::Refused(why) = add(checks) else {
                panic!("{checks:?}: {:?}", add(checks));
            };
            assert!(
                why.contains("its first check does not add it to /design"),
                "{why}"
            );
        }
        // a name /design shows already passes with any checks
        let line = words(&[
            "task",
            "add",
            "rust-x",
            "--layer",
            "design:board",
            "--owner",
            "rust",
            "--epic",
            "rust-poc",
        ]);
        assert!(matches!(
            filing(&line, &Vec::new),
            Filing::Keep { dry: false, .. }
        ));
    }

    /// Done-when 1 of rust-orch-pick-dynamic: a new task with no epic is refused unless it is urgent with a deadline, one case
    /// each; the deadline is taken out for Rust's store to keep, as a layer is.
    #[test]
    fn a_task_with_no_epic_is_refused_unless_urgent_with_a_deadline() {
        let _budget = TestBudget::new();
        let file = |line: &[&str]| {
            let mut all = words(&["task", "add", "klar-x", "--title", "T"]);
            all.extend(words(line));
            filing(&all, &|| {
                panic!("no RFC is read for a task that is not Rust's")
            })
        };
        let why = "klar-x: refused, a task belongs to an epic or is urgent with a deadline: --epic ID (dash focus \
                   roadmap lists them) | --priority 1 --deadline DAY (written 2026-11-15)";
        // no epic, nothing urgent
        assert_eq!(file(&[]), Filing::Refused(why.into()));
        // an epic flag naming none
        for none in [
            &["--epic", "nil"][..],
            &["--epic", " "],
            &["--epic", "--title"],
        ] {
            assert_eq!(file(none), Filing::Refused(why.into()), "{none:?}");
        }
        // urgent without a deadline, or a deadline without urgency
        assert_eq!(file(&["--priority", "1"]), Filing::Refused(why.into()));
        let later = file(&["--priority", "2", "--deadline", "2026-11-12"]);
        assert_eq!(later, Filing::Refused(why.into()));
        // a deadline that is no day
        for bad in ["2026-11-31", "tomorrow", "2026-11-12T10:00"] {
            let refused = file(&["--priority", "1", "--deadline", bad]);
            let said = "klar-x: refused, --deadline takes a day, written 2026-11-15";
            assert_eq!(refused, Filing::Refused(said.into()), "{bad}");
        }
        assert_eq!(
            file(&["--deadline=2026-11-12"]),
            Filing::Refused("klar-x: refused, write --deadline 2026-11-12, a word apart".into())
        );
        // an epic: as it came
        assert_eq!(file(&["--epic", "klar-plus-beta"]), Filing::Pass);
        assert_eq!(file(&["--epic=klar-plus-beta"]), Filing::Pass);
        // urgent with a deadline: filed, its deadline taken out for Rust's store
        let urgent = file(&["--priority", "1", "--deadline", "2026-11-12", "--dry-run"]);
        let kept = words(&[
            "task",
            "add",
            "klar-x",
            "--title",
            "T",
            "--priority",
            "1",
            "--dry-run",
        ]);
        assert_eq!(
            urgent,
            Filing::Keep {
                id: "klar-x".into(),
                layer: None,
                deadline: Some("2026-11-12".into()),
                dry: true,
                update: false,
                owned: false,
                same_layer: None,
                into: false,
                args: kept,
            }
        );
        // checks added to a task filed already, or an update: no new task, no epic asked
        assert_eq!(file(&["--into", "klar-y"]), Filing::Pass);
        let update = words(&["task", "update", "klar-x", "--title", "T"]);
        assert_eq!(filing(&update, &Vec::new), Filing::Pass);
        // a Rust task with both a layer and a deadline: each taken out
        let both = words(&[
            "task",
            "add",
            "rust-x",
            "--layer",
            "design:board",
            "--priority",
            "1",
            "--deadline",
            "2026-11-12",
        ]);
        let Filing::Keep {
            layer,
            deadline,
            args,
            ..
        } = filing(&both, &Vec::new)
        else {
            panic!("{:?}", filing(&both, &Vec::new));
        };
        assert_eq!(
            (layer.as_deref(), deadline.as_deref()),
            (Some("design:board"), Some("2026-11-12"))
        );
        assert_eq!(
            args,
            words(&[
                "task",
                "add",
                "rust-x",
                "--priority",
                "1",
                "--owner",
                "rust"
            ])
        );
    }

    /// Done-when 1 of rust-layer-means-owner: a new task naming a layer and no owner is filed `--owner rust`; an update, or
    /// checks added `--into` a task filed already, gets no owner.
    #[test]
    fn a_task_add_naming_a_layer_and_no_owner_is_owned_by_rust() {
        let _budget = TestBudget::new();
        let line = words(&[
            "task",
            "add",
            "x",
            "--layer",
            "model:task",
            "--epic",
            "rust-poc",
        ]);
        let Filing::Keep { owned, args, .. } = filing(&line, &Vec::new) else {
            panic!("{:?}", filing(&line, &Vec::new));
        };
        assert!(owned);
        let filed = ["task", "add", "x", "--epic", "rust-poc", "--owner", "rust"];
        assert_eq!(args, words(&filed));
        for line in [
            &["task", "update", "x", "--layer", "model:task"][..],
            &["task", "add", "x", "--layer", "model:task", "--into", "y"],
        ] {
            let Filing::Keep { owned, args, .. } = filing(&words(line), &Vec::new) else {
                panic!("{line:?}");
            };
            assert!(!owned, "{line:?}");
            assert!(!args.contains(&"--owner".to_string()), "{line:?}");
        }
    }

    /// Done-when 2 of rust-layer-means-owner: an `--owner` given is kept as given, written apart or with an `=`, a bare one too.
    #[test]
    fn an_owner_given_is_kept_as_given() {
        let _budget = TestBudget::new();
        for owner in [
            &["--owner", "klar"][..],
            &["--owner=klar"],
            &["--owner", "rust"],
            &["--owner"],
        ] {
            let mut line = words(&[
                "task",
                "add",
                "x",
                "--layer",
                "model:task",
                "--epic",
                "rust-poc",
            ]);
            line.extend(words(owner));
            let Filing::Keep { owned, args, .. } = filing(&line, &Vec::new) else {
                panic!("{owner:?}: {:?}", filing(&line, &Vec::new));
            };
            assert!(!owned, "{owner:?}");
            let mut filed = words(&["task", "add", "x", "--epic", "rust-poc"]);
            filed.extend(words(owner));
            assert_eq!(args, filed, "{owner:?}");
        }
    }

    fn on(id: &str, layer: &str) -> Queued {
        Queued {
            id: id.into(),
            layer: Some(layer.into()),
            ..Queued::default()
        }
    }

    /// Done-when 1 of rust-add-layer-load: an add says the open tasks sharing its layer, that they run one at a time, and the
    /// closest other sections of its kind with their own open tasks.
    #[test]
    fn task_add_says_its_layers_open_tasks_and_the_closest_other_sections() {
        let _budget = TestBudget::new();
        let rfc: Vec<String> = ["orchestration", "orchestrator", "orch-pick", "dataflow"]
            .map(String::from)
            .into();
        let open = [
            on("rust-a", "rfc:orchestration"),
            on("rust-b", "rfc:orchestration"),
            on("rust-c", "rfc:orchestrator"),
            on("rust-x", "rfc:orchestration"),
        ];
        let said = load("rust-x", "rfc:orchestration", &open, &rfc, None).expect("said");
        assert_eq!(
            said,
            "rust-x: on rfc:orchestration, 2 open task(s) share it, and its tasks run one at a time: rust-a, rust-b; \
             closest other sections: rfc:orchestrator (1 open), rfc:orch-pick (0 open), rfc:dataflow (0 open)\n"
        );
        let alone = load("rust-y", "rfc:dataflow", &open, &rfc, None).expect("said");
        assert!(
            alone.starts_with("rust-y: on rfc:dataflow, no other open task shares it; closest"),
            "{alone}"
        );
    }

    /// Done-when 2 of rust-add-layer-load: past 5 open tasks on one layer an add is refused unless `--same-layer <why>`, taken
    /// out of the words the older app files.
    #[test]
    fn past_five_open_tasks_on_a_layer_an_add_is_refused_unless_same_layer() {
        let _budget = TestBudget::new();
        let rfc = vec!["architecture".to_string(), "orchestration".to_string()];
        let five: Vec<Queued> = (1..=5)
            .map(|n| on(&format!("rust-{n}"), "rfc:architecture"))
            .collect();
        let refused = load("rust-x", "rfc:architecture", &five, &rfc, None).expect_err("refused");
        assert_eq!(
            refused,
            "rust-x: refused, rfc:architecture holds 5 open tasks, which run one at a time (rust-1, rust-2, rust-3, rust-4, \
             rust-5): name a closer section, or --same-layer WHY; closest other sections: rfc:orchestration (0 open)"
        );
        let said = load(
            "rust-x",
            "rfc:architecture",
            &five,
            &rfc,
            Some("one module"),
        )
        .expect("let in");
        assert!(
            said.contains("5 open task(s) share it, and its tasks run one at a time: rust-1, rust-2, rust-3, rust-4, \
                           rust-5, let in by --same-layer: one module"),
            "{said}"
        );
        // a task on it already is never counted, nor refused for naming it again
        assert!(load("rust-5", "rfc:architecture", &five, &rfc, None).is_ok());
        let mut six = five.clone();
        six.push(on("rust-6", "rfc:architecture"));
        assert!(load("rust-6", "rfc:architecture", &six, &rfc, None).is_ok());
        let line = words(&[
            "task",
            "add",
            "rust-x",
            "--layer",
            "rfc:architecture",
            "--same-layer",
            "one module",
            "--epic",
            "rust-poc",
        ]);
        let Filing::Keep {
            same_layer, args, ..
        } = filing(&line, &|| rfc.clone())
        else {
            panic!("{:?}", filing(&line, &|| rfc.clone()));
        };
        assert_eq!(same_layer.as_deref(), Some("one module"));
        assert_eq!(
            args,
            words(&[
                "task", "add", "rust-x", "--epic", "rust-poc", "--owner", "rust"
            ])
        );
        let bare = words(&[
            "task",
            "add",
            "rust-x",
            "--layer",
            "rfc:architecture",
            "--same-layer",
            "--epic",
            "e",
        ]);
        assert_eq!(
            filing(&bare, &|| rfc.clone()),
            Filing::Refused(
                "rust-x: refused, --same-layer takes why it joins a crowded layer".into()
            )
        );
        // with no layer it joins nothing: refused, never dropped unsaid
        let alone = words(&["task", "update", "rust-x", "--same-layer", "why"]);
        assert_eq!(
            filing(&alone, &|| rfc.clone()),
            Filing::Refused("rust-x: refused, --same-layer comes with the --layer it joins".into())
        );
    }

    /// Done-when 3 of rust-add-layer-load: a done-when citing a file:line is refused, the test or function asked instead; a
    /// URL's port, a time or a section number is no line.
    #[test]
    fn a_done_when_citing_a_file_line_is_refused() {
        let _budget = TestBudget::new();
        for cite in [
            "main.rs:2001",
            "`rust/crates/dash/src/main.rs:12:5`",
            "(lib.ex:40)",
            "bin/dash:40",
            "Makefile:12",
        ] {
            let line = words(&[
                "task",
                "add",
                "klar-x",
                "--epic",
                "e",
                "--done-when",
                &format!("1. the guard at {cite} holds: a test"),
            ]);
            let bare = cite.trim_matches(|c| "`()".contains(c));
            assert_eq!(
                refused(&line.iter().map(String::as_str).collect::<Vec<_>>()),
                format!(
                    "klar-x: refused, its done-when cites {bare}, a line that moves under a task: name the test or the \
                     function instead"
                )
            );
        }
        let joined = words(&["task", "update", "klar-x", "--done-when=1. foo.rs:3 holds"]);
        assert!(
            matches!(filing(&joined, &Vec::new), Filing::Refused(r) if r.contains("cites foo.rs:3"))
        );
        let fine = "1. http://100.72.158.115:7788/tasks shows it at 10:30, RFC 20:3 kept, layers::load holds, \
                    nicolas-ha.com:443 answers and mac.tail1.ts.net:7788/tasks too: a test";
        let line = words(&["task", "add", "klar-x", "--epic", "e", "--done-when", fine]);
        assert_eq!(filing(&line, &Vec::new), Filing::Pass);
    }

    /// Done-when 1 of rust-style-no-elixir-edit: an add or update whose check names an .ex or .exs file with any verb but
    /// delete is refused, the no-elixir rule named; its deletion passes, the other way.
    #[test]
    fn a_check_editing_an_elixir_file_is_refused_its_deletion_passes() {
        let _budget = TestBudget::new();
        let add = |check: &str| {
            let line = words(&["task", "add", "x", "--epic", "e", "--done-when", check]);
            filing(&line, &Vec::new)
        };
        for edit in [
            "1. a test. 2. drop the paths from lint.ex's @shell_files",
            "1. `private/dashboard/lib/dashboard/watch.ex` reads the new field",
            "1. test/watch_test.exs covers it",
        ] {
            let Filing::Refused(why) = add(edit) else {
                panic!("{edit}: not refused")
            };
            assert!(
                why.starts_with("x: refused, its check « ")
                    && why.contains("no-elixir step refuses it"),
                "{why}"
            );
        }
        let rust_of = "1. dash lint does what lint.ex did: lint.ex deleted, no callers left";
        assert_eq!(
            add("1. lint.ex deleted, its callers (the Makefile) moved to dash lint"),
            Filing::Pass
        );
        assert_eq!(add("1. delete the lint.ex file: no callers"), Filing::Pass);
        // a delete word elsewhere in the check deletes none of the Elixir files it names (/code-review)
        for edit in [
            rust_of,
            "1. lint.ex deleted, its callers (Makefile, mix.exs aliases) gone",
            "1. task_live.html.heex shows the column",
            "1. private/dashboard/assets/app.css gets the style",
            "1. watch.ex#L40 reads the field",
        ] {
            assert!(
                matches!(add(edit), Filing::Refused(r) if r.contains("no-elixir step")),
                "{edit}"
            );
        }
        let joined = words(&[
            "task",
            "update",
            "x",
            "--done-when=1. watch.ex holds the field",
        ]);
        assert!(
            matches!(filing(&joined, &Vec::new), Filing::Refused(r) if r.contains("names watch.ex"))
        );
        // a check naming no Elixir file, nor a kind alone, nor a tool run from the app's folders, is no edit of one
        assert_eq!(add("1. no .ex file changes: a test"), Filing::Pass);
        assert_eq!(
            add("1. a screen from private/dashboard/test/scripts/shot"),
            Filing::Pass
        );
    }

    /// Done-when 2 of rust-style-no-elixir-edit: a whole-file deletion passes only when it names the callers.
    #[test]
    fn a_whole_file_deletion_passes_only_naming_its_callers() {
        let _budget = TestBudget::new();
        let bare = "1. bin/outbound_gate.py deleted";
        assert_eq!(
            unmeetable(bare).as_deref(),
            Some(
                "its check « 1. bin/outbound_gate.py deleted » deletes a whole file and names none of its callers: \
                 name each, read with `git -c core.fsmonitor=false grep`, or write « no callers » (task-writing)"
            )
        );
        for named in [
            "1. bin/outbound_gate.py deleted, its callers (dash mail send, the hook) moved to dash",
            "1. bin/outbound_gate.py deleted: no callers",
            "1. bin/outbound_gate.py, called by the hook alone, deleted with it",
        ] {
            assert_eq!(unmeetable(named), None, "{named}");
        }
        assert_eq!(unmeetable("1. the old rows deleted from the store"), None);
        // a delete word not beside the file deletes no file (/code-review)
        assert_eq!(unmeetable("1. older.rs skips deleted rows, a test"), None);
        assert!(unmeetable("1. delete x.py, then a test").is_some());
    }

    /// Done-when 3 of rust-style-no-elixir-edit, its lines: `dash task style` lists each open check no gate lets pass, then
    /// their count; one task's alone with its id.
    #[test]
    fn task_style_lists_each_unmeetable_check_and_counts_them() {
        let _budget = TestBudget::new();
        let task = |id: &str, done_when: &str| dash_store::similar::Text {
            id: id.into(),
            done_when: done_when.into(),
            ..Default::default()
        };
        let open = [
            task(
                "a",
                "1. a test. 2. lint.ex drops @shell_files. 3. x.py deleted",
            ),
            task("b", "1. a test"),
        ];
        let all = unmeetable_lines(&open, None);
        let lines: Vec<&str> = all.lines().collect();
        assert_eq!(lines.len(), 3, "{all}");
        assert!(lines[0].starts_with("a: its check « 2. lint.ex drops @shell_files. »"));
        assert!(lines[1].starts_with("a: its check « 3. x.py deleted » deletes a whole file"));
        assert_eq!(lines[2], "2 check(s) no gate lets pass");
        assert_eq!(unmeetable_lines(&open, Some("b")), "");
    }

    /// The task as the list files it, for `runnable`'s `stored`.
    fn filed(prompt: &str, layer: Option<&str>) -> impl Fn() -> Result<Option<Foldable>, String> {
        let row = Foldable {
            id: "t".into(),
            stage: "open".into(),
            prompt: prompt.into(),
            layer: layer.map(String::from),
            run_project: "site_perso".into(),
            ..Foldable::default()
        };
        move || Ok(Some(row.clone()))
    }

    fn none() -> Result<Option<Foldable>, String> {
        Ok(None)
    }

    /// Done-when 1 of run-fields-at-add: `--project <repo>` on an add or an update of a task with no run prompt sends the
    /// older app the default prompt beside it, so its run is made on that project; a prompt given or filed is kept.
    #[test]
    fn project_fills_the_run_s_project_and_a_default_prompt() {
        let _budget = TestBudget::new();
        let prompted = |line: &[&str], stored: &dyn Fn() -> Result<Option<Foldable>, String>| {
            runnable(&words(line), stored).expect("passes")
        };
        let added = prompted(
            &["task", "add", "t", "--epic", "e", "--project", "nanolathe"],
            &none,
        );
        assert_eq!(
            added.0[5..],
            words(&["--project", "nanolathe", "--prompt", START_PROMPT])
        );
        assert!(
            added.1.contains("t: its run gets the default prompt"),
            "{}",
            added.1
        );
        let updated = prompted(
            &["task", "update", "t", "--project", "nanolathe"],
            &filed("", None),
        );
        assert_eq!(
            updated.0[3..],
            words(&["--project", "nanolathe", "--prompt", START_PROMPT])
        );
        // a prompt filed, or given, is never written over
        let kept = prompted(
            &["task", "update", "t", "--project", "nanolathe"],
            &filed("Port it.", None),
        );
        assert_eq!(
            kept.0,
            words(&["task", "update", "t", "--project", "nanolathe"])
        );
        let given = [
            "task",
            "update",
            "t",
            "--project",
            "n",
            "--prompt",
            "Port it.",
        ];
        assert_eq!(prompted(&given, &filed("", None)).0, words(&given));
        // no --project, no read of the row
        let other = ["task", "update", "t", "--title", "T"];
        let unread = || -> Result<Option<Foldable>, String> { panic!("read") };
        assert_eq!(prompted(&other, &unread).0, words(&other));
    }

    /// Done-when 3 of run-fields-at-add: `--owner rust` is refused for a task with no layer or no run prompt, each missing
    /// field named; one naming both, or filed with both, passes.
    #[test]
    fn owner_rust_is_refused_naming_each_missing_field() {
        let _budget = TestBudget::new();
        let line = words(&["task", "update", "t", "--owner", "rust"]);
        let why = runnable(&line, &filed("", None)).expect_err("refused");
        assert!(why.starts_with("t: refused, --owner rust:"), "{why}");
        assert!(why.contains("it has no layer, no run prompt"), "{why}");
        let why = runnable(&line, &filed("", Some("model:task"))).expect_err("refused");
        assert!(
            why.contains("it has no run prompt (") && !why.contains("no layer"),
            "{why}"
        );
        let why = runnable(&line, &filed("Port it.", None)).expect_err("refused");
        assert!(
            why.contains("it has no layer (") && !why.contains("no run prompt"),
            "{why}"
        );
        assert!(runnable(&line, &filed("Port it.", Some("model:task"))).is_ok());
        let both = words(&[
            "task",
            "update",
            "t",
            "--owner",
            "rust",
            "--layer",
            "model:task",
            "--prompt",
            "Port it.",
        ]);
        assert!(runnable(&both, &filed("", None)).is_ok());
        // an add gets the older app's default prompt unless it clears it; its layer is `filing`'s check
        let add = ["task", "add", "rust-t", "--owner", "rust", "--epic", "e"];
        assert!(runnable(&words(&add), &none).is_ok());
        let cleared = words(&[&add[..], &["--prompt", "nil"]].concat());
        let why = runnable(&cleared, &none).expect_err("refused");
        assert!(why.contains("it has no run prompt"), "{why}");
        // an add naming a layer and no owner is filed --owner rust: its prompt is checked the same (/code-review)
        let layered = ["task", "add", "t", "--epic", "e", "--layer", "model:task"];
        let layered = words(&[&layered[..], &["--prompt", "nil"]].concat());
        let why = runnable(&layered, &none).expect_err("refused");
        assert!(why.contains("it has no run prompt"), "{why}");
        // an update of a task the list does not hold says so, never a missing field (/code-review)
        let why = runnable(&line, &none).expect_err("refused");
        assert_eq!(why, "t: refused, the list holds no such task");
    }

    /// A task he owns starts no session: `--project` or `--start` makes no run for it (/code-review of run-fields-at-add).
    #[test]
    fn a_task_he_owns_gets_no_run() {
        let _budget = TestBudget::new();
        let his = Foldable {
            owner: "nicolas".into(),
            run_project: String::new(),
            ..filed("", None)().expect("read").expect("a row")
        };
        let line = words(&["task", "update", "t", "--project", "klar"]);
        let stored = || Ok(Some(his.clone()));
        assert_eq!(runnable(&line, &stored).expect("passes").0, line);
        let start = words(&["task", "update", "t", "--start", "now"]);
        let (sent, _) = runnable(&start, &stored).expect("passes");
        assert_eq!(sent, words(&["task", "update", "t", "--later", "nil"]));
    }

    /// Done-when 4 of run-fields-at-add: `task update <id> --start <why>` clears `:later` (and fills a run prompt it lacks),
    /// said after the older app's answer; a `--start` with no why is refused.
    #[test]
    fn update_takes_start_with_its_why_and_clears_later() {
        let _budget = TestBudget::new();
        let start = words(&[
            "task",
            "update",
            "rust-scripted-edit-fmt",
            "--start",
            "its fix is planned",
        ]);
        let (sent, said) =
            runnable(&start, &filed("Port it.", Some("model:task"))).expect("passes");
        assert_eq!(
            sent,
            words(&["task", "update", "rust-scripted-edit-fmt", "--later", "nil"])
        );
        assert_eq!(
            said,
            "rust-scripted-edit-fmt: its :later cleared, so the next pass may start it (why: its fix is planned)\n"
        );
        let (sent, _) = runnable(&start, &filed("", None)).expect("passes");
        assert!(
            sent.ends_with(&words(&["--later", "nil", "--prompt", START_PROMPT])),
            "{sent:?}"
        );
        // a run with no project gets the task's own, as `Records.start/1` gave it: the runner skips one with none (/code-review)
        let unplaced = Foldable {
            project: "nanolathe".into(),
            run_project: String::new(),
            ..filed("p", None)().expect("read").expect("a row")
        };
        let (sent, said) = runnable(&start, &|| Ok(Some(unplaced.clone()))).expect("passes");
        assert!(
            sent.ends_with(&words(&["--later", "nil", "--project", "nanolathe"])),
            "{sent:?}"
        );
        assert!(
            said.ends_with("its run's project is its own, nanolathe\n"),
            "{said}"
        );
        for bare in [
            &["--start"][..],
            &["--start", " "],
            &["--start", "--title", "T"],
        ] {
            let line = words(&[&["task", "update", "t"][..], bare].concat());
            let why = runnable(&line, &filed("p", None)).expect_err("refused");
            assert_eq!(
                why,
                "t: refused, --start takes why it starts now, --start <why>"
            );
        }
        let both = words(&["task", "update", "t", "--start", "now", "--later", "x"]);
        assert!(runnable(&both, &filed("p", None)).is_err());
    }

    /// Done-when 4 of run-fields-at-add: an empty `--later` or `--duplicate-of` on an update clears it, sent as `nil`; an add's
    /// words stay as they came.
    #[test]
    fn an_empty_later_or_duplicate_of_clears_it() {
        let _budget = TestBudget::new();
        for flag in ["--later", "--duplicate-of"] {
            for empty in ["", "  "] {
                let line = words(&["task", "update", "t", flag, empty, "--title", "T"]);
                let (sent, _) = runnable(&line, &none).expect("passes");
                assert_eq!(
                    sent,
                    words(&["task", "update", "t", flag, "nil", "--title", "T"])
                );
            }
        }
        let add = words(&["task", "add", "t", "--epic", "e", "--later", ""]);
        assert_eq!(runnable(&add, &none).expect("passes").0, add);
    }

    /// Done-when 3 of rust-task-rfc-judge, the front door's half: a Rust task's check adding interaction with no route is
    /// refused; the same check naming its handler, or any check of a task that is not Rust's, passes.
    #[test]
    fn task_add_refuses_a_check_adding_interaction_with_no_route() {
        let _budget = TestBudget::new();
        let add = |id: &str, checks: &str| {
            let line = words(&[
                "task",
                "add",
                id,
                "--epic",
                "rust-poc",
                "--layer",
                "rfc:dataflow",
                "--done-when",
                checks,
            ]);
            filing(&line, &|| vec!["dataflow".into()])
        };
        let checks = "1. the board reads the view: a test 2. a new-issue menu opens from the sidebar: a test";
        assert_eq!(
            add("rust-x", checks),
            Filing::Refused(
                "rust-x: refused, its check « 2. a new-issue menu opens from the sidebar: a test » adds interaction (menu) \
                 and names no route: name its Dioxus handler, run in the window and through liveview, never a script of \
                 ours (rfc-rust-unified#ui-lib)"
                    .into()
            )
        );
        let routed = "1. a new-issue menu opens from the sidebar, an onclick handler run in the window and through liveview";
        assert!(matches!(add("rust-x", routed), Filing::Keep { .. }));
        let klar = words(&[
            "task",
            "add",
            "klar-x",
            "--epic",
            "e",
            "--done-when",
            "1. the button sends it",
        ]);
        assert_eq!(filing(&klar, &Vec::new), Filing::Pass);
    }
}
