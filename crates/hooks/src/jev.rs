//! Jev's readiness verdict on every page of private/docs without a click: the port of the older app's Dashboard.DocJev poll
//! (task doc-jev-first-verdict; @nha, 2026-11-02: « I had to ask Jev myself, I asked before to rate all docs after writing
//! automatically - this was marked done and evidently it is not »). `dash orch` runs a pass each minute (dash's
//! `watchers::doc_jev`): this plugin holds the pass, the binary only the wiring (task rust-binary-ratchet).
//! A page Jev never judged is asked at its first pass: no verdict is the stalest one, where the older poll left it alone. A
//! judged page is asked once its text changed since it was last asked and has been still for a pass, so a run of saves is
//! judged once. What each page was asked for is kept in the data folder's `doc-jev.json`, so a restart asks nothing again.
//! A judged page this file never saw is taken as asked for its text, not asked: the older poll still runs and judges its
//! edits until no-elixir lets application.ex drop it. A failed ask is tried again next pass, at most `TRIES` times for one
//! text; at most `PER_PASS` are asked a pass, the least tried first, so a failing page never holds the others back.
//! The ask is `dash doc ready`'s, doc_ready.py, as the older app's `Readiness.judge` ran it: Jev's answer lands in its log.
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::Duration;

const FOLDER: &str = "private/docs/";
const STATE: &str = "doc-jev.json";
const PER_PASS: usize = 5;
const TRIES: u64 = 3;
/// One page's ask at most: Jev answers each section in about 2 s, and the longest RFC has some 40.
const ASK: Duration = Duration::from_secs(600);

/// Why no paid call runs here, when CI or nextest runs it.
pub fn paid_refused() -> Option<String> {
    let under = ["CI", "NEXTEST"]
        .into_iter()
        .find(|v| std::env::var_os(v).is_some())?;
    Some(format!(
        "{under} is set: a check never pays Jev or an agent there"
    ))
}

/// Jev's « noul » for `question` in its answer.
pub fn noul(answer: &serde_json::Value, question: &str) -> Result<f64, String> {
    let n = answer["answers"][question]["noul"].as_f64();
    n.ok_or_else(|| format!("Jev gave no score for {question}"))
}

/// The input tokens an answer was billed for: 0 when it does not say.
pub fn input_tokens(answer: &serde_json::Value) -> u64 {
    answer["usage"]["input_tokens"].as_u64().unwrap_or(0)
}

/// The FNV-1a hash of `s`: short, and the same in every build.
pub fn fnv(s: &str) -> u64 {
    (s.bytes()).fold(0xcbf29ce484222325u64, |h, b| {
        (h ^ b as u64).wrapping_mul(0x100000001b3)
    })
}

/// What the passes keep: the text each page was last asked for, its text at the last pass, and the failed asks of a text.
#[derive(Default, Debug, PartialEq)]
pub struct Seen {
    asked: BTreeMap<String, u64>,
    last: BTreeMap<String, u64>,
    tries: BTreeMap<String, (u64, u64)>,
}

/// One pass over `docs` (each page's path and text hash). `judged` reads which pages Jev ever answered, only when a page
/// changed; `ask` asks Jev. The state to keep, and what it said.
pub fn pass(
    seen: &Seen,
    docs: &BTreeMap<String, u64>,
    judged: impl FnOnce() -> Result<BTreeSet<String>, String>,
    mut ask: impl FnMut(&str) -> Result<(), String>,
) -> Result<(Seen, String), String> {
    let mut next = Seen {
        asked: seen.asked.clone(),
        last: docs.clone(),
        tries: seen.tries.clone(),
    };
    next.asked.retain(|rel, _| docs.contains_key(rel));
    next.tries.retain(|rel, (h, _)| docs.get(rel) == Some(h));
    let changed: Vec<&String> = docs
        .iter()
        .filter(|(rel, h)| seen.asked.get(*rel) != Some(*h))
        .map(|(rel, _)| rel)
        .collect();
    if changed.is_empty() {
        return Ok((next, format!("{} pages, none to ask", docs.len())));
    }
    let judged = judged()?;
    let mut due = vec![];
    for rel in changed {
        let h = docs[rel];
        match (judged.contains(rel), seen.asked.contains_key(rel)) {
            // no verdict at all: asked now, a new page judged within a pass
            (false, _) => due.push(rel),
            // judged before this file kept it: taken as asked for its text (the module's note)
            (true, false) => {
                next.asked.insert(rel.clone(), h);
            }
            // an edit, still since the last pass
            (true, true) if seen.last.get(rel) == Some(&h) => due.push(rel),
            (true, true) => {}
        }
    }
    let count =
        |tries: &BTreeMap<String, (u64, u64)>, rel: &str| tries.get(rel).map_or(0, |(_, n)| *n);
    due.sort_by_key(|rel| count(&next.tries, rel));
    let (mut asked, mut failed) = (vec![], vec![]);
    for rel in due.into_iter().take(PER_PASS) {
        let h = docs[rel];
        match ask(rel) {
            Ok(()) => {
                next.asked.insert(rel.clone(), h);
                next.tries.remove(rel);
                asked.push(rel.as_str());
            }
            Err(why) => {
                let n = count(&next.tries, rel) + 1;
                failed.push(format!("{rel}: {why}"));
                if n >= TRIES {
                    // left until its next edit
                    next.asked.insert(rel.clone(), h);
                    next.tries.remove(rel);
                } else {
                    next.tries.insert(rel.clone(), (h, n));
                }
            }
        }
    }
    let mut said = format!("{} pages, Jev asked for {}", docs.len(), asked.len());
    if !asked.is_empty() {
        said += &format!(": {}", asked.join(", "));
    }
    if !failed.is_empty() {
        said += &format!("; not asked: {}", failed.join("; "));
    }
    Ok((next, said))
}

fn docs(repo: &Path) -> Result<BTreeMap<String, u64>, String> {
    let folder = repo.join(FOLDER);
    let names = std::fs::read_dir(&folder).map_err(|e| format!("{FOLDER}: {e}"))?;
    Ok(names
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|n| n.ends_with(".html") && !n.starts_with('.'))
        .filter_map(|n| {
            Some((
                format!("{FOLDER}{n}"),
                // FNV-1a: stable across builds, so a new binary reads every page as the one it asked for
                fnv(&String::from_utf8_lossy(
                    &std::fs::read(folder.join(&n)).ok()?,
                )),
            ))
        })
        .collect())
}

/// The pages Jev answered, a section's answer included: a line of its log with this `doc`, status 200 and answers. A line
/// cut mid-character (an append read while it lands) is read lossily and skipped as no JSON, never the pass's end.
/// ponytail: the whole log read on a pass with a page to judge (2.8 MB, 2026-11-13); its offset kept if it grows past that.
pub fn judged(repo: &Path) -> Result<BTreeSet<String>, String> {
    let mut all = BTreeSet::new();
    for log in crate::outbound::jev_logs(repo) {
        let Ok(bytes) = std::fs::read(&log) else {
            continue;
        };
        for line in String::from_utf8_lossy(&bytes).lines() {
            // most lines are about drafts and CVs: parsed only when they name a doc
            if !line.contains("\"doc\"") {
                continue;
            }
            let Ok(v) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            let answered =
                v["status"] == 200 && v.pointer("/response/answers").is_some_and(Value::is_object);
            if let (true, Some(doc)) = (answered, v["doc"].as_str()) {
                all.insert(doc.to_string());
            }
        }
    }
    Ok(all)
}

/// doc_ready.py's ask, `Readiness.judge`'s: Ok when Jev answered, ready or not, else its last line. CI and a test never pay.
fn ask(repo: &Path, rel: &str) -> Result<(), String> {
    if let Some(why) = paid_refused() {
        return Err(why);
    }
    static VERDICT: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r" \(reader: .*\): (ready|NOT READY)").expect("a pattern")
    });
    let script = ".claude/skills/nicolas-daily/doc_ready.py";
    let (_, out, err) = dash_process::answer("python3", &[script, rel], Some(repo), &[], ASK)
        .map_err(String::from)?;
    let said = format!("{out}\n{err}");
    if VERDICT.is_match(&said) {
        return Ok(());
    }
    let last = said
        .lines()
        .rfind(|l| !l.trim().is_empty())
        .unwrap_or("no answer");
    Err(last
        .trim()
        .trim_start_matches("Jev not asked: ")
        .to_string())
}

fn map(m: &BTreeMap<String, u64>) -> Value {
    m.iter()
        .map(|(k, v)| (k.clone(), json!(v)))
        .collect::<Map<_, _>>()
        .into()
}

fn read(dir: &Path) -> Seen {
    let Ok(v) = std::fs::read(dir.join(STATE))
        .map(|b| serde_json::from_slice::<Value>(&b).unwrap_or_default())
    else {
        return Seen::default();
    };
    let hashes = |k: &str| -> BTreeMap<String, u64> {
        let m = v[k].as_object().into_iter().flatten();
        m.filter_map(|(rel, h)| Some((rel.clone(), h.as_u64()?)))
            .collect()
    };
    let tries = v["tries"].as_object().into_iter().flatten();
    Seen {
        asked: hashes("asked"),
        last: hashes("last"),
        tries: tries
            .filter_map(|(rel, t)| Some((rel.clone(), (t[0].as_u64()?, t[1].as_u64()?))))
            .collect(),
    }
}

fn write(dir: &Path, seen: &Seen) -> Result<(), String> {
    let tries: Map<_, _> = seen
        .tries
        .iter()
        .map(|(k, (h, n))| (k.clone(), json!([h, n])))
        .collect();
    let v = json!({"asked": map(&seen.asked), "last": map(&seen.last), "tries": tries});
    let (path, tmp) = (dir.join(STATE), dir.join(format!("{STATE}.tmp")));
    std::fs::write(&tmp, v.to_string()).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("{}: {e}", path.display()))
}

/// A pass over `repo`'s pages, its state in `dir`; each ask calls `beat` first with its line, so a pass of long asks (5
/// pages, a minute or more each) never reads LATE on /watchers. Its line, and whether every page due was asked.
pub fn once(
    dir: &Path,
    repo: &Path,
    beat: impl Fn(&str) -> Result<(), String>,
) -> (Result<String, String>, bool) {
    let seen = read(dir);
    let ask = |rel: &str| {
        beat(&format!("asking Jev for {rel}"))?;
        ask(repo, rel)
    };
    let said = docs(repo).and_then(|docs| pass(&seen, &docs, || judged(repo), ask));
    let said = said.and_then(|(next, said)| {
        (next != seen).then(|| write(dir, &next)).transpose()?;
        Ok(said)
    });
    let ok = said.as_ref().is_ok_and(|l| !l.contains("; not asked: "));
    (said, ok)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;
    use std::cell::RefCell;

    const REL: &str = "private/docs/bilan.html";

    fn docs(h: u64) -> BTreeMap<String, u64> {
        BTreeMap::from([(REL.to_string(), h)])
    }

    /// One pass, the pages Jev answered `judged`: the state after it and the pages it asked.
    fn run(seen: &Seen, h: u64, judged: &[&str]) -> (Seen, Vec<String>) {
        let asked = RefCell::new(vec![]);
        let judged: BTreeSet<String> = judged.iter().map(|s| s.to_string()).collect();
        let (next, _) = pass(
            seen,
            &docs(h),
            || Ok(judged),
            |rel| {
                asked.borrow_mut().push(rel.to_string());
                Ok(())
            },
        )
        .expect("a pass");
        (next, asked.into_inner())
    }

    #[dash_test]
    fn a_page_never_judged_is_asked_on_the_first_pass() {
        let (seen, asked) = run(&Seen::default(), 1, &[]);
        assert_eq!(asked, [REL]);
        // Jev's answer logged: nothing left to ask
        assert!(run(&seen, 1, &[REL]).1.is_empty());
    }

    #[dash_test]
    fn a_judged_unchanged_page_is_not_asked_an_edit_once_still() {
        let (seen, asked) = run(&Seen::default(), 1, &[REL]);
        assert!(
            asked.is_empty(),
            "judged before: taken as asked for its text"
        );
        assert!(run(&seen, 1, &[REL]).1.is_empty());
        // an edit: left a pass to settle, then asked once
        let (seen, asked) = run(&seen, 2, &[REL]);
        assert!(asked.is_empty());
        let (seen, asked) = run(&seen, 2, &[REL]);
        assert_eq!(asked, [REL]);
        assert!(run(&seen, 2, &[REL]).1.is_empty());
    }

    #[dash_test]
    fn a_failing_ask_is_tried_three_times_then_left_until_an_edit() {
        let mut seen = Seen::default();
        let mut calls = 0;
        for _ in 0..5 {
            let (next, _) = pass(
                &seen,
                &docs(1),
                || Ok(BTreeSet::new()),
                |_| {
                    calls += 1;
                    Err("Jev down".into())
                },
            )
            .expect("a pass");
            seen = next;
        }
        assert_eq!(calls, 3);
        assert_eq!(run(&seen, 2, &[]).1, [REL]);
    }

    #[dash_test]
    fn the_state_survives_a_restart() {
        let dir = dash_meter::TestDir::new("doc-jev-state");
        let (seen, _) = run(&Seen::default(), 7, &[]);
        write(&dir.0, &seen).expect("written");
        assert_eq!(read(&dir.0), seen);
    }
}
