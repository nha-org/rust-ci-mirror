//! `dash task add`'s duplicate check at the front door, before the older app files it (task task-dedupe; @nha, 2026-11-03:
//! « this is either double work or stepping in the exact same area in parallel »; 2026-11-11: « something like the result of
//! filing a new task returns similar tasks », « this could be used to detect dupes or to add dependencies/or just related
//! tasks »). A close open task refuses the add, the closest three printed in full; `--distinct-from` files it past them only
//! with `--because` giving each id its own reason; every add that goes on prints its similar tasks, each marked with the line
//! that acts on it. The scoring is `dash_store::similar`'s, the older app's `TaskStyle.overlaps/3` ported.

use dash_store::similar::{self, Mark, Near, Text};

/// Every value of `flag` after the id, in order: the word after it, read at a flag place only, so a field's text that reads
/// « --into » is never a flag (/code-review of task-dedupe, as the older app's `dup_pick` reads them). A flag followed by
/// another flag (`--dry-run`, a bare `--start`) holds no value. ponytail: the `--flag=value` form the older app never parses
/// is no flag here either.
fn values<'a>(args: &'a [String], flag: &str) -> Vec<&'a str> {
    let mut out = vec![];
    let mut at = 3;
    while at < args.len() {
        let value = args.get(at + 1).filter(|v| !v.starts_with("--"));
        if args[at] == flag {
            out.extend(value.map(String::as_str));
        }
        at += if value.is_some() { 2 } else { 1 };
    }
    out
}

/// What the check makes of an add: nothing to say, a refusal, or the similar tasks said once it is filed.
#[derive(Debug, PartialEq)]
pub enum Said {
    Nothing,
    Refused(String),
    After(String),
}

/// `task add <id> …` checked against `open`, which reads the open tasks of a project. `stdin` is what `-` stands for. Not an
/// add, an add `--into` a task, or one filed closed (`--status`, not open): nothing. A list that cannot be read is said after
/// the add and refuses nothing: the older app's own check still runs.
pub fn check(
    args: &[String],
    stdin: &str,
    open: &dyn Fn(&str) -> Result<Vec<Text>, String>,
) -> Said {
    let (Some("task"), Some("add"), Some(id)) = (
        args.first().map(String::as_str),
        args.get(1).map(String::as_str),
        args.get(2),
    ) else {
        return Said::Nothing;
    };
    let one = |flag: &str| values(args, flag).first().copied();
    let status = one("--status").map(|s| s.trim_start_matches(':'));
    if one("--into").is_some() || status.is_some_and(|s| s != "open") {
        return Said::Nothing;
    }
    let text = |v: &str| match v {
        "-" => stdin.to_string(),
        v => v.to_string(),
    };
    let new = Text {
        id: id.clone(),
        title: one("--title").map(text).unwrap_or_default(),
        why: one("--why").map(text).unwrap_or_default(),
        done_when: (values(args, "--done-when").into_iter().map(text))
            .collect::<Vec<_>>()
            .join(" "),
        layer: one("--layer").map(String::from),
    };
    let project = one("--project").unwrap_or("site_perso");
    let others = match open(project) {
        Ok(o) => o,
        Err(e) => return Said::After(format!("{id}: its similar tasks could not be read: {e}\n")),
    };
    let apart: Vec<String> = (values(args, "--distinct-from").iter())
        .flat_map(|ids| ids.split(','))
        .map(|i| i.trim().to_string())
        .filter(|i| !i.is_empty())
        .fold(vec![], |mut ids, i| {
            // an id named twice is named once (/code-review: two labels at one place cut a reason backwards)
            if !ids.contains(&i) {
                ids.push(i);
            }
            ids
        });
    let scored = similar::scored(&new, &others);
    // the twins it refuses over: close on its layer, a file or its title, or porting a file its checks port
    let twins = similar::duplicates(&new, &others, &scored);
    let close: Vec<&Near> = twins.iter().collect();
    if !apart.is_empty() {
        if let Err(why) = reasons(&apart, one("--because").map(text).as_deref(), &others) {
            return Said::Refused(format!("{id}: refused, nothing written: {why}\n"));
        }
        let missing: Vec<&str> = (close.iter())
            .filter(|n| !apart.contains(&n.id))
            .map(|n| n.id.as_str())
            .collect();
        if !missing.is_empty() {
            let shown = shown(&new, &others, &scored, &twins, &apart);
            return Said::Refused(format!(
                "{id}: refused, nothing written: --distinct-from also close, not named: {}\n{}{shown}",
                missing.join(","),
                distinct_line(id, &close)
            ));
        }
    }
    let shown = shown(&new, &others, &scored, &twins, &apart);
    if apart.is_empty() && !close.is_empty() {
        return Said::Refused(format!(
            "{id}: refused, nothing written: {} open task(s) close to it. Update one instead (its line below), or file it \
             anyway, each named with its own reason:\n{}{shown}",
            close.len(),
            distinct_line(id, &close)
        ));
    }
    match shown.is_empty() {
        true => Said::After(format!("{id}: no similar open task\n")),
        false => Said::After(shown),
    }
}

/// The one line that files `id` past every close task, each named with a reason to fill in.
fn distinct_line(id: &str, close: &[&Near]) -> String {
    let ids: Vec<&str> = close.iter().map(|n| n.id.as_str()).collect();
    let because: Vec<String> = ids
        .iter()
        .map(|i| format!("{i}: <why it differs>"))
        .collect();
    format!(
        "  dash task add {id} … --distinct-from {} --because \"{}\"\n",
        ids.join(","),
        because.join("; ")
    )
}

/// The similar tasks of `new`, marked, each with its line; the three closest duplicates in full (why, files, shared terms).
fn shown(new: &Text, others: &[Text], scored: &[Near], twins: &[Near], apart: &[String]) -> String {
    let found = similar::similar(new, others, scored, twins, apart);
    if found.is_empty() {
        return String::new();
    }
    let mut out = format!(
        "{}: similar open tasks, each marked, with the line to run:\n",
        new.id
    );
    for (i, (near, mark)) in found.iter().enumerate() {
        let o = others.iter().find(|o| o.id == near.id);
        let title = o.map_or("", |o| o.title.as_str());
        out += &format!(
            "  {}. {} [{}] {title}\n     run: {}\n",
            i + 1,
            near.id,
            mark.word(),
            mark.line(&new.id, &near.id)
        );
        if let (true, Mark::Duplicate, Some(o)) = (i < 3, mark, o) {
            let why: String = o.why.chars().take(160).collect();
            let files = similar::files_named(o);
            let files = match files.is_empty() {
                true => "none".to_string(),
                false => files.join(" "),
            };
            let shared: Vec<&str> = near.shared.iter().take(8).map(String::as_str).collect();
            out += &format!(
                "     why: {why}\n     files: {files}\n     shared: {}\n",
                shared.join(" ")
            );
        }
    }
    out
}

/// Whether `because` names each id of `apart` with its own reason (« a: why; b: why »), every one an open task.
fn reasons(apart: &[String], because: Option<&str>, open: &[Text]) -> Result<(), String> {
    let gone: Vec<&str> = (apart.iter())
        .filter(|i| !open.iter().any(|o| &o.id == *i))
        .map(String::as_str)
        .collect();
    if !gone.is_empty() {
        return Err(format!("--distinct-from {}: no open task", gone.join(",")));
    }
    let form: Vec<String> = apart.iter().map(|i| format!("{i}: <why>")).collect();
    let form = format!("--because \"{}\"", form.join("; "));
    let Some(because) = because.filter(|b| !b.trim().is_empty()) else {
        return Err(format!(
            "--distinct-from needs each id's own reason, {form}"
        ));
    };
    // where each id's reason starts: « id: » at a word's start
    let mut at: Vec<(usize, usize, &str)> = vec![];
    for id in apart {
        let label = format!("{id}:");
        let found = (because.match_indices(&label)).find(|(i, _)| {
            let before = because[..*i].chars().next_back();
            before.is_none_or(|c| !(c.is_alphanumeric() || c == '-' || c == '_'))
        });
        match found {
            Some((i, _)) => at.push((i, i + label.len(), id)),
            None => return Err(format!("--because names no reason for {id}: {form}")),
        }
    }
    at.sort();
    let mut said: Vec<String> = vec![];
    for (n, (_, from, id)) in at.iter().enumerate() {
        let to = at
            .get(n + 1)
            .map_or(because.len(), |next| next.0.max(*from));
        let reason = because[*from..to]
            .trim()
            .trim_end_matches([';', ',', '.'])
            .trim();
        if reason.is_empty() {
            return Err(format!("--because gives {id} no reason: {form}"));
        }
        if said.iter().any(|s| s.eq_ignore_ascii_case(reason)) {
            return Err(format!(
                "--because gives {id} the reason another id has: each its own, {form}"
            ));
        }
        said.push(reason.to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::memory::TestBudget;

    fn t(id: &str, title: &str, why: &str, done_when: &str) -> Text {
        Text {
            id: id.into(),
            title: title.into(),
            why: why.into(),
            done_when: done_when.into(),
            layer: None,
        }
    }

    /// The incident's twins (incident-2026-11-03-arrow-not-ready, problem J): resume modes filed twice, and two neighbours.
    fn board() -> Vec<Text> {
        vec![
            t(
                "account-ends-planned",
                "Session resume modes: resume, brief, summary",
                "A session resumes by its id, from its brief, or from its brief and summary",
                "1. sessions.ex resume modes picked by flag: a test",
            ),
            t(
                "threads-resume-first",
                "Threads resume first: resume modes id, brief, summary",
                "Resume a session from its brief and summary",
                "1. resume modes chosen with --mode: a test",
            ),
            t(
                "pane-names",
                "Pane names follow their window",
                "Names drift",
                "1. lib/dashboard/orch/sessions.ex renames panes: a test",
            ),
            t(
                "mail-poll",
                "Mail poll reads klar",
                "Inbox",
                "1. polled: a test",
            ),
        ]
    }

    fn args(words: &[&str]) -> Vec<String> {
        words.iter().map(|w| w.to_string()).collect()
    }

    fn add(extra: &[&str]) -> Said {
        let mut a = args(&[
            "task",
            "add",
            "resume-again",
            "--title",
            "Resume modes for sessions: id, brief, summary",
            "--why",
            "Pick how a session resumes: its id, its brief, its brief and summary",
            "--done-when",
            "-",
        ]);
        a.extend(args(extra));
        let open = |project: &str| {
            assert_eq!(project, "site_perso");
            Ok(board())
        };
        check(
            &a,
            "1. lib/dashboard/orch/sessions.ex picks the resume mode: a test",
            &open,
        )
    }

    /// Done-when 1: a close match refuses, printing the top 3 to update with the line for each.
    #[test]
    fn a_twin_is_refused_naming_the_tasks_to_update() {
        let _budget = TestBudget::new();
        let Said::Refused(said) = add(&[]) else {
            panic!("not refused: {:?}", add(&[]));
        };
        assert!(
            said.starts_with("resume-again: refused, nothing written: 2 open task(s) close to it"),
            "{said}"
        );
        assert!(
            said.contains("1. account-ends-planned [duplicate]"),
            "{said}"
        );
        assert!(
            said.contains("2. threads-resume-first [duplicate]"),
            "{said}"
        );
        assert!(
            said.contains("run: dash task add resume-again … --into account-ends-planned"),
            "{said}"
        );
        assert!(
            said.contains("     why: A session resumes by its id"),
            "{said}"
        );
        assert!(
            said.contains("--distinct-from account-ends-planned,threads-resume-first --because"),
            "{said}"
        );
        assert!(!said.contains("mail-poll"), "{said}");
    }

    /// Done-when 2: --distinct-from files it only when --because names each listed id with its own reason.
    #[test]
    fn distinct_from_needs_each_id_s_own_reason() {
        let _budget = TestBudget::new();
        let ids = "account-ends-planned,threads-resume-first";
        let refused = |because: &[&str]| {
            let mut a = vec!["--distinct-from", ids];
            a.extend(because);
            match add(&a) {
                Said::Refused(why) => why,
                other => panic!("{because:?} not refused: {other:?}"),
            }
        };
        assert!(refused(&[]).contains("--distinct-from needs each id's own reason"));
        let one = refused(&["--because", "the modes ship in a new place"]);
        assert!(
            one.contains("--because names no reason for account-ends-planned"),
            "{one}"
        );
        let half = refused(&["--because", "account-ends-planned: the account side"]);
        assert!(
            half.contains("no reason for threads-resume-first"),
            "{half}"
        );
        let same = refused(&[
            "--because",
            "account-ends-planned: other area; threads-resume-first: other area",
        ]);
        assert!(same.contains("the reason another id has"), "{same}");
        let empty = refused(&[
            "--because",
            "account-ends-planned: ; threads-resume-first: the threads page",
        ]);
        assert!(
            empty.contains("gives account-ends-planned no reason"),
            "{empty}"
        );
        let short = match add(&[
            "--distinct-from",
            "account-ends-planned",
            "--because",
            "account-ends-planned: x",
        ]) {
            Said::Refused(why) => why,
            other => panic!("{other:?}"),
        };
        assert!(
            short.contains("also close, not named: threads-resume-first"),
            "{short}"
        );
        // an id named twice is named once, never a panic; the refusal gives the whole line to run again
        let twice = match add(&[
            "--distinct-from",
            "account-ends-planned,account-ends-planned",
            "--because",
            "account-ends-planned: x",
        ]) {
            Said::Refused(why) => why,
            other => panic!("{other:?}"),
        };
        assert!(
            twice.contains("also close, not named: threads-resume-first"),
            "{twice}"
        );
        assert!(
            twice.contains("--distinct-from account-ends-planned,threads-resume-first --because"),
            "{twice}"
        );
        // a flag word inside a field's text is no flag: the check still runs
        assert!(matches!(
            add(&["--labels", "--into is a word here"]),
            Said::Refused(_)
        ));
        let gone = match add(&["--distinct-from", "nope", "--because", "nope: x"]) {
            Said::Refused(why) => why,
            other => panic!("{other:?}"),
        };
        assert!(
            gone.contains("--distinct-from nope: no open task"),
            "{gone}"
        );
        let filed = add(&[
            "--distinct-from",
            ids,
            "--because",
            "account-ends-planned: that one plans the account end; threads-resume-first: that one is the threads page",
        ]);
        let Said::After(said) = filed else {
            panic!("refused: {filed:?}");
        };
        assert!(!said.contains("[duplicate]"), "{said}");
    }

    /// Done-when 4: every add prints its similar tasks, each marked duplicate, depends-on or related, with the line to run.
    #[test]
    fn every_add_prints_its_similar_tasks_marked() {
        let _budget = TestBudget::new();
        let filed = add(&[
            "--distinct-from",
            "account-ends-planned,threads-resume-first",
            "--because",
            "account-ends-planned: the account side; threads-resume-first: the threads page",
        ]);
        let Said::After(said) = filed else {
            panic!("refused: {filed:?}");
        };
        assert!(
            said.starts_with(
                "resume-again: similar open tasks, each marked, with the line to run:"
            ),
            "{said}"
        );
        assert!(said.contains("[depends-on] Pane names follow their window\n     run: dash task update resume-again --blocked-by pane-names"), "{said}");
        assert!(said.contains("[related] Session resume modes"), "{said}");
        assert!(
            said.contains("run: dash task update resume-again --related account-ends-planned"),
            "{said}"
        );
        // an add with none similar says so
        let open = |_: &str| Ok(board());
        let lone = args(&[
            "task",
            "add",
            "x",
            "--title",
            "Paint the favicon",
            "--why",
            "Brand",
            "--done-when",
            "1. shown",
        ]);
        assert_eq!(
            check(&lone, "", &open),
            Said::After("x: no similar open task\n".into())
        );
        // not an add, an add into a task, or one filed closed: nothing
        assert_eq!(
            check(&args(&["task", "show", "x"]), "", &open),
            Said::Nothing
        );
        assert_eq!(
            check(&args(&["task", "add", "x", "--into", "y"]), "", &open),
            Said::Nothing
        );
        assert_eq!(
            check(
                &args(&["task", "add", "x", "--status", "backlog"]),
                "",
                &open
            ),
            Said::Nothing
        );
    }

    /// The board and adds of 2026-11-12 (task dedupe-duplicate-precision): each open task an add was refused over, its text and
    /// layer as the stores held them that day.
    fn fixture() -> (Vec<Text>, Vec<Text>) {
        let json: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/dedupe-2026-11-12.json"))
                .expect("fixture");
        let texts = |key: &str| -> Vec<Text> {
            (json[key].as_array().expect(key).iter())
                .map(|t| Text {
                    id: t["id"].as_str().unwrap_or_default().into(),
                    title: t["title"].as_str().unwrap_or_default().into(),
                    why: t["why"].as_str().unwrap_or_default().into(),
                    done_when: t["done_when"].as_str().unwrap_or_default().into(),
                    layer: t["layer"].as_str().map(String::from),
                })
                .collect()
        };
        (texts("adds"), texts("board"))
    }

    /// The `task add` of `new` as its flags file it, against `board`.
    fn filed(new: &Text, board: &[Text]) -> Said {
        let mut a = args(&[
            "task",
            "add",
            &new.id,
            "--title",
            &new.title,
            "--why",
            &new.why,
            "--done-when",
            &new.done_when,
        ]);
        if let Some(layer) = &new.layer {
            a.extend(args(&["--layer", layer]));
        }
        let open = |_: &str| Ok(board.to_vec());
        check(&a, "", &open)
    }

    /// Done-when 1 of dedupe-duplicate-precision: today's three adds, refused over 23 « duplicates » that shared only words
    /// (rust, review, still, land, session), file with each of those tasks marked related, none duplicate.
    #[test]
    fn today_s_three_adds_share_only_words_and_mark_no_duplicate() {
        let _budget = TestBudget::new();
        let (adds, board) = fixture();
        assert_eq!(board.len(), 23);
        let mut related = 0;
        for new in &adds {
            // the overlap that refused it is still there: close by score, two terms or more shared
            let scored = similar::scored(new, &board);
            assert!(
                !similar::close(&scored).is_empty(),
                "{}: no close match left to test",
                new.id
            );
            let said = match filed(new, &board) {
                Said::After(said) => said,
                other => panic!("{}: {other:?}", new.id),
            };
            assert!(!said.contains("[duplicate]"), "{said}");
            related += said.matches("[related]").count();
            // and against each alone, where every term weighs 1 and the scores run higher (the live board of 1012 open
            // tasks made ci-speed-lane-findings close, which this board of 23 does not)
            for one in &board {
                let said = format!("{:?}", filed(new, std::slice::from_ref(one)));
                assert!(
                    !said.contains("[duplicate]"),
                    "{} against {}: {said}",
                    new.id,
                    one.id
                );
            }
        }
        assert!(
            related >= 3,
            "each add shows its word matches as related: {related}"
        );
    }

    /// Done-when 2: a close match on its layer, naming one of its files, or with a twin title is still a duplicate.
    #[test]
    fn a_real_twin_by_layer_file_or_title_is_still_a_duplicate() {
        let _budget = TestBudget::new();
        let (adds, board) = fixture();
        let new = adds
            .iter()
            .find(|a| a.id == "rust-review-next-port")
            .expect("add");
        let marked = |board: &[Text], id: &str| match filed(new, board) {
            Said::Refused(said) => said.contains(&format!("{id} [duplicate]")),
            Said::After(said) => said.contains(&format!("{id} [duplicate]")),
            Said::Nothing => false,
        };
        let scored = similar::scored(new, &board);
        let closest = similar::close(&scored)[0].id.clone();
        let with = |id: &str, edit: &dyn Fn(&mut Text)| {
            let mut b = board.clone();
            edit(b.iter_mut().find(|t| t.id == id).expect(id));
            b
        };
        // the closest match by words alone: related
        assert!(!marked(&board, &closest));
        // same layer
        let layer = with(&closest, &|t| t.layer = Some("rfc:conformance".into()));
        assert!(marked(&layer, &closest));
        // a file both name
        let mut named = new.clone();
        named.done_when += " 5. rust/crates/forge/src/review.rs holds it.";
        let file = with(&closest, &|t| {
            t.done_when += " 5. rust/crates/forge/src/review.rs claims."
        });
        let open = |_: &str| Ok(file.clone());
        let a = args(&[
            "task",
            "add",
            &named.id,
            "--title",
            &named.title,
            "--why",
            &named.why,
            "--done-when",
            &named.done_when,
        ]);
        assert!(
            matches!(check(&a, "", &open), Said::Refused(s) if s.contains(&format!("{closest} [duplicate]")))
        );
        // a twin title
        let title = with(&closest, &|t| {
            t.title = "dash review next hands out PRs from Rust, never Elixir".into()
        });
        assert!(marked(&title, &closest));
        // filed again, reworded, with no layer, file or twin title: past TWIN_FLOOR, still a duplicate (/code-review)
        let mut again = new.clone();
        again.id = "review-next-in-rust".into();
        again.title = "Port the PR handout so the lane needs no older app".into();
        again.layer = None;
        let filed_again = filed(&again, std::slice::from_ref(new));
        assert!(
            matches!(&filed_again, Said::Refused(s) if s.contains("rust-review-next-port [duplicate]")),
            "{filed_again:?}"
        );
    }

    /// Done-when 3: outbound-gate and rust-mail-send-port both delete outbound_gate.py: the second is a duplicate at its add,
    /// and held as one at its start.
    #[test]
    fn outbound_gate_py_s_two_ports_are_duplicates_at_add_and_at_start() {
        use dash_core::tasks::{Pick, Policy, Queue, Queued, Snapshot, Step, Verdict, plan};
        let _budget = TestBudget::new();
        let gate = Text {
            id: "outbound-gate".into(),
            title: "One Rust gate on every outgoing message; dash mail send in Rust; the .py goes".into(),
            why: "(@nha, 2026-11-10: « all these python scripts are also very brittle. ») rust-hooks-rest-port kept \
                  outbound_gate.py: Dashboard.Webmail's send still calls it."
                .into(),
            done_when: "1. One Rust gate every send path calls; a test per path. 2. dash mail send <id> is Rust and calls it; \
                        a test. 4. Dashboard.Webmail's send named retired; outbound_gate.py and its drafts.check call \
                        deleted; scripts count lowered."
                .into(),
            layer: Some("model:mail".into()),
        };
        let port = Text {
            id: "rust-mail-send-port".into(),
            title: "dash mail send runs in Rust through a Rust outbound gate; the .py goes".into(),
            why: gate.why.clone(),
            done_when: "1. dash mail send <id> is Rust: the outbound gate's checks run in Rust; a test per refusal. 3. \
                        Dashboard.Webmail's send path named retired; outbound_gate.py and its drafts.check call deleted; \
                        scripts count lowered."
                .into(),
            layer: Some("model:outbound".into()),
        };
        assert_eq!(similar::ported(&port), ["outbound_gate.py"]);
        // at add, on another layer, close or not
        let said = match filed(&port, std::slice::from_ref(&gate)) {
            Said::Refused(said) => said,
            other => panic!("{other:?}"),
        };
        assert!(said.contains("1. outbound-gate [duplicate]"), "{said}");
        // a check naming the file without porting it is no duplicate
        let mut edits = gate.clone();
        edits.id = "linkedin-send-hold-out".into();
        edits.done_when = "1. holds are gone from outbound_gate.py, selftest green (rust-mail-send-port ports it)".into();
        assert!(similar::ported(&edits).is_empty());
        // nor where a port lands; an unbalanced parenthesis hides nothing after it (/code-review)
        edits.done_when =
            "2. Dashboard.Webmail ported to rust/crates/dash/src/mail.rs; a test".into();
        assert!(similar::ported(&edits).is_empty());
        edits.done_when = "1. (see rfc 4. delete outbound_gate.py".into();
        assert_eq!(similar::ported(&edits), ["outbound_gate.py"]);
        // nor one citing it in the possessive, as this task's own check does
        edits.done_when = "3. Two open tasks whose checks port or delete one file are marked duplicate at add and at start; \
                           a test with outbound_gate.py's two ports."
            .into();
        assert!(similar::ported(&edits).is_empty());
        // at start: the first in the list goes, the other is held as its duplicate
        struct First;
        impl Policy for First {
            fn pick(&self, snapshot: &Snapshot) -> Pick {
                match snapshot.queue.tasks.first() {
                    Some(t) => Pick::Start {
                        id: t.id.clone(),
                        account: None,
                        pool: None,
                    },
                    None => Pick::Wait("none".into()),
                }
            }
            fn after(&self, _: &Verdict, _: u32, _: bool) -> Step {
                Step::Keep
            }
            fn retry(&self, _: u32) -> bool {
                false
            }
        }
        let queued = |t: &Text| Queued {
            id: t.id.clone(),
            ported: similar::ported(t),
            ..Queued::default()
        };
        let queue = Queue {
            seats: 5,
            seats_why: String::new(),
            tasks: vec![queued(&gate), queued(&port)],
        };
        let planned = plan(&First, queue, &[], 0, vec![], None);
        let held = planned
            .waiting
            .iter()
            .find(|h| h.id == "rust-mail-send-port")
            .map(|h| h.why.as_str());
        assert_eq!(
            held,
            Some("duplicate of outbound-gate: both port or delete outbound_gate.py")
        );
        assert_eq!(
            planned.next,
            Pick::Start {
                id: "outbound-gate".into(),
                account: None,
                pool: None
            }
        );
        // a twin that cannot start (held by him) leaves the work to the other; two paths of one name are two files
        let mut held_gate = queued(&gate);
        held_gate.held = Some("he moved it to Backlog".into());
        let queue = Queue {
            seats: 5,
            seats_why: String::new(),
            tasks: vec![held_gate, queued(&port)],
        };
        let planned = plan(&First, queue, &[], 0, vec![], None);
        assert!(
            !planned
                .waiting
                .iter()
                .any(|h| h.id == "rust-mail-send-port"),
            "{:?}",
            planned.waiting
        );
        let path = |id: &str, f: &str| Queued {
            id: id.into(),
            ported: vec![f.into()],
            ..Queued::default()
        };
        let queue = Queue {
            seats: 5,
            seats_why: String::new(),
            tasks: vec![
                path("a", "rust/crates/dash/src/main.rs"),
                path("b", "rust/crates/site/src/main.rs"),
            ],
        };
        let planned = plan(&First, queue, &[], 0, vec![], None);
        assert!(
            !planned
                .waiting
                .iter()
                .any(|h| h.why.starts_with("duplicate")),
            "{:?}",
            planned.waiting
        );
    }
}
