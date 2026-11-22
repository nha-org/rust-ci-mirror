//! The PR gate's two checks on a pull request's text, ported from bin/explainer-gate and bin/pr-title-gate (task
//! rust-prepush-gates): one function each, run by CI (`dash ci explainer-gate <base> <head>`, `dash ci pr-title`, from
//! explainer-gate.yml) and by the pre-push on the pushed range, so a push the gate would refuse never leaves (PRs 1847, 1851
//! and 1855 went red on these alone, 2026-11-20). Each returns its pass line, or the refusal in the gate's own words.
//!
//! The explainer's merge gate (rfc-project-boards section 6; task explainer-fragments): a change that touches behaviour adds
//! a fragment explainer.d/<task-id>.md, or says « architecture: no change » in its description. It edits EXPLAINER.md only
//! when its description says « explainer: rewrite ». A fragment's first line is the heading of the section it joins, the
//! rest its text; the fold appends each to its section, so two PRs never edit the same lines of the explainer.
//!
//! The title gate (task pr-title-kind; @nha, 2026-10-29: « Adopt a PR title convention ?[fix|feat][task-123] title etc. or
//! in the branch name »): « [fix|feat|chore][<task-id>] <title> », and a description's « Task: <id> » line names that task
//! (task pr-body-own-task).
use regex::Regex;

/// Behaviour: a change under these, as the explainer reads it. Documents, the store's data and tests alone change none.
/// private/dashboard keeps what still runs or is read from there (dash/rfcs/RFC-0025 section 5.2).
const BEHAVIOUR_UNDER: [&str; 8] = [
    "private/dashboard/bin/",
    "private/dashboard/priv/commands/",
    "private/dashboard/priv/gh-meter/",
    "private/dashboard/extension/",
    "bin/",
    "herdr/",
    ".claude/hooks/",
    ".github/workflows/",
];
const BEHAVIOUR_FILES: [&str; 5] = [
    "private/dashboard/priv/ai_isms.json",
    "private/dashboard/assets/css/theme.css",
    "fly.toml",
    "Makefile",
    "sandbox.sb",
];
const EXPLAINER: &str = "EXPLAINER.md";
const FOLDER: &str = "explainer.d";
/// A PR opened before the fragments may still edit EXPLAINER.md. GitHub's created_at is ISO-8601 in UTC with a Z, so
/// strings compare as times.
const CUTOVER: &str = "2026-10-29T18:00:00Z";
/// A PR opened before the title convention passes untitled.
const TITLE_CUTOVER: &str = "2026-11-01T06:00:00Z";

/// Whether a change to `path` is behaviour.
pub fn behaviour(path: &str) -> bool {
    BEHAVIOUR_UNDER.iter().any(|p| path.starts_with(p)) || BEHAVIOUR_FILES.contains(&path)
}

fn says(body: &str, pattern: &str) -> bool {
    Regex::new(pattern).is_ok_and(|r| r.is_match(body))
}

/// A fragment's text the fold can paste: no heading of its own (it would start a new section) and every ``` fence closed
/// (an open one hides every later heading from the fold).
fn pasteable(text: &str) -> bool {
    let mut fenced = false;
    for line in text.split('\n') {
        if line.starts_with("```") {
            fenced = !fenced;
        } else if !fenced && (line.starts_with("# ") || line.starts_with("## ")) {
            return false;
        }
    }
    !fenced
}

/// The explainer gate on `base...head`, `git` run at the repository's top; `body` the PR's description, `created` its
/// opening time (empty when unknown).
pub fn explainer(
    git: &dyn Fn(&[&str]) -> Result<String, String>,
    base: &str,
    head: &str,
    body: &str,
    created: &str,
) -> Result<String, String> {
    let names = |args: &[&str]| -> Result<Vec<String>, String> {
        // -z: a path git would quote comes back as it is
        let mut all = vec!["diff", "--name-only", "-z", "--no-renames"];
        all.extend_from_slice(args);
        let out = git(&all).map_err(|e| format!("explainer-gate: {e}"))?;
        Ok(out
            .split('\0')
            .filter(|f| !f.is_empty())
            .map(String::from)
            .collect())
    };
    let three_dots = format!("{base}...{head}");
    // only the files that still differ from base: a squash-merged branch's commits stay in the three-dot diff though base
    // holds them all (review-gh-quota-burn, after PR 792 merged)
    let still = names(&[base, head])?;
    let changed = |only: &[&str]| -> Result<Vec<String>, String> {
        let mut args = only.to_vec();
        args.push(&three_dots);
        Ok(names(&args)?
            .into_iter()
            .filter(|f| still.contains(f))
            .collect())
    };
    // every line end a \n, as the script's text mode read them: a heading never keeps a \r
    let show = |path: &str| {
        (git(&["show", &format!("{head}:{path}")]).ok())
            .map(|text| text.replace("\r\n", "\n").replace('\r', "\n"))
    };
    let files = changed(&[])?;
    let legacy = !created.is_empty() && created < CUTOVER;
    let touched: Vec<&String> = files.iter().filter(|f| behaviour(f)).collect();
    // every fragment at head: the fold reads them all, so each must be one it can place
    let listed = git(&["ls-tree", "--name-only", &format!("{head}:{FOLDER}")]).unwrap_or_default();
    let frags: Vec<(String, String)> = listed
        .lines()
        .filter(|n| n.ends_with(".md"))
        .map(|n| {
            let path = format!("{FOLDER}/{n}");
            let text = show(&path).unwrap_or_default();
            (path, text)
        })
        .collect();
    let heading = Regex::new(r"(?m)^(## .+?)[ \t]*$").expect("the heading pattern");
    let explainer = show(EXPLAINER).unwrap_or_default();
    let mut known: Vec<&str> = heading
        .captures_iter(&explainer)
        .filter_map(|c| c.get(1))
        .map(|m| m.as_str())
        .collect();
    known.sort_unstable();
    known.dedup();
    let edits_explainer = files.iter().any(|f| f == EXPLAINER);
    // a rewrite of the explainer answers for every fragment, since one stranded by a renamed section never folds
    let bad: Vec<String> = frags
        .iter()
        .filter(|(f, _)| edits_explainer || files.contains(f))
        .filter_map(|(f, t)| {
            let (line, text) = t.split_once('\n').unwrap_or((t, ""));
            let line = line.trim_end();
            let text = text.trim();
            (!known.contains(&line) || text.is_empty() || !pasteable(text))
                .then(|| format!("{f} (« {line} »)"))
        })
        .collect();
    if !bad.is_empty() {
        let known = if known.is_empty() {
            "none".to_string()
        } else {
            known.join(", ")
        };
        return Err(format!(
            "explainer-gate: refused. A fragment's first line must be one of {EXPLAINER}'s section headings ({known}), its \
             text below it, with no « # » or « ## » heading and every ``` fence closed: {}.",
            bad.join(", ")
        ));
    }
    let rewrite = says(body, r"(?i)explainer:\s*rewrite");
    if edits_explainer && !rewrite && !legacy {
        return Err(format!(
            "explainer-gate: refused. This change edits {EXPLAINER}: put its text in {FOLDER}/<task-id>.md, its first line \
             the heading of the section it joins, or say « explainer: rewrite » in the description when the change is about \
             the explainer itself."
        ));
    }
    // a fragment it adds or edits: an edit counts, a reopened task's second PR rewrites its own (PR 1537's red)
    let added: Vec<String> = changed(&["--diff-filter=AM"])?
        .into_iter()
        .filter(|f| frags.iter().any(|(p, _)| p == f))
        .collect();
    if touched.is_empty() {
        Ok("explainer-gate: no behaviour changed".into())
    } else if !added.is_empty() {
        Ok(format!(
            "explainer-gate: {} added or edited with the behaviour",
            added.join(", ")
        ))
    } else if edits_explainer {
        let how = match legacy {
            true => format!("edited by a PR opened before {CUTOVER}"),
            false => "rewritten".into(),
        };
        Ok(format!(
            "explainer-gate: {EXPLAINER} {how} with the behaviour"
        ))
    } else if says(body, r"(?i)architecture:\s*no change") {
        Ok("explainer-gate: « architecture: no change », said in the description".into())
    } else {
        let shown: Vec<&str> = touched.iter().take(5).map(|f| f.as_str()).collect();
        let more = if touched.len() > 5 { ", ..." } else { "" };
        Err(format!(
            "explainer-gate: refused. This change touches behaviour ({}{more}) and neither adds a fragment \
             {FOLDER}/<task-id>.md (its first line a section heading of {EXPLAINER}) nor says « architecture: no change » in \
             its description.",
            shown.join(", ")
        ))
    }
}

/// The title gate on a PR's `title`, `body` and opening time `created` (empty when unknown).
pub fn title(title: &str, body: &str, created: &str) -> Result<String, String> {
    // ASCII \w and spaces, as the board's link key reads it and as the script's re.ASCII did: a no-break space is a
    // title's first character there, and no space before « Task: »
    let form = Regex::new(r"\A\[(fix|feat|chore)\]\[((?-u:[\w.-])+)\] [^ \t\n\r\x0B\x0C]")
        .expect("the title pattern");
    // the body's opening line, bold or not, as the board links the PR by it
    let task_line = Regex::new(r"\A(?-u:[*_\s])*Task:[*_ \t]*((?-u:[\w.-])*(?-u:\w))")
        .expect("the task pattern");
    let named = form
        .captures(title)
        .and_then(|c| c.get(2))
        .map(|m| m.as_str());
    let task = task_line
        .captures(body)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str());
    // a reviewer's PR is titled review-<id> and may keep its task's line, and a task's PR may carry its reviewer's
    let bare = |s: &str| s.strip_prefix("review-").unwrap_or(s).to_string();
    if let (Some(named), Some(task)) = (named, task)
        && bare(named) != bare(task)
    {
        return Err(format!(
            "pr-title-gate: refused. The description's « Task: {task} » is another task's than the title's « {named} »: \
             write this PR's own body to a file of your session's scratchpad, never a shared $TMPDIR/body.md, and gh pr edit \
             <the PR> --body-file <it>."
        ));
    }
    if named.is_some() {
        return Ok("pr-title-gate: title follows [fix|feat|chore][<task-id>]".into());
    }
    if !created.is_empty() && created < TITLE_CUTOVER {
        return Ok(format!(
            "pr-title-gate: opened before {TITLE_CUTOVER}, title not checked"
        ));
    }
    Err(format!(
        "pr-title-gate: refused. « {title} » must read « [fix|feat|chore][<task-id>] <title> », as « [fix][pr-title-kind] \
         Gate PR titles »: gh pr edit <the PR> --title '...' reruns this check."
    ))
}

/// The PR gate's steps (explainer-gate.yml), run at the repository's top: `dash ci explainer-gate <base> <head>`, `dash ci
/// pr-title`, and `dash ci no-elixir <base> <head>` on a pull request's own change (the pre-commit's step on what is staged,
/// task rust-git-hooks-port). The PR's text comes from the environment (PR_BODY, PR_CREATED, PR_TITLE), never the command
/// line, so its text is never run. Its line on stdout, a refusal on stderr; exit 1 for a refusal, never 2, which CI's
/// fallback reads as a main dash that lacks the command.
pub fn ci(words: &[&str]) -> ! {
    let (gate, args) = (
        words.first().copied().unwrap_or(""),
        words.get(1..).unwrap_or(&[]),
    );
    let env = |k: &str| std::env::var(k).unwrap_or_default();
    let top = crate::ratchet::top;
    let judged = match (gate, args) {
        ("explainer-gate", [base, head]) => top().and_then(|top| {
            explainer(
                &|a| super::git_in(&top, a),
                base,
                head,
                &env("PR_BODY"),
                &env("PR_CREATED"),
            )
        }),
        ("pr-title", []) => title(&env("PR_TITLE"), &env("PR_BODY"), &env("PR_CREATED")),
        ("no-elixir", [base, head]) => top()
            .and_then(|top| super::no_elixir::range(&top, base, head))
            .map(|()| String::new()),
        _ => Err(format!(
            "usage: dash ci {gate} {}",
            match gate {
                "pr-title" => "",
                _ => "<base> <head>",
            }
        )),
    };
    std::process::exit(match judged {
        Ok(line) => {
            if !line.is_empty() {
                println!("{line}");
            }
            0
        }
        Err(why) => {
            eprintln!("{why}");
            1
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;

    /// bin/pr-title-gate's cases, ported: the form, another task's body line, a reviewer's PR, the cutover.
    #[dash_test]
    fn a_title_reads_kind_and_task_and_the_body_names_the_same_task() {
        let ok = |t: &str, b: &str, c: &str| title(t, b, c).is_ok();
        assert!(ok("[fix][pr-title-kind] Gate PR titles", "", ""));
        assert!(ok("[feat][a.b-c_d] x", "**Task:** a.b-c_d\n", ""));
        assert!(ok("[fix][review-x] y", "Task: x", ""), "a reviewer's PR");
        assert!(ok("[fix][x] y", "Task: review-x", ""));
        assert!(
            ok("x: no form", "", "2026-10-30T00:00:00Z"),
            "opened before"
        );
        assert!(!ok("[docs][x] y", "", ""), "a kind of its own");
        assert!(!ok("[fix][x]y", "", ""), "no space");
        assert!(!ok("[fix][é] y", "", ""), "ASCII ids");
        // re.ASCII's spaces, as bin/pr-title-gate read them: a no-break space opens a title, and no « Task: » line
        assert!(ok("[fix][x] \u{a0}y", "", ""));
        assert!(ok("[fix][x] y", "\u{a0}Task: z\n", ""));
        assert!(!ok("[fix][x] \ty", "", ""));
        assert!(!ok("[fix][x] y", "\n *Task:* z\n", ""));
        let other = title("[fix][x] y", "Task: z\n", "").expect_err("another task's body");
        assert!(
            other.contains("« Task: z » is another task's than the title's « x »"),
            "{other}"
        );
        let bare = title("rust-x: a title", "", "2026-11-20T00:00:00Z").expect_err("no form");
        assert!(
            bare.starts_with("pr-title-gate: refused. « rust-x: a title »"),
            "{bare}"
        );
    }

    /// bin/explainer-gate's verdicts on a scratch repository: behaviour with no fragment refused in its words, passed by the
    /// description's marker or a fragment; a fragment under no known heading, or an edit of EXPLAINER.md, refused.
    #[dash_test]
    fn the_explainer_gate_judges_a_range_as_bin_explainer_gate_did() {
        let dir = dash_meter::TestDir::new("pr-gates-explainer");
        let top = dir.0.clone();
        let git = |a: &[&str]| super::super::git_in(&top, a);
        let put = |path: &str, text: &str| {
            let file = top.join(path);
            std::fs::create_dir_all(file.parent().expect("a folder")).expect("mkdir");
            std::fs::write(file, text).expect("write");
        };
        let commit = |what: &str| {
            git(&["add", "-A"]).expect("add");
            git(&[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "commit",
                "-qm",
                what,
            ])
            .expect("commit");
            git(&["rev-parse", "HEAD"]).expect("rev").trim().to_string()
        };
        git(&["init", "-q", "-b", "main"]).expect("init");
        put(
            EXPLAINER,
            "# E\n\n## Concepts\n\none\n\n## Architecture now  \n\ntwo\n",
        );
        put("docs/a.md", "a\n");
        let base = commit("base");
        put("docs/a.md", "b\n");
        let docs = commit("docs only");
        assert_eq!(
            explainer(&git, &base, &docs, "", ""),
            Ok("explainer-gate: no behaviour changed".into())
        );
        put("bin/tool", "#!/bin/sh\n");
        let tool = commit("a tool");
        let refused = explainer(&git, &base, &tool, "", "").expect_err("no fragment");
        assert!(
            refused.starts_with(
                "explainer-gate: refused. This change touches behaviour (bin/tool) and neither adds"
            ),
            "{refused}"
        );
        assert!(explainer(&git, &base, &tool, "Architecture:  No change\n", "").is_ok());
        put("explainer.d/t.md", "## Architecture now\n1. **x**\n");
        let fragment = commit("its fragment");
        assert_eq!(
            explainer(&git, &base, &fragment, "", ""),
            Ok("explainer-gate: explainer.d/t.md added or edited with the behaviour".into())
        );
        put("explainer.d/t.md", "## Nowhere\n1. **x**\n");
        let nowhere = commit("a heading the explainer lacks");
        let bad = explainer(&git, &base, &nowhere, "", "").expect_err("no such heading");
        assert!(
            bad.contains("(## Architecture now, ## Concepts)")
                && bad.ends_with("explainer.d/t.md (« ## Nowhere »)."),
            "{bad}"
        );
        put("explainer.d/t.md", "## Architecture now\n1. **x**\n");
        put(
            EXPLAINER,
            "# E\n\n## Concepts\n\none, edited\n\n## Architecture now\n\ntwo\n",
        );
        let edited = commit("the explainer edited");
        let edit = explainer(&git, &base, &edited, "", "").expect_err("an edit");
        assert!(edit.contains("This change edits EXPLAINER.md"), "{edit}");
        assert!(explainer(&git, &base, &edited, "explainer: rewrite", "").is_ok());
        assert!(
            explainer(&git, &base, &edited, "", "2026-10-01T00:00:00Z").is_ok(),
            "opened before the cutover"
        );
        // CRLF line ends, which the script's text mode read as \n: a heading keeps no \r, so its fragment still joins it
        put(
            EXPLAINER,
            "# E\r\n\r\n## Concepts\r\n\r\none\r\n\r\n## Architecture now\r\n\r\ntwo\r\n",
        );
        put("explainer.d/t.md", "## Architecture now\r\n1. **x**\r\n");
        let crlf = commit("CRLF line ends");
        assert!(
            explainer(&git, &base, &crlf, "explainer: rewrite", "").is_ok(),
            "CRLF line ends"
        );
    }

    /// A fragment the fold cannot paste: a heading of its own, or a fence left open.
    #[dash_test]
    fn a_fragment_s_text_has_no_heading_and_every_fence_closed() {
        assert!(pasteable("1. **x**\n   - y\n```\n# in a fence\n```"));
        assert!(!pasteable("1. x\n## Another section"));
        assert!(!pasteable("1. x\n```\nopen"));
        assert!(pasteable("1. x\n### a subheading"));
    }
}
