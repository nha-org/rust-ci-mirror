//! The pre-commit's cv-locks (once `europass.py --check-locks --staged`, which the custom-cv skill keeps for `dash cv
//! check`): a sent CV is immutable (private/cv/sent.lock, @nha, 2026-09-26). A locked file changed or gone in the index
//! refuses the commit, and so does a CV the CRM says went out that is not locked, when this commit stages it: another
//! session's unlocked one is named, never refused (tether-qvac-inference, 2026-10-30). The CRM is the app's model; an app that
//! does not answer skips that half with a note, never a refused commit.
use super::Ctx;
use regex::Regex;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

const LOCK: &str = "private/cv/sent.lock";

/// A value of the model's tagged JSON as its bare name: "~:done" and "~$x" lose the tag, "~~x" its escaping ~, a tagged set
/// or list ({"~#set": [...]}) is its list.
pub fn untag(v: &Value) -> Value {
    if let Some(o) = v.as_object()
        && o.len() == 1
        && let Some(list) = o.get("~#set").or_else(|| o.get("~#list"))
    {
        return list.clone();
    }
    match v.as_str() {
        Some(s) if s.starts_with("~~") => Value::from(&s[1..]),
        Some(s) if s.starts_with("~:") || s.starts_with("~$") => Value::from(&s[2..]),
        _ => v.clone(),
    }
}

/// sent.lock's rows by path: `sha256<TAB>path<TAB>opp<TAB>date`, as staged
pub fn locks(lock: &str) -> BTreeMap<String, Vec<String>> {
    lock.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.split('\t').map(String::from).collect::<Vec<_>>())
        .filter(|r| r.len() >= 2)
        .map(|r| (r[1].clone(), r))
        .collect()
}

/// {opportunity: the files that went to it}: an applied opportunity's :cv and :cv-en, and the :attach of each sent draft
/// that carried a file (an application intro, a « -cv » message, or one whose header names the attachment).
pub fn sent(
    opportunities: &BTreeMap<String, String>,
    drafts: &BTreeMap<String, String>,
) -> BTreeMap<String, Vec<String>> {
    let carried_name = Regex::new(r"-(intro|cv)(-\w+)?\.md$").expect("pattern");
    let carried_text = Regex::new(r"pi\x{e8}ce jointe|\.pdf").expect("pattern");
    let mut by_draft: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (name, text) in drafts {
        let head = text.split("\n---\n").next().unwrap_or_default();
        let Ok(h) = serde_json::from_str::<Value>(head) else {
            continue;
        };
        let Some(o) = untag(&h["opportunity"]).as_str().map(String::from) else {
            continue;
        };
        let carried = carried_name.is_match(name) || carried_text.is_match(head);
        if carried && untag(&h["status"]).as_str() == Some("sent") {
            let files = by_draft.entry(o).or_default();
            match untag(&h["attach"]) {
                Value::String(s) => files.push(s),
                Value::Array(a) => {
                    files.extend(a.iter().filter_map(|v| v.as_str().map(String::from)))
                }
                _ => {}
            }
        }
    }
    let mut out = BTreeMap::new();
    for text in opportunities.values() {
        let Ok(e) = serde_json::from_str::<Value>(text) else {
            continue;
        };
        let cvs: Vec<String> = ["cv", "cv-en"]
            .iter()
            .filter_map(|k| e[*k].as_str().map(String::from))
            .collect();
        let Some(id) = untag(&e["id"]).as_str().map(String::from) else {
            continue;
        };
        if cvs.is_empty() {
            continue;
        }
        let applied = e["applied"].is_string();
        if applied || by_draft.contains_key(&id) {
            let mut all = cvs;
            all.extend(by_draft.get(&id).cloned().unwrap_or_default());
            out.insert(id, all);
        }
    }
    out
}

/// A folder of the CRM as {file: text}, from the app (`dash store files <folder>`); why it did not answer
fn store_files(ctx: &Ctx, folder: &str) -> Result<BTreeMap<String, String>, String> {
    let dash = std::env::var_os("DASH_BIN")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| ctx.top.join("private/dashboard/bin/dash"));
    let mut c = ctx.cmd(&ctx.top, &dash);
    c.args(["store", "files", folder])
        .env("DASH_TIMEOUT", "5")
        .env("DASH_RESTART_TRIES", "0");
    let out = c
        .output()
        .map_err(|e| format!("dash store files {folder}: {e}"))?;
    let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
    if out.status.code() == Some(1) && (err.contains("holds no") || err.contains("is no folder")) {
        return Ok(BTreeMap::new());
    }
    if !out.status.success() {
        return Err(format!(
            "dash store files {folder}: {}",
            if err.is_empty() {
                format!("exit {:?}", out.status.code())
            } else {
                err
            }
        ));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    serde_json::from_str(if text.trim().is_empty() { "{}" } else { &text })
        .map_err(|e| format!("dash store files {folder}: not JSON ({e})"))
}

/// The step on the index: its refusals, each « FAIL <why> »
pub fn check(ctx: &Ctx) -> Result<(), String> {
    let have = locks(&ctx.git(&["show", &format!(":{LOCK}")]).unwrap_or_default());
    let mut bad = vec![];
    // a lock only grows: a row HEAD holds that the index drops (sent.lock unstaged, truncated) refuses, or the CV it locked
    // could change in the same commit unseen (/code-review; europass.py read sent.lock from disk)
    for (rel, row) in locks(
        &ctx.git(&["show", &format!("HEAD:{LOCK}")])
            .unwrap_or_default(),
    ) {
        if !have.contains_key(&rel) {
            let (opp, date) = (
                row.get(2).map_or("", String::as_str),
                row.get(3).map_or("", String::as_str),
            );
            bad.push(format!(
                "{LOCK} drops the lock of {rel} ({opp}, {date}): a lock is never removed"
            ));
        }
    }
    if !have.is_empty() {
        // one cat-file for every locked file: a git show per file took 10 s over 340 (dashboard-fast-tests)
        let mut c = ctx.cmd(&ctx.top, "git");
        c.args(["cat-file", "--batch"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped());
        let mut child = c.spawn().map_err(|e| format!("git cat-file: {e}"))?;
        let ask: String = have.keys().map(|p| format!(":{p}\n")).collect();
        let mut stdin = child.stdin.take().expect("stdin");
        let writer =
            std::thread::spawn(move || std::io::Write::write_all(&mut stdin, ask.as_bytes()));
        let out = child
            .wait_with_output()
            .map_err(|e| format!("git cat-file: {e}"))?
            .stdout;
        let _ = writer.join();
        let mut at = 0;
        for (rel, row) in &have {
            let end = out[at..]
                .iter()
                .position(|b| *b == b'\n')
                .map_or(out.len(), |i| at + i);
            let header = String::from_utf8_lossy(&out[at..end]).into_owned();
            at = end + 1;
            // « <oid> <type> <size> », else « <name> missing »: a name may hold spaces, an oid never does
            let words: Vec<&str> = header.split(' ').collect();
            let found = match words[..] {
                [oid, kind, n] if !header.ends_with(" missing") && oid.len() >= 40 => {
                    n.parse::<usize>().ok().map(|n| (kind == "blob", n))
                }
                _ => None,
            };
            let now = found.and_then(|(blob, n)| {
                let bytes = &out[at..(at + n).min(out.len())];
                at += n + 1;
                // a tree or a submodule where the CV was: the file is gone
                blob.then(|| {
                    Sha256::digest(bytes)
                        .iter()
                        .map(|b| format!("{b:02x}"))
                        .collect::<String>()
                })
            });
            let date = row.get(2).map_or("", String::as_str);
            match now {
                None => bad.push(format!("{rel} was sent ({date}) and is gone")),
                Some(sha) if sha != row[0] => bad.push(format!(
                    "{rel} changed after it was sent ({date}, {})",
                    row.get(3).map_or("", String::as_str)
                )),
                _ => {}
            }
        }
    }
    match store_files(ctx, "opportunities").and_then(|o| Ok((o, store_files(ctx, "drafts")?))) {
        Err(why) => ctx.say(&format!(
            "cv-lock: the CRM half skipped, the app did not answer ({})",
            why.lines().next().unwrap_or_default()
        )),
        Ok((opps, drafts)) => {
            // in a merge, what differs from the merged-in side: a merge of main stages every file main added (review-lane)
            let mut args = vec!["diff", "--cached", "--name-only"];
            let mh = ctx.rev("MERGE_HEAD");
            args.extend(mh.as_deref());
            let touched: BTreeSet<String> = ctx.git(&args)?.lines().map(String::from).collect();
            for (o, cvs) in sent(&opps, &drafts) {
                for cv in cvs.iter().filter(|cv| !have.contains_key(*cv)) {
                    let why = format!(
                        "{o}: sent per the CRM but {cv} is not locked (private/dashboard/bin/dash cv lock {o})"
                    );
                    if touched.contains(cv) {
                        bad.push(why)
                    } else {
                        ctx.say(&format!("cv-lock: {why}"))
                    }
                }
            }
        }
    }
    if bad.is_empty() {
        return Ok(());
    }
    Err(bad
        .iter()
        .map(|b| format!("FAIL {b}"))
        .collect::<Vec<_>>()
        .join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;

    #[dash_test]
    fn a_cv_went_out_when_applied_or_a_sent_draft_carried_it() {
        let opps = BTreeMap::from([
            (
                "a.json".into(),
                r#"{"id":"~:a","cv":"private/cv/a.pdf","applied":"2026-10-01"}"#.into(),
            ),
            (
                "b.json".into(),
                r#"{"id":"~:b","cv":"private/cv/b.pdf"}"#.into(),
            ),
            (
                "c.json".into(),
                r#"{"id":"~:c","cv":"private/cv/c.pdf"}"#.into(),
            ),
        ]);
        let drafts = BTreeMap::from([
            ("b-intro.md".into(), "{\"opportunity\":\"~:b\",\"status\":\"~:sent\",\"attach\":\"private/x.pdf\"}\n---\nhi".into()),
            ("c-reply.md".into(), "{\"opportunity\":\"~:c\",\"status\":\"~:sent\"}\n---\nno file".into()),
        ]);
        let got = sent(&opps, &drafts);
        assert_eq!(
            got,
            BTreeMap::from([
                ("a".into(), vec!["private/cv/a.pdf".to_string()]),
                (
                    "b".into(),
                    vec!["private/cv/b.pdf".to_string(), "private/x.pdf".to_string()]
                ),
            ]),
            "a applied, b's intro carried its CV and attachment, c's reply carried none"
        );
        assert_eq!(
            locks("abc\tprivate/cv/a.pdf\ta\t2026-10-01\n\n").len(),
            1,
            "one row per locked file"
        );
        assert_eq!(
            untag(&serde_json::json!({"~#set": ["x"]})),
            serde_json::json!(["x"])
        );
    }
}
