//! The older app's task style bars (Dashboard.TaskStyle), run by the door before it answers a task write (task
//! rust-copy-refusal-answered): 12 of 23 copies were refused after an « added », 4 adds lost (@nha, 2026-10-29: « You fixed
//! the mistake but didn't find the cause and what could actually fix that class of errors right ? »). The bars every refused
//! copy hit are here, said in TaskStyle's words; an add also runs the older app's own dry run (copies::checked).
//! ponytail: the rules read by sentence (a :why's own line, a prompt's sentences) and by repo file (paths, AI tells) stay the
//! older app's: an add's dry run runs them, an update's copy flags them; port them if those flags come.

use regex::Regex;
use std::sync::LazyLock;

const LINE: usize = 160;
const TITLE: usize = 80;
const CHECKS: usize = 5;
const WHY: usize = 360;
const SIZE: usize = 1000;

/// The four fields a task's style is read from, as TaskStyle's `fields/1`: none is an absent field.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Task {
    pub title: Option<String>,
    pub why: Option<String>,
    pub done_when: Option<String>,
    pub prompt: Option<String>,
    /// Labelled Unplanned: his words filed as they are, the :done-when, whole-size and own-line bars waived.
    pub unplanned: bool,
}

impl Task {
    /// A filed task's fields, an empty one absent.
    pub fn filed(title: &str, why: &str, done_when: &str, prompt: &str) -> Task {
        let some = |v: &str| (!v.is_empty()).then(|| v.to_string());
        Task {
            title: some(title),
            why: some(why),
            done_when: some(done_when),
            prompt: some(prompt),
            unplanned: false,
        }
    }
}

fn len(v: &str) -> usize {
    v.chars().count()
}

/// The numbered checks of a :done-when, `1. a 2. b` into `a` and `b`; none when it does not open on « 1. ».
pub fn checks(text: &str) -> Vec<String> {
    let Some(mut rest) = text.trim().strip_prefix("1.") else {
        return vec![];
    };
    if !rest.starts_with(char::is_whitespace) {
        return vec![];
    }
    static NUMBER: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\s(\d+)\.\s").expect("a check's number"));
    // the first « n. » in `text` from `from` on, a number other than `n` passed over
    let next = |text: &str, n: usize| {
        let mut from = 0;
        while let Some(c) = NUMBER.captures_at(text, from) {
            let m = c.get(0).expect("the match");
            if c[1].parse::<usize>() == Ok(n) {
                return Some((m.start(), m.end()));
            }
            from = m.start() + 1;
        }
        None
    };
    let mut out = vec![];
    // « 2. » closes check 1 only after it: a « 4.5 » or « 2. » inside a check stays in it
    for n in 2.. {
        match next(rest, n) {
            Some((start, end)) => {
                out.push(rest[..start].trim().to_string());
                rest = &rest[end..];
            }
            None => break,
        }
    }
    out.push(rest.trim().to_string());
    out
}

/// A :done-when given as `--done-when` once or more, numbered as the older app numbers it (`numbered/1`).
pub fn numbered(given: &[&str]) -> String {
    static NUMBER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\d+\.\s+").expect("a number"));
    match given {
        [one] if one.starts_with("1.") && one[2..].starts_with(char::is_whitespace) => {
            one.to_string()
        }
        [one] => format!("1. {one}"),
        many => (many.iter().enumerate())
            .map(|(i, c)| format!("{}. {}", i + 1, NUMBER.replace(c, "")))
            .collect::<Vec<_>>()
            .join(" "),
    }
}

/// Its checks as `--into` adds them (`as_checks/1`): numbered ones, else the whole text as one, none when blank.
pub fn as_checks(d: Option<&str>) -> Vec<String> {
    let d = d.unwrap_or_default();
    match (checks(d), d.trim()) {
        (cs, _) if !cs.is_empty() => cs,
        (_, "") => vec![],
        (_, one) => vec![one.to_string()],
    }
}

/// What task `t` breaks among the ported bars, one line each, in TaskStyle's words.
pub fn problems(t: &Task) -> Vec<String> {
    let mut out = vec![];
    match t.title.as_deref().map(len) {
        None | Some(0) => out.push(":title: missing".to_string()),
        Some(n) if n >= TITLE => out.push(format!(":title: {n} characters, under {TITLE}")),
        _ => {}
    }
    // his quote whole: only that it is there and cited
    if t.unplanned {
        let cited =
            |p: &String| p.starts_with(":why: missing") || p.starts_with(":why: no words of his");
        out.extend(why(t.why.as_deref()).into_iter().filter(cited));
        return out;
    }
    out.extend(why(t.why.as_deref()));
    out.extend(done_when(t.done_when.as_deref()));
    let size: usize = [&t.title, &t.why, &t.done_when, &t.prompt]
        .iter()
        .map(|f| f.as_deref().map_or(0, len))
        .sum();
    if size > SIZE {
        out.push(format!(
            ":task: {size} characters, at most {SIZE}: cut what the reader skips"
        ));
    }
    out
}

fn why(w: Option<&str>) -> Vec<String> {
    static CITED: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"\(@nha, \d{4}-\d\d-\d\d[^«()]*(?:\([^()]*\)[^«()]*)*«").expect("a citation")
    });
    static QUOTE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"«[^»]*»").expect("a quote"));
    static FRAME: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\(@nha[^()]*(?:\([^()]*\)[^()]*)*\)").expect("a frame"));
    let Some(w) = w else {
        return vec![":why: missing: his words, cited (@nha, <date>: « … »), plus one line".into()];
    };
    let own = FRAME
        .replace_all(&QUOTE.replace_all(w, ""), "")
        .trim()
        .to_string();
    let mut out = vec![];
    if !CITED.is_match(w) {
        out.push(":why: no words of his, cited (@nha, <date>: « … »)".to_string());
    }
    if own.is_empty() {
        out.push(":why: no line of ours after his words".to_string());
    }
    if len(w) > WHY {
        out.push(format!(
            ":why: {} characters, at most {WHY}: quote the sentence that asks",
            len(w)
        ));
    }
    if len(&own) > LINE {
        out.push(format!(
            ":why: {} characters of its own, one line of at most {LINE}",
            len(&own)
        ));
    }
    out
}

fn done_when(d: Option<&str>) -> Vec<String> {
    let Some(d) = d else {
        return vec![":done-when: missing".into()];
    };
    let cs = checks(d);
    if cs.is_empty() {
        return vec![":done-when: not numbered checks (1. … 2. …)".into()];
    }
    let mut out = vec![];
    if cs.len() > CHECKS {
        out.push(format!(":done-when: {} checks, at most {CHECKS}", cs.len()));
    }
    for (i, c) in cs.iter().enumerate().filter(|(_, c)| len(c) > LINE) {
        out.push(format!(
            ":done-when check {}: {} characters, one line of at most {LINE}",
            i + 1,
            len(c)
        ));
    }
    out
}

/// What `after` breaks that `before` did not, as the older app's `styled/3` judges an update: a count is not the rule, so a
/// field over its bar may shrink or stay, never grow; a task past the whole-task bar may still grow.
pub fn new_problems(before: &Task, after: &Task) -> Vec<String> {
    static COUNT: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(\d+) (characters|sentences|checks)").expect("a count"));
    let rule = |p: &str| COUNT.replace(p, "# $2").into_owned();
    let n = |p: &str| COUNT.captures(p).and_then(|c| c[1].parse::<usize>().ok());
    let was = problems(before);
    let long = was.iter().any(|p| p.starts_with(":task:"));
    (problems(after).into_iter())
        .filter(|p| {
            let known = was.iter().find(|b| rule(b) == rule(p));
            let kept = match (known.map(|b| n(b)), n(p)) {
                (Some(Some(old)), Some(new)) => new <= old,
                (Some(None), None) => true,
                _ => false,
            };
            !kept && !(long && p.starts_with(":task:"))
        })
        .collect()
}

/// A style refusal as the older app says it (`styled/3`, `retry/3`): every problem at once, then each field's bar.
pub fn refused(id: &str, ps: &[String]) -> String {
    let each: Vec<String> = (ps.iter().enumerate())
        .map(|(i, p)| format!("{}) {p}", i + 1))
        .collect();
    format!(
        "{id}: nothing written, {} style problems: {}; limits: --title under {TITLE} characters; --why at most {WHY}, his \
         cited quote then one line of ours of at most {LINE}; --done-when at most {CHECKS} checks of at most {LINE} each; \
         the whole task at most {SIZE}",
        ps.len(),
        each.join(" ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;

    fn task(title: &str, why: &str, done_when: &str) -> Task {
        Task::filed(title, why, done_when, "")
    }

    const WHY_OK: &str = "(@nha, 2026-11-12: « fix it ») A refused copy is lost.";

    /// The bars each refused copy of 2026-11-12 hit, in the older app's words.
    #[dash_test]
    fn the_ported_bars_say_what_the_older_app_said() {
        assert_eq!(
            problems(&task("A title", WHY_OK, "1. a 2. b")),
            Vec::<String>::new()
        );
        assert_eq!(
            problems(&task(&"t".repeat(81), WHY_OK, "1. a")),
            [":title: 81 characters, under 80"]
        );
        let six = "1. a 2. b 3. c 4. d 5. e 6. f";
        assert_eq!(
            problems(&task("T", WHY_OK, six)),
            [":done-when: 6 checks, at most 5"]
        );
        let long = format!("1. {} 2. b", "c".repeat(187));
        assert_eq!(
            problems(&task("T", WHY_OK, &long)),
            [":done-when check 1: 187 characters, one line of at most 160"]
        );
        assert_eq!(
            problems(&task("T", "A line of ours alone.", "1. a")),
            [":why: no words of his, cited (@nha, <date>: « … »)"]
        );
        let big = task(
            "T",
            WHY_OK,
            &format!("1. {} 2. {}", "a".repeat(150), "b".repeat(150)),
        );
        let big = Task {
            prompt: Some("p".repeat(800)),
            ..big
        };
        assert_eq!(problems(&big).len(), 1);
        assert!(problems(&big)[0].starts_with(":task: "));
        assert_eq!(checks("1. a 4.5 b 2. c"), ["a 4.5 b", "c"]);
        assert_eq!(numbered(&["a", "2. b"]), "1. a 2. b");
        assert_eq!(numbered(&["1. a 2. b"]), "1. a 2. b");
    }

    /// An update is refused only on a problem it adds or grows (`styled/3`).
    #[dash_test]
    fn an_update_is_refused_only_on_what_it_adds() {
        let before = task("T", WHY_OK, &format!("1. {}", "c".repeat(170)));
        let shorter = task("T", WHY_OK, &format!("1. {}", "c".repeat(165)));
        assert_eq!(new_problems(&before, &shorter), Vec::<String>::new());
        let longer = task("T", WHY_OK, &format!("1. {}", "c".repeat(175)));
        assert_eq!(
            new_problems(&before, &longer),
            [":done-when check 1: 175 characters, one line of at most 160"]
        );
        let retitled = Task {
            title: Some("t".repeat(81)),
            ..before.clone()
        };
        assert_eq!(
            new_problems(&before, &retitled),
            [":title: 81 characters, under 80"]
        );
    }
}
