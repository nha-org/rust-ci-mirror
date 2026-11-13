//! `dash task report <id> <file>`'s proof check, in Rust before the older app files the record (`Dashboard.Orch.Reports`
//! keeps the store, the Knowledge gate and its own shape checks): each done-when part a done report claims quotes its own proof (task
//! report-proof-quotes; @nha, 2026-10-26: « It needs to be convincing. »; 4 PM rounds lost on 2026-11-10 to reports that
//! named a test per part and quoted no assertion).

use serde_json::Value;

/// Task `id`'s done-when numbered parts: « 1. a 2. b » is `[a, b]`, a « N. » counting only at the start or after a blank,
/// so a heading before « 1. » is dropped (the older app's `unproven/2` reads markers anywhere); text with no « 1. » is one part.
pub fn parts(done_when: &str) -> Vec<String> {
    let text = done_when.trim();
    let first = (text.match_indices("1. "))
        .find(|(at, _)| text[..*at].is_empty() || text[..*at].ends_with(char::is_whitespace));
    let Some((at, _)) = first else {
        return vec![text.to_string()]
            .into_iter()
            .filter(|t| !t.is_empty())
            .collect();
    };
    let mut rest = &text[at + 3..];
    let mut out = vec![];
    loop {
        let marker = format!("{}. ", out.len() + 2);
        let next = (rest.match_indices(&marker))
            .find(|(at, _)| rest[..*at].ends_with(char::is_whitespace));
        match next {
            Some((at, _)) => {
                out.push(rest[..at].trim().to_string());
                rest = &rest[at + marker.len()..];
            }
            None => {
                out.push(rest.trim().to_string());
                return out;
            }
        }
    }
}

/// The parts `text` names: « part 2 », « parts 1, 3 and 5 », « parts 2-4 », « done-when 4 ». Numbers past `of` are not parts.
fn named_in(text: &str, of: usize) -> Vec<usize> {
    numbers(text, of, true)
}

/// The numbers in `text` up to `of`, « 2-4 » and « 2 to 4 » read as every number between; `after_word`: only those right
/// after « part », « parts » or « done-when ».
fn numbers(text: &str, of: usize, after_word: bool) -> Vec<usize> {
    let lower = text.to_lowercase();
    let words: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric() && c != '-')
        .flat_map(|w| split_range(w))
        .filter(|w| !w.is_empty())
        .collect();
    let mut out = vec![];
    let mut i = 0;
    while i < words.len() {
        let named = ["part", "parts", "done-when"].contains(&words[i]);
        if !named && (after_word || words[i].parse::<usize>().is_err()) {
            i += 1;
            continue;
        }
        let mut j = if named { i + 1 } else { i };
        let mut last = None;
        while let Some(w) = words.get(j) {
            match (w.parse::<usize>(), *w) {
                (Ok(n), _) => {
                    // « 2 - 4 » or « 2 to 4 »: every part between, bounded first, so « 1-4000000000 » builds nothing
                    let ranged = last.filter(|_| ["-", "to"].contains(&words[j - 1]));
                    if let Some(from) = ranged.filter(|from| *from <= n && n <= of) {
                        out.extend(from..=n);
                    }
                    out.push(n);
                    last = Some(n);
                }
                (_, "and" | "-" | "to" | "&") => {}
                _ => break,
            }
            j += 1;
        }
        i = j.max(i + 1);
    }
    out.retain(|n| (1..=of).contains(n));
    out.sort_unstable();
    out.dedup();
    out
}

/// « 2-4 » read as three words, so a range is seen.
fn split_range(word: &str) -> Vec<&str> {
    match word.split_once('-') {
        Some((a, b)) if a.parse::<usize>().is_ok() && b.parse::<usize>().is_ok() => vec![a, "-", b],
        _ => vec![word],
    }
}

/// The parts a check's `part` field claims: its numbers (« 1, 2 on / », « 1-3 »), a number or a list of them; « all parts »
/// names none, so it is no part's own proof. With one part, any check is that part's.
fn claimed(part: &Value, of: usize) -> Vec<usize> {
    if of == 1 {
        return vec![1];
    }
    let text = match part {
        Value::Array(each) => each.iter().map(text_of).collect::<Vec<_>>().join(", "),
        other => text_of(other),
    };
    numbers(&text, of, false)
}

fn text_of(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// Whether an output line only names a test or counts them (nextest's « PASS [0.01s] crate tests::name », cargo's « test x
/// ... ok », a summary), so it quotes no assertion and no output of the command's own.
// ponytail: a list of runner shapes; a runner printing another shape counts as a quote until it is added here
fn names_only(line: &str) -> bool {
    let l = line.trim().to_lowercase();
    let first = l.split_whitespace().next().unwrap_or_default();
    let mut rest = l[first.len()..].to_string();
    for (open, close) in [('[', ']'), ('(', ')')] {
        while let (Some(a), Some(b)) = (rest.find(open), rest.find(close)) {
            if b < a {
                break;
            }
            rest.replace_range(a..=b, " ");
        }
    }
    let counted = [
        "summary",
        "starting",
        "running",
        "finished",
        "compiling",
        "------------",
        "────────────",
    ];
    l.is_empty()
        || counted.iter().any(|c| l.starts_with(c))
        || l.starts_with("test result:")
        || ["randomized with seed", "doc-tests", "warning:", "finished in"].iter().any(|c| l.starts_with(c))
        || l.chars().all(|c| c == '.' || c == '*' || c.is_whitespace())
        // mix test's « 5 tests, 0 failures »
        || (first.parse::<u64>().is_ok() && l.contains(" test") && l.contains("failure"))
        || (first == "test" && l.ends_with(" ok"))
        || (["pass", "ok"].contains(&first) && rest.split_whitespace().count() <= 2)
}

/// Whether `output` quotes something beyond test names: an assertion, or a line the command printed.
fn quotes(output: &str) -> bool {
    // « PASS a; PASS b » is two test names
    output.split(['\n', ';']).any(|l| !names_only(l))
}

/// Whether `text` argues something out of scope.
fn out_of_scope(text: &str) -> bool {
    let l = text.to_lowercase();
    let na = l
        .split(|c: char| c.is_whitespace() || "`'\"«»(),;:".contains(c))
        .any(|w| w.trim_end_matches('.') == "n/a");
    na || [
        "out of scope",
        "out-of-scope",
        "outside the scope",
        "not in scope",
        "not applicable",
    ]
    .iter()
    .any(|w| l.contains(w))
}

/// Whether an outcome clause lets a part go unproven: waived, moved, not met or checked after the merge, as words (« removed »
/// is not « moved »). « met: parts 1-5 » holds none.
fn excused(clause: &str) -> bool {
    let l = clause.to_lowercase();
    let words: Vec<&str> = l.split(|c: char| !c.is_alphanumeric()).collect();
    let said = |w: &[&str]| words.windows(w.len()).any(|at| at == w);
    [
        &["waived"][..],
        &["moved"],
        &["not", "met"],
        &["not", "yet", "met"],
        &["after", "merge"],
        &["after", "the", "merge"],
    ]
    .iter()
    .any(|w| said(w))
}

/// The clauses of `text`: its lines, cut at « ; », « . » and at a comma not followed by a number, so « parts 1-3 hold, part
/// 3 after merge » excuses part 3 alone while « parts 1, 2 waived » keeps its list.
fn clauses(text: &str) -> Vec<&str> {
    let mut out = vec![];
    let mut from = 0;
    for (at, c) in text.char_indices() {
        let next_is_number = || {
            text[at + 1..]
                .trim_start()
                .starts_with(|c: char| c.is_ascii_digit())
        };
        if matches!(c, '\n' | ';' | '.') || (c == ',' && !next_is_number()) {
            out.push(&text[from..at]);
            from = at + 1;
        }
    }
    out.push(&text[from..]);
    out
}

/// Whether `text` names code: a file (`crates/x/src/y.rs`, `reports.ex`), a path (`tests::x`) or a test's snake_case name.
fn names_code(text: &str) -> bool {
    const KINDS: [&str; 16] = [
        ".rs", ".ex", ".exs", ".md", ".toml", ".yml", ".yaml", ".json", ".edn", ".sh", ".py",
        ".html", ".js", ".ts", ".css", ".sql",
    ];
    text.split(|c: char| c.is_whitespace() || "`'\"«»(),;".contains(c))
        .any(|w| {
            let w = w.trim_end_matches(['.', ':']);
            let snake = w.matches('_').count() >= 2
                && w.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
            !w.contains("://")
                && (w.contains("::") || KINDS.iter().any(|k| w.ends_with(k)) || snake)
        })
}

/// A text cut to `n` characters on one line, for a refusal that names it.
fn cut(text: &str, n: usize) -> String {
    let one = text.split_whitespace().collect::<Vec<_>>().join(" ");
    match one.chars().count() > n {
        true => one.chars().take(n - 1).collect::<String>() + "…",
        false => one,
    }
}

/// The line of `text` that names part `n`, else its first line.
fn line_naming(text: &str, n: usize, of: usize) -> String {
    let lines = text.lines().filter(|l| !l.trim().is_empty());
    let named = lines.clone().find(|l| named_in(l, of).contains(&n));
    cut(named.or(lines.clone().next()).unwrap_or(text), 160)
}

/// Why a done report `r` for a task with done-when `done_when` is bounced, each problem naming its part, else none. A part
/// holds by a check claiming it whose output quotes its own proof, or by an after_merge, not_checked or outcome line naming it
/// (waived, moved, not met, checked after the merge); one argued out of scope names the code that shows it; one claimed by
/// a check while not_checked names it is bounced, both lines quoted.
pub fn problems(done_when: &str, r: &Value) -> Vec<String> {
    let parts = parts(done_when);
    let of = parts.len();
    if r["kind"].as_str() != Some("done") || of == 0 {
        return vec![];
    }
    let field = |f: &str| text_of(&r[f]);
    let not_checked = field("not_checked");
    let after_merge = field("after_merge");
    let outcome = field("outcome");
    let checks: Vec<&Value> = r["checks"]
        .as_array()
        .map(|c| c.iter().collect())
        .unwrap_or_default();
    let mut out = vec![];
    for (i, text) in parts.iter().enumerate() {
        let n = i + 1;
        let name = format!("part {n} (« {} »)", cut(text, 60));
        let own: Vec<&Value> = checks
            .iter()
            .copied()
            .filter(|c| claimed(&c["part"], of).contains(&n))
            .collect();
        let said = |f: &str| text_of(&own.first().map_or(Value::Null, |c| c[f].clone()));
        // a part checked after the merge, as its text or after_merge says: a pending check of it is not a claim it holds
        let after = (text.to_lowercase().starts_with("after merge")
            && !matches!(after_merge.trim().to_lowercase().as_str(), "" | "none"))
            || named_in(&after_merge, of).contains(&n);
        let in_not_checked = named_in(&not_checked, of).contains(&n);
        if !own.is_empty() && in_not_checked && !after {
            out.push(format!(
                "{name} is called met by the check « {} » while not_checked names it: « {} »",
                cut(&said("command"), 80),
                line_naming(&not_checked, n, of)
            ));
            continue;
        }
        // every statement on the part: its checks, and the lines of the report's text fields naming it
        let mut statements: Vec<String> = own
            .iter()
            .map(|c| format!("{} {}", text_of(&c["command"]), text_of(&c["output"])))
            .collect();
        for f in [&not_checked, &after_merge, &outcome] {
            let lines = f.lines().filter(|l| named_in(l, of).contains(&n));
            statements.extend(lines.map(str::to_string));
        }
        if let Some(argued) = statements
            .iter()
            .find(|s| out_of_scope(s) && !names_code(s))
        {
            out.push(format!(
                "{name} is argued out of scope with no code that shows it: name the file or test (« {} »)",
                cut(argued, 100)
            ));
            continue;
        }
        if own.is_empty() {
            let excuses = clauses(&outcome).into_iter().filter(|c| excused(c));
            let named = [not_checked.as_str()]
                .into_iter()
                .chain(excuses)
                .any(|f| named_in(f, of).contains(&n));
            if !after && !named {
                out.push(format!(
                    "{name} has no check whose part names it, and no after_merge, not_checked or outcome line names it"
                ));
            }
            continue;
        }
        let argued = statements.iter().any(|s| out_of_scope(s));
        if !argued && !after && !own.iter().any(|c| quotes(&text_of(&c["output"]))) {
            out.push(format!(
                "{name} quotes no assertion or command output of its own, only test names (« {} »): quote the assertion \
                 or the lines the command printed that show it",
                cut(&said("output"), 80)
            ));
        }
    }
    out
}

/// The report of a task whose PR `pr` merged after its run ended with none (task rust-rejudge-merged-closes), for the judge to
/// weigh: its parts claimed by that merge, `body` (the PR's own words on what changed and was checked) as the proof each quotes,
/// and a part whose text opens « After merge » left to `after_merge`, for the live check after the merge. `problems` and
/// `shape` pass it, so the older app files it.
pub fn from_merged(done_when: &str, pr: &str, body: &str) -> Value {
    let each = parts(done_when);
    let after = |t: &String| t.to_lowercase().starts_with("after merge");
    let numbered = |keep: bool| -> Vec<String> {
        let all = each.iter().enumerate().filter(|(_, t)| after(t) == keep);
        all.map(|(i, _)| (i + 1).to_string()).collect()
    };
    let (claimed, later) = match each.len() {
        0 | 1 => (vec!["1".to_string()], vec![]),
        _ => (numbered(false), numbered(true)),
    };
    let after_merge = match later.as_slice() {
        [] => "none".to_string(),
        [one] => format!("part {one}: checked on the live app after the merge"),
        many => format!(
            "parts {}: checked on the live app after the merge",
            many.join(", ")
        ),
    };
    let note =
        "its run ended before its report: Rust's orchestrator wrote this one from its merged PR";
    let body: String = body.trim().chars().take(4000).collect();
    let mut checks = vec![];
    if !claimed.is_empty() {
        checks.push(serde_json::json!({
            "command": format!("gh pr view {pr} --json state,body"),
            "output": format!("MERGED: {pr}. Its body:\n{body}"),
            "part": claimed.join(", "),
        }));
    }
    serde_json::json!({
        "kind": "done",
        "outcome": format!("met: {pr} merged; {note}"),
        "checks": checks,
        "prs": [format!("{pr} merged")],
        "after_merge": after_merge,
        "not_checked": format!("each part's own run of its check: {note}, from the PR's body alone"),
        "next": "none",
        "feedback": "none",
    })
}

/// The older app's shape problems of `r` (`Reports.problems/1`, task report-errors-at-once: every problem at once), so a
/// proof refusal never hides them for a second round: its missing fields, a check lacking its command or output, prs not
/// a list of links, a text field that is not text, usage not numbers with their unit.
fn shape(r: &Value) -> Vec<String> {
    let blank = |v: &Value| match v {
        Value::Null => true,
        Value::String(s) => s.is_empty(),
        Value::Array(a) => a.is_empty(),
        _ => false,
    };
    let need: &[&str] = match r["kind"].as_str() {
        Some("done") => &["outcome", "checks", "not_checked", "next"],
        Some("blocked") => &["needs", "link"],
        _ => {
            return vec![format!(
                "the report lacks kind (done or blocked, not {})",
                r["kind"]
            )];
        }
    };
    let missing: Vec<&str> = need.iter().copied().filter(|f| blank(&r[*f])).collect();
    let mut out = vec![];
    if !missing.is_empty() {
        out.push(format!("the report lacks {}", missing.join(", ")));
    }
    let prs_ok = match &r["prs"] {
        Value::Null => true,
        Value::Array(a) => a.iter().all(Value::is_string),
        _ => false,
    };
    if !prs_ok {
        out.push("prs is a list of strings, each a link with merged or ready for review".into());
    }
    let usage = [&r["usage_before"], &r["usage_after"]];
    let unit_ok = ["usd", "weekly_pct"].contains(&r["usage_unit"].as_str().unwrap_or_default());
    let usage_ok = usage.iter().all(|v| v.is_null() || v.is_number())
        && (usage.iter().all(|v| v.is_null()) || unit_ok);
    if !usage_ok {
        out.push(
            "usage_before and usage_after are numbers from dash task usage, with usage_unit weekly_pct (or usd for a key reading)"
                .into(),
        );
    }
    if !missing.contains(&"checks") {
        let checks = r["checks"]
            .as_array()
            .cloned()
            .unwrap_or_else(|| vec![r["checks"].clone()]);
        let held = |c: &Value| {
            c["command"].as_str().is_some_and(|s| !s.is_empty()) && c["output"].is_string()
        };
        let bad: Vec<&Value> = checks.iter().filter(|c| !held(c)).collect();
        for c in bad.iter().take(3) {
            out.push(format!(
                "each check needs its command and its output: {}",
                cut(&c.to_string(), 200)
            ));
        }
        if bad.len() > 3 {
            out.push(format!(
                "{} more checks lack their command or output",
                bad.len() - 3
            ));
        }
    }
    let text = [
        "outcome",
        "after_merge",
        "not_checked",
        "next",
        "needs",
        "link",
        "feedback",
        "model",
        "good_at",
        "bad_at",
    ];
    for f in text.iter().filter(|f| !missing.contains(f)) {
        if !(r[*f].is_null() || r[*f].is_string()) {
            out.push(format!("{f} is text, not {}", cut(&r[*f].to_string(), 100)));
        }
    }
    out
}

/// The refusal for `problems`, on one line, numbered past one, as the older app's `Reports.problem/1` writes it.
pub fn refusal(problems: &[String]) -> Option<String> {
    match problems {
        [] => None,
        [one] => Some(one.clone()),
        many => {
            let each = many
                .iter()
                .enumerate()
                .map(|(i, p)| format!("{}. {p}", i + 1));
            Some(format!(
                "{} problems: {}",
                many.len(),
                each.collect::<Vec<_>>().join(" ")
            ))
        }
    }
}

/// `dash task report <id> <src> [--read|--unread]` from bin/dash, read as the older app reads it (`Orch.CLI`): `src` is the
/// record itself when it opens with « { », else a file from `cwd`, « ~ » his home. The refusal when the report's proofs fall
/// short, its shape problems first, else none and the older app files it. A source it cannot read or parse, or a task with
/// no row, goes on to the older app, which names that problem itself.
pub fn bounced(args: &[String], cwd: &str, db: &std::path::Path) -> Option<String> {
    let (id, record) = record(args, cwd)?;
    let filed = dash_store::older::brief(db, &id).ok()??;
    let proofs = problems(&filed.done_when, &record);
    if proofs.is_empty() {
        return None;
    }
    refusal(&[shape(&record), proofs].concat()).map(|why| format!("{id}: report refused, {why}"))
}

/// `dash task report`'s task id and record, or none for any other command or a source that does not read.
fn record(args: &[String], cwd: &str) -> Option<(String, Value)> {
    let (id, src) = match args {
        [_, _, id, src] => (id, src),
        [_, _, id, src, read] if ["--read", "--unread"].contains(&read.as_str()) => (id, src),
        _ => return None,
    };
    if (args[0].as_str(), args[1].as_str()) != ("task", "report") {
        return None;
    }
    let text = match src.trim_start().starts_with('{') {
        true => src.clone(),
        false => std::fs::read_to_string(expanded(src, cwd)?).ok()?,
    };
    Some((id.clone(), serde_json::from_str(&text).ok()?))
}

/// The lesson a report's feedback line carries, as a row of the watcher's wake (the older app's `Attention` lesson, task
/// rust-watch-port): none when it says « none » or nothing.
pub fn lesson(args: &[String], cwd: &str, now_ms: i64) -> Option<dash_comments::watch::Item> {
    use dash_comments::watch::{Item, Kind};
    let (id, record) = record(args, cwd)?;
    let said = record["feedback"]
        .as_str()?
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let bare = said.trim_end_matches('.').to_lowercase();
    if bare.is_empty() || bare == "none" {
        return None;
    }
    let key = format!("lesson {id} {said}");
    let title = format!("task {id}'s feedback, a lesson to fold");
    Some(Item::new(
        &key,
        Kind::Lesson,
        Some(&id),
        &title,
        &said,
        now_ms,
    ))
}

/// `src` as `Path.expand(src, cwd)` reads it: « ~ » his home, an absolute path as it is, else under `cwd`; none for a relative
/// path with no folder to read it from.
fn expanded(src: &str, cwd: &str) -> Option<std::path::PathBuf> {
    let home = || std::env::var("HOME").ok().map(std::path::PathBuf::from);
    match src.strip_prefix('~') {
        Some("") => home(),
        Some(rest) if rest.starts_with('/') => Some(home()?.join(&rest[1..])),
        _ if std::path::Path::new(src).is_absolute() => Some(src.into()),
        _ if cwd.is_empty() => None,
        _ => Some(std::path::Path::new(cwd).join(src)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Task rust-rejudge-merged-closes, done-when 1: the report written from a merged PR passes both checks a filing runs, so
    /// the older app files it; 1511's own done-when included, its third part left to after the merge, never claimed.
    #[dash_test]
    fn a_report_written_from_a_merged_pr_passes_the_proof_and_shape_checks() {
        let linear = "1. dash linear sync writes tasks: a test 2. Rust's 30-minute sync is written, not armed 3. After merge: \
                      Linear's projects match /plan after a sync";
        let body = "## What changed\n1. dash linear sync is Rust's";
        for done_when in [linear, "one part, no number", ""] {
            let r = from_merged(done_when, "https://github.com/o/r/pull/1511", body);
            assert_eq!(problems(done_when, &r), Vec::<String>::new(), "{done_when}");
            assert_eq!(shape(&r), Vec::<String>::new(), "{done_when}");
        }
        let r = from_merged(linear, "https://github.com/o/r/pull/1511", body);
        assert_eq!(r["checks"][0]["part"], "1, 2");
        assert!(
            text_of(&r["checks"][0]["output"]).ends_with(body),
            "the PR's body is its proof"
        );
        assert_eq!(
            r["after_merge"],
            "part 3: checked on the live app after the merge"
        );
        assert_eq!(r["prs"][0], "https://github.com/o/r/pull/1511 merged");
    }

    /// Done-when 2 of rust-watch-port: a report's feedback is a lesson row of the watcher, « none » none.
    #[dash_test]
    fn a_report_s_feedback_is_a_lesson_for_the_watcher() {
        let args = |record: &str| {
            ["task", "report", "rust-x", record]
                .map(String::from)
                .to_vec()
        };
        let said = r#"{"kind": "done", "feedback": "herdr's  read source cost two turns."}"#;
        let row = lesson(&args(said), "", 5).expect("a lesson");
        assert_eq!(row.kind, dash_comments::watch::Kind::Lesson);
        assert_eq!(row.task.as_deref(), Some("rust-x"));
        assert_eq!(row.body, "herdr's read source cost two turns.");
        assert_eq!(row.title, "task rust-x's feedback, a lesson to fold");
        for none in [
            r#"{"feedback": "none"}"#,
            r#"{"feedback": "None."}"#,
            r#"{"feedback": " "}"#,
            r#"{"kind": "done"}"#,
        ] {
            assert_eq!(lesson(&args(none), "", 5), None, "{none}");
        }
        let other = ["task", "show", "rust-x", said].map(String::from).to_vec();
        assert_eq!(lesson(&other, "", 5), None);
    }
    use dash_meter::dash_test;
    use serde_json::json;

    const DONE_WHEN: &str = "1. dash task report bounces an unquoted part: a test 2. A quoted report passes: a test \
                             3. After merge: the next 5 reports lose no round";

    fn report(checks: Value, not_checked: &str, after_merge: &str) -> Value {
        json!({"kind": "done", "outcome": "met", "checks": checks, "not_checked": not_checked,
               "after_merge": after_merge, "next": "none"})
    }

    /// Done-when 1 of report-proof-quotes: a part whose checks only name tests is bounced, the part named.
    #[dash_test]
    fn a_part_quoting_no_assertion_of_its_own_is_bounced_by_name() {
        let r = report(
            json!([
                {"command": "cargo nextest run -p dash reports", "part": "1",
                 "output": "PASS [   0.011s] dash reports::tests::a_part_is_bounced\n     Summary [0.2s] 1 test run: 1 passed"},
                {"command": "cargo nextest run -p dash quoted", "part": "2",
                 "output": "assert_eq!(problems(..), vec![]) holds: PASS"},
                {"command": "cargo nextest run", "part": "all parts", "output": "the whole suite printed this line"},
            ]),
            "none",
            "part 3: dash task run --feedback after five reports",
        );
        let why = problems(DONE_WHEN, &r);
        assert_eq!(why.len(), 1, "{why:?}");
        assert!(
            why[0].starts_with("part 1 (« dash task report bounces an unquoted part: a test ») quotes no assertion or command output of its own"),
            "{why:?}"
        );
        // cargo's own shape, and a check claiming « all parts », hold no part either
        let cargo = report(
            json!([{"command": "cargo test", "part": "1, 2", "output": "test tests::x ... ok\ntest result: ok. 1 passed"}]),
            "none",
            "part 3",
        );
        let why = problems(DONE_WHEN, &cargo);
        assert!(
            why[0].starts_with("part 1 ") && why[1].starts_with("part 2 "),
            "{why:?}"
        );
        let all = report(
            json!([{"command": "c", "part": "all parts", "output": "assert!(x)"}]),
            "none",
            "part 3",
        );
        let why = refusal(&problems(DONE_WHEN, &all)).expect("refused");
        assert!(
            why.starts_with("2 problems: 1. part 1 (« dash task report bounces an unquoted part: a test ») has no check whose part names it"),
            "{why}"
        );
    }

    /// Done-when 2: a report quoting each part's own assertion or output passes, a part after the merge held by after_merge.
    #[dash_test]
    fn a_report_quoting_each_part_s_own_assertion_passes() {
        let r = report(
            json!([
                {"command": "cargo nextest run -p dash a_part_quoting_no_assertion", "part": "1",
                 "output": "PASS [0.01s] dash reports::tests::a_part\nassert!(why[0].starts_with(\"part 1 (« dash task report"},
                {"command": "dash task report x r.json", "part": 2, "output": "x: report filed"},
            ]),
            "the live app's front door, untested here",
            "part 3: dash task run --feedback after five Rust reports",
        );
        assert_eq!(problems(DONE_WHEN, &r), Vec::<String>::new());
        // a part waived in the outcome holds too, and a blocked report is not this check's
        let waived = json!({"kind": "done", "outcome": "met; part 2 waived by his answer « fine »", "after_merge": "3 later",
                            "not_checked": "none", "checks": [{"command": "c", "part": "1", "output": "assert!(held)"}]});
        assert_eq!(problems(DONE_WHEN, &waived), Vec::<String>::new());
        assert_eq!(
            problems(DONE_WHEN, &json!({"kind": "blocked"})),
            Vec::<String>::new()
        );
        // an outcome calling every part met holds none of them
        let claimed_only = json!({"kind": "done", "outcome": "met: parts 1-3 hold", "after_merge": "none",
                                  "not_checked": "none", "checks": []});
        assert_eq!(problems(DONE_WHEN, &claimed_only).len(), 3);
        // an excuse is a word in the part's own clause (/code-review of ccdf1033e): « removed » is no « moved », and a
        // clause calling parts met is not excused by its neighbour's « after merge »
        let removed = json!({"kind": "done", "outcome": "met: part 2 removed the old hook", "after_merge": "part 3",
                             "not_checked": "none", "checks": [{"command": "c", "part": "1", "output": "assert!(a)"}]});
        assert!(problems(DONE_WHEN, &removed)[0].starts_with("part 2 "));
        let neighbour = json!({"kind": "done", "outcome": "met: parts 1-2 hold, part 3 checked after merge",
                               "after_merge": "none", "not_checked": "none", "checks": []});
        let why = problems(DONE_WHEN, &neighbour);
        assert_eq!(why.len(), 2, "{why:?}");
        let listed = json!({"kind": "done", "outcome": "not met: parts 1, 2 and 3 moved to x", "after_merge": "none",
                            "not_checked": "none", "checks": []});
        assert_eq!(problems(DONE_WHEN, &listed), Vec::<String>::new());
        // one part with no numbers: any check is its own
        let one = report(
            json!([{"command": "c", "part": "done-when", "output": "it printed 3"}]),
            "none",
            "none",
        );
        assert_eq!(problems("It holds", &one), Vec::<String>::new());
    }

    /// Done-when 3: a part argued out of scope names the code (a file or a test) that shows it.
    #[dash_test]
    fn a_part_argued_out_of_scope_names_the_code_that_shows_it() {
        let bare = report(
            json!([
                {"command": "read the diff", "part": "1", "output": "out of scope: nothing draws a page"},
                {"command": "c", "part": "2", "output": "assert!(held)"},
            ]),
            "none",
            "part 3",
        );
        let why = problems(DONE_WHEN, &bare);
        assert_eq!(
            why,
            vec![
                "part 1 (« dash task report bounces an unquoted part: a test ») is argued out of scope with no code that \
                  shows it: name the file or test (« read the diff out of scope: nothing draws a page »)"
            ]
        );
        // in not_checked too
        let line = report(
            json!([{"command": "c", "part": "1", "output": "assert!(a)"}]),
            "part 2 is n/a here",
            "part 3",
        );
        assert!(
            problems(DONE_WHEN, &line)[0].starts_with("part 2 "),
            "{:?}",
            problems(DONE_WHEN, &line)
        );
        for code in [
            "out of scope: rust/crates/surface/src/lib.rs draws nothing here",
            "out of scope, as layers::tests::a_rust_task_with_no_layer shows",
            "not applicable: the_brief_names_the_layer covers it",
        ] {
            let named = report(
                json!([{"command": "read", "part": "1", "output": code}, {"command": "c", "part": "2", "output": "assert!(x)"}]),
                "none",
                "part 3",
            );
            assert_eq!(problems(DONE_WHEN, &named), Vec::<String>::new(), "{code}");
        }
        // a link is no code
        assert!(!names_code("out of scope, see https://example.org/x.html"));
        assert!(!out_of_scope("the plan/approve page") && out_of_scope("part 2: n/a."));
    }

    /// Done-when 5: a part a check calls met while not_checked names it is bounced, both lines quoted.
    #[dash_test]
    fn a_part_called_met_while_not_checked_names_it_is_bounced_naming_both_lines() {
        let r = report(
            json!([
                {"command": "cargo nextest run -p dash reports", "part": "1", "output": "assert!(why.contains(\"part 1\"))"},
                {"command": "cargo nextest run -p dash quoted", "part": "2", "output": "assert_eq!(problems(..), vec![])"},
            ]),
            "the live app\nparts 2 and 3 on a real report, only after the merge",
            "part 3",
        );
        // an after-merge part listed as a pending check, not_checked saying so, is no claim it holds
        let pending = report(
            json!([
                {"command": "c", "part": "1", "output": "assert!(a)"},
                {"command": "c", "part": "2", "output": "assert!(b)"},
                {"command": "after merge, main checkout: dash task run --feedback", "part": "3", "output": "pending"},
            ]),
            "part 3 until after the merge",
            "part 3",
        );
        assert_eq!(problems(DONE_WHEN, &pending), Vec::<String>::new());
        assert_eq!(
            problems(DONE_WHEN, &r),
            vec![
                "part 2 (« A quoted report passes: a test ») is called met by the check « cargo nextest run -p dash \
                  quoted » while not_checked names it: « parts 2 and 3 on a real report, only after the merge »"
            ]
        );
    }

    /// Done-when 1 through the front door: `dash task report <id> <file>`, its file read from the caller's folder and its
    /// done-when from the older app's row, is refused naming the part; a quoting one, a review's or a garbled file goes on.
    #[dash_test]
    fn dash_task_report_bounces_from_the_file_and_the_task_s_row() {
        let dir = dash_meter::TestDir::new("report-bounce");
        let db = dir.0.join("older.db");
        let tables = "CREATE TABLE tasks (collection TEXT, pos INTEGER, id TEXT, stage TEXT, data TEXT NOT NULL, title TEXT,
              why TEXT, done_when TEXT, epic TEXT, plan_first INTEGER);
            CREATE TABLE records (collection TEXT, pos INTEGER, id TEXT, title TEXT, data TEXT NOT NULL);
            CREATE TABLE comments (pos INTEGER, id TEXT, stage TEXT, task TEXT, file TEXT, quote TEXT, text TEXT,
              data TEXT NOT NULL);
            INSERT INTO tasks VALUES ('tasks.edn', 0, 't', 'open', '{}', 'T', 'w', '1. It bounces 2. It passes', NULL, NULL);";
        let store = dash_store::Connection::open(&db).expect("db");
        store.execute_batch(tables).expect("tables");
        let write = |name: &str, r: Value| {
            std::fs::write(dir.0.join(name), r.to_string()).expect("a report")
        };
        write(
            "bare.json",
            report(
                json!([{"command": "c", "part": "1", "output": "PASS t"}, {"command": "c", "part": "2", "output": "assert!(b)"}]),
                "none",
                "none",
            ),
        );
        write(
            "quoted.json",
            report(
                json!([{"command": "c", "part": "1", "output": "assert!(a)"}, {"command": "c", "part": "2", "output": "assert!(b)"}]),
                "none",
                "none",
            ),
        );
        std::fs::write(dir.0.join("garbled.json"), "{").expect("garbled");
        let cwd = dir.0.display().to_string();
        let asked = |id: &str, file: &str| {
            bounced(&["task", "report", id, file].map(String::from), &cwd, &db)
        };
        let asked_in = |id: &str, file: &str, cwd: &str| {
            bounced(&["task", "report", id, file].map(String::from), cwd, &db)
        };
        let why = asked("t", "bare.json").expect("bounced");
        assert!(
            why.starts_with("t: report refused, part 1 (« It bounces ») quotes no assertion or command output of its own"),
            "{why}"
        );
        assert_eq!(asked("t", "quoted.json"), None);
        assert_eq!(asked("review-t", "bare.json"), None);
        assert_eq!(asked("t", "garbled.json"), None);
        // the older app's other forms (/code-review of ccdf1033e): « --read », the record inline, an absolute path
        let read = ["task", "report", "t", "bare.json", "--read"].map(String::from);
        assert!(bounced(&read, &cwd, &db).is_some());
        let bare = std::fs::read_to_string(dir.0.join("bare.json")).expect("bare");
        assert!(
            bounced(
                &["task", "report", "t", &format!("  {bare}")].map(String::from),
                "",
                &db
            )
            .is_some()
        );
        let absolute = dir.0.join("bare.json").display().to_string();
        assert!(
            bounced(
                &["task", "report", "t", &absolute].map(String::from),
                "",
                &db
            )
            .is_some()
        );
        assert_eq!(asked_in("t", "bare.json", ""), None);
        let home = std::env::var("HOME").expect("HOME");
        assert_eq!(
            expanded("~/r.json", "/w"),
            Some(std::path::Path::new(&home).join("r.json"))
        );
        // its shape problems come in the same refusal, never a second round
        let shapeless = json!({"kind": "done", "outcome": "met", "prs": "1414", "not_checked": "none", "next": "none",
                               "checks": [{"command": "c", "part": "1", "output": "PASS t"}, {"part": "2"}]});
        std::fs::write(dir.0.join("shapeless.json"), shapeless.to_string()).expect("shapeless");
        let why = asked("t", "shapeless.json").expect("bounced");
        assert!(
            why.starts_with("t: report refused, 4 problems: 1. prs is a list of strings"),
            "{why}"
        );
        assert!(
            why.contains("2. each check needs its command and its output"),
            "{why}"
        );
        assert_eq!(
            bounced(&["task", "show", "t"].map(String::from), &cwd, &db),
            None
        );
    }

    #[dash_test]
    fn parts_and_their_names_read_as_written() {
        assert_eq!(parts("1. a 2. b 1.5 x 3. c"), vec!["a", "b 1.5 x", "c"]);
        assert_eq!(parts("1. it is 2.5 big"), vec!["it is 2.5 big"]);
        assert_eq!(
            named_in("parts 1, 3 and 5; done-when 2", 5),
            vec![1, 2, 3, 5]
        );
        assert_eq!(named_in("parts 2-4, part 9", 5), vec![2, 3, 4]);
        assert_eq!(named_in("none, 4 tests", 5), Vec::<usize>::new());
        assert_eq!(claimed(&json!("1, 2 on /design"), 3), vec![1, 2]);
        assert_eq!(claimed(&json!([1, "3"]), 3), vec![1, 3]);
        // a range claims every part in it, and a huge one builds nothing (/code-review of ccdf1033e)
        assert_eq!(claimed(&json!("1-3"), 3), vec![1, 2, 3]);
        assert_eq!(named_in("parts 1-4000000000", 5), vec![1]);
        // a heading before « 1. » is no part
        assert_eq!(parts("Done when: 1. a 2. b"), vec!["a", "b"]);
        for counted in [
            "5 tests, 0 failures",
            "Randomized with seed 123",
            "....",
            "   Doc-tests dash",
            "warning: unused x",
        ] {
            assert!(names_only(counted), "{counted}");
        }
        assert!(names_only(
            "        PASS [   0.011s] dash reports::tests::x"
        ));
        assert!(names_only("PASS (3/7) dash mail::tests::the_thread_page"));
        assert!(!names_only(
            "PASS: for / (Ask): assert !form.contains(\"notion\")"
        ));
    }
}
