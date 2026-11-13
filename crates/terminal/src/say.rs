//! `dash pane say`: words into another session's prompt, signed by who sent them, arriving whole and submitted (the older
//! app's `Dashboard.Orch.Message.send/2`, ported, task rust-pane-say-signed). A message is signed « From the orchestrator: »,
//! or « From the task <id>: » in a task's session, unless the words are his, given with `--his`: review-rust-doc-types
//! quoted an orchestrator pane message as « his word » and listed itself as waiting on him (2026-11-09).
use dash_core::tasks::Terminal;
use std::thread::sleep;
use std::time::Duration;

/// Its usage, which bin/dash also looks for in this binary before handing it `pane say`.
pub const USAGE: &str = "dash pane say [--his] <pane|task id> <text>...";

/// How many times Enter is pressed before the message is said stuck in its box: two orchestrator messages to the CV task sat
/// pasted but unsent for 10 minutes (task pane-say-confirms).
const ENTERS: u32 = 6;

/// The words as sent: his as they are, anyone else's signed. `session` is DASH_SESSION, which a task's launch sets and the
/// orchestrator's scrubs (herdr/session.sh), so a task telling its reviewer is never taken for the orchestrator.
pub fn signed(text: &str, his: bool, session: Option<&str>) -> String {
    match (his, session.filter(|s| !s.is_empty())) {
        (true, _) => text.to_string(),
        (false, Some(task)) => format!("From the task {task}: {text}"),
        (false, None) => format!("From the orchestrator: {text}"),
    }
}

/// `pane say`'s words after the verb: the pane, the text, and whether it is his. `--his` comes first, before the pane: a later
/// one is a word of the text (/code-review: a signed text starting « --his » lost its signature).
pub fn parse<'a>(args: &[&'a str]) -> Option<(&'a str, String, bool)> {
    let (his, args) = match args {
        ["--his", rest @ ..] => (true, rest),
        _ => (false, args),
    };
    let (pane, words) = args.split_first()?;
    let text = words.join(" ");
    (!text.trim().is_empty()).then_some((*pane, text, his))
}

/// Sends `text` to `pane` and presses Enter until its input box no longer holds it. Nothing is typed into a pane whose agent
/// asks a question (the keys would answer it for him) or into one with no agent (its shell would run the words).
pub fn say(terminal: &dyn Terminal, pane: &str, text: &str, pause: Duration) -> Result<(), String> {
    match terminal.agent(pane)? {
        None => return Err(format!("{pane}: no agent there, nothing typed")),
        Some(seen) if seen.asking => return Err(format!("{pane} shows a dialog, nothing typed")),
        Some(_) => {}
    }
    terminal.send(pane, text)?;
    sleep(pause);
    for _ in 0..ENTERS {
        terminal.key(pane, "Enter")?;
        sleep(pause);
        if !pending(terminal, pane, text) {
            // ponytail: no read-back on the screen or in the transcript, as the older app did after this; the box emptied and
            // herdr refuses a read of a working pane anyway. Add one if a message is ever lost after leaving the box.
            return Ok(());
        }
    }
    Err(format!(
        "{pane}: the message is still in its input box after {ENTERS} Enters"
    ))
}

/// Still waiting in the box: the box holds its words, or Claude's « [Pasted text #1] », and its agent is idle. An agent at
/// work or asking took it: another Enter would answer a dialog shown since (/code-review of #390).
fn pending(terminal: &dyn Terminal, pane: &str, text: &str) -> bool {
    let held = terminal
        .read(pane)
        .is_ok_and(|screen| holds(&input_box(&screen), text));
    held && terminal
        .agent(pane)
        .map_or(true, |seen| seen.is_none_or(|s| s.idle))
}

/// What Claude's input box holds: the part between its last two rules (a line of ─), its prompt sign dropped. None when the
/// screen ends in no box: more than 8 lines under it is a transcript, not a footer.
fn input_box(screen: &str) -> String {
    let rule = |l: &str| {
        let l = l.trim();
        l.chars().count() >= 3 && l.chars().all(|c| c == '─')
    };
    let lines: Vec<&str> = screen.lines().collect();
    let rules: Vec<usize> = (0..lines.len()).filter(|&i| rule(lines[i])).collect();
    let [.., top, bottom] = rules[..] else {
        return String::new();
    };
    let below = lines[bottom + 1..].join("\n");
    if below.trim().lines().count() > 8 {
        return String::new();
    }
    let inside = lines[top + 1..bottom].join("\n");
    let inside = inside.trim();
    let inside = inside.strip_prefix(['>', '❯']).unwrap_or(inside);
    inside.trim().to_string()
}

/// The box starts with the message, at least its first 30 characters: a shorter box is a greyed suggestion, and a box that
/// starts otherwise is his own typing (/code-review of pane-say-confirms).
fn holds(boxed: &str, text: &str) -> bool {
    let squash = |s: &str| s.split_whitespace().collect::<String>();
    let (b, m) = (squash(boxed), squash(text));
    let head: String = b.chars().take(30).collect();
    b.contains("[Pastedtext")
        || (!b.is_empty()
            && head.chars().count() >= m.chars().count().min(30)
            && m.starts_with(&head))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_core::tasks::Seen;
    use dash_meter::dash_test;
    use std::cell::RefCell;

    /// A pane that logs every key and text it gets, and shows `screens` in turn, the last one for good.
    struct Pane {
        seen: Option<Seen>,
        screens: RefCell<Vec<String>>,
        keys: RefCell<Vec<String>>,
    }

    impl Pane {
        fn new(seen: Option<Seen>, screens: &[&str]) -> Self {
            let screens = screens.iter().rev().map(|s| s.to_string()).collect();
            Pane {
                seen,
                screens: RefCell::new(screens),
                keys: RefCell::new(vec![]),
            }
        }
    }

    impl Terminal for Pane {
        fn open(&self, _: &str, _: &str) -> Result<String, String> {
            unreachable!()
        }
        fn run(&self, _: &str, _: &str) -> Result<(), String> {
            unreachable!()
        }
        fn agent(&self, _: &str) -> Result<Option<Seen>, String> {
            Ok(self.seen.clone())
        }
        fn sessions(&self) -> Result<usize, String> {
            unreachable!()
        }
        fn read(&self, _: &str) -> Result<String, String> {
            let mut s = self.screens.borrow_mut();
            Ok(if s.len() > 1 {
                s.pop()
            } else {
                s.last().cloned()
            }
            .unwrap_or_default())
        }
        fn send(&self, pane: &str, text: &str) -> Result<(), String> {
            self.keys.borrow_mut().push(format!("send {pane} {text}"));
            Ok(())
        }
        fn key(&self, pane: &str, key: &str) -> Result<(), String> {
            self.keys.borrow_mut().push(format!("key {pane} {key}"));
            Ok(())
        }
        fn close(&self, _: &str) -> Result<(), String> {
            unreachable!()
        }
    }

    fn idle() -> Option<Seen> {
        Some(Seen {
            idle: true,
            asking: false,
            session: Some("s1".into()),
        })
    }

    const EMPTY: &str = "● done\n───────\n❯ \n───────\n  ? for shortcuts";

    /// Done-when 1 of rust-pane-say-signed: dash pane say prefixes « From the orchestrator: » unless the text is his, given
    /// with --his; the keys the pane gets are read back.
    #[dash_test]
    fn pane_say_signs_the_orchestrator_s_words_never_his() {
        let sent = |args: &[&str], session: Option<&str>| {
            let (pane, text, his) = parse(args).expect("words");
            let pane_ = Pane::new(idle(), &[EMPTY]);
            say(&pane_, pane, &signed(&text, his, session), Duration::ZERO).expect("sent");
            pane_.keys.into_inner()
        };
        assert_eq!(
            sent(&["w1:p2", "merge", "it", "now"], None),
            [
                "send w1:p2 From the orchestrator: merge it now",
                "key w1:p2 Enter"
            ]
        );
        assert_eq!(
            sent(&["--his", "w1:p2", "go ahead"], None),
            ["send w1:p2 go ahead", "key w1:p2 Enter"],
            "his words go as he wrote them"
        );
        assert_eq!(
            sent(&["--his", "w1:p2", "go ahead"], Some("rust-x")),
            ["send w1:p2 go ahead", "key w1:p2 Enter"]
        );
        assert_eq!(
            sent(&["w1:p2", "--his", "is", "a", "flag"], None),
            [
                "send w1:p2 From the orchestrator: --his is a flag",
                "key w1:p2 Enter"
            ],
            "--his after the pane is a word of the text"
        );
        assert_eq!(
            sent(&["review-x", "pushed the fix"], Some("rust-x")),
            [
                "send review-x From the task rust-x: pushed the fix",
                "key review-x Enter"
            ],
            "a task telling its reviewer is never the orchestrator"
        );
        assert_eq!(parse(&["w1:p2"]), None, "no text");
        assert_eq!(parse(&["--his", "w1:p2", " "]), None);
    }

    #[dash_test]
    fn pane_say_presses_enter_until_the_box_is_empty_and_types_nothing_into_a_dialog() {
        let text = "From the orchestrator: please pick the CV now, the lead is waiting on it";
        let stuck = format!("● done\n───────\n❯ {text}\n───────\n  ? for shortcuts");
        let pane = Pane::new(idle(), &[&stuck, &stuck, EMPTY]);
        say(&pane, "w9:m1", text, Duration::ZERO).expect("sent");
        let enters = pane
            .keys
            .borrow()
            .iter()
            .filter(|k| k.ends_with("Enter"))
            .count();
        assert_eq!(enters, 3, "Enter again while the box holds it");

        let pane = Pane::new(idle(), &[&stuck]);
        let why = say(&pane, "w9:m1", text, Duration::ZERO).expect_err("stuck");
        assert_eq!(
            why,
            "w9:m1: the message is still in its input box after 6 Enters"
        );

        let paste = "● done\n───────\n❯ [Pasted text #1 +12 lines]\n───────\n";
        let pane = Pane::new(idle(), &[paste]);
        assert!(
            say(&pane, "w9:m1", text, Duration::ZERO).is_err(),
            "a paste placeholder still waits"
        );

        let asking = Some(Seen {
            idle: false,
            asking: true,
            session: None,
        });
        let pane = Pane::new(asking, &[EMPTY]);
        let why = say(&pane, "w9:m1", text, Duration::ZERO).expect_err("a dialog");
        assert_eq!(why, "w9:m1 shows a dialog, nothing typed");
        assert!(pane.keys.borrow().is_empty(), "no key answers his question");

        let pane = Pane::new(None, &[EMPTY]);
        assert!(
            say(&pane, "w9:m1", text, Duration::ZERO).is_err(),
            "no agent, no shell line"
        );
        assert!(pane.keys.borrow().is_empty());

        let working = Some(Seen {
            idle: false,
            asking: false,
            session: None,
        });
        let pane = Pane::new(working, &[&stuck]);
        say(&pane, "w9:m1", text, Duration::ZERO).expect("an agent at work took it");
    }

    #[dash_test]
    fn the_box_is_the_last_ruled_part_and_a_suggestion_is_not_the_message() {
        assert_eq!(
            input_box("a\n───\n> hello\n  there\n───\nfooter"),
            "hello\n  there"
        );
        assert_eq!(input_box("no box here"), "");
        let tall = format!("───\n❯ x\n───\n{}", "line\n".repeat(9));
        assert_eq!(input_box(&tall), "", "a transcript under it, no footer");
        assert!(!holds(
            "pick",
            "pick the CV now please, the lead waits on it"
        ));
        assert!(holds(
            "pick the CV now please, the lead waits",
            "pick the CV now please, the lead waits on it"
        ));
        assert!(
            !holds(
                "pick the CV now please, the lead",
                "pick the CV now please, the lead waits on it"
            ),
            "under 30 characters of a longer message: a suggestion"
        );
        assert!(!holds(
            "something else he typed in the box",
            "pick the CV now please, the lead waits"
        ));
        assert!(holds("ok", "ok"), "a short message whole");
    }
}
