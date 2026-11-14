//! `dash ci affected <base> [-- <cargo arguments>]`: the workspace crates a change touches and every crate depending on them,
//! read from `cargo metadata` (task rust-affected-tests; @nha, 2026-11-11: « we can then skip CI if the crate doesn't change »).
//! The mirror's CI and the pre-push test those crates alone; a change to OCR runs no mail test. The diff is from the merge-base
//! with `<base>` to the working tree, or from `<base>` itself when the two share no history (the mirror's parentless commits).
//! A changed file under the workspace but in no crate (Cargo.lock, .config, ci/, vendor/) affects every crate; one outside
//! the workspace (site/, private/) none, as the mirror never holds it. dash-gate always runs once anything does: its tests
//! hold the whole workspace to its rules (rust/CLAUDE.md rule 2). Main's mirror run still tests the whole workspace.
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::process::Command;

/// The crate whose tests read every other crate's files.
pub const ALWAYS: &str = "dash-gate";

/// The affected crates, sorted: from `cargo metadata --no-deps`'s JSON, the workspace's folder relative to the repository's
/// top (`""` when they are one, as on the mirror), and the changed paths relative to that top.
pub fn affected(
    metadata: &str,
    workspace: &str,
    changed: &[String],
) -> Result<Vec<String>, String> {
    let meta: Value = serde_json::from_str(metadata).map_err(|e| format!("cargo metadata: {e}"))?;
    let root = meta["workspace_root"]
        .as_str()
        .ok_or("cargo metadata: no workspace_root")?;
    let mut crates = vec![];
    for p in meta["packages"]
        .as_array()
        .ok_or("cargo metadata: no packages")?
    {
        let name = p["name"].as_str().unwrap_or_default().to_string();
        let manifest = Path::new(p["manifest_path"].as_str().unwrap_or_default());
        let dir = manifest
            .parent()
            .and_then(|d| d.strip_prefix(root).ok())
            .unwrap_or(Path::new(""));
        // every kind: a dev-dependency's change reaches the tests that use it
        let deps: BTreeSet<String> = p["dependencies"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|d| d["path"].is_string())
            .filter_map(|d| d["name"].as_str().map(String::from))
            .collect();
        crates.push((name, dir.to_string_lossy().into_owned(), deps));
    }
    let prefix = if workspace.is_empty() {
        String::new()
    } else {
        format!("{}/", workspace.trim_end_matches('/'))
    };
    let mut hit = BTreeSet::new();
    for path in changed {
        let Some(rel) = path.strip_prefix(&prefix) else {
            continue;
        };
        // the longest crate folder holding it: a crate's folder never names another's
        match crates
            .iter()
            .filter(|(_, dir, _)| !dir.is_empty() && rel.starts_with(&format!("{dir}/")))
            .max_by_key(|c| c.1.len())
        {
            Some((name, _, _)) => hit.insert(name.clone()),
            // ponytail: any other workspace file runs everything (a README too); a list of inert files if that ever costs
            None => {
                return Ok(crates
                    .into_iter()
                    .map(|c| c.0)
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect());
            }
        };
    }
    // each crate depending on one already hit, until none is added
    while let Some(next) = crates
        .iter()
        .find(|(name, _, deps)| !hit.contains(name) && deps.iter().any(|d| hit.contains(d)))
    {
        hit.insert(next.0.clone());
    }
    if !hit.is_empty() && crates.iter().any(|c| c.0 == ALWAYS) {
        hit.insert(ALWAYS.to_string());
    }
    Ok(hit.into_iter().collect())
}

/// `cargo <arguments>` with a `-p <crate>` for each affected one, before the first `--` (clippy's own flags follow it).
pub fn cargo_args(crates: &[String], args: &[String]) -> Vec<String> {
    let at = args.iter().position(|a| a == "--").unwrap_or(args.len());
    let packages = crates.iter().flat_map(|c| ["-p".to_string(), c.clone()]);
    args[..at]
        .iter()
        .cloned()
        .chain(packages)
        .chain(args[at..].iter().cloned())
        .collect()
}

/// The workspace holding `dir`: its manifest, the repository's top and the workspace's folder relative to that top.
pub fn workspace(dir: &Path) -> Result<(PathBuf, PathBuf, String), String> {
    let top = PathBuf::from(run("git", dir, &["rev-parse", "--show-toplevel"])?.trim());
    // site_perso keeps it in rust/; the mirror holds rust/ at its top
    for folder in ["rust", ""] {
        let manifest = top.join(folder).join("Cargo.toml");
        if manifest.is_file() {
            return Ok((manifest, top, folder.to_string()));
        }
    }
    Err(format!(
        "{}: no Cargo.toml at its top or in rust/",
        top.display()
    ))
}

/// The affected crates of the repository holding `dir`, against `base`, with `cargo` (CARGO when a cargo runs us).
pub fn of_repo(dir: &Path, base: &str, cargo: &str) -> Result<Vec<String>, String> {
    let (manifest, top, folder) = workspace(dir)?;
    let manifest = manifest.to_string_lossy();
    let meta = run(
        cargo,
        &top,
        &[
            "metadata",
            "--format-version",
            "1",
            "--no-deps",
            "--offline",
            "--manifest-path",
            &manifest,
        ],
    )?;
    // unrelated histories (the mirror's main against its branch commit) have no merge-base: their trees are compared
    let from = run("git", &top, &["merge-base", base, "HEAD"]).unwrap_or_else(|_| base.to_string());
    let names = run(
        "git",
        &top,
        &[
            "-c",
            "core.quotePath=false",
            "diff",
            "--name-only",
            "--no-renames",
            from.trim(),
        ],
    )?;
    let changed: Vec<String> = names.lines().map(String::from).collect();
    affected(&meta, &folder, &changed)
}

/// Why the checkout holding `dir` is not `sha` as pushed: HEAD another commit, or a change to the workspace not committed.
/// None when it is: the pre-push then tests what leaves, in this warm worktree rather than a cold checkout.
pub fn not_as_pushed(dir: &Path, sha: &str) -> Result<Option<String>, String> {
    let (_, top, folder) = workspace(dir)?;
    let head = run("git", &top, &["rev-parse", "HEAD"])?;
    let pushed = run("git", &top, &["rev-parse", &format!("{sha}^{{commit}}")])?;
    if head.trim() != pushed.trim() {
        return Ok(Some(format!(
            "this checkout is at {}, not {sha} as pushed",
            head.trim()
        )));
    }
    let scope = if folder.is_empty() {
        ".".to_string()
    } else {
        folder
    };
    let dirty = run("git", &top, &["status", "--porcelain", "--", &scope])?;
    Ok((!dirty.trim().is_empty()).then(|| format!("{scope}/ has changes {sha} does not hold")))
}

fn run(program: &str, dir: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new(program)
        .args(args)
        .current_dir(dir)
        .output()
        .map_err(|e| format!("{program} {}: {e}", args.join(" ")))?;
    if !out.status.success() {
        return Err(format!(
            "{program} {} failed ({}): {}",
            args.join(" "),
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// A test as nextest names it: its binary id and its name.
pub type Test = (String, String);

/// The seconds a test never timed here is taken to cost: a new test runs in the first slice, its time learnt after.
pub const UNKNOWN_SECS: f64 = 1.0;

/// The tests `cargo nextest list --message-format json` prints as run: matching its filters, not ignored. `said` is the
/// command's whole output, cargo's lines included: the JSON is its one line opening on `{` that holds `"rust-suites"` (a
/// build script's own JSON line is not it, /code-review).
pub fn listed(said: &str) -> Result<Vec<Test>, String> {
    let line = said
        .lines()
        .find(|l| l.starts_with('{') && l.contains("\"rust-suites\""))
        .ok_or("nextest list printed no JSON")?;
    let v: Value = serde_json::from_str(line).map_err(|e| format!("nextest list: {e}"))?;
    let mut out = vec![];
    for (id, suite) in v["rust-suites"]
        .as_object()
        .ok_or("nextest list: no rust-suites")?
    {
        for (name, case) in suite["testcases"].as_object().into_iter().flatten() {
            if case["ignored"] != Value::Bool(true) && case["filter-match"]["status"] == "matches" {
                out.push((id.clone(), name.clone()));
            }
        }
    }
    Ok(out)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Outcome {
    Pass,
    /// past its cap, nextest's slow-timeout
    Timeout,
    Fail,
}

/// A test's run in nextest's junit.xml
#[derive(Debug, Clone, PartialEq)]
pub struct Case {
    pub test: Test,
    pub secs: f64,
    pub outcome: Outcome,
}

/// The cases of a nextest junit.xml (`[profile.prepush.junit]`): each testcase's binary (classname), name and time, a
/// `<failure type="test timeout">` read as a timeout, any other failure or error as a failure.
pub fn junit(xml: &str) -> Vec<Case> {
    let attr = |tag: &str, key: &str| -> Option<String> {
        let at = tag.find(&format!(" {key}=\""))? + key.len() + 3;
        let end = tag[at..].find('"')? + at;
        Some(
            tag[at..end]
                .replace("&lt;", "<")
                .replace("&gt;", ">")
                .replace("&quot;", "\"")
                .replace("&apos;", "'")
                .replace("&amp;", "&"),
        )
    };
    let mut out = vec![];
    for chunk in xml.split("<testcase ").skip(1) {
        // the space split off with « <testcase » put back, so the first attribute reads as the others
        let tag = format!(" {}", &chunk[..chunk.find('>').unwrap_or(chunk.len())]);
        let body = if tag.ends_with('/') {
            ""
        } else {
            &chunk[..chunk.find("</testcase>").unwrap_or(chunk.len())]
        };
        let (Some(name), Some(binary)) = (attr(&tag, "name"), attr(&tag, "classname")) else {
            continue;
        };
        let outcome = if body.contains("<failure type=\"test timeout\"") {
            Outcome::Timeout
        } else if body.contains("<failure") || body.contains("<error") {
            Outcome::Fail
        } else {
            Outcome::Pass
        };
        let secs = attr(&tag, "time")
            .and_then(|t| t.parse().ok())
            .unwrap_or(0.0);
        out.push(Case {
            test: (binary, name),
            secs,
            outcome,
        });
    }
    out
}

/// The times store, « <seconds> <binary> <name> » a line.
pub fn times(text: &str) -> HashMap<Test, f64> {
    text.lines()
        .filter_map(|l| {
            let mut w = l.splitn(3, ' ');
            let secs = w.next()?.parse().ok()?;
            Some(((w.next()?.to_string(), w.next()?.to_string()), secs))
        })
        .collect()
}

/// The store with `cases` learnt: a known test moves halfway to its new time, so one run under load does not double it.
/// ponytail: a renamed or deleted test's line stays, and two pushes at once keep the last writer's store; prune by the
/// workspace's full list if it ever grows past use
pub fn learn(store: &str, cases: &[Case]) -> String {
    let mut t = times(store);
    for c in cases {
        let secs = t.get(&c.test).map_or(c.secs, |old| (old + c.secs) / 2.0);
        t.insert(c.test.clone(), secs);
    }
    let mut lines: Vec<String> = t
        .into_iter()
        .map(|((b, n), s)| format!("{s:.3} {b} {n}"))
        .collect();
    lines.sort();
    lines.join("\n") + "\n"
}

/// The cases of a slice cut before its report: each test at twice its cost, UNKNOWN_SECS for one never timed.
pub fn slower(tests: &[Test], times: &HashMap<Test, f64>) -> Vec<Case> {
    tests
        .iter()
        .map(|t| Case {
            test: t.clone(),
            secs: 2.0 * times.get(t).copied().unwrap_or(UNKNOWN_SECS),
            outcome: Outcome::Timeout,
        })
        .collect()
}

/// The slice that fits: the tests fastest first, each known one by its time here, until their summed seconds would pass
/// `capacity`; then the rest, the slowest. At least one test runs.
pub fn slice(
    mut tests: Vec<Test>,
    times: &HashMap<Test, f64>,
    capacity: f64,
) -> (Vec<Test>, Vec<Test>) {
    let cost = |t: &Test| times.get(t).copied().unwrap_or(UNKNOWN_SECS);
    tests.sort_by(|a, b| cost(a).total_cmp(&cost(b)).then_with(|| a.cmp(b)));
    let mut sum = 0.0;
    let at = tests
        .iter()
        .position(|t| {
            sum += cost(t);
            sum > capacity
        })
        .unwrap_or(tests.len())
        .max(1.min(tests.len()));
    let rest = tests.split_off(at);
    (tests, rest)
}

/// A nextest filterset naming exactly `tests`: one `binary_id(=b) & test(/^(?:n1|n2)$/)` a binary.
pub fn filterset(tests: &[Test]) -> String {
    let mut by: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for (b, n) in tests {
        by.entry(b).or_default().push(regex::escape(n));
    }
    by.iter()
        .map(|(b, n)| format!("(binary_id(={b}) & test(/^(?:{})$/))", n.join("|")))
        .collect::<Vec<_>>()
        .join(" | ")
}

/// The red tests of each kind rerun alone: the first few, so a broad break costs seconds, never one 40 s bound a test
/// (/code-review). More past their cap at once is the load itself.
pub const ALONE: usize = 3;

/// The tests of a run to rerun alone: the first ALONE failures and the first ALONE past their cap.
pub fn rerun(run: &[Case]) -> Vec<Test> {
    let first = |o: Outcome| run.iter().filter(move |c| c.outcome == o).take(ALONE);
    first(Outcome::Fail)
        .chain(first(Outcome::Timeout))
        .map(|c| c.test.clone())
        .collect()
}

/// A run's red tests split by their rerun alone (task pre-push-fits-budget, done-when 2): (the push's failures, the load).
/// One that passes alone is load; one that fails alone, or that the alone run never reported, is the push's. A test past
/// the first ALONE of its kind takes their verdict: load only when each of them passed alone (/code-review). A failure that
/// passes alone is load too, as the --retries 2 this replaces passed it (FLAKY).
/// ponytail: a real hang among 4+ load timeouts goes as load; CI's run still holds it.
pub fn verdict(run: &[Case], alone: &[Case]) -> (Vec<Test>, Vec<Test>) {
    let again = rerun(run);
    let passed = |t: &Test| {
        alone
            .iter()
            .any(|a| &a.test == t && a.outcome == Outcome::Pass)
    };
    let kind_passed = |o: Outcome| {
        run.iter()
            .filter(|c| c.outcome == o && again.contains(&c.test))
            .all(|c| passed(&c.test))
    };
    let (mut failed, mut load) = (vec![], vec![]);
    for c in run.iter().filter(|c| c.outcome != Outcome::Pass) {
        let ok = if again.contains(&c.test) {
            passed(&c.test)
        } else {
            kind_passed(c.outcome)
        };
        if ok {
            load.push(c.test.clone());
        } else {
            failed.push(c.test.clone());
        }
    }
    (failed, load)
}

#[cfg(test)]
mod slice_tests {
    use super::*;
    use dash_meter::dash_test;

    fn t(b: &str, n: &str) -> Test {
        (b.into(), n.into())
    }

    /// Done-when 1 (task pre-push-fits-budget): the fastest tests run up to the budget, the slowest are left, a new test
    /// runs at UNKNOWN_SECS, and the filterset names exactly the slice.
    #[dash_test]
    fn the_slice_takes_the_fastest_tests_up_to_its_budget() {
        let store =
            "0.100 dash-gate a::fast\n30.000 dash::hooks slow_one\n2.000 dash-gate b::mid\n";
        let tests = vec![
            t("dash::hooks", "slow_one"),
            t("dash-gate", "b::mid"),
            t("dash-gate", "a::fast"),
            t("dash-gate", "c::new"),
        ];
        let (run, rest) = slice(tests.clone(), &times(store), 5.0);
        assert_eq!(
            run,
            [
                t("dash-gate", "a::fast"),
                t("dash-gate", "c::new"),
                t("dash-gate", "b::mid")
            ]
        );
        assert_eq!(rest, [t("dash::hooks", "slow_one")]);
        assert_eq!(
            filterset(&run),
            "(binary_id(=dash-gate) & test(/^(?:a::fast|c::new|b::mid)$/))"
        );
        // all fit: nothing left; none fits: the fastest still runs
        let (_, rest) = slice(tests.clone(), &times(store), 1000.0);
        assert!(
            rest.is_empty(),
            "a budget past every test's time leaves none to CI: {rest:?}"
        );
        assert_eq!(
            slice(tests, &times(store), 0.0).0,
            [t("dash-gate", "a::fast")]
        );
        assert_eq!(slice(vec![], &times(store), 0.0), (vec![], vec![]));
    }

    /// Done-when 2 (task pre-push-fits-budget): a test past its cap that passes alone is load, never the push's failure; one
    /// that fails alone, or times out alone, is.
    #[dash_test]
    fn a_test_past_its_cap_that_passes_alone_is_load() {
        let case = |n: &str, outcome| Case {
            test: t("dash-gate", n),
            secs: 30.0,
            outcome,
        };
        let run = [
            case("ok", Outcome::Pass),
            case("slow_under_load", Outcome::Timeout),
            case("broken", Outcome::Fail),
            case("hangs", Outcome::Timeout),
        ];
        assert_eq!(
            rerun(&run),
            [
                t("dash-gate", "broken"),
                t("dash-gate", "slow_under_load"),
                t("dash-gate", "hangs")
            ]
        );
        let alone = [
            case("slow_under_load", Outcome::Pass),
            case("broken", Outcome::Fail),
            case("hangs", Outcome::Timeout),
        ];
        let (failed, load) = verdict(&run, &alone);
        assert_eq!(failed, [t("dash-gate", "broken"), t("dash-gate", "hangs")]);
        assert_eq!(load, [t("dash-gate", "slow_under_load")]);
        // no alone run reported (cut, crashed): nothing excused
        assert_eq!(verdict(&run, &[]).0.len(), 3);
        // past ALONE, the fourth timeout is load beside three that passed alone
        let many: Vec<Case> = (0..4)
            .map(|i| case(&format!("t{i}"), Outcome::Timeout))
            .collect();
        let passed: Vec<Case> = (0..3)
            .map(|i| case(&format!("t{i}"), Outcome::Pass))
            .collect();
        assert_eq!(
            verdict(&many, &passed),
            (vec![], many.iter().map(|c| c.test.clone()).collect())
        );
        // ... and a hang when the three hang alone too (/code-review)
        let hung: Vec<Case> = (0..3)
            .map(|i| case(&format!("t{i}"), Outcome::Timeout))
            .collect();
        assert_eq!(
            verdict(&many, &hung).0.len(),
            4,
            "the fourth follows its three siblings"
        );
        // a broad break reruns three, never one 40 s bound for each of 150
        let broad: Vec<Case> = (0..150)
            .map(|i| case(&format!("f{i}"), Outcome::Fail))
            .collect();
        assert_eq!(rerun(&broad).len(), ALONE);
        assert_eq!(
            verdict(&broad, &[]).0.len(),
            150,
            "a failure past the three is the push's when they fail alone"
        );
    }

    #[dash_test]
    fn junit_and_the_list_read_as_nextest_writes_them() {
        let xml = r#"<testsuite name="dash-gate">
        <testcase name="x::slow" classname="dash-gate" timestamp="t" time="30.002">
            <failure type="test timeout"/>
        </testcase>
        <testcase name="x::ok" classname="dash-gate" timestamp="t" time="1.0"/>
        <testcase name="x::bad" classname="dash::hooks" timestamp="t" time="1.5">
            <failure type="test failure" message="a &amp; b"/>
        </testcase>"#;
        let cases = junit(xml);
        let outcomes: Vec<_> = cases
            .iter()
            .map(|c| (c.test.1.as_str(), c.outcome))
            .collect();
        assert_eq!(
            outcomes,
            [
                ("x::slow", Outcome::Timeout),
                ("x::ok", Outcome::Pass),
                ("x::bad", Outcome::Fail)
            ]
        );
        assert_eq!(cases[2].test.0, "dash::hooks");
        let learnt = learn("2.000 dash-gate x::ok\n", &cases);
        assert_eq!(times(&learnt)[&t("dash-gate", "x::ok")], 1.5);
        assert_eq!(times(&learnt)[&t("dash-gate", "x::slow")], 30.002);
        let list = "   Compiling x\n{\"rust-suites\":{\"dash-gate\":{\"testcases\":{\"a\":{\"ignored\":false,\"filter-match\":{\"status\":\"matches\"}},\"b\":{\"ignored\":true,\"filter-match\":{\"status\":\"matches\"}},\"c\":{\"ignored\":false,\"filter-match\":{\"status\":\"mismatch\"}}}}}}\n";
        assert_eq!(listed(list).unwrap(), [t("dash-gate", "a")]);
        // a build script's JSON line before nextest's is not the list (/code-review)
        assert_eq!(
            listed(&format!("{{\"reason\":\"x\"}}\n{list}")).unwrap(),
            [t("dash-gate", "a")]
        );
        let cut = slower(
            &[t("dash-gate", "x::ok"), t("dash-gate", "new")],
            &times("2.000 dash-gate x::ok\n"),
        );
        assert_eq!(
            cut.iter().map(|c| c.secs).collect::<Vec<_>>(),
            [4.0, 2.0 * UNKNOWN_SECS]
        );
    }
}
