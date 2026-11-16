//! Every command registers its usage line from its own module, `inventory::submit! { Command::usage("dash ..") }` beside its
//! code, as pages register their routes (pages.rs), and `dash help` prints them sorted: no one-line USAGE in main.rs for
//! every new command to edit (task rust-usage-registered; rust-inbox-triage met that conflict twice in one PR). A command
//! that also registers `run` needs no arm in main.rs's match either: two branches each adding one meet only if their `mod`
//! lines sit side by side (rfc-rust-unified section 11, flat dispatch).
use std::fs::File;
use std::io::Write;

// the SDK's command, the core's since task rust-move-core-ports: a plugin registers one as the binary's modules do
pub use dash_core::Command;

/// Every registered line, sorted: the first after « usage: », the others under it, as the old USAGE read.
pub fn text() -> String {
    let mut lines: Vec<&str> = inventory::iter::<Command>
        .into_iter()
        .map(|c| c.usage)
        .collect();
    lines.sort_unstable();
    format!("usage: {}", lines.join("\n       "))
}

/// The registered command these arguments name, answered; else the usage on stderr, exit 2.
pub fn run(host: &dyn dash_core::Host, args: &[&str], mut out: File) -> ! {
    let found = inventory::iter::<Command>
        .into_iter()
        .filter_map(|c| c.run.map(|run| (c.words, run)))
        .find(|(words, _)| args.starts_with(words));
    match found {
        Some((words, run)) => {
            run(host, &args[words.len()..], &mut out);
            std::process::exit(0)
        }
        None => {
            eprintln!("{}", text());
            std::process::exit(2)
        }
    }
}

fn help(_: &dyn dash_core::Host, _: &[&str], out: &mut dyn Write) {
    // a reader that closed early (`dash help | head`) is no failure
    let _ = writeln!(out, "{}", text());
}

inventory::submit! {
    Command { usage: "dash help", words: &["help"], run: Some(help) }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Done-when 1: lines registered from several modules come out of `text` sorted, one a line, each a `dash` command.
    #[dash_meter::dash_test]
    fn help_assembles_every_registered_line_sorted() {
        let text = text();
        let lines: Vec<&str> = text
            .lines()
            .map(|l| l.trim_start_matches("usage: ").trim_start())
            .collect();
        let mut sorted = lines.clone();
        sorted.sort_unstable();
        assert_eq!(lines, sorted, "dash help's lines, sorted");
        assert!(lines.iter().all(|l| l.starts_with("dash ")), "{lines:?}");
        // one from this module, main.rs, a module of this crate and another crate's const
        for from in [
            "dash help",
            "dash access",
            "dash ui [--data <dir>]",
            "dash pane say [--his]",
        ] {
            assert!(
                lines.iter().any(|l| l.starts_with(from)),
                "{from} registered: {lines:?}"
            );
        }
    }

    /// Which run answers is never left to link order, nor to an arm of main.rs's match tried first: no registered words
    /// start another's, and main.rs holds no arm on a registered command's first word.
    #[dash_meter::dash_test]
    fn each_registered_command_has_words_no_other_answers() {
        let words: Vec<&[&str]> = inventory::iter::<Command>
            .into_iter()
            .filter(|c| c.run.is_some())
            .map(|c| c.words)
            .collect();
        assert!(words.len() >= 2, "{words:?}");
        // flags kept: « orch --data » never takes « orch takeover » (task orch-takeover-state)
        let arms = main_arms(include_str!("main.rs"), true);
        for (i, a) in words.iter().enumerate() {
            assert!(
                !a.is_empty(),
                "a run with no words would answer every command"
            );
            for b in &words[i + 1..] {
                assert!(
                    !a.starts_with(b) && !b.starts_with(a),
                    "{a:?} and {b:?} answer the same words"
                );
            }
            let armed = armed_first(a, &arms);
            assert!(
                armed.is_none(),
                "main.rs's {armed:?} arm would answer {a:?} first"
            );
        }
    }

    /// The arm of `arms` that answers some of `words`' commands before the registry: one whose words lead them, or that they
    /// lead. One sharing only a first word does not: `ci affected` stays an arm beside dash-forge's `ci list` (task
    /// rust-hostman-ci-wire).
    fn armed_first<'a>(words: &[&str], arms: &'a [String]) -> Option<&'a String> {
        let leads = |x: &[&str], y: &[&str]| y.len() <= x.len() && x[..y.len()] == *y;
        (arms.iter().filter(|w| !w.is_empty())).find(|arm| {
            let arm: Vec<&str> = arm.split(' ').collect();
            leads(words, &arm) || leads(&arm, words)
        })
    }

    #[dash_meter::dash_test]
    fn an_arm_that_leads_a_registered_command_s_words_is_found() {
        let main = "\nfn main() {\n    match x {\n        [\"ci\", \"affected\", base] => a(),\n        [\"pane\", rest @ ..] => b(),\n    }\n}\n";
        let arms = main_arms(main, false);
        assert_eq!(arms, ["ci affected", "pane"]);
        let flagged = main_arms(
            "\nfn main() {\n    [\"orch\", \"--data\", dir] => a,\n}\n",
            true,
        );
        assert_eq!(
            armed_first(&["orch", "takeover"], &flagged),
            None,
            "a flag is a literal the arm needs"
        );
        assert_eq!(
            armed_first(&["ci", "list"], &arms),
            None,
            "a first word alone"
        );
        assert_eq!(
            armed_first(&["pane", "say"], &arms).map(String::as_str),
            Some("pane")
        );
        assert_eq!(
            armed_first(&["ci"], &arms).map(String::as_str),
            Some("ci affected")
        );
    }

    /// The words each arm of main.rs's `main` match names a command by: its leading string literals, up to the first flag
    /// or binding (`flags`: through its flags too, which the arm needs as much as its words). An arm rustfmt wrapped over
    /// several lines is read whole.
    fn main_arms(main: &str, flags: bool) -> Vec<String> {
        let body = main.split_once("\nfn main() {").map_or("", |(_, b)| b);
        let body = body.split_once("\n}\n").map_or(body, |(b, _)| b);
        let (mut arms, mut open) = (vec![], None::<String>);
        // the arms under `Some(dir)` run on a folder `--data` names: the flag is theirs as much as their words
        let mut named = false;
        for line in body.lines().map(str::trim) {
            match line {
                "Some(dir) => {" => named = true,
                "None => match words[..] {" => named = false,
                _ => {}
            }
            let pattern = match open.take() {
                Some(mut held) if !line.starts_with(']') => {
                    held.push_str(line);
                    open = Some(held);
                    continue;
                }
                Some(held) => held,
                None if line == "[" => {
                    open = Some(String::new());
                    continue;
                }
                None if line.starts_with("[\"") => {
                    line[1..].split(']').next().unwrap_or("").to_string()
                }
                None => continue,
            };
            let words: Vec<&str> = (pattern.split(','))
                .map(str::trim)
                .map_while(|w| w.strip_prefix('"').and_then(|w| w.strip_suffix('"')))
                .take_while(|w| flags || !w.starts_with('-'))
                .collect();
            let flag = if named && flags { " --data" } else { "" };
            arms.push(words.join(" ") + flag);
        }
        arms
    }

    /// A usage line names an arm's words when its own words, after `dash`, each offer that word: `[on|off]` offers on and off.
    fn listed(arm: &str, usage: &str) -> bool {
        let arm: Vec<&str> = arm.split(' ').collect();
        usage.lines().any(|line| {
            let line = line.trim_start_matches("usage:").trim();
            let said: Vec<&str> = line.split_whitespace().skip(1).collect();
            said.len() >= arm.len()
                && (arm.iter().zip(&said)).all(|(w, s)| {
                    s.trim_matches(|c| c == '[' || c == ']')
                        .split('|')
                        .any(|alt| alt == *w)
                })
        })
    }

    /// Each command main.rs's match answers is listed by `dash help`: an arm whose words no registered line names fails
    /// here, so a new arm comes with its line (the PM check on rust-usage-registered: launchd, host, land, chances,
    /// watchers, code map and others were answered and never listed).
    #[dash_meter::dash_test]
    fn every_command_main_answers_has_a_registered_usage_line() {
        let usage = text();
        let arms = main_arms(include_str!("main.rs"), false);
        assert!(arms.len() > 50, "main's arms read: {arms:?}");
        assert!(
            arms.contains(&"launchd install".to_string()),
            "a wrapped arm read: {arms:?}"
        );
        assert!(listed(
            "fallback off",
            "usage: dash fallback [on|off] --data <dir>"
        ));
        assert!(!listed("orch route", "usage: dash orch --data <dir>"));
        let mut unlisted: Vec<&String> = (arms.iter())
            .filter(|a| !a.is_empty() && !listed(a, &usage))
            .collect();
        unlisted.dedup();
        assert!(
            unlisted.is_empty(),
            "answered by main.rs, never listed: {unlisted:#?}"
        );
    }
}
