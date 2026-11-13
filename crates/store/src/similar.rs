//! How close two tasks are, `dash task add`'s duplicate check and the start plan's file hold (task task-dedupe; @nha, 2026-11-03:
//! « this is either double work or stepping in the exact same area in parallel »; 2026-11-11: « something like the result of
//! filing a new task returns similar tasks », « this could be used to detect dupes or to add dependencies/or just related
//! tasks »). The older app's `TaskStyle.overlaps/3` ported, same terms and floors, so its own refusal and this one name the
//! same tasks while both run: it retires with `Dashboard.Records`' close check.
//! ponytail: bag of words, no stemming nor synonyms; an embedding when two near tasks share no word.

use std::collections::{BTreeMap, BTreeSet};

/// The closest task is close from this score, the others from `FLOOR`.
pub const BEST_FLOOR: f64 = 0.1;
pub const FLOOR: f64 = 0.15;
/// Under this many sets every term weighs 1: a shared term is in two by definition, so the IDF would weigh it under the rest.
const IDF_MIN: usize = 20;
/// Below the close floors, a task sharing this much is related: printed, never refused.
pub const RELATED_FLOOR: f64 = 0.05;

/// English stop words of four letters or more, and the words the task-writing skill makes every task use.
const STOPWORDS: &str = "this that with from into when each what their there they them have been were will than then also \
    only once same task tasks done open never every none just like make made does about above again against because before \
    being below between both could doing down during further having here hers herself himself itself more most myself other \
    ours ourselves over should some such theirs themselves these those through under until very which while whom would your \
    yours yourself must after merge merged dash test tests check checks name named names case cases prints print shows says \
    list lists live main command commands file files tested testing checked printed showing shown merges merging";

/// A task as the check reads it: its id and the three fields its words come from.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Text {
    pub id: String,
    pub title: String,
    pub why: String,
    pub done_when: String,
}

/// A task close to another: its score and the terms they share, sorted.
#[derive(Clone, Debug, PartialEq)]
pub struct Near {
    pub id: String,
    pub score: f64,
    pub shared: Vec<String>,
}

fn word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

fn run_char(c: char) -> bool {
    word_char(c) || matches!(c, '.' | '-' | '/')
}

/// The file paths `text` names, as `TaskStyle.paths/1` reads them: a run of folders and a name ending in an extension, or a
/// file of a `bin/` or `commands/` folder; never one after « new », a host's (`docs.fly.io/x.md`), nor one leaving the repo.
/// ponytail: a scan, not its regex; a path glued to `<>*{}` is no path, where the regex backtracks to a shorter one first.
pub fn paths(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut found: Vec<String> = vec![];
    let mut at = 0;
    while at < chars.len() {
        if !run_char(chars[at]) {
            at += 1;
            continue;
        }
        let start = at;
        while at < chars.len() && run_char(chars[at]) {
            at += 1;
        }
        // the few chars before it only, never a copy of all the text before (/code-review: quadratic on a long why)
        let before = &chars[..start];
        let after_new = ["new ", "New ", "new `", "New `"].iter().any(|n| {
            let n: Vec<char> = n.chars().collect();
            before.ends_with(&n)
        });
        let glued = start > 0 && matches!(chars[start - 1], ':' | '~' | '*' | '$');
        let next_bad = chars.get(at).is_some_and(|c| "<>*{}".contains(*c));
        let run: String = chars[start..at].iter().collect();
        let path = run.trim_end_matches(['.', '/']);
        let ok = !after_new
            && !glued
            && !next_bad
            && !run.starts_with('/')
            && !run[path.len()..].starts_with('/')
            && path.contains('/')
            && !path.contains("//")
            && path.ends_with(|c: char| word_char(c) || c == '-');
        if !ok {
            continue;
        }
        let name = path.rsplit('/').next().unwrap_or_default();
        let ext = name
            .rsplit_once('.')
            .is_some_and(|(_, e)| e.starts_with(|c: char| c.is_ascii_alphabetic()));
        let folder = path.split('/').rev().nth(1);
        let tool = matches!(folder, Some("bin" | "commands")) && !name.is_empty();
        let first = path.split('/').next().unwrap_or_default();
        let host = first.contains('.') && first.split('.').all(|p| !p.is_empty());
        if (ext || tool) && !host && !path.starts_with("../") && !found.iter().any(|f| f == path) {
            found.push(path.to_string());
        }
    }
    found
}

/// Its text with his quotes left out, « … » nested or not: tasks citing one quote share no subject.
fn unquoted(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let (mut open, mut spans) = (vec![], vec![]);
    for (i, c) in chars.iter().enumerate() {
        match c {
            '«' => open.push(i),
            '»' => {
                if let Some(o) = open.pop() {
                    spans.push((o, i));
                }
            }
            _ => {}
        }
    }
    let inside = |i: usize| spans.iter().any(|(o, c)| (*o..=*c).contains(&i));
    (chars.iter().enumerate())
        .map(|(i, c)| if inside(i) { ' ' } else { *c })
        .collect()
}

/// The files task `t`'s title, why and done-when name, a file of the app as the repo's, leaving out what a task cites
/// without changing it: the dash command and the docs it reads (/code-review: a task citing rfc-rust-unified.html held every
/// other one citing it).
pub fn files_named(t: &Text) -> Vec<String> {
    let all = paths(&format!("{} {} {}", t.title, t.why, t.done_when));
    let mut out: Vec<String> = vec![];
    for p in all {
        let f = p
            .strip_prefix("private/dashboard/")
            .unwrap_or(&p)
            .to_string();
        if f != "bin/dash" && !f.starts_with("private/docs/") && !out.contains(&f) {
            out.push(f);
        }
    }
    out
}

/// Its terms: the words of four letters or more, stop words out, and the files it names, a file of the app as the repo's.
fn terms(t: &Text) -> BTreeSet<String> {
    let text = unquoted(&format!("{} {} {}", t.title, t.why, t.done_when));
    let mut ps = paths(&text);
    // longest first: a short path inside a long one would leave the long one's folders behind
    ps.sort_by_key(|p| std::cmp::Reverse(p.len()));
    let bare = ps.iter().fold(text.to_lowercase(), |acc, p| {
        acc.replace(&p.to_lowercase(), " ")
    });
    let stop: BTreeSet<&str> = STOPWORDS.split_whitespace().collect();
    let words = bare
        .split(|c: char| !c.is_alphabetic())
        .filter(|w| w.chars().count() >= 4 && !stop.contains(w))
        .map(String::from);
    let files = (ps.iter())
        .map(|p| {
            p.strip_prefix("private/dashboard/")
                .unwrap_or(p)
                .to_string()
        })
        .filter(|f| f != "bin/dash");
    words.chain(files).collect()
}

/// Every task of `others` (the open tasks of `new`'s project) sharing a term with `new`, scored by cosine over their terms,
/// each weighted by how rare it is among them (smoothed IDF), best first, ties by id. One shared term is never close nor
/// related (`close`, `similar`), but a shared file alone makes a dependency.
pub fn scored(new: &Text, others: &[Text]) -> Vec<Near> {
    let mine = terms(new);
    let theirs: Vec<(&Text, BTreeSet<String>)> = (others.iter())
        .filter(|o| o.id != new.id)
        .map(|o| (o, terms(o)))
        .collect();
    let n = theirs.len() + 1;
    let mut df: BTreeMap<&str, usize> = BTreeMap::new();
    for set in std::iter::once(&mine).chain(theirs.iter().map(|(_, s)| s)) {
        for term in set {
            *df.entry(term.as_str()).or_default() += 1;
        }
    }
    let w = |term: &str| match n < IDF_MIN {
        true => 1.0,
        false => ((n + 1) as f64 / (df[term] + 1) as f64).ln() + 1.0,
    };
    let sq = |set: &mut dyn Iterator<Item = &String>| set.map(|t| w(t) * w(t)).sum::<f64>();
    let mine_sq = sq(&mut mine.iter());
    let mut near: Vec<Near> = (theirs.iter())
        .filter_map(|(o, set)| {
            let shared: Vec<String> = mine.intersection(set).cloned().collect();
            let score = sq(&mut shared.iter()) / (mine_sq * sq(&mut set.iter())).sqrt();
            (!shared.is_empty() && score.is_finite()).then(|| Near {
                id: o.id.clone(),
                score,
                shared,
            })
        })
        .collect();
    near.sort_by(|a, b| b.score.total_cmp(&a.score).then_with(|| a.id.cmp(&b.id)));
    near
}

/// The close ones of `scored`, the older app's refusal set: of those sharing two terms or more, the closest from `BEST_FLOOR`,
/// the others from `FLOOR`.
pub fn close(scored: &[Near]) -> Vec<&Near> {
    // sorted best first: one past FLOOR after the first means the first passed BEST_FLOOR too
    (scored.iter().filter(|n| n.shared.len() >= 2).enumerate())
        .filter(|(i, n)| n.score >= if *i == 0 { BEST_FLOOR } else { FLOOR })
        .map(|(_, n)| n)
        .collect()
}

/// What a similar task is to a new one, and the line that acts on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    /// Close: the same work, so the new one's checks go there instead.
    Duplicate,
    /// Not close, but it names a file the new one names: the new one waits on it.
    DependsOn,
    /// Shares words, neither of the above.
    Related,
}

impl Mark {
    pub fn word(self) -> &'static str {
        match self {
            Mark::Duplicate => "duplicate",
            Mark::DependsOn => "depends-on",
            Mark::Related => "related",
        }
    }

    /// The command that acts on `other` for new task `new`.
    pub fn line(self, new: &str, other: &str) -> String {
        match self {
            Mark::Duplicate => format!(
                "dash task add {new} … --into {other} (the same add, its checks added to {other})"
            ),
            Mark::DependsOn => format!("dash task update {new} --blocked-by {other}"),
            Mark::Related => format!("dash task update {new} --related {other}"),
        }
    }
}

/// Up to this many depends-on and related tasks are printed after the duplicates, which are all printed.
pub const SHOWN: usize = 5;

/// The tasks of `others` similar to `new`, each marked: the close ones duplicate, unless `apart` names them (set apart by
/// `--distinct-from`), then those naming a file it names depends-on, then the related ones over `RELATED_FLOOR`.
/// `scored` is `scored(new, others)`, computed once by the caller.
pub fn similar(
    new: &Text,
    others: &[Text],
    scored: &[Near],
    apart: &[String],
) -> Vec<(Near, Mark)> {
    let close: Vec<String> = close(scored).iter().map(|n| n.id.clone()).collect();
    let mine = files_named(new);
    let shares_file = |id: &str| {
        let o = others.iter().find(|o| o.id == id);
        o.is_some_and(|o| files_named(o).iter().any(|f| mine.contains(f)))
    };
    let mut dup = vec![];
    let mut rest = vec![];
    for n in scored.iter().cloned() {
        let mark = match (
            close.contains(&n.id) && !apart.contains(&n.id),
            shares_file(&n.id),
        ) {
            (true, _) => Mark::Duplicate,
            (false, true) => Mark::DependsOn,
            (false, false)
                if n.shared.len() >= 2 && (n.score >= RELATED_FLOOR || close.contains(&n.id)) =>
            {
                Mark::Related
            }
            _ => continue,
        };
        match mark {
            Mark::Duplicate => dup.push((n, mark)),
            _ => rest.push((n, mark)),
        }
    }
    // depends-on first: a shared file is the stronger tie
    rest.sort_by_key(|(_, m)| *m != Mark::DependsOn);
    dup.into_iter()
        .chain(rest.into_iter().take(SHOWN))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;

    fn t(id: &str, title: &str, why: &str, done_when: &str) -> Text {
        Text {
            id: id.into(),
            title: title.into(),
            why: why.into(),
            done_when: done_when.into(),
        }
    }

    #[dash_test]
    fn paths_are_read_as_the_older_app_reads_them() {
        let text = "Fix rust/crates/dash/src/main.rs. Also bin/site-check, .claude/hooks/x.py, priv/commands/mcp-x and private/dashboard/lib/a.ex; \
            not docs.fly.io/x.md, ../up/x.rs, ~/repo/x.rs, new foo/bar.rs nor foo/bar";
        assert_eq!(
            paths(text),
            [
                "rust/crates/dash/src/main.rs",
                "bin/site-check",
                ".claude/hooks/x.py",
                "priv/commands/mcp-x",
                "private/dashboard/lib/a.ex"
            ]
        );
    }

    /// Done-when 1 and 4: two tasks from one subject, the second is close to the first and marked duplicate; one naming the
    /// same file only is depends-on; one sharing words only is related.
    #[dash_test]
    fn a_twin_is_a_duplicate_a_shared_file_a_dependency_and_shared_words_related() {
        let first = t(
            "resume-modes",
            "Session resume modes: resume, brief, summary",
            "A session resumes by its id, from its brief, or from its brief and summary",
            "1. sessions.ex resume modes picked by flag: a test",
        );
        let twin = t(
            "threads-resume",
            "Threads resume first: resume modes id, brief, summary",
            "Resume a session from its brief and summary",
            "1. resume modes chosen with --mode: a test",
        );
        let file = t(
            "pane-names",
            "Pane names follow their window",
            "Names drift",
            "1. lib/dashboard/orch/sessions.ex renames panes: a test",
        );
        let words = t(
            "summary-length",
            "A session summary stays under a page",
            "Long summaries overflow narrow phone screens, wrap badly, hide buttons, annoy readers, waste scrolling, confuse \
             layout reviewers, break printing margins",
            "1. capped: a test",
        );
        let unrelated = t(
            "mail-poll",
            "Mail poll reads klar",
            "Inbox",
            "1. polled: a test",
        );
        let new = t(
            "resume-again",
            "Resume modes for sessions: id, brief, summary",
            "Pick how a session resumes: its id, its brief, its brief and summary",
            "1. lib/dashboard/orch/sessions.ex picks the resume mode: a test",
        );
        let others = [first.clone(), twin.clone(), file, words, unrelated];
        let got = similar(&new, &others, &scored(&new, &others), &[]);
        let marks: Vec<(&str, &str)> = got.iter().map(|(n, m)| (n.id.as_str(), m.word())).collect();
        assert_eq!(
            marks,
            [
                ("resume-modes", "duplicate"),
                ("threads-resume", "duplicate"),
                ("pane-names", "depends-on"),
                ("summary-length", "related"),
            ],
            "{got:?}"
        );
        // set apart, a twin is no longer a duplicate
        let apart = similar(
            &new,
            &others,
            &scored(&new, &others),
            &["resume-modes".into(), "threads-resume".into()],
        );
        assert!(
            apart.iter().all(|(_, m)| *m != Mark::Duplicate),
            "{apart:?}"
        );
        assert_eq!(
            Mark::DependsOn.line("resume-again", "pane-names"),
            "dash task update resume-again --blocked-by pane-names"
        );
    }

    /// A file a task only cites is not one it changes: the dash command and the docs are left out, an app file is the repo's.
    #[dash_test]
    fn a_cited_doc_or_the_dash_command_is_no_named_file() {
        let cites = t(
            "x",
            "Per private/docs/rfc-rust-unified.html",
            "run private/dashboard/bin/dash task run, then bin/dash ship",
            "1. private/dashboard/lib/a.ex and lib/a.ex change: a test",
        );
        assert_eq!(files_named(&cites), ["lib/a.ex"]);
    }

    #[dash_test]
    fn his_quote_alone_makes_no_twin() {
        let quote = "(@nha, 2026-11-03: « this is either double work or stepping in the exact same area in parallel »)";
        let a = t("a", "Pane colours", quote, "1. coloured: a test");
        let b = t("b", "Mail filters", quote, "1. filtered: a test");
        assert!(scored(&b, &[a]).is_empty());
    }
}
