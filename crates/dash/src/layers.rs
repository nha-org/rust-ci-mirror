//! `dash task add` for a Rust task names the layer it changes, checked at the front door before the older app files it (task
//! rust-task-names-layer; @nha, 2026-11-09: « We don't do one-offs. We must prevent this systematically. », « HTML is an impl
//! detail of our components that are reusable. Components are impl. details of our pages »). A layer is `design:<name>` (a
//! component or view /design shows), `model:<name>` or `rfc:<section id>` (of rfc-rust-unified or rfc-rust-plugins-ui). The
//! older app knows no `--layer`: the front door takes it out, and Rust's store keeps it for the picker and the brief.
//!
//! Every new task belongs to an epic or is urgent with a deadline, a `--deadline` kept the same way (RFC 20e rule 2; task
//! rust-orch-pick-dynamic; @nha, 2026-11-09: « Tasks must belong to an epic OR be urgent », « Urgent tasks are time-bound
//! usually. Ex. A deadline to prepare a meeting. »).
use std::path::Path;

/// The data shapes Rust holds, a task naming one when it changes the shape itself: the stores' records and the document model
/// every page and message is drawn from (rust/CLAUDE.md rule 7, canvist_model). ponytail: kept by hand; read from the stores'
/// schemas once one is added without a line here.
pub const MODELS: [&str; 9] = [
    "document", "task", "comment", "draft", "mail", "key", "fact", "agent", "access",
];

/// The RFCs whose sections a Rust task may name (rust/CLAUDE.md rule 8).
const RFCS: [&str; 2] = [
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
    Keep {
        id: String,
        layer: Option<String>,
        deadline: Option<String>,
        dry: bool,
        update: bool,
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
    for flag in ["--layer", "--deadline"] {
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
    if !update && value(args, "--into").is_none() {
        let named = |e: &str| !e.trim().is_empty() && e != "nil" && !e.starts_with("--");
        let epic = value(args, "--epic").is_some_and(named);
        let urgent = value(args, "--priority") == Some("1") && day.is_some();
        if !epic && !urgent {
            return Err(EPIC_OR_URGENT.into());
        }
    }
    let mut taken: Vec<usize> = layer.into_iter().chain(day_at).collect();
    if taken.is_empty() {
        return Ok(Filing::Pass);
    }
    taken.sort_unstable_by(|a, b| b.cmp(a));
    let mut kept = args.to_vec();
    for at in taken {
        kept.drain(at..=at + 1);
    }
    Ok(Filing::Keep {
        id: id.into(),
        layer: layer.map(|at| args[at + 1].clone()),
        deadline: day.cloned(),
        dry: args.iter().any(|a| a == "--dry-run"),
        update,
        args: kept,
    })
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
                shown
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
        assert_eq!(args, words(&["task", "add", "rust-x", "--priority", "1"]));
    }
}
