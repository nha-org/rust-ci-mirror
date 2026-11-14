//! Jev judges a Rust diff against its RFC section (task rust-task-rfc-judge; @nha, 2026-11-11: « I suggested using JEV for
//! task review against the RFC automatically in the past. … Let's use it. »; rfc-rust-unified#conformance's back-test): the
//! diff's added and removed lines under rust/, never its context, since a kept line read as new was the back-test's one false
//! alarm. A change is judged in chunks of whole files, `CHUNK` bytes each, its score the lowest chunk's: one 100 KB call
//! drowned a breach in one file (3 of 10 breaches scored over the bar) or was refused for its size. `dash task rfc-check
//! <pr|sha>` prints the score and the rule; the orchestrator's PM check hands back a met run scored under `RFC_BAR`
//! (dash_orch's `held`). Each answer is kept in the data folder's `rfc-judge.jsonl`, keyed by its chunk, the section's words,
//! the questions and Jev's model: the same chunk is never paid twice, and the file counts each call's cost.
use crate::jev;
use dash_core::tasks::{RFC_BAR, RfcScore};
use dash_process::now_ms;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

const QUESTIONS: &str = r#"{
  "keeps": {"type": "noul",
    "instructions": "`section` is what a design document (an RFC) decides for the part of the code `diff` changes. `diff` lists, file by file, the lines the change removes (`-`) and adds (`+`), with no unchanged lines. Does the change keep to what `section` decides? Removing code that went against the section keeps it.",
    "criteria": {"true": "The change follows what the section decides, or touches nothing it decides.", "false": "The change adds what the section rules out, or goes against its default."}}
}"#;

/// The bytes of changed lines one call judges, at most: a file past it is split. Jev refused a 103 KB diff beside the section
/// (« max_tokens_exceeded », 2026-11-11).
const CHUNK: usize = 10_000;
/// The chunks of one change judged, at most: past them the rest is left unjudged, and the answer says so.
const CHUNKS: usize = 24;
/// Paid calls in a day, at most, counted from the record: a loop that judged every stop could not run up a bill. 300 held
/// three tasks' merges on 2026-11-12 for about $0.06 of calls (248 calls cost $0.049); 3000 still bounds a loop, near $0.60 a day.
const CALLS_A_DAY: usize = 3000;
const DAY_MS: i64 = 24 * 3600 * 1000;
/// The calls of one check, at most this long: the orchestrator's run waits on it. Jev answered each part in about 2 s,
/// 2026-11-11; a check that runs past it fails open, flagged, and its parts answered so far are kept for the next.
const CHECK_TIME: std::time::Duration = std::time::Duration::from_secs(120);
const RECORD: &str = "rfc-judge.jsonl";

/// The section a layer is judged against: its own for `rfc:<id>`, section 10 (the UI lib, the design system's) for
/// `design:<name>`; none for a `model:` layer, whose shape no section decides alone.
pub(crate) fn section_of(layer: &str) -> Option<&str> {
    match layer.split_once(':')? {
        ("rfc", id) if !id.is_empty() => Some(id),
        ("design", _) => Some("ui-lib"),
        _ => None,
    }
}

/// A file the judge reads: Rust's own, never a vendored crate or the lock file.
fn ours(file: &str) -> bool {
    file.starts_with("rust/") && !file.starts_with("rust/vendor/") && !file.ends_with("Cargo.lock")
}

/// A unified diff's added and removed lines under rust/, by file; context, hunk heads and other files dropped. A file's name
/// is read from its header only, so an added line « ++ x » is never taken for one.
fn changed_files(diff: &str) -> Vec<(String, Vec<&str>)> {
    let (mut files, mut file, mut header) = (vec![], None::<&str>, false);
    for line in diff.lines() {
        if line.starts_with("diff --git ") {
            (file, header) = (None, true);
            continue;
        }
        if header {
            let named = (line
                .strip_prefix("--- ")
                .map(|p| p.strip_prefix("a/").unwrap_or(p)))
            .or_else(|| {
                line.strip_prefix("+++ ")
                    .map(|p| p.strip_prefix("b/").unwrap_or(p))
            });
            if let Some(path) = named.filter(|p| *p != "/dev/null") {
                file = Some(path);
            }
            if line.starts_with("@@") {
                header = false;
                if let Some(f) = file.filter(|f| ours(f)) {
                    files.push((f.to_string(), vec![]));
                }
            }
            continue;
        }
        if file.is_some_and(ours) && line.starts_with(['+', '-']) {
            files
                .last_mut()
                .expect("its file, pushed at its first hunk")
                .1
                .push(line);
        }
    }
    files.retain(|(_, lines)| !lines.is_empty());
    files
}

/// The change as the chunks Jev reads: whole files after a `--- <file>` line, up to `CHUNK` bytes a chunk, a longer file split
/// with its name on each part, so no chunk ends on a name with nothing under it; a line past `CHUNK / 2` bytes is cut. At most
/// `CHUNKS`, and the bytes left past them.
pub(crate) fn chunks(diff: &str) -> (Vec<String>, usize) {
    let mut out: Vec<String> = vec![];
    for (file, lines) in changed_files(diff) {
        let header = format!("--- {file}\n");
        let mut opened = false;
        for line in lines {
            let mut cut = line.len().min(CHUNK / 2);
            while !line.is_char_boundary(cut) {
                cut -= 1;
            }
            let line = format!("{}\n", &line[..cut]);
            let head = if opened { "" } else { header.as_str() };
            let fits = out
                .last()
                .is_some_and(|c| c.len() + head.len() + line.len() <= CHUNK);
            match fits {
                true => out
                    .last_mut()
                    .expect("there")
                    .push_str(&format!("{head}{line}")),
                false => out.push(format!("{header}{line}")),
            }
            opened = true;
        }
    }
    let left = out.iter().skip(CHUNKS).map(String::len).sum();
    out.truncate(CHUNKS);
    (out, left)
}

/// The file `path` as a diff leaves it, when the diff holds it whole: one hunk from its first line, as the PM check's diff
/// (`--unified=1000000`) gives each changed file. None for a file the diff leaves alone or shows only in part.
fn after(diff: &str, path: &str) -> Option<String> {
    let header = format!("+++ b/{path}");
    let at = diff.find(&format!("\n{header}\n"))? + header.len() + 2;
    let mut lines = diff[at..].lines();
    let hunk = lines.next()?;
    // one hunk from line 1 holds the file whole: « @@ -a[,b] +c[,d] @@ », a at 0 (a new file) or 1, c at 1; git drops « ,b »
    // for one line (« @@ -1 +1,40 @@ »), so the starts are parsed, never matched as text (/code-review: « +10 » read as « +1 »)
    let (old, new) = hunk.strip_prefix("@@ -")?.split_once(" +")?;
    let start = |r: &str| r.split([',', ' ']).next()?.parse::<u32>().ok();
    if !matches!(start(old), Some(0 | 1)) || start(new) != Some(1) {
        return None;
    }
    let mut out = String::new();
    for line in lines {
        match line.as_bytes().first() {
            Some(b' ' | b'+') => {
                out.push_str(&line[1..]);
                out.push('\n');
            }
            Some(b'-' | b'\\') => {}
            // a blank context line whose leading space a pipe trimmed stays a line, never the end (/code-review)
            None => out.push('\n'),
            // the next file's header, or a second hunk: not whole after all
            _ if line.starts_with("@@") => return None,
            _ => break,
        }
    }
    Some(out)
}

/// The RFC holding section `id`, by name, and its words: the first of `layers::RFCS` that has it, each read as the change
/// leaves it (`changed`, none for one it leaves alone), else from `repo`. A task changes its section in the same PR, as its
/// brief asks: judged against main's words, a port that moves a decision read as breaking the one it replaces (task
/// rust-linear-sync-port: 0.46 against main's 20g, 0.50 against its own).
fn section(
    repo: &Path,
    id: &str,
    changed: &dyn Fn(&str) -> Option<String>,
) -> Result<(String, String), String> {
    for rfc in crate::layers::RFCS {
        let html = (changed(rfc))
            .unwrap_or_else(|| std::fs::read_to_string(repo.join(rfc)).unwrap_or_default());
        if let Some(text) = dash_launcher::brief::section_text(&html, id) {
            let name = rfc
                .rsplit('/')
                .next()
                .unwrap_or(rfc)
                .trim_end_matches(".html");
            return Ok((name.into(), text));
        }
    }
    Err(format!(
        "no section {id} in {}",
        crate::layers::RFCS.join(" or ")
    ))
}

/// The rule a score is held to: the section by its link, and its decided default when it states one, else its heading.
pub(crate) fn rule(rfc: &str, id: &str, text: &str) -> String {
    let said = match text.find("Default: ") {
        Some(at) => {
            let rest = &text[at..];
            let end = rest.find(". ").map_or(rest.len(), |e| e + 1);
            rest[..end].to_string()
        }
        None => text.chars().take(120).collect(),
    };
    format!("{rfc}#{id} ({})", said.trim())
}

/// One change judged: its lowest chunk's score and the first file of that chunk, the rule, and what it cost: `paid` calls, their input tokens.
#[derive(Debug, PartialEq)]
pub(crate) struct Judged {
    pub score: f64,
    pub lowest: String,
    pub rule: String,
    pub paid: usize,
    pub tokens: u64,
}

/// What the record says of the last day: its paid calls and their cost in US dollars.
fn today(lines: &[serde_json::Value], now: i64) -> (usize, f64) {
    let paid = (lines.iter())
        .filter(|l| l["paid"] == true && l["at_ms"].as_i64().is_some_and(|at| now - at < DAY_MS));
    paid.fold((0, 0.0), |(n, usd), l| {
        (n + 1, usd + l["usd"].as_f64().unwrap_or(0.0))
    })
}

fn read_record(data: &Path) -> Vec<serde_json::Value> {
    let text = std::fs::read_to_string(data.join(RECORD)).unwrap_or_default();
    text.lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

/// A part's key in the record: Jev's model, the question, the section's words and the part, so a new model, question or
/// section asks again.
fn part_key(model: &str, text: &str, chunk: &str) -> String {
    format!(
        "{:016x}",
        jev::fnv(&format!("{model}\n{QUESTIONS}\n{text}\n{chunk}"))
    )
}

/// `chunks` (`chunks`) of `of` (a task id, a PR or a commit) judged against section `id` of `rfc`, whose words are `text`:
/// each from the record when the same chunk, words, questions and model were judged before, else one paid call, refused past
/// `CALLS_A_DAY`, past `budget` of calls on this change, and under CI or a test (`jev::ask`). Each paid answer is added to the
/// record with its cost.
pub(crate) fn judge(
    data: &Path,
    of: &str,
    (rfc, id, text): (&str, &str, &str),
    chunks: &[String],
    (now, budget): (i64, std::time::Duration),
) -> Result<Judged, String> {
    let mut got = Judged {
        score: 1.0,
        lowest: String::new(),
        rule: rule(rfc, id, text),
        paid: 0,
        tokens: 0,
    };
    let mut lines = read_record(data);
    let began = std::time::Instant::now();
    for chunk in chunks {
        let key = part_key(jev::MODEL, text, chunk);
        let score = match lines.iter().rev().find(|l| l["key"] == key.as_str()) {
            Some(l) => l["score"].as_f64().ok_or("the record's score")?,
            None => {
                let (calls, _) = today(&lines, now);
                if calls >= CALLS_A_DAY {
                    return Err(format!(
                        "{calls} paid RFC checks in the last day, the most: it waits for tomorrow"
                    ));
                }
                if began.elapsed() >= budget {
                    return Err(format!(
                        "past {} s of calls on one change: the parts left are judged at its next check",
                        budget.as_secs()
                    ));
                }
                let line = paid(data, of, &key, (rfc, id, text), chunk, now)?;
                let (score, tokens) = (
                    line["score"].as_f64().unwrap_or(0.0),
                    line["tokens"].as_u64().unwrap_or(0),
                );
                (got.paid, got.tokens) = (got.paid + 1, got.tokens + tokens);
                lines.push(line);
                score
            }
        };
        if got.lowest.is_empty() || score < got.score {
            let file = chunk
                .lines()
                .next()
                .unwrap_or_default()
                .trim_start_matches("--- ");
            (got.score, got.lowest) = (score, file.to_string());
        }
    }
    Ok(got)
}

/// One paid call on `chunk`, its answer added to the record: the record's new line, its score and input tokens in it.
fn paid(
    data: &Path,
    of: &str,
    key: &str,
    (rfc, id, text): (&str, &str, &str),
    chunk: &str,
    now: i64,
) -> Result<serde_json::Value, String> {
    let questions: serde_json::Value =
        serde_json::from_str(QUESTIONS).expect("the questions are JSON");
    let answer = jev::ask(
        &questions,
        serde_json::json!({"section": text, "diff": chunk}),
        "site_perso/rfc-check",
    )?;
    let score = jev::noul(&answer, "keeps")?;
    let tokens = jev::input_tokens(&answer);
    let line = serde_json::json!({"key": key, "of": of, "section": format!("{rfc}#{id}"), "model": jev::MODEL,
        "returned": answer["model"], "score": score, "paid": true, "tokens": tokens,
        "usd": tokens as f64 * jev::USD_PER_INPUT_TOKEN, "at_ms": now});
    std::fs::create_dir_all(data).map_err(|e| format!("{}: {e}", data.display()))?;
    let path = data.join(RECORD);
    let mut file = File::options()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    writeln!(file, "{line}").map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(line)
}

/// The PM check's judge for a met run of task `id` naming `layer`: `None` for a layer no section judges, or a diff that
/// changes nothing under rust/.
pub(crate) fn for_run(
    data: &Path,
    id: &str,
    layer: &str,
    diff: &str,
) -> Result<Option<RfcScore>, String> {
    let Some(section_id) = section_of(layer) else {
        return Ok(None);
    };
    let (chunks, left) = chunks(diff);
    if chunks.is_empty() {
        return Ok(None);
    }
    let (rfc, text) = section(&crate::pages::repo(), section_id, &|rfc| after(diff, rfc))?;
    let got = judge(
        data,
        id,
        (&rfc, section_id, &text),
        &chunks,
        (now_ms(), CHECK_TIME),
    )?;
    let left = match left {
        0 => String::new(),
        n => format!("; {n} bytes past its {CHUNKS} parts unjudged"),
    };
    let rule = format!("{}, lowest in the part from {}{left}", got.rule, got.lowest);
    Ok(Some(RfcScore {
        score: got.score,
        rule,
    }))
}

/// `dash task rfc-check <pr|sha> [--section <id>] [--data <dir>]`: a PR (its number or link) read with gh, its section the
/// layer its task names in the store unless `--section` says; a commit read with git, `--section` required. Exit 0 when it
/// keeps the section, 1 when it scores under the bar, 2 when it cannot be judged.
/// `layer_of`: the layer a task names in the store at a data folder, listed or not.
pub(crate) fn command(args: &[&str], layer_of: &LayerOf, mut out: File) -> i32 {
    match checked(args, layer_of, &mut out) {
        Ok(keeps) => i32::from(!keeps),
        Err(e) => {
            eprintln!("rfc-check: {e}");
            2
        }
    }
}

fn flag<'a>(args: &[&'a str], name: &str) -> Option<&'a str> {
    let at = args.iter().position(|a| *a == name)?;
    args.get(at + 1).copied()
}

type LayerOf = dyn Fn(&Path, &str) -> Result<Option<String>, String>;

const USAGE: &str = "dash task rfc-check <pr|sha> [--section <id>] [--data <dir>]";
inventory::submit! { crate::usage::Command::usage(USAGE) }

fn checked(args: &[&str], layer_of: &LayerOf, out: &mut File) -> Result<bool, String> {
    let target = *(args.first().filter(|a| !a.starts_with("--")))
        .ok_or_else(|| format!("usage: {USAGE}"))?;
    let data = flag(args, "--data").map_or_else(crate::pages::live_data, PathBuf::from);
    let repo = crate::pages::repo();
    let run = |bin: &str, words: &[&str]| {
        dash_process::run(bin, words, Some(&repo), std::time::Duration::from_secs(60))
    };
    let pr = target.trim_start_matches('#');
    // a PR's number, never a sha of digits alone: the shortest sha git prints is 7
    let is_pr = (pr.len() < 7 && pr.chars().all(|c| c.is_ascii_digit())) || pr.contains("/pull/");
    // the commit the change leaves: its RFC is read there, never main's (`section`)
    let (diff, task, head) = match is_pr {
        true => {
            // one call for both (gh-meter): the head's sha, then the title
            let q = r#".headRefOid + " " + .title"#;
            let view = run(
                "gh",
                &["pr", "view", pr, "--json", "headRefOid,title", "-q", q],
            )?;
            let (head, title) = view.trim().split_once(' ').unwrap_or((view.trim(), ""));
            let task = title.split('[').nth(2).and_then(|t| t.split(']').next());
            (
                run("gh", &["pr", "diff", pr])?,
                task.map(String::from),
                head.to_string(),
            )
        }
        false => (
            run(
                "git",
                &["show", "--format=", "--no-color", target, "--", "rust/"],
            )?,
            None,
            target.to_string(),
        ),
    };
    let layer = match (flag(args, "--section"), &task) {
        (Some(s), _) => format!("rfc:{s}"),
        (None, Some(t)) => layer_of(&data, t)?.ok_or(format!(
            "{t} names no layer in {}: give --section <id>",
            data.display()
        ))?,
        (None, None) => {
            return Err(format!(
                "a commit names no task: give --section <id>\n{USAGE}"
            ));
        }
    };
    let id = section_of(&layer).ok_or(format!("{layer}: no RFC section judges it"))?;
    let (chunks, left) = chunks(&diff);
    if chunks.is_empty() {
        return Err(format!("{target} changes nothing under rust/"));
    }
    // a PR's head this clone has not fetched is fetched once; one still missing (a fork's PR) falls back to the checkout's RFC,
    // and says so: a silent fallback judged a port against main's words again (/code-review)
    if is_pr && run("git", &["cat-file", "-e", &format!("{head}^{{commit}}")]).is_err() {
        // the number after « pull/ », a URL's trailing slash or « /files » left out (/code-review)
        let num = pr.split_once("/pull/").map_or(pr, |(_, n)| n);
        let num = num.split('/').next().unwrap_or(num);
        let _ = run("git", &["fetch", "origin", &format!("pull/{num}/head")]);
    }
    let at_head = |rfc: &str| {
        let got = run("git", &["show", &format!("{head}:{rfc}")]);
        if let Err(e) = &got {
            eprintln!("{rfc} at {head}: {e}; the checkout's words are judged instead");
        }
        got.ok()
    };
    let (rfc, text) = section(&repo, id, &at_head)?;
    let of = task.as_deref().unwrap_or(target);
    let got = judge(
        &data,
        of,
        (&rfc, id, &text),
        &chunks,
        (now_ms(), CHECK_TIME),
    )?;
    let keeps = got.score >= RFC_BAR;
    let (calls, usd) = today(&read_record(&data), now_ms());
    let left = match left {
        0 => String::new(),
        n => format!(", {n} bytes past them unjudged"),
    };
    let task = task.map_or(String::new(), |t| format!(" (task {t})"));
    let word = if keeps { "keeps it" } else { "breaks it" };
    writeln!(
        out,
        "rfc-check {target}{task} against {rfc}#{id}, in {} chunks{left}\nscore: {:.2}, {word} (the bar: {RFC_BAR}), lowest in \
         the part from {}\nrule: {}\ncost: {} paid calls, {} input tokens, ${:.6}; the last day: {calls} paid calls, ${usd:.6}",
        chunks.len(),
        got.score,
        got.lowest,
        got.rule,
        got.paid,
        got.tokens,
        got.tokens as f64 * jev::USD_PER_INPUT_TOKEN
    )
    .map_err(|e| e.to_string())?;
    Ok(keeps)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;

    /// A change judged against its RFC section as the change leaves it (task rust-linear-sync-port: its port scored 0.46
    /// against main's 20g, which still gave the projects to the older app's sync, and 0.50 against the 20g it shipped): the PM
    /// check's whole-file diff gives the section's new words; a diff showing the RFC in part, or not at all, gives none.
    #[dash_test]
    fn the_section_is_read_as_the_change_leaves_it() {
        const MAIN: &str = include_str!("../tests/fixtures/rfc-t.html");
        let rfc = crate::layers::RFCS[0];
        let (old, new) = (
            "The check pass drops what is settled.",
            "The check pass keeps what is settled.",
        );
        let mut body = String::new();
        for line in MAIN.lines() {
            match line.contains(old) {
                true => body.push_str(&format!("-{line}\n+{}\n", line.replace(old, new))),
                false => body.push_str(&format!(" {line}\n")),
            }
        }
        let n = MAIN.lines().count();
        let head = format!("diff --git a/{rfc} b/{rfc}\n--- a/{rfc}\n+++ b/{rfc}\n");
        let whole =
            format!("{head}@@ -1,{n} +1,{n} @@\n{body}diff --git a/rust/x.rs b/rust/x.rs\n");
        let got = after(&whole, rfc).expect("the RFC whole");
        assert_eq!(got, MAIN.replace(old, new));
        let dir = dash_meter::TestDir::new("rfc-judge-after");
        let held = dir.0.join(rfc);
        std::fs::create_dir_all(held.parent().expect("its folder")).expect("folder");
        std::fs::write(&held, MAIN).expect("main's RFC");
        let (_, words) = section(&dir.0, "intent", &|r| after(&whole, r)).expect("the section");
        assert!(words.contains(new) && !words.contains(old), "{words}");
        let part = format!("{head}@@ -40,{n} +40,{n} @@\n{body}");
        assert_eq!(
            after(&part, rfc),
            None,
            "a part of the file is not the file"
        );
        let one = format!("{head}@@ -1 +1,2 @@\n-was\n+is\n+more\n");
        assert_eq!(
            after(&one, rfc).as_deref(),
            Some("is\nmore\n"),
            "a one-line file's header has no « ,b »"
        );
        let later = format!("{head}@@ -10,2 +10,2 @@\n-was\n+is\n");
        assert_eq!(after(&later, rfc), None, "« +10 » is not « +1 »");
        let (_, words) = section(&dir.0, "intent", &|r| after(&part, r)).expect("the section");
        assert!(
            words.contains(old),
            "left alone, the checkout's words: {words}"
        );
    }

    #[dash_test]
    fn the_diff_sent_is_its_added_and_removed_lines_under_rust() {
        let diff = "diff --git a/rust/a.rs b/rust/a.rs\n--- a/rust/a.rs\n+++ b/rust/a.rs\n@@ -1,3 +1,3 @@\n kept\n-old\n+new\n\
                    diff --git a/private/x.md b/private/x.md\n--- a/private/x.md\n+++ b/private/x.md\n@@ -1 +1 @@\n+not rust\n\
                    diff --git a/rust/gone.rs b/rust/gone.rs\n--- a/rust/gone.rs\n+++ /dev/null\n@@ -1 +0,0 @@\n--- removed\n";
        assert_eq!(
            chunks(diff),
            (
                vec!["--- rust/a.rs\n-old\n+new\n--- rust/gone.rs\n--- removed\n".to_string()],
                0
            )
        );
        let line = format!("+{}\n", "x".repeat(CHUNK / 4));
        let long = format!(
            "diff --git a b\n+++ b/rust/b.rs\n@@ -0,0 +1 @@\n{}",
            line.repeat(4 * CHUNKS)
        );
        let (parts, left) = chunks(&long);
        assert_eq!(parts.len(), CHUNKS);
        assert!(
            parts
                .iter()
                .all(|c| c.starts_with("--- rust/b.rs\n+x") && c.len() <= CHUNK),
            "{:?}",
            parts[0].len()
        );
        assert!(left > 0);
        // a file's name opens a chunk only with a line under it; a line of wide characters is cut by bytes
        let near = format!("+{}", "a".repeat(CHUNK - 40));
        let wide = format!("+{}", "é".repeat(CHUNK));
        let two = format!(
            "diff --git a b\n+++ b/rust/a.rs\n@@ -0,0 +1 @@\n{near}\ndiff --git c d\n+++ b/rust/c.rs\n@@ -0,0 +1 @@\n{wide}\n"
        );
        let (parts, _) = chunks(&two);
        assert_eq!(parts.len(), 2);
        assert!(!parts[0].contains("rust/c.rs") && parts[1].starts_with("--- rust/c.rs\n+é"));
        assert!(
            parts.iter().all(|c| c.len() <= CHUNK),
            "{:?}",
            parts.iter().map(String::len).collect::<Vec<_>>()
        );
        assert_eq!(
            (
                section_of("rfc:ui-lib"),
                section_of("design:board"),
                section_of("model:task")
            ),
            (Some("ui-lib"), Some("ui-lib"), None)
        );
        let text = "10. A UI lib Default: every screen is a UI tree. default More.";
        assert_eq!(
            rule("rfc-rust-unified", "ui-lib", text),
            "rfc-rust-unified#ui-lib (Default: every screen is a UI tree.)"
        );
    }

    /// Done-when 4: a chunk judged before is read from the record, at no cost, the lowest chunk's score kept; another model,
    /// section or chunk is a new call, refused under a test (nextest sets NEXTEST) before any key is read; past `CALLS_A_DAY`
    /// paid in the last day, refused; the record counts the day's calls and their cost.
    #[dash_test]
    fn calls_are_cached_bounded_counted_and_never_paid_in_tests() {
        let data = std::env::temp_dir().join(format!("rfc-judge-{}", std::process::id()));
        std::fs::create_dir_all(&data).expect("dir");
        let section = (
            "rfc-rust-unified",
            "ui-lib",
            "Default: behaviour is Rust. More.",
        );
        let parts = [
            "--- rust/a.rs\n+fine\n".to_string(),
            "--- rust/b.rs\n+const MENU_JS: &str = \"x\";\n".to_string(),
        ];
        let key = |chunk: &str| part_key(jev::MODEL, section.2, chunk);
        let now = 10 * DAY_MS;
        let record: String = [(0, 0.9), (1, 0.08)]
            .iter()
            .map(|(i, s)| {
                let line = serde_json::json!({"key": key(&parts[*i]), "score": s, "paid": true, "usd": 0.0001, "at_ms": now - 1});
                format!("{line}\n")
            })
            .collect();
        std::fs::write(data.join(RECORD), record).expect("record");
        let got = judge(&data, "t", section, &parts, (now, CHECK_TIME)).expect("cached");
        assert_eq!(
            got,
            Judged {
                score: 0.08,
                lowest: "rust/b.rs".into(),
                rule: "rfc-rust-unified#ui-lib (Default: behaviour is Rust.)".into(),
                paid: 0,
                tokens: 0
            }
        );
        assert_eq!(today(&read_record(&data), now), (2, 0.0002));
        let other = judge(
            &data,
            "t",
            section,
            &["--- rust/a.rs\n+other\n".into()],
            (now, CHECK_TIME),
        )
        .expect_err("a new call");
        assert!(other.contains("is set: a check never pays Jev"), "{other}");
        // cached by model version: the same part under another model's key is not read, it is a new call
        let old_model = serde_json::json!({"key": part_key("jev-1.12.0", section.2, &parts[0]), "score": 0.9, "paid": true, "at_ms": now - 1});
        std::fs::write(data.join(RECORD), format!("{old_model}\n")).expect("record");
        assert_ne!(part_key("jev-1.12.0", section.2, &parts[0]), key(&parts[0]));
        let new_model =
            judge(&data, "t", section, &parts[..1], (now, CHECK_TIME)).expect_err("asked again");
        assert!(
            new_model.contains("is set: a check never pays Jev"),
            "{new_model}"
        );
        // bounded in time: a check past its budget pays nothing more
        let late = judge(
            &data,
            "t",
            section,
            &parts[..1],
            (now, std::time::Duration::ZERO),
        )
        .expect_err("late");
        assert!(
            late.starts_with("past 0 s of calls on one change"),
            "{late}"
        );
        let full: String = (0..CALLS_A_DAY)
            .map(|i| {
                format!(
                    "{}\n",
                    serde_json::json!({"key": i, "paid": true, "usd": 0.0, "at_ms": now - 1})
                )
            })
            .collect();
        std::fs::write(data.join(RECORD), full).expect("record");
        let capped = judge(&data, "t", section, &parts, (now, CHECK_TIME)).expect_err("capped");
        assert!(
            capped.contains("paid RFC checks in the last day"),
            "{capped}"
        );
        let _ = std::fs::remove_file(data.join(RECORD));
        let _ = std::fs::remove_dir(&data);
    }

    /// Done-when 2: the 21 back-test items of rfc-rust-unified#conformance, as `dash task rfc-check <sha> --section ui-lib`
    /// scored them on 2026-11-11 (rfc-backtest.tsv, the run's own lines): every breach under the bar, at most one keeper.
    /// Moving `RFC_BAR` or the chunking past what was measured fails here first; the next 20 PRs measure it again.
    #[dash_test]
    fn the_back_test_holds_ten_of_ten_breaches_under_the_bar_and_at_most_one_keeper() {
        let rows: Vec<(&str, f64)> = (include_str!("rfc-backtest.tsv").lines())
            .filter(|l| !l.starts_with('#'))
            .map(|l| {
                let w: Vec<&str> = l.split('\t').collect();
                (w[0], w[2].parse().expect("a score"))
            })
            .collect();
        let under = |kind: &str| {
            rows.iter()
                .filter(|(k, s)| *k == kind && *s < RFC_BAR)
                .count()
        };
        let all = |kind: &str| rows.iter().filter(|(k, _)| *k == kind).count();
        assert_eq!((all("breach"), all("keeper")), (10, 11));
        assert_eq!(under("breach"), 10, "{rows:?}");
        assert!(under("keeper") <= 1, "{rows:?}");
    }
}
