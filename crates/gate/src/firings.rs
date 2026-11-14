//! `dash gate firings`: how often each rule of workspace_rules.rs failed on this Mac, zero included (task
//! rust-gate-rule-firings; @nha, 2026-11-06: « slow is smooth and smooth is fast. we aim for slow but systemic
//! improvement. »). Ported from Dashboard.Orch.GateFirings when the rules took `#[dash_test]`, which its regex did not
//! read (task rust-test-budget-attr). Each failure is a line `<rule>\t<UTC day number>` the rule's `Firing` adds to one file
//! for every worktree, so a count is the days it failed: a rerun of one failure adds none. Each Bash guard's refusal, and
//! pipe-cut's and review-claim's answers, are firings too, listed after the rules (task stress-run-scoped).
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

pub const USAGE: &str = "dash gate firings (how often each rule of rust/crates/gate/tests/workspace_rules.rs failed on \
                         this Mac, zero included, one line per rule)";
/// A rule's attribute, alone on its line above its `fn`: the test's budget and its firing in one guard.
pub const RULE: &str = "#[dash_test(rule!())]";
const RULES: &str = "rust/crates/gate/tests/workspace_rules.rs";
const LOG: &str = ".claude/run/gate-firings.tsv";

/// The firings file under `home`, the one every rule and hook appends to.
pub fn log(home: &Path) -> std::path::PathBuf {
    home.join(LOG)
}

/// The names pipe-cut's and review-claim's firings are recorded and listed under, here since hook/ reads this module.
pub const PIPE_CUT: &str = "pipe_cut";
pub const REVIEW_CLAIM: &str = "review_claim";

/// The firings file under this process's HOME, none when HOME is not absolute: every writer of dash records there or not at
/// all, and `dash gate firings` reads the same.
pub fn log_from_env() -> Option<std::path::PathBuf> {
    let home = std::env::var_os("HOME").filter(|h| Path::new(h).is_absolute())?;
    Some(log(Path::new(&home)))
}

/// One firing of `rule` in [`log_from_env`]'s file, as a guard's refusal records it (task stress-run-scoped): no absolute
/// HOME, no count, and a firing it cannot write is dropped, never changing the deny.
pub fn record_from_env(rule: &str) {
    if let Some(log) = log_from_env() {
        let _ = record(&log, rule);
    }
}

/// One `<rule>\t<UTC day number>` line appended to `file`, the firings `dash gate firings` counts.
pub fn record(file: &Path, rule: &str) -> std::io::Result<()> {
    use std::io::Write;
    std::fs::create_dir_all(file.parent().unwrap_or(Path::new(".")))?;
    let day = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() / 86_400);
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(file)?
        .write_all(format!("{rule}\t{day}\n").as_bytes())
}

/// The rules of `source`: each fn under [`RULE`], or under the bare `#[test]` an older checkout holds. workspace_rules.rs's
/// every_rule_opens_with_its_guard holds it to that file's attributes, so the two readers cannot drift (/code-review).
pub fn rules(source: &str) -> Vec<&str> {
    let lines: Vec<&str> = source.lines().map(str::trim).collect();
    lines
        .windows(2)
        .filter(|w| w[0] == RULE || w[0] == "#[test]")
        .filter_map(|w| w[1].strip_prefix("fn ")?.split(['(', '<']).next())
        .collect()
}

/// Each rule of `source` with the days `log` recorded it, zero included, then each recorded rule `source` lacks, marked.
fn lines(source: &str, log: &str) -> Vec<String> {
    let mut fired: BTreeMap<&str, usize> = BTreeMap::new();
    for line in log
        .lines()
        .filter(|l| !l.is_empty())
        .collect::<BTreeSet<_>>()
    {
        *fired
            .entry(line.split('\t').next().unwrap_or(line))
            .or_default() += 1;
    }
    // the hooks that record firings too, after the rules: each Bash guard's refusal, pipe-cut's and review-claim's answers
    let rules: Vec<&str> = (rules(source).into_iter())
        .chain(crate::guard::NAMES)
        .chain([PIPE_CUT, REVIEW_CLAIM])
        .collect();
    let listed = rules
        .iter()
        .map(|r| format!("{} {r}", fired.get(r).copied().unwrap_or(0)));
    let gone = fired
        .iter()
        .filter(|(r, _)| !rules.contains(r))
        .map(|(r, n)| format!("{n} {r} (not in this checkout)"));
    listed.chain(gone).collect()
}

/// The report for the checkout around the current folder, from the log under `home`.
pub fn say_here(home: &Path) -> Result<String, String> {
    let top = crate::hook::git_in(Path::new("."), &["rev-parse", "--show-toplevel"])
        .map_err(|e| format!("gate firings: {e}"))?;
    say(Path::new(top.trim()), home)
}

/// The report for the checkout at `top`, from the log under `home`: no log yet is every rule at 0, one it cannot read an
/// error, never a zero.
pub fn say(top: &Path, home: &Path) -> Result<String, String> {
    let failed = |path: &Path, e: std::io::Error| format!("gate firings: {}: {e}", path.display());
    let source_path = top.join(RULES);
    let source = std::fs::read_to_string(&source_path).map_err(|e| failed(&source_path, e))?;
    let log = home.join(LOG);
    let text = match std::fs::read_to_string(&log) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(failed(&log, e)),
    };
    let header = format!(
        "gate firings: days each rule's failure was recorded, from {}",
        log.display()
    );
    Ok(std::iter::once(header)
        .chain(lines(&source, &text))
        .collect::<Vec<_>>()
        .join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::{TestDir, dash_test};

    const SOURCE: &str = "const X: &str = \"#[dash_test(rule!())]\";\n@\nfn the_dash_crate_holds_no_html() {}\n\n    @\n    \
                          fn a_rule_that_fired() {}\n\n#[test]\nfn an_older_rule() {}\n\nfn a_helper() {}\n";

    /// The ported cases of gate_firings_test.exs: every rule listed, then each hook that records firings, zero included; a
    /// day counted once; a rule the checkout lacks kept, marked; a log it cannot read an error.
    #[dash_test]
    fn each_rule_is_listed_with_the_days_it_fired() {
        let source = SOURCE.replace('@', RULE);
        let log = "a_rule_that_fired\t20763\na_rule_that_fired\t20763\na_rule_that_fired\t20764\ngone_rule\t20758\nnextest-stress\t20763\n";
        assert_eq!(
            lines(&source, log),
            [
                "0 the_dash_crate_holds_no_html",
                "2 a_rule_that_fired",
                "0 an_older_rule",
                "0 no-pkill",
                "0 no-verify-commit",
                "0 cargo-test",
                "0 fly-legacy-guard",
                "0 download-guard",
                "0 main-checkout-code",
                "1 nextest-stress",
                "0 pipe_cut",
                "0 review_claim",
                "1 gone_rule (not in this checkout)"
            ]
        );
        let dir = TestDir::new("gate-firings");
        let rules = dir.0.join(RULES);
        std::fs::create_dir_all(rules.parent().expect("a folder")).expect("mkdir");
        std::fs::write(&rules, &source).expect("write");
        let said = say(&dir.0, &dir.0).expect("no log yet");
        assert!(
            said.contains(
                "\n0 the_dash_crate_holds_no_html\n0 a_rule_that_fired\n0 an_older_rule\n0 no-pkill\n"
            ) && said.ends_with("\n0 review_claim"),
            "{said}"
        );
        std::fs::create_dir_all(dir.0.join(LOG)).expect("a folder where the log goes");
        let err = say(&dir.0, &dir.0).expect_err("a log it cannot read");
        assert!(
            err.starts_with("gate firings: ") && err.contains("gate-firings.tsv"),
            "{err}"
        );
        // bin/dash hands the words to the Rust binary only when it holds the usage bin/dash greps for
        let Some(top) = dash_meter::monorepo() else {
            return;
        };
        let wrapper =
            std::fs::read_to_string(top.join("private/dashboard/bin/dash")).expect("bin/dash");
        assert!(
            wrapper.contains(&format!("\"gate firings\") known='{USAGE}'")),
            "bin/dash does not route gate firings to Rust: copy USAGE into its case"
        );
    }
}
