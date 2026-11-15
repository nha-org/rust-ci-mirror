//! The week's Rust-served share at the front door, read from Datadog (task rust-forwarded-top; @nha, 2026-11-14: « If you
//! don't need it killing elixir should reclaim ram/cpu too. »). The front door counts each page and command asked
//! (`dash_rust.front_door.asked`) and each handed to the older app (`.forwarded`, tagged `kind` and `what`). `dash fallback`
//! prints the share and the forwards by what was asked, most first: the commands to port next; /status/front-door shows
//! them, and they gate nothing. What the gate holds is what code controls (@nha, 2026-11-14, in the task's dialog: « ratchet
//! what code controls, the count of commands Rust answers; the live share shows on /status and gates nothing. »): each
//! front door branch that answers a command from Rust names it in its condition, [`answers`], and dash-gate's
//! `the_commands_rust_answers_only_grow` compares those names with the merge base's.

/// A week, Datadog's window for the share.
pub const WEEK: u64 = 7 * 24 * 3600;
/// A metric's tag value from a client's words: lower case letters, digits, « - » and « _ », 32 at most, so a request never
/// mints a malformed tag; nothing left is « other ».
fn tag(word: &str) -> String {
    let kept: String = (word.chars())
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(32)
        .collect::<String>()
        .to_ascii_lowercase();
    if kept.is_empty() {
        "other".into()
    } else {
        kept
    }
}

/// A page's tag: its path's first part, `/tasks/x` as `tasks`, `/` as `home`.
pub fn page_tag(path: &str) -> String {
    match path.trim_start_matches('/').split('/').next() {
        Some("") | None => "home".into(),
        Some(first) => tag(first),
    }
}

/// A command's tag and the key it is filed under: its first word, and its second when it reads as a subcommand's name
/// (`task show`): one short word of lower case letters, as ids hold a « - » or a digit (/code-review: `contact acme-corp`).
pub fn command_tag(args: &[String]) -> String {
    let sub =
        |w: &&String| w.len() <= 12 && !w.is_empty() && w.chars().all(|c| c.is_ascii_lowercase());
    let words: Vec<String> = (args.first().into_iter())
        .chain(args.get(1).filter(sub))
        .map(|w| tag(w))
        .collect();
    if words.is_empty() {
        "none".into()
    } else {
        words.join("_")
    }
}

/// The week's sum of front door counter `name`, a browser's own fetches left out, grouped `by` (« » for none).
pub fn query(name: &str, by: &str) -> String {
    format!("sum:dash_rust.front_door.{name}{{!kind:fetch}}{by}.as_count().rollup(sum, {WEEK})")
}

/// The week at the front door: pages and commands asked, those forwarded to the older app, and the forwards by what was
/// asked, most first.
pub struct Week {
    pub asked: f64,
    pub forwarded: f64,
    pub by: Vec<(String, f64)>,
}

impl Week {
    /// The share Rust answered itself, in percent; nothing asked is none.
    pub fn served(&self) -> f64 {
        if self.asked <= 0.0 {
            0.0
        } else {
            100.0 * (1.0 - self.forwarded / self.asked)
        }
    }

    /// `dash fallback`'s lines: the share, then each forward, most first, every one: no cap, so the lines sum to the
    /// forwarded total, each count rounded to a whole one (Datadog's sums of counts are whole but for float noise).
    pub fn said(&self) -> String {
        let by: String = (self.by.iter())
            .map(|(what, n)| format!("\n  {n:>6.0} {what}"))
            .collect();
        let by = match by.is_empty() {
            true => by,
            false => format!("\nforwarded, most first:{by}"),
        };
        format!(
            "the last 7 days: {:.0} asked, {:.0} forwarded to the older app, {:.0}% Rust-served{by}",
            self.asked,
            self.forwarded,
            self.served()
        )
    }
}

/// Each series of a Datadog query's answer, named by what was asked (`dash task show`, `/tasks`) and summed, most first.
pub fn series(body: &str) -> Result<Vec<(String, f64)>, String> {
    let answer: serde_json::Value =
        serde_json::from_str(body).map_err(|e| format!("Datadog's answer: {e}"))?;
    let tag = |s: &serde_json::Value, name: &str| {
        (s["tag_set"].as_array().into_iter().flatten())
            .filter_map(|t| t.as_str()?.strip_prefix(name)?.strip_prefix(':'))
            .next()
            .map(str::to_string)
    };
    let mut by: Vec<(String, f64)> = (answer["series"].as_array().into_iter().flatten())
        .map(|s| {
            let what = match (tag(s, "kind").as_deref(), tag(s, "what")) {
                (Some("command"), Some(w)) => format!("dash {}", w.replace('_', " ")),
                (Some("page"), Some(w)) if w == "home" => "/".into(),
                (Some("page"), Some(w)) => format!("/{w}"),
                (_, w) => w.unwrap_or_else(|| "all".into()),
            };
            let points = s["pointlist"].as_array().into_iter().flatten();
            // f64's empty sum is -0.0, printed « -0 »: + 0.0 makes it 0
            (
                what,
                points.filter_map(|p| p[1].as_f64()).sum::<f64>() + 0.0,
            )
        })
        .collect();
    by.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    Ok(by)
}

/// A GET of Datadog's query API with these pairs, its answer's body: the binary's, which holds the keys.
pub type Ask = dyn Fn(&[(&str, &str)]) -> Result<String, String> + Sync;

/// The week up to `now` (seconds) from Datadog: pages and commands asked, and the forwards by what was asked, their sum the
/// forwarded.
pub fn week(now: i64, ask: &Ask) -> Result<Week, String> {
    let (from, to) = (now.saturating_sub(WEEK as i64).to_string(), now.to_string());
    let get = |q: &str| ask(&[("from", from.as_str()), ("to", to.as_str()), ("query", q)]);
    let asked = sum(&series(&get(&query("asked", ""))?)?);
    let by = series(&get(&query("forwarded", " by {kind,what}"))?)?;
    let forwarded = sum(&by);
    Ok(Week {
        asked,
        forwarded,
        by,
    })
}

/// A series' total; f64's empty sum is -0.0, printed « -0 »: + 0.0 makes it 0.
fn sum(by: &[(String, f64)]) -> f64 {
    by.iter().map(|(_, n)| n).sum::<f64>() + 0.0
}

/// Whether `args` opens on command `name`'s words (« task show », « fallback » for `fallback off`): the front door's mark on
/// a branch that answers it from Rust, which [`answered`] reads back from the source.
pub fn answers(name: &str, args: &[String]) -> bool {
    let words: Vec<&str> = name.split(' ').collect();
    args.len() >= words.len() && words.iter().zip(args).all(|(w, a)| w == a)
}

/// The commands a source answers from Rust: each name its [`answers`] calls give, a line comment's never: a mark left
/// in a comment once its branch went would pass the ratchet (/code-review of PRs 1589, 1599).
/// ponytail: a `/* */` block still counts; no branch condition holds one.
/// ponytail: a text read, so an arm moved out of main.rs reads as gone; a registry (`inventory::submit!`) once arms move.
pub fn answered(source: &str) -> std::collections::BTreeSet<String> {
    let mark = "served::answers(\"";
    (source.lines())
        .filter_map(|l| l.split("//").next())
        .flat_map(|l| l.split(mark).skip(1))
        .filter_map(|rest| rest.split('"').next())
        .map(str::to_string)
        .collect()
}

/// The ratchet: every command the base answered from Rust is answered still.
pub fn kept(
    head: &std::collections::BTreeSet<String>,
    base: &std::collections::BTreeSet<String>,
    rev: &str,
) -> Result<String, String> {
    let gone: Vec<&String> = base.difference(head).collect();
    match gone.is_empty() {
        true => Ok(format!(
            "{} commands answered from Rust, {} at {rev}",
            head.len(),
            base.len()
        )),
        false => Err(format!(
            "the commands Rust answers only grow: {gone:?} answered from Rust at {rev}, gone now: put its branch back in the \
             front door (crates/dash/src/main.rs, `cli`), its condition naming it with served::answers"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;

    /// Task rust-forwarded-top, done-when 1: the week's forwards by command, most first, read from Datadog's series
    /// (stand-in numbers), as `dash fallback` prints them.
    #[dash_test]
    fn the_week_s_forwards_print_by_command_most_first() {
        let body = r#"{"series": [
            {"tag_set": ["kind:command", "what:store_text"], "pointlist": [[1, 900.0], [2, 300.0]]},
            {"tag_set": ["kind:page", "what:home"], "pointlist": [[1, 40.0]]},
            {"tag_set": ["kind:command", "what:task_show"], "pointlist": [[1, 5000.0]]},
            {"tag_set": ["kind:page", "what:tasks"], "pointlist": [[1, null]]}
        ]}"#;
        let ask = |pairs: &[(&str, &str)]| {
            assert_eq!(pairs[..2], [("from", "1000"), ("to", "605800")]);
            match pairs[2].1.contains(" by {kind,what}") {
                true => Ok(body.to_string()),
                false => Ok(r#"{"series": [{"pointlist": [[1, 8000.0]]}]}"#.to_string()),
            }
        };
        let week = week(605_800, &ask).expect("week");
        assert_eq!(
            week.said(),
            "the last 7 days: 8000 asked, 6240 forwarded to the older app, 22% Rust-served\n\
             forwarded, most first:\n    5000 dash task show\n    1200 dash store text\n      40 /\n       0 /tasks"
        );
        assert!(series("not json").is_err());
        // a rollup's sum that is no whole number prints as one: the column stays aligned (/code-review of PR 1589)
        let odd = Week {
            asked: 1199.9999999,
            forwarded: 600.2,
            by: vec![("dash store text".into(), 600.2)],
        };
        assert!(
            odd.said()
                .starts_with("the last 7 days: 1200 asked, 600 forwarded"),
            "{}",
            odd.said()
        );
        assert!(
            odd.said().ends_with("\n     600 dash store text"),
            "{}",
            odd.said()
        );

        // no cap and no « other »: every command prints, and the lines sum to the forwarded total (101 a live week)
        let many: Vec<String> = (1..=150)
            .map(|i| {
                format!(
                    r#"{{"tag_set": ["kind:command", "what:c{i}"], "pointlist": [[1, {i}.0]]}}"#
                )
            })
            .collect();
        let body = format!(r#"{{"series": [{}]}}"#, many.join(","));
        let week = super::week(605_800, &move |pairs: &[(&str, &str)]| match pairs[2]
            .1
            .contains(" by ")
        {
            true => Ok(body.clone()),
            false => Ok(r#"{"series": [{"pointlist": [[1, 20000.0]]}]}"#.to_string()),
        })
        .expect("week");
        let said = week.said();
        let lines: Vec<f64> = (said.lines().skip(2))
            .map(|l| l.split_whitespace().next().unwrap().parse().unwrap())
            .collect();
        assert_eq!(lines.len(), 150, "{said}");
        assert_eq!(lines.iter().sum::<f64>(), week.forwarded);
        assert!(
            said.starts_with("the last 7 days: 20000 asked, 11325 forwarded"),
            "{said}"
        );
        assert!(said.ends_with("\n       1 dash c1"), "{said}");
    }

    /// Task rust-forwarded-top, done-when 3: the commands a source answers from Rust, read from its marks, only grow.
    #[dash_test]
    fn the_commands_answered_from_rust_only_grow() {
        let args = |w: &[&str]| w.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(answers("task show", &args(&["task", "show", "rust-x"])));
        assert!(!answers("task show", &args(&["task", "style"])));
        assert!(
            answers("fallback", &args(&["fallback", "off"])) && !answers("fallback", &args(&[]))
        );
        let base = answered(
            "if served::answers(\"outbox\", &args) {}\nif served::answers(\"task show\", &args) {}",
        );
        assert_eq!(
            base.iter().map(String::as_str).collect::<Vec<_>>(),
            ["outbox", "task show"]
        );
        let more = answered(
            "served::answers(\"outbox\", &a) served::answers(\"task show\", &a) served::answers(\"store text\", &a)",
        );
        assert_eq!(
            kept(&more, &base, "b").expect("grew"),
            "3 commands answered from Rust, 2 at b"
        );
        let fewer = answered(
            "served::answers(\"outbox\", &a) // was served::answers(\"task show\", &a)\n    // served::answers(\"task show\", &a)",
        );
        let said = kept(&fewer, &base, "b").expect_err("fewer");
        assert!(
            said.starts_with(
                "the commands Rust answers only grow: [\"task show\"] answered from Rust at b"
            ),
            "{said}"
        );
    }

    #[dash_test]
    fn tags_never_carry_an_id_or_his_words() {
        assert_eq!(page_tag("/tasks/rust-x"), "tasks");
        assert_eq!(page_tag("/"), "home");
        assert_eq!(page_tag("/Ma%20il"), "ma20il");
        let words = |w: &[&str]| w.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(command_tag(&words(&["task", "show", "x"])), "task_show");
        assert_eq!(command_tag(&words(&["contact", "rust-x1"])), "contact");
        assert_eq!(command_tag(&words(&["contact", "acme-corp"])), "contact");
        assert_eq!(
            command_tag(&words(&["task", "show", "rust-fallback-off"])),
            "task_show"
        );
        assert_eq!(command_tag(&words(&["request", "Klar plus"])), "request");
        assert_eq!(command_tag(&[]), "none");
    }
}
