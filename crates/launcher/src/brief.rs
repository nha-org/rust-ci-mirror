//! A task's brief, the words its session starts from, written here (task brief-tests-prepush-skip; @nha, 2026-11-10:
//! « remember we move to rust. no more elixir, so if something needs fixing port and then fix »): the older app's
//! `Dashboard.Orch.Launch.argv/2` and herdr's `session.sh brief`, ported for Rust's runs, which are all site_perso's. The
//! launcher's crate, as the older app's Launch module was: its words name the agent's tools and files, which orch's never do.
//! ponytail: what no Rust run takes yet is left out: a clone of another repository (Rust's launcher clones none), roles, the
//! vibe brief and the CI billing note (the older app's state); each comes back with the first Rust run that needs it.
use dash_core::tasks::Filed;

/// Where Rust's runs ship: their PR's title convention, their task's page.
const PROJECT: &str = "site_perso";
const PAGE: &str = "http://100.72.158.115:7788/projects/site_perso?issue=";
/// A run's turns when its row names none, as `session.sh` gave.
const TURNS: u32 = 40;

/// The standing rules, the same for every task: only what any task, whatever it touches, can break (@nha, 2026-10-18:
/// « concise. Clear. precise. this is ramblings. »). Each rule's cause and his words are in `git log -p` of
/// private/dashboard/lib/dashboard/orch/launch.ex, where they were written first. Rule 2 asks each done-when part's one
/// proving test: a report that proved a part by a screen alone took rust-one-doc-component two report rounds, and a
/// pre-push that compiled nothing it changed cost four tasks minutes (this task's lessons). Keep it short: every task pays it
/// in every turn; a rule that a check or hook can hold goes there instead (@nha, 2026-10-20: « seems the prompt is getting
/// longer and longer ... consider one time cost vs repeated insertions etc. we need to be strategic here »).
pub fn standing(b: &str) -> String {
    format!(
        "Rules: 1. Ask me only in the AskUserQuestion dialog, never as plain text or a PushNotification; waiting on my answer \
         is a complete outcome. 2. Each done-when part has its one proving test, named beside it in your report (write it \
         where none holds the part yet); run each check on your branch merged with {b}, its command and output recorded in \
         your report, and the touched tests before TASK_DONE. 3. Re-read and show every line you changed by script. 4. \
         Merge {b} at the start and once right before the final checks, which wait for its clean exit; commit first, never \
         stash; push with git push, never dash ship (it flattens a merge); an edit meant for {b} rides your PR. 5. If \
         /code-review forks to the background, end the turn without TASK_DONE: its notification wakes you; post its \
         findings on the PR, then finish. 6. One /code-review of your own commits (never a range holding a merge of {b}), \
         each finding fixed or answered, then at most one more pass on the fixes. 7. Push your commits with your merge of {b}, \
         then read gh pr view --json mergeable once (a conflicting PR turns MERGEABLE only after that push): CONFLICTING, \
         merge {b} again and push; then stop: only the PM reviewer (review-<id>) polls CI (a reviewer: your steps 3 and 4 \
         instead); no local precommit unless CI cannot run it (:mac or :host tests, uncovered paths). 8. A done-when \
         clause that reads two ways: ask which in the dialog; until answered, build to the stricter. 9. Run dash from \
         your worktree, never \
         the main checkout. 10. Commit the paths you changed by name, never git commit -a; read HEAD (git log -1) before \
         /code-review or a push. 11. A part answered « moved to <id> » is no longer yours: stop it, push nothing more of \
         it, name its branch or PR in your report (offer « Moved to <task id> », review-<id> for CI). 12. Where your prompt \
         and these rules disagree, the prompt wins; say which you followed. 13. Your task is this brief and dash task show \
         <id>, the live row; a new task goes through dash task add. 14. A failure the Mac causes (a missing library, a \
         tool the sandbox cannot repair), quoted in your report, never holds a reviewer's merge on green CI: merge. 15. \
         One task, one PR: a phase is a commit on it. 16. A behaviour change adds explainer.d/<id>.md (line 1 ## \
         Architecture now), never EXPLAINER.md, or the PR body says « architecture: no change » (a rerun reads the old \
         body). 17. A reversible choice, or an irreversible one nobody would dispute, is yours: decide and say why, never \
         TASK_BLOCKED on it; only an irreversible, controversial one waits on me. 18. The store is SQLite: write through \
         dash, never private/store. 19. Code goes where the architecture doc places it (rust/ARCHITECTURE.md section \
         2: a feature in its plugin crate, no new module in crates/dash beyond its wiring line, no plugin taking \
         another); a core port it needs and lacks is a TASK_BLOCKED naming that port, never code built where it compiles."
    )
}

/// The waiver rule: a part the last message lets go unproven, « waived » or « moved » to a named task, by a dialog answer
/// it quotes. One rule for the brief's words (`words`) and the PM check's verdict on a report (`refusal`, dash's reports.rs),
/// so what the brief offers is what the check takes (task move-names-task; @nha, 2026-10-22: « No double work. »).
pub struct Waiver {
    /// Its verbs, as words: « removed » is no « moved ».
    pub verbs: [&'static str; 2],
    /// The marks around a quoted answer, open and close.
    quotes: [(char, char); 3],
}

/// The one waiver rule (task move-names-task).
pub fn waiver() -> Waiver {
    Waiver {
        verbs: ["waived", "moved"],
        quotes: [('«', '»'), ('“', '”'), ('"', '"')],
    }
}

impl Waiver {
    /// The brief's words, after « names it ».
    pub fn words(&self) -> String {
        let [waived, moved] = self.verbs;
        let (open, close) = self.quotes[0];
        format!(
            "{waived} or {moved} to a named task (its id, or one you filed with dash task add whose --why names this task's \
             id and the part's number) by an answer in the question dialog (his, or the question pass's, which answers a \
             reversible one), the answer quoted as {open} … {close}"
        )
    }

    /// The first verb `clause` says, as a word, or none.
    pub fn verb(&self, clause: &str) -> Option<&'static str> {
        self.verbs.into_iter().find(|v| self.says(clause, v))
    }

    /// Whether `clause` says `verb` as a word.
    pub fn says(&self, clause: &str, verb: &str) -> bool {
        let l = clause.to_lowercase();
        l.split(|c: char| !c.is_alphanumeric()).any(|w| w == verb)
    }

    /// The mark that closes a quote `open` opens, none for a character opening none.
    pub fn closing(&self, open: char) -> Option<char> {
        self.quotes
            .iter()
            .find(|(o, _)| *o == open)
            .map(|(_, c)| *c)
    }

    /// Whether `clause` quotes something between one pair of marks.
    fn quoted(&self, clause: &str) -> bool {
        self.quotes.iter().any(|(open, close)| {
            clause.split_once(*open).is_some_and(|(_, rest)| {
                rest.split_once(*close)
                    .is_some_and(|(said, _)| !said.trim().is_empty())
            })
        })
    }

    /// Why `clause`, saying a part waived or moved, is no waiver: it quotes no dialog answer. None when it says neither verb.
    pub fn refusal(&self, clause: &str) -> Option<String> {
        let verb = self.verb(clause)?;
        let (open, close) = self.quotes[0];
        (!self.quoted(clause)).then(|| {
            format!("is called {verb} with no dialog answer quoted: quote the answer that {verb} it, as {open} … {close}")
        })
    }
}

/// The standing rule `n`'s own words, none past the last: what a refusal quotes, so a renumbered rule shows in its test.
pub fn rule(n: usize) -> Option<String> {
    let rules = standing("main");
    let from = rules.find(&format!(" {n}. "))? + format!(" {n}. ").len();
    let to = (rules[from..].find(&format!(" {}. ", n + 1))).map_or(rules.len(), |at| from + at);
    Some(rules[from..to].trim().to_string())
}

/// Task `t`'s brief. `rules`: the decided rules of the RFCs it cites (`cited`), `(rfc#rule-id, text)`; `branch`: the one it
/// merges and ships on; `repo`: the main checkout, whose dash answers its live row; `writable`: the folders its sandbox
/// profile lets it write (`writable`), none when the profile could not be read.
pub fn brief(
    t: &Filed,
    rules: &[(String, String)],
    branch: &str,
    repo: &str,
    writable: &[String],
) -> String {
    let id = &t.id;
    let prompt = one_line(&t.prompt);
    let lines = [
        comments_line(t),
        kind_line(t),
        epic_line(t),
        t.linear.as_ref().map_or_else(String::new, |k| {
            format!("Your Linear issue is {k}: your PR body says « Fixes {k} ».")
        }),
        rules_line(rules),
        extension_line(&format!("{} {}", t.prompt, t.done_when)),
        standing(branch),
    ];
    let task = std::iter::once(prompt.trim_end_matches('.').to_string() + ".")
        .chain(lines.into_iter().filter(|l| !l.is_empty()))
        .collect::<Vec<_>>()
        .join(" ");
    let done_when = one_line(&t.done_when);
    let outcome = match done_when.trim_end_matches('.') {
        "" => "the outcome the task asks for holds, not only its first step",
        d => d,
    };
    let plan = match t.plan_first {
        true => {
            "Plan first: your first turn is a plan only (the cause, where to fix, how the outcome will be measured), \
             asked for approval in the question dialog; change nothing until it is approved. "
        }
        false => "",
    };
    let turns = t.turns.unwrap_or(TURNS);
    let waiver = waiver().words();
    let live = live_line(repo, writable);
    let focus = t.focus.iter().enumerate();
    let focus = focus.map(|(n, (fid, title))| format!("{}. {title} ({fid}).", n + 1));
    let focus = focus.collect::<Vec<_>>().join(" ");
    let focus = match focus.is_empty() {
        true => String::new(),
        false => format!(" The overall focuses your choices serve (dash focus show): {focus}"),
    };
    format!(
        "{plan}Complete task {id}: {task} Start where it says, with {repo}/private/dashboard/bin/dash task show {id} for the \
         live row. Done when: {outcome}. A part also holds when the last message names it {waiver}, or lists it under \
         After merge in the TASK_DONE block, checkable only from the main checkout after the merge; with none, « waiver: \
         none ». The goal also ends, met, when the last message is a TASK_BLOCKED block naming what only he can do (an \
         approval of a host change included: its PR left for review) or an orchestrator pause. The last message proves each \
         part with the command run and what it printed, or names the part not met and why; a step that moves something \
         without making the outcome hold is not done. {live}The bar, before TASK_DONE: a change that is not trivial, or on a host, \
         process, security, data or money path, runs /code-review medium (name the level: it reuses the last one), each \
         finding fixed or answered, and the checks name it. Ship it the way {PROJECT}'s CLAUDE.md prescribes (never a \
         direct push to the main branch), as a pull request ready for review, pushed as rule 7 \
         says, with an honest assessment (what changed, what was checked, what was not, the risks); never merge it. A \
         pull request in another repository (a fork you vendor) is left for review the same way: the orchestrator's \
         reviewer takes it, so never write that its review or merge is his. Your pull request is titled \
         [fix|feat|chore][{id}] <title> (CI refuses any other). Its body opens with this line: Task: {id} {PAGE}{id} (the \
         task page lists the PR by it). Before the block, write a JSON record (kind done; outcome; checks, a list of \
         command, output as printed, and part, the done-when part it proves by its number, the output quoting that part's \
         own assertion or the lines its command printed, never only a test's PASS line; prs, each a link with merged or ready for \
         review; after_merge as text, not_checked, next, feedback) and run {repo}/private/dashboard/bin/dash task report \
         {id} <that file>: the PM check judges it, never your screen; if the command fails, the block alone stands. End the \
         last message with this block, one field per line: TASK_DONE, task, outcome (met or not met, and why), pr (link, \
         merged or ready for review), checks (each command and its result), After merge (each part only the main checkout \
         can check after the merge, or none), not checked, next, feedback (what cost you turns, or none). A decision you \
         cannot make: ask it in the question dialog. Waiting on him for something not a question (an approval, a login): \
         end with a TASK_BLOCKED block, one field per line: TASK_BLOCKED, task, needs (one line: the verb, the object and \
         where, as « Approve section 23 on /approve », never only a link), link, feedback (and the same record with kind \
         blocked, needs, link, feedback); then wait. Edit files only in a worktree: from the main checkout, git worktree \
         add first, never git switch, a checkout of a branch or a branch move there. Never wait on a CI run or a merge \
         gate: report and stop, the PM reviewer watches CI and merges. Stop after {turns} turns (a turn ends each time you \
         stop; tool calls are not turns) and say what is left.{focus}"
    )
}

/// Where a check on live data reads, and what the session may write (task rust-live-copy): rust-window-in-process and
/// rust-pools-fable each found the live store read-only and built a copy of their own. The folders are its profile's, never
/// a list typed here, so the brief says what the sandbox holds.
fn live_line(repo: &str, writable: &[String]) -> String {
    let copy = format!(
        "A check on live data reads a copy, never the live store, which your sandbox keeps read-only: \
         {repo}/private/live/rust/dash data copy <a folder in your worktree>, then dash <command> --data <that folder>, the \
         command quoted in your report. "
    );
    match writable.is_empty() {
        true => copy,
        false => format!(
            "{copy}Among the project folders, your sandbox profile lets you write {} only; the rest, the live store \
             included, is read-only to you. ",
            writable.join(", ")
        ),
    }
}

/// A Seatbelt profile's words: comments dropped, a `#"…"` regex read as a string, each string as written.
enum Word {
    Open,
    Close,
    Atom(String),
    Str(String),
}

fn words_of(profile: &str) -> Vec<Word> {
    let mut out = vec![];
    let mut chars = profile.chars().peekable();
    let string = |chars: &mut std::iter::Peekable<std::str::Chars>| {
        let mut s = String::new();
        while let Some(c) = chars.next() {
            match c {
                '"' => break,
                '\\' => s.extend([Some(c), chars.next()].into_iter().flatten()),
                c => s.push(c),
            }
        }
        Word::Str(s)
    };
    while let Some(c) = chars.next() {
        match c {
            ';' => while chars.next_if(|&c| c != '\n').is_some() {},
            '(' => out.push(Word::Open),
            ')' => out.push(Word::Close),
            '"' => out.push(string(&mut chars)),
            '#' if chars.next_if_eq(&'"').is_some() => out.push(string(&mut chars)),
            c if c.is_whitespace() => {}
            c => {
                let mut atom = c.to_string();
                while let Some(c) = chars.next_if(|c| !c.is_whitespace() && !"()\";".contains(*c)) {
                    atom.push(c);
                }
                out.push(Word::Atom(atom));
            }
        }
    }
    out
}

/// The folders a sandbox profile lets a session write, as `sandbox-apply profile claude` prints it: each path an `allow` of
/// a file-write operation names, its home paths read through the profile's own `HOME_DIR` and shown as ~, kept when it lies
/// under one of `roots` or holds one (the projects' folder, the worktrees' folder: the toolchains' caches say nothing to a
/// task). A regex grant reads as a glob, `[^/]+` a `*`. Sorted, each once.
pub fn writable(profile: &str, roots: &[&str]) -> Vec<String> {
    let words = words_of(profile);
    let home = words.windows(4).find_map(|w| match w {
        [Word::Atom(d), Word::Atom(h), Word::Str(s), Word::Close]
            if d == "define" && h == "HOME_DIR" =>
        {
            Some(s.clone())
        }
        _ => None,
    });
    let home = home.unwrap_or_default();
    let mut forms = vec![];
    let (mut depth, mut start) = (0, 0);
    for (i, w) in words.iter().enumerate() {
        match w {
            Word::Open if depth == 0 => (depth, start) = (1, i),
            Word::Open => depth += 1,
            Word::Close => {
                depth -= 1;
                if depth == 0 {
                    forms.push(&words[start..=i]);
                }
            }
            _ => {}
        }
    }
    let mut found = vec![];
    for form in forms {
        let ops = form.iter().skip(1).map_while(|w| match w {
            Word::Atom(a) => Some(a.as_str()),
            _ => None,
        });
        let ops: Vec<&str> = ops.collect();
        // a write is creating or writing a file, never only its mode, xattrs or unlink; a form with a require-not filter
        // grants all but some path, so its paths would read inverted: it is left out
        let writes = ["file-write*", "file-write-create", "file-write-data"];
        let negated = form
            .iter()
            .any(|w| matches!(w, Word::Atom(a) if a == "require-not"));
        if ops.first() != Some(&"allow") || !ops.iter().any(|o| writes.contains(o)) || negated {
            continue;
        }
        for w in form.windows(4) {
            let [Word::Open, Word::Atom(kind), Word::Str(path), Word::Close] = w else {
                continue;
            };
            let (path, head) = match kind.as_str() {
                "subpath" | "literal" | "prefix" => (path.clone(), path.clone()),
                "home-subpath" | "home-literal" | "home-prefix" => {
                    (format!("{home}{path}"), format!("{home}{path}"))
                }
                // its literal start: up to the first character a regex reads otherwise, then back to the last whole folder
                "regex" => {
                    let start = path.trim_start_matches('^').trim_end_matches('$');
                    let start = start
                        .strip_suffix("(/.*)?")
                        .unwrap_or(start)
                        .replace("\\.", ".");
                    let plain = start
                        .split(|c| "[(|+?*$\\".contains(c))
                        .next()
                        .unwrap_or_default();
                    let head = match plain.len() == start.len() {
                        true => plain,
                        false => plain.rsplit_once('/').map_or(plain, |p| p.0),
                    };
                    (glob(path), head.to_string())
                }
                _ => continue,
            };
            let head = head.trim_end_matches('/');
            let near = |r: &&str| {
                let under = |a: &str, b: &str| a == b || a.starts_with(&format!("{b}/"));
                under(head, r) || under(r, head)
            };
            if !head.is_empty() && roots.iter().any(near) {
                found.push(match path.strip_prefix(&home) {
                    Some(rest) if !home.is_empty() && rest.starts_with('/') => format!("~{rest}"),
                    _ => path,
                });
            }
        }
    }
    found.sort();
    found.dedup();
    found
}

/// A Seatbelt path regex as a glob: `^/a/[^/]+/b(/.*)?$` is `/a/*/b`. Any other regex stays as written, `#"…"`, never
/// half read as a path.
fn glob(regex: &str) -> String {
    let r = regex.trim_start_matches('^').trim_end_matches('$');
    let r = r.strip_suffix("(/.*)?").unwrap_or(r);
    let r = r.replace("[^/]+", "*").replace("\\.", ".");
    match r.contains(['[', '(', '|', '+', '?', '\\']) || r.contains(".*") {
        true => format!("#\"{regex}\""),
        false => r,
    }
}

/// A text on one line: the launcher types it, where a newline is Enter.
fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// His comments still open on the task (`Dashboard.Watch.brief/1`): what each asks is done or answered, under it.
fn comments_line(t: &Filed) -> String {
    if t.comments.is_empty() {
        return String::new();
    }
    let one = |c: &dash_core::tasks::Comment| {
        let quote = match one_line(&c.quote) {
            q if q.is_empty() => "the whole page".to_string(),
            q if q.chars().count() > 120 => q.chars().take(119).collect::<String>() + "…",
            q => q,
        };
        format!(
            "comment {} on {} (« {quote} »): « {} ».",
            c.id,
            c.file,
            one_line(&c.text)
        )
    };
    let each = t.comments.iter().map(one).collect::<Vec<_>>().join(" ");
    format!(
        "His comments on your work: {each} Do what each asks or answer it, then answer each under it, on his app: \
         DASHBOARD_URL=http://127.0.0.1:7788 private/dashboard/bin/dash comment answer <comment id> '<what you did or why \
         not>', with a link to each task, step, PR or page it names (@nha, 2026-10-22: « In general comments are good with \
         links »). Your stop is not met while one is unanswered."
    )
}

/// The words of `text`, lowercased, split at anything but a letter or digit.
fn words(text: &str) -> Vec<String> {
    let text = text.to_lowercase();
    let words = text.split(|c: char| !c.is_alphanumeric());
    words.filter(|w| !w.is_empty()).map(String::from).collect()
}

/// A drafting task reads its recipient's context first (task draft-recipient-context; @nha, 2026-10-21: « remember the
/// context of paddle and their conditions »): one of kind writing, or one whose prompt names a draft. A task that fills a form
/// in his Chrome keeps what he typed (task form-fill-keeps-his; @nha, 2026-10-23: « you seems to have discarded my pre
/// filled answers. Mix please. »). ponytail: a form is a word « form » beside one of filling it, never within 40 characters
/// as the older app's pattern read it; an extra sentence is what a looser match costs.
fn kind_line(t: &Filed) -> String {
    let drafts = words(&t.prompt)
        .iter()
        .any(|w| w == "draft" || w == "drafts");
    let writing = match t.kind == "writing" || drafts {
        true => {
            "Before writing or reviewing a draft, run dash draft context <draft id or address>, and cite its contact card or \
             a document it names in the draft's :why (dash check warns otherwise). A person is they/them, or what their \
             card's :pronouns says; never a guess from the name (dash check fails it)."
        }
        false => "",
    };
    let said = words(&format!("{} {} {}", t.prompt, t.title, t.done_when));
    let has = |ws: &[&str]| said.iter().any(|w| ws.contains(&w.as_str()));
    let filling = [
        "fill",
        "filled",
        "filling",
        "apply",
        "application",
        "submit",
        "screening",
        "signup",
        "chrome",
    ];
    let form = has(&["formulaire", "formulaires", "typeform"])
        || (has(&["form", "forms"]) && has(&filling))
        || said.windows(2).any(|w| w[0] == "his" && w[1] == "chrome");
    let form = match form {
        true => {
            "Fill any form in his Chrome through the browser-apply skill's fill-merge.js: his answers stay, ours wait \
             beside them for his choice."
        }
        false => "",
    };
    [writing, form]
        .iter()
        .filter(|l| !l.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join(" ")
}

/// An epic replicate task's standing orders (task replicate-brief-orders; @nha, 2026-10-24: « make sure it is examplary
/// code. Run /simplify and /ponytail:ponytail on every PR. »).
fn epic_line(t: &Filed) -> String {
    match t.epic == "replicate" {
        true => "Epic replicate: run /simplify and /ponytail:ponytail on the diff; those two passes leave each file the judge \
                 scores (SLICES[slice].scored in harness/judge.ts) untouched. Fold any choice that differs from \
                 private/docs/rfc-originator-takehome.html into its section, the reason in its writer notes. Write outputs \
                 only in your worktree (its own private/live): the orchestrator moves them. A doc reaches the default branch \
                 through your PR, never dash ship."
            .to_string(),
        false => String::new(),
    }
}

/// A task whose words touch a browser extension tests it on Chrome for Testing, never his Chrome, which holds his sign-ins
/// (page-comments-extension).
fn extension_line(text: &str) -> String {
    let w = words(text);
    let pair = |a: &[&str], b: &[&str]| {
        w.windows(2)
            .any(|p| a.contains(&p[0].as_str()) && b.contains(&p[1].as_str()))
    };
    let ext = ["extension", "extensions"];
    match pair(&["browser", "chrome", "web"], &ext) || pair(&ext, &["chrome", "navigateur"]) {
        true => "This touches a browser extension: test it in Chrome for Testing against a worktree app, never his Chrome."
            .to_string(),
        false => String::new(),
    }
}

/// His decided rules the task's work holds, verbatim (task design-rules-held; @nha, 2026-10-30: « I propose a design. It is
/// dropped. Then the implementation fails. Multiple times. … How can this be achieved ? »).
fn rules_line(rules: &[(String, String)]) -> String {
    if rules.is_empty() {
        return String::new();
    }
    let each = rules.iter().map(|(at, text)| format!("{at}: {text}"));
    format!(
        "His decided rules, which this work holds, verbatim (each held by the test tagged rule: \"<rfc>#<id>\"; never bend \
         or drop one without his word, quoted): {}",
        each.collect::<Vec<_>>().join(" | ")
    )
}

/// The RFC pages task `t` cites, `rfc-<name>`: in its title, why, done-when or prompt, then its epic's, as
/// `Dashboard.DesignRules.cited/1` reads them. A module's own docs no longer cite one: Rust's modules have no such docs read.
pub fn cited(t: &Filed) -> Vec<String> {
    let text = format!("{} {} {} {}", t.title, t.why, t.done_when, t.prompt);
    let tokens = text.split(|c: char| !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'));
    let named = tokens.filter_map(|w| {
        let w = w.trim_end_matches('-');
        (w.starts_with("rfc-") && w.len() > 4).then(|| w.to_string())
    });
    // the one epic whose RFC has another name (`Dashboard.Labels`' @rfc_epics)
    let epic = match t.epic.as_str() {
        "" => None,
        "board" => Some("rfc-project-boards".to_string()),
        e => Some(format!("rfc-{e}")),
    };
    let mut all: Vec<String> = vec![];
    for name in named.chain(epic) {
        if !all.contains(&name) {
            all.push(name);
        }
    }
    all
}

/// The decided rules of an RFC page, `(rule-id, text)`: each list item with an id `rule-…`, its text as he reads it, tags
/// stripped, the lists nested in it included (`Dashboard.DesignRules.of/1`).
pub fn rules_of(html: &str) -> Vec<(String, String)> {
    let mut found = vec![];
    let mut at = 0;
    while let Some(i) = html[at..].find("<li") {
        let start = at + i;
        at = start + 3;
        let Some(end) = html[start..].find('>').map(|e| start + e) else {
            break;
        };
        let tag = &html[start..end];
        let is_li = tag[3..].chars().next().is_none_or(char::is_whitespace);
        // after any blank, as the older app's \bid= read it: an attribute wrapped to its own line too
        let mark = "id=\"rule-";
        let at = tag
            .match_indices(mark)
            .find(|(i, _)| tag[..*i].ends_with(char::is_whitespace));
        let id = at.and_then(|(i, _)| tag[i + mark.len()..].split_once('"'));
        if let (true, Some((id, _))) = (is_li, id) {
            found.push((format!("rule-{id}"), text(item(&html[start..]))));
        }
    }
    found
}

/// What section `id` of an RFC page decides, tags stripped, on one line: its `<section id="…">` up to its own `</section>`, the
/// sections nested in it included; a heading's id reads up to the next heading or section. Its `<details>` (writer notes, the
/// asides shut by default) and `<figure>`s are left out: the decision is the body he reads open. `None` when nothing has that
/// id. What the RFC judge holds a diff to (task rust-task-rfc-judge: the whole section, notes and all, scored 3 of 10 breaches
/// over the bar).
pub fn section_text(html: &str, id: &str) -> Option<String> {
    let mark = format!(" id=\"{id}\"");
    let at = html.match_indices(&mark).find_map(|(i, _)| {
        let start = html[..i].rfind('<')?;
        let tag = html[start + 1..i]
            .split_whitespace()
            .next()
            .unwrap_or_default();
        (!html[start..i].contains('>')).then_some((start, tag))
    })?;
    let (start, tag) = at;
    let body = &html[start..];
    if tag != "section" {
        let open = body.find('>')? + 1;
        let end = ["<h2", "<h3", "<h4", "<section", "</section"]
            .iter()
            .filter_map(|t| body[open..].find(t))
            .min()
            .map_or(body.len(), |e| open + e);
        return Some(text(&shut(&body[..end])));
    }
    let mut depth = 0;
    for (i, _) in body.match_indices("section") {
        match &body[..i] {
            b if b.ends_with("</") => depth -= 1,
            b if b.ends_with('<') => depth += 1,
            _ => continue,
        }
        if depth == 0 {
            return Some(text(&shut(&body[..i + "section>".len()])));
        }
    }
    Some(text(&shut(body)))
}

/// `html` with each `<details>` and `<figure>` cut out whole, those nested in it included.
fn shut(html: &str) -> String {
    let mut out = html.to_string();
    for tag in ["details", "figure"] {
        let (open, close) = (format!("<{tag}"), format!("</{tag}>"));
        while let Some(start) = out.find(&open) {
            let (mut depth, mut at) = (0, start);
            let end = loop {
                let next_open = out[at + 1..].find(&open).map(|i| at + 1 + i);
                let Some(next_close) = out[at..].find(&close).map(|i| at + i) else {
                    break out.len();
                };
                match next_open.filter(|o| *o < next_close) {
                    Some(o) => (depth, at) = (depth + 1, o),
                    None if depth == 0 => break next_close + close.len(),
                    None => (depth, at) = (depth - 1, next_close + 1),
                }
            };
            out.replace_range(start..end, " ");
        }
    }
    out
}

/// A list item up to its own `</li>`, the items nested in it included; the rest of the page when it never closes.
fn item(rest: &str) -> &str {
    let mut depth = 0;
    let mut at = 0;
    while at < rest.len() {
        let here = &rest[at..];
        if here.starts_with("</li>") {
            depth -= 1;
            if depth == 0 {
                return &rest[..at + 5];
            }
            at += 5;
        } else if here.starts_with("<li")
            && here[3..].starts_with(|c: char| c == '>' || c.is_whitespace())
        {
            depth += 1;
            at += 3;
        } else {
            at += here.chars().next().map_or(1, char::len_utf8);
        }
    }
    rest
}

/// An item's text: a block tag is a space, any other tag nothing, the entities read, the blanks one space.
fn text(html: &str) -> String {
    let mut out = String::new();
    let mut rest = html;
    while let Some(i) = rest.find('<') {
        out.push_str(&rest[..i]);
        let Some(end) = rest[i..].find('>') else {
            rest = &rest[i..];
            break;
        };
        let name = rest[i + 1..i + end].trim_start_matches('/');
        let name = name
            .split(|c: char| !c.is_ascii_alphanumeric())
            .next()
            .unwrap_or("");
        let block = [
            "li", "ol", "ul", "p", "br", "div", "td", "th", "tr", "h2", "h3", "h4", "pre",
        ];
        if block.contains(&name) {
            out.push(' ');
        }
        rest = &rest[i + end + 1..];
    }
    out.push_str(rest);
    let entities = [
        ("&lt;", "<"),
        ("&gt;", ">"),
        ("&quot;", "\""),
        ("&#39;", "'"),
        ("&nbsp;", " "),
    ];
    let out = entities
        .iter()
        .fold(out, |s, (e, c)| s.replace(e, c))
        .replace("&amp;", "&");
    one_line(&out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_core::tasks::Comment;
    use dash_meter::dash_test;

    fn task() -> Filed {
        Filed {
            id: "a-task".into(),
            title: "A title".into(),
            done_when: "1. It holds: a test\n 2. It shows".into(),
            prompt: "Build this task's :done-when.".into(),
            focus: vec![("rust-poc".into(), "The Rust tool builds itself".into())],
            ..Filed::default()
        }
    }

    /// Done-when 1 of rust-brief-merge-push-order: rule 7 pushes the merge of main before it reads mergeable, since a
    /// conflicting PR turns MERGEABLE only once that merge is pushed (outbound-gate waited on it, 2026-11-12).
    #[dash_test]
    fn rule_7_pushes_the_merge_then_reads_mergeable() {
        let rules = standing("main");
        let seven = &rules[rules.find(" 7. ").unwrap()..rules.find(" 8. ").unwrap()];
        let push = seven
            .find("Push your commits with your merge of main")
            .expect(seven);
        let read = seven
            .find("then read gh pr view --json mergeable once")
            .expect(seven);
        let again = seven
            .find("CONFLICTING, merge main again and push")
            .expect(seven);
        assert!(push < read && read < again, "{seven}");
        assert!(!seven.contains("Push once"), "{seven}");
        let brief = brief(&task(), &[], "main", "/r", &[]);
        assert!(
            brief.contains("ready for review, pushed as rule 7 says"),
            "{brief}"
        );
        assert!(!brief.contains("pushed once gh pr view"), "{brief}");
    }

    /// Done-when 1 of brief-tests-prepush-skip: the standing rules come from Rust, nineteen in order, and rule 2 asks each
    /// done-when part's one proving test.
    #[dash_test]
    fn rule_2_asks_each_part_its_one_proving_test() {
        let rules = standing("main");
        let numbers: Vec<usize> = (1..=19)
            .filter_map(|n| rules.find(&format!(" {n}. ")))
            .collect();
        assert_eq!(numbers.len(), 19, "{rules}");
        assert!(
            numbers.windows(2).all(|p| p[0] < p[1]),
            "out of order: {rules}"
        );
        let two = &rules[numbers[1]..numbers[2]];
        assert!(
            two.contains("Each done-when part has its one proving test"),
            "{two}"
        );
        assert!(two.contains("merged with main"), "{two}");
        assert!(!rules.contains("{b}"), "{rules}");
        // task report-proof-quotes: the record asks each part's quote before dash task report bounces its absence
        let brief = brief(&task(), &[], "main", "/r", &[]);
        assert!(
            brief.contains("the output quoting that part's own assertion or the lines its command printed, never only a test's PASS line"),
            "{brief}"
        );
    }

    /// Done-when 1 of brief-placement-blocks (@nha, 2026-11-14: « there needs to be some questioning sometimes instead of
    /// blind execution. »): every brief's rule 19 sends code where the architecture doc places it, and a missing core port to
    /// a TASK_BLOCKED naming it, never to crates/dash because a Page lives there.
    #[dash_test]
    fn rule_19_places_code_by_the_architecture_doc_and_blocks_on_a_missing_port() {
        let nineteen = rule(19).expect("rule 19");
        for said in [
            "Code goes where the architecture doc places it (rust/ARCHITECTURE.md section 2: a feature in its plugin crate",
            "no new module in crates/dash beyond its wiring line, no plugin taking another",
            "a core port it needs and lacks is a TASK_BLOCKED naming that port, never code built where it compiles",
        ] {
            assert!(nineteen.contains(said), "{nineteen}");
        }
        assert!(brief(&task(), &[], "main", "/r", &[]).contains(&nineteen));
    }

    /// Done-when 2 of move-names-task: the brief's waiver words are the waiver rule's, the one the PM check judges by.
    #[dash_test]
    fn the_brief_offers_the_waiver_the_check_takes() {
        let b = brief(&task(), &[], "main", "/r", &[]);
        let words = waiver().words();
        assert!(
            b.contains(&format!("A part also holds when the last message names it {words}, or lists it under After merge")),
            "{b}"
        );
        assert!(
            words.starts_with("waived or moved to a named task")
                && words.ends_with("the answer quoted as « … »"),
            "{words}"
        );
    }

    /// Done-when 2 of move-names-task: the waiver rule refuses a waiver or a move quoting no dialog answer.
    #[dash_test]
    fn a_waiver_quoting_no_dialog_answer_is_refused() {
        let w = waiver();
        assert_eq!(
            w.refusal("part 2 waived").as_deref(),
            Some(
                "is called waived with no dialog answer quoted: quote the answer that waived it, as « … »"
            )
        );
        assert!(w.refusal("part 2 moved to x by his answer").is_some());
        assert!(w.refusal("part 2 moved by « »").is_some(), "an empty quote");
        for held in [
            "part 2 waived by his answer « fine »",
            "part 2 moved to x by the answer “Move to x”",
            r#"part 2 moved to x by the answer "Move to x""#,
        ] {
            assert_eq!(w.refusal(held), None, "{held}");
        }
        // no verb, no waiver: « removed » is no « moved »
        assert_eq!(w.refusal("part 2 removed the old hook"), None);
        assert_eq!(w.verb("part 2 removed the old hook"), None);
    }

    /// Done-when 1: the whole brief, its prompt, rules, done-when, ship and block lines, as the older app's launch wrote it.
    #[dash_test]
    fn a_brief_holds_its_task_rules_outcome_and_focus() {
        let b = brief(&task(), &[], "main", "/repo", &[]);
        assert!(
            b.starts_with(
                "Complete task a-task: Build this task's :done-when. Rules: 1. Ask me only"
            ),
            "{b}"
        );
        assert!(b.contains("never code built where it compiles. Start where it says, with /repo/private/dashboard/bin/dash task show a-task"));
        assert!(
            b.contains("Done when: 1. It holds: a test 2. It shows. A part also holds"),
            "{b}"
        );
        assert!(
            b.contains("Task: a-task http://100.72.158.115:7788/projects/site_perso?issue=a-task"),
            "{b}"
        );
        assert!(b.contains("[fix|feat|chore][a-task]"), "{b}");
        assert!(b.contains("Stop after 40 turns"), "{b}");
        assert!(
            b.ends_with("serve (dash focus show): 1. The Rust tool builds itself (rust-poc)."),
            "{b}"
        );
        assert!(!b.contains('\n') && !b.contains(".."), "{b}");
        for absent in [
            "Linear",
            "His comments",
            "Plan first",
            "dash draft context",
            "browser extension",
            "decided rules",
        ] {
            assert!(!b.contains(absent), "{absent}: {b}");
        }
    }

    /// Done-when 1: each line a row asks for, ported from `Launch.argv/2`'s.
    #[dash_test]
    fn each_line_comes_with_what_asks_for_it() {
        let t = Filed {
            kind: "writing".into(),
            epic: "replicate".into(),
            plan_first: true,
            turns: Some(12),
            linear: Some("AAA-1".into()),
            prompt: "Fill the application form in his Chrome; the chrome extension too.".into(),
            comments: vec![Comment {
                id: "c1".into(),
                file: "/tasks".into(),
                quote: String::new(),
                text: "Why\nthis?".into(),
            }],
            ..task()
        };
        let rules = [("rfc-x#rule-1".to_string(), "Never twice.".to_string())];
        let b = brief(&t, &rules, "main", "/repo", &[]);
        let said = [
            "Plan first: your first turn is a plan only",
            "comment c1 on /tasks (« the whole page »): « Why this? ».",
            "dash draft context",
            "browser-apply skill's fill-merge.js",
            "Epic replicate:",
            "Your Linear issue is AAA-1: your PR body says « Fixes AAA-1 ».",
            "verbatim (each held by the test tagged rule: \"<rfc>#<id>\"; never bend or drop one without his word, quoted): rfc-x#rule-1: Never twice.",
            "This touches a browser extension",
            "Stop after 12 turns",
        ];
        for s in said {
            assert!(b.contains(s), "{s}: {b}");
        }
        assert!(b.starts_with("Plan first:"), "{b}");
        let drafts = Filed {
            prompt: "Send the drafts out.".into(),
            ..task()
        };
        assert!(brief(&drafts, &[], "main", "/r", &[]).contains("dash draft context"));
        let form = Filed {
            prompt: "Build the form page.".into(),
            ..task()
        };
        assert!(!brief(&form, &[], "main", "/r", &[]).contains("fill-merge.js"));
    }

    /// A profile as `sandbox-apply profile claude` prints it, cut to the forms that matter: its home, a comment, reads, a
    /// deny, caches, temp and the grants under the projects' and worktrees' folders.
    const PROFILE: &str = r#"(version 1)
;; (allow file-write* (subpath "/Users/me/repo/in-a-comment"))
(define HOME_DIR "/Users/me")
(define (home-subpath rel) (subpath (string-append HOME_DIR rel)))
(allow file-read* (home-subpath "/repo"))
(allow file-read* file-write* (subpath "/tmp") (home-subpath "/.cargo"))
(allow file-read* file-write* (subpath "/Users/me/.herdr/worktrees/site_perso/a-task"))
(allow file-write* (regex #"^/Users/me/repo/[^/]+/to_process(/.*)?$"))
(allow file-write*
    (home-subpath "/repo/klar/backlog/tasks") ;; another board's tasks
    (home-subpath "/.herdr/worktrees"))
(allow file-read* file-write* (subpath "/Users/me/repo/site_perso/.git"))
(deny file-write* (literal "/Users/me/.herdr/worktrees/site_perso/a-task/sandbox.sb"))
(allow file-write* (require-all (subpath "/Users/me/repo") (require-not (subpath "/Users/me/repo/site_perso/private/live"))))
(allow file-write-xattr (subpath "/Users/me/repo/site_perso"))
(allow file-write-unlink (regex #"^/Users/me/repo/[^/]+/\.git/worktrees(/.*)?$"))
(allow file-write* (regex #"^/Users/me/repo/[^/]+/\.cache/.*\.lock$") (regex #"^/Users/me/\.cache/x$"))"#;

    /// Done-when 4 of rust-live-copy: the folders the session may write are its profile's write grants under the projects'
    /// and worktrees' folders, read from the profile: never the comment's, a read's, a deny's, a cache's or temp, nor a
    /// require-not's (inverted), an xattr's or a removal's; a regex no glob reads stays a regex.
    #[dash_test]
    fn the_brief_lists_the_folders_its_profile_lets_it_write() {
        let roots = ["/Users/me/repo", "/Users/me/.herdr/worktrees/site_perso"];
        let folders = writable(PROFILE, &roots);
        assert_eq!(
            folders,
            [
                "#\"^/Users/me/repo/[^/]+/\\.cache/.*\\.lock$\"",
                "~/.herdr/worktrees",
                "~/.herdr/worktrees/site_perso/a-task",
                "~/repo/*/to_process",
                "~/repo/klar/backlog/tasks",
                "~/repo/site_perso/.git",
            ]
        );
        let b = brief(&task(), &[], "main", "/r", &folders);
        assert!(
            b.contains(
                "Among the project folders, your sandbox profile lets you write \
                 #\"^/Users/me/repo/[^/]+/\\.cache/.*\\.lock$\", ~/.herdr/worktrees, \
                 ~/.herdr/worktrees/site_perso/a-task, ~/repo/*/to_process, ~/repo/klar/backlog/tasks, \
                 ~/repo/site_perso/.git only; the rest, the live store included, is read-only to you. The bar"
            ),
            "{b}"
        );
        let other = PROFILE.replace("/Users/me", "/Users/other");
        let moved = writable(&other, &["/Users/other/repo"]);
        assert!(
            moved.contains(&"~/repo/klar/backlog/tasks".to_string()),
            "{moved:?}"
        );
        assert!(!brief(&task(), &[], "main", "/r", &[]).contains("lets you write"));
    }

    /// Done-when 2 of rust-live-copy: the checks name dash data copy for any check on live data, with the --data it gives.
    #[dash_test]
    fn the_checks_name_the_live_copy_for_a_live_check() {
        let b = brief(&task(), &[], "main", "/repo", &[]);
        assert!(
            b.contains(
                "without making the outcome hold is not done. A check on live data reads a copy, never the live store, \
                 which your sandbox keeps read-only: /repo/private/live/rust/dash data copy <a folder in your worktree>, \
                 then dash <command> --data <that folder>, the command quoted in your report. The bar, before TASK_DONE"
            ),
            "{b}"
        );
    }

    #[dash_test]
    fn a_task_cites_the_rfcs_its_words_and_epic_name() {
        let t = Filed {
            title: "Per rfc-rust-unified, section 3".into(),
            why: "rfc-rust-unified again, and (rfc-store-format-).".into(),
            epic: "board".into(),
            ..task()
        };
        assert_eq!(
            cited(&t),
            ["rfc-rust-unified", "rfc-store-format", "rfc-project-boards"]
        );
        assert!(cited(&task()).is_empty());
    }

    #[dash_test]
    fn an_rfc_s_rules_are_its_rule_items_text_only() {
        let html = r#"<ul><li>not one</li><li class="r"
            id="rule-8-1">Keep <b>one</b> &amp; only
            <ul><li>a nested <code>&lt;x&gt;</code></li></ul> done.</li><li id="rule-9">Last<br>line</li></ul>
            <link id="rule-no">"#;
        assert_eq!(
            rules_of(html),
            [
                (
                    "rule-8-1".to_string(),
                    "Keep one & only a nested <x> done.".to_string()
                ),
                ("rule-9".to_string(), "Last line".to_string()),
            ]
        );
    }

    /// The RFC judge's section: a section whole, the one nested in it included and the next left out, its writer notes, asides
    /// and figures cut; a heading's id up to the next heading; an id nobody holds, none (task rust-task-rfc-judge).
    #[dash_test]
    fn a_section_reads_up_to_its_own_end() {
        let html = "<section id=\"ui-lib\"><h2>10. UI lib</h2><details class=\"wn\"><p>Notes.</p><details>In.</details><p>Note \
                    after.</p></details>\
                    <p>Behaviour is Rust.</p><figure><svg></svg></figure><details><summary>Aside</summary></details>\
                    <section id=\"inner\"><p>Nested.</p></section><table><tr><td>a</td><td>b</td></tr></table></section>\
                    <section id=\"next\"><p>Other.</p></section><h3 id=\"h\">Head</h3><p>Under it.</p><h3>Then</h3>";
        assert_eq!(
            section_text(html, "ui-lib").as_deref(),
            Some("10. UI lib Behaviour is Rust. Nested. a b")
        );
        assert_eq!(section_text(html, "h").as_deref(), Some("Head Under it."));
        let classed = "<section class=\"x\" id=\"s\"><h2>S</h2><h3>Part</h3><p>Body.</p></section>";
        assert_eq!(section_text(classed, "s").as_deref(), Some("S Part Body."));
        assert_eq!(section_text(html, "nobody"), None);
    }
}
