//! `dash hook pipe-cut`, Claude's PreToolUse hook on Bash: a `git push`, `dash ship` or `dash task add|update` piped into
//! `tail` or `head` runs with that stage dropped, through `updatedInput`, so the agent reads every pre-push failure and
//! every style problem (task push-tail-rewritten; @nha, 2026-10-29: « This should be second nature »). The other commands
//! of the call run as typed. It was drop_cuts() in .claude/hooks/pipe-keeps-exit.py (#981), removed with 21 hooks in #1120
//! and ported here, the one part of that hook its task asks for (his pick in the task's dialog, 2026-11-11).
//! A cut that is captured (`$( )`, backticks, `<( )`), piped on (`| tail | grep -q`) or written to a file cannot drop
//! cleanly and runs as typed, and so does a tail or head reading a file of its own (`| tail -n 20 log`). The call's exit
//! becomes the push's, not tail's: `git push | tail || echo retry` now retries on a failed push, as meant (/code-review).
//! Each cut that cannot drop stays alone; the others still go. ponytail: tail and head only; a push inside a quoted
//! `bash -c` string is blanked, and a push inside a loop or an if piped whole (`for ...; done | tail`, `if ...; fi | tail`)
//! is not seen past its first `;`: both keep their tail, nothing is changed. A `( )` or `{ }` group holding a push is read
//! whole, so its tail goes (`(git push; echo ok) | tail`).
use regex::Regex;
use serde_json::{Value, json};
use std::path::Path;
use std::sync::LazyLock;

/// The command words at command position whose every line matters, after a separator or an opener.
const RUNNERS: &str = r"(?:\w+=\S*\s+|(?:\S*/)?bounded\s+\d+\s+|(?:env|exec|nohup|time|command|sudo|xargs|!|do|then|else|elif|if|while|until)\s+)*";

static FULL: LazyLock<Regex> = LazyLock::new(|| {
    // git's own options before push (-C dir, -c k=v, --no-pager), never the folder of `git -C push`
    let git = r"(?:\S*/)?git(?:\s+-[cC]\s+\S+|\s+-(?:[^cC\s]\S*|[cC]\S+))*\s+push";
    let dash = r"(?:\S*/)?dash\s+(?:ship|task\s+(?:add|update))";
    Regex::new(&format!(
        r"(?:^|[;&|({{`\n])\s*{RUNNERS}(?:{git}|{dash})(?:[^\w.-]|$)"
    ))
    .expect("FULL")
});

static HEREDOC: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"^<<(-?)\s*['"]?([\w.-]+)['"]?"#).expect("heredoc"));

/// The command with heredoc bodies (their newlines too), quoted strings and `#` comments blanked, byte for byte, so its
/// offsets cut the command.
fn blank(cmd: &str) -> Vec<u8> {
    let b = cmd.as_bytes();
    let mut t = b.to_vec();
    let mut i = 0;
    // open $(( )): a << there is a shift, counted on code only, quotes and bodies skipped (/code-review)
    let mut arith = 0usize;
    // terminators whose body starts at the next newline, and whether `<<-` strips the tabs before them
    let mut pending: Vec<(String, bool)> = vec![];
    while i < b.len() {
        let c = b[i];
        if c == b'\n' && !pending.is_empty() {
            // the body runs to a line that is exactly the terminator, as bash reads it (/code-review)
            let (word, tabs) = pending.remove(0);
            let mut j = i + 1;
            let end = loop {
                let line_end = b[j..]
                    .iter()
                    .position(|&x| x == b'\n')
                    .map_or(b.len(), |p| j + p);
                let line = &cmd[j..line_end];
                let line = if tabs {
                    line.trim_start_matches('\t')
                } else {
                    line
                };
                if line == word || line_end == b.len() {
                    break line_end;
                }
                j = line_end + 1;
            };
            t[i..end].fill(b'x');
            i = end;
            continue;
        }
        if c == b'<'
            && b.get(i + 1) == Some(&b'<')
            && (i == 0 || b[i - 1] != b'<')
            && b.get(i + 2) != Some(&b'<')
            && arith == 0
            && let Some(m) = HEREDOC.captures(&cmd[i..])
        {
            pending.push((m[2].to_string(), !m[1].is_empty()));
            i += m[0].len();
            continue;
        }
        if c == b'\\' {
            // an escaped character is text, for every scanner below: \` opens nothing (/code-review)
            if let Some(x) = t.get_mut(i + 1) {
                *x = b'x';
            }
            i += 2;
            continue;
        }
        if b[i..].starts_with(b"((") {
            arith += 1;
            i += 2;
            continue;
        }
        if b[i..].starts_with(b"))") && arith > 0 {
            arith -= 1;
            i += 2;
            continue;
        }
        if c == b'\'' || c == b'"' {
            let mut j = i + 1;
            while j < b.len() && b[j] != c {
                j += if c == b'"' && b[j] == b'\\' { 2 } else { 1 };
            }
            t[i + 1..j.min(b.len())].fill(b'x'); // an unclosed quote runs to the end
            i = j + 1;
            continue;
        }
        if c == b'#' && (i == 0 || b" \t\n;|&(".contains(&b[i - 1])) {
            let end = b[i..]
                .iter()
                .position(|&x| x == b'\n')
                .map_or(b.len(), |p| i + p);
            t[i..end].fill(b' ');
            i = end;
            continue;
        }
        i += 1;
    }
    t
}

/// A `| tail` or `| head` stage at `at` (never `||`): the end of its command word.
fn cut_at(t: &[u8], at: usize) -> Option<usize> {
    if t.get(at) != Some(&b'|') || (at > 0 && t[at - 1] == b'|') || t.get(at + 1) == Some(&b'|') {
        return None;
    }
    let mut j = at + 1;
    if t.get(j) == Some(&b'&') {
        j += 1;
    }
    while j < t.len() && t[j].is_ascii_whitespace() {
        j += 1;
    }
    let start = j;
    while j < t.len() && !t[j].is_ascii_whitespace() && !b";|&()<>`{}".contains(&t[j]) {
        j += 1;
    }
    let word = &t[start..j];
    let base = word.rsplit(|&c| c == b'/').next().unwrap_or(word);
    (base == b"tail" || base == b"head").then_some(j)
}

/// The separator ending the stage that starts at `at`: (its start, its length), or None at the end. A `$( )`, `${ }` or
/// backtick pair in tail's own arguments (`tail -n ${N:-5}`) is part of the stage.
fn stage_end(t: &[u8], at: usize) -> Option<(usize, usize)> {
    let (mut depth, mut tick, mut i) = (0usize, false, at);
    while i < t.len() {
        let c = t[i];
        let next = t.get(i + 1).copied();
        let len = match c {
            b'\\' => {
                i += 2;
                continue;
            }
            b'`' => {
                tick = !tick;
                i += 1;
                continue;
            }
            b'(' | b'{' => {
                depth += 1;
                i += 1;
                continue;
            }
            b')' | b'}' if depth > 0 => {
                depth -= 1;
                i += 1;
                continue;
            }
            b'&' if next == Some(b'&') => 2,
            b'|' if next == Some(b'|') || next == Some(b'&') => 2,
            b'|' | b';' | b'\n' | b')' | b'}' => 1,
            b'&' if next != Some(b'>') && (i == 0 || !b"<>".contains(&t[i - 1])) => 1,
            _ => {
                i += 1;
                continue;
            }
        };
        if depth == 0 && !tick {
            return Some((i, len));
        }
        i += len;
    }
    None
}

/// The pipeline left of the pipe at `at`: back to a `;`, `&&`, `||`, a lone `&`, a newline or an opener left open, a group
/// closed before it read whole, a closed backtick pair too (/code-review).
fn upstream(t: &[u8], at: usize) -> &[u8] {
    let (mut depth, mut i) = (0usize, at);
    let mut ticks = t[..at].iter().filter(|&&x| x == b'`').count(); // the backticks in t[..i]
    while i > 0 {
        let c = t[i - 1];
        if c == b'`' && ticks % 2 == 0 {
            // a closing backtick: on to the one that opens it, none between
            i = t[..i - 1].iter().rposition(|&x| x == b'`').unwrap_or(0);
            ticks = ticks.saturating_sub(2);
            continue;
        }
        let pair = if i >= 2 { &t[i - 2..i] } else { &t[..0] };
        if c == b')' || c == b'}' {
            depth += 1;
        } else if (c == b'(' || c == b'{') && depth > 0 {
            depth -= 1;
        } else if b"({`".contains(&c)
            || depth == 0
                && (c == b';'
                    || c == b'\n'
                    || pair == b"&&"
                    || pair == b"||"
                    || c == b'&'
                        && (i < 2 || !b"<>|".contains(&t[i - 2]))
                        && !matches!(t.get(i), Some(b'>' | b'&')))
        {
            break;
        }
        i -= 1;
    }
    &t[i..at]
}

/// Whether the end of `t` sits inside an open `$( )`, `<( )`, `>( )` or backtick pair.
fn in_substitution(t: &[u8]) -> bool {
    let mut stack: Vec<u8> = vec![]; // b'$' a capture, b'(' a group, b'`' a backtick
    let mut i = 0;
    while i < t.len() {
        match t[i] {
            b'\\' => i += 1,
            b'`' if stack.last() == Some(&b'`') => {
                stack.pop();
            }
            b'`' => stack.push(b'`'),
            b'(' if i > 0 && b"$<>".contains(&t[i - 1]) => stack.push(b'$'),
            b'(' => stack.push(b'('),
            b')' if stack.last().is_some_and(|&s| s != b'`') => {
                stack.pop();
            }
            _ => {}
        }
        i += 1;
    }
    stack.iter().any(|&s| s == b'$' || s == b'`')
}

/// Whether tail's own arguments `t` change what dropping it does: its stdout sent to a file (`> log`, `-1>log`, `&> log`;
/// never an fd copy `2>&1`, `>&2`, nor a `2>` of its stderr), or a file read in place of the push's output (`tail -n 20
/// log`, `< log`) (/code-review).
fn keeps_stage(t: &[u8]) -> bool {
    let to_file = (0..t.len()).any(|i| {
        if t[i] != b'>' || (i > 0 && t[i - 1] == b'>') {
            return false;
        }
        // a run of digits that starts a word names the fd: only 1 is stdout
        let k = t[..i]
            .iter()
            .rposition(|x| !x.is_ascii_digit())
            .map_or(0, |p| p + 1);
        let fd = k < i && (k == 0 || t[k - 1].is_ascii_whitespace());
        let after = if t.get(i + 1) == Some(&b'>') {
            i + 2
        } else {
            i + 1
        };
        (!fd || &t[k..i] == b"1") && t.get(after) != Some(&b'&')
    });
    // tail's words, a $( ), ${ } or backtick pair kept whole
    let (mut words, mut word, mut depth, mut tick) = (vec![], vec![], 0usize, false);
    for &c in t {
        match c {
            b'`' => tick = !tick,
            b'(' | b'{' => depth += 1,
            b')' | b'}' => depth = depth.saturating_sub(1),
            _ => {}
        }
        if c.is_ascii_whitespace() && depth == 0 && !tick {
            if !word.is_empty() {
                words.push(std::mem::take(&mut word));
            }
        } else {
            word.push(c);
        }
    }
    words.extend((!word.is_empty()).then_some(word));
    let mut words = words.iter();
    let mut reads_file = false;
    while let Some(w) = words.next() {
        if w.contains(&b'<') {
            reads_file = true;
        } else if w.contains(&b'>') {
            if w.ends_with(b">") {
                words.next(); // `2> /dev/null`: its target
            }
        } else if [
            &b"-n"[..],
            b"-c",
            b"-s",
            b"--lines",
            b"--bytes",
            b"--pid",
            b"--sleep-interval",
        ]
        .contains(&w.as_slice())
        {
            words.next(); // the option's value
        } else if !w.starts_with(b"-") && !w.starts_with(b"+") {
            reads_file = true;
        }
    }
    to_file || reads_file
}

/// The command with each tail or head stage after a push or a `dash task add|update` dropped, or None when there is none.
/// A cut that cannot drop cleanly (captured, piped on, written to a file, reading a file) stays as typed, alone.
pub fn drop_cuts(cmd: &str) -> Option<String> {
    let flat = cmd.replace("\\\n", "  ");
    let t = blank(&flat);
    let mut cuts = vec![];
    let mut at = 0;
    while at < t.len() {
        let Some(mut word_end) = cut_at(&t, at) else {
            at += 1;
            continue;
        };
        let s = at;
        if !FULL.is_match(&String::from_utf8_lossy(upstream(&t, s))) {
            at += 1;
            continue;
        }
        let mut stuck = in_substitution(&t[..s]);
        let e = loop {
            let end = stage_end(&t, word_end);
            // a heredoc body is blanked from the newline ending this line: the stage stops there too
            let line_end = flat[word_end..]
                .find('\n')
                .map_or(t.len(), |p| word_end + p);
            let e = end.map_or(t.len(), |(p, _)| p).min(line_end);
            stuck |= keeps_stage(&t[word_end..e]);
            // a | or |& goes on to another stage; || does not
            let piped =
                end.is_some_and(|(p, _)| p == e && t[p] == b'|' && t.get(p + 1) != Some(&b'|'));
            match piped.then(|| cut_at(&t, e)).flatten() {
                Some(next_end) => word_end = next_end, // | tail -20 | head -5: the whole chain goes
                None => {
                    stuck |= piped; // | tail | grep -q x: dropping tail changes what grep reads
                    break e;
                }
            }
        };
        if !stuck {
            cuts.push((s, e));
        }
        at = e.max(s + 1);
    }
    if cuts.is_empty() {
        return None;
    }
    let mut out = cmd.to_string();
    for (s, e) in cuts.into_iter().rev() {
        out.replace_range(s..e, "");
    }
    Some(out)
}

// the name its firings are recorded and listed under
use crate::firings::PIPE_CUT as FIRING;

/// The hook's answer to Claude's PreToolUse event, or None to let the call run as typed. No permissionDecision: the call
/// still goes through the session's own permission rules, never auto-approved by a rewrite.
/// Each rewrite is a firing, a line `pipe_cut\t<UTC day number>` appended to `firings`
/// (`~/.claude/run/gate-firings.tsv`), which `dash gate firings` counts: a check is kept only while it fires (CLAUDE.md:
/// « a new check needs a test and a count of its firings »). A firing it cannot write never blocks the call.
pub fn answer(event: &Value, firings: Option<&Path>) -> Option<Value> {
    let input = event.get("tool_input")?;
    let new = drop_cuts(input.get("command")?.as_str()?)?;
    if let Some(file) = firings {
        let _ = crate::firings::record(file, FIRING);
    }
    let mut updated = input.clone();
    updated["command"] = Value::String(new.clone());
    Some(json!({"hookSpecificOutput": {
        "hookEventName": "PreToolUse",
        "updatedInput": updated,
        "additionalContext": format!(
            "dash hook pipe-cut dropped the tail or head after a push, dash ship or dash task add or update, so every line shows. Ran: {new}"
        ),
    }}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::TestDir;
    use dash_meter::dash_test;

    fn hook(cmd: &str) -> Option<Value> {
        answer(
            &json!({"hook_event_name": "PreToolUse", "tool_name": "Bash",
                       "tool_input": {"command": cmd, "description": "d", "timeout": 600000}}),
            None,
        )
    }

    /// Done-when 1: a push piped into tail or head comes back as updatedInput with that stage dropped, allowed (no deny).
    #[dash_test]
    fn push_into_tail_is_rewritten_and_allowed() {
        let out = hook("git push -u origin HEAD 2>&1 | tail -5").expect("an answer");
        let o = &out["hookSpecificOutput"];
        assert_eq!(
            o["updatedInput"]["command"],
            "git push -u origin HEAD 2>&1 "
        );
        assert_eq!(
            o["updatedInput"]["description"], "d",
            "the other tool_input fields stay"
        );
        assert_eq!(o["updatedInput"]["timeout"], 600000);
        assert!(o.get("permissionDecision").is_none(), "never a deny: {o}");
        for (c, want) in [
            ("git push | tail", "git push "),
            ("cd a && git push |& head -3", "cd a && git push "),
            (
                "git -C x push | grep -v y | tail",
                "git -C x push | grep -v y ",
            ),
            ("git push | tail -n ${N:-5}", "git push "),
            ("git push | tail -n `echo 5` && ls", "git push && ls"),
            (
                "echo origin | xargs git push 2>&1 | tail -3",
                "echo origin | xargs git push 2>&1 ",
            ),
            ("git push | tail &", "git push &"),
            ("bin/dash ship m a | tail", "bin/dash ship m a "),
            ("git push|tail", "git push"),
            ("{ git push; } 2>&1 | tail", "{ git push; } 2>&1 "),
            ("(git push; echo ok) | tail", "(git push; echo ok) "),
            ("/usr/bin/git push | tail", "/usr/bin/git push "),
            ("env A=1 git push | tail", "env A=1 git push "),
            ("git push \\\n  | tail", "git push \\\n  "),
            ("git push | tail -20 | head -5; ls", "git push ; ls"),
            ("git push | tail -1 >&2", "git push "),
            ("git push | tail 2>/dev/null", "git push "),
            (
                "if git push | tail -3; then ls; fi",
                "if git push ; then ls; fi",
            ),
            ("! git push | tail", "! git push "),
            // /code-review: the exit becomes the push's, a shift is no heredoc, a closed backtick pair is read whole,
            // `<<-` strips tabs, and a stuck cut leaves the others to drop
            ("git push | tail -3 || echo retry", "git push || echo retry"),
            (
                "n=$((1<<2))\ngit push 2>&1 | tail -5",
                "n=$((1<<2))\ngit push 2>&1 ",
            ),
            (
                "git push origin `git branch --show-current` 2>&1 | tail -3",
                "git push origin `git branch --show-current` 2>&1 ",
            ),
            (
                "cat <<-EOF\n\tEOF\ngit push | tail",
                "cat <<-EOF\n\tEOF\ngit push ",
            ),
            // second /code-review: a hyphenated heredoc word, quotes holding (( or )), an escaped backtick, tail -s
            (
                "cat <<'END-MSG'\nbody\nEND-MSG\ngit push | tail",
                "cat <<'END-MSG'\nbody\nEND-MSG\ngit push ",
            ),
            (
                "echo \"))\"; x=$((1<<2)); git push | tail -3",
                "echo \"))\"; x=$((1<<2)); git push ",
            ),
            (
                "printf \\` ; git push origin `git rev-parse HEAD` | tail",
                "printf \\` ; git push origin `git rev-parse HEAD` ",
            ),
            ("git push | tail -s 1 -f", "git push "),
            (
                "git push 2>&1 | tail -3; id=$(dash task add x | head -1)",
                "git push 2>&1 ; id=$(dash task add x | head -1)",
            ),
        ] {
            assert_eq!(drop_cuts(c).as_deref(), Some(want), "{c}");
        }
        // left as typed: no push upstream, the push elsewhere, or a cut that cannot drop cleanly
        for c in [
            "git push",
            "git push 2>&1 | grep -v remote",
            "mix lint | tail",
            "git log | tail",
            "git commit -m 'git push | tail'",
            "git push && ls | tail",
            "git push; git log | head",
            "git pushx | tail",
            "mygit push | tail",
            "git push 2>&1 & ls | tail",
            "git push origin $(git branch --show-current | head -1)",
            "echo git push | tail",
            "git -C push status | head",
            "git push || ls | tail",
            "git push; (ls) | tail",
            "echo $(git push 2>&1 | tail -1)",
            "x=$(git push 2>&1 | head)",
            "read id < <(dash task add x | head -1)",
            "git push | tail | grep -q x",
            "git push 2>&1 | tail -1 > log; cat log",
            "git push | tail &> log",
            // /code-review: a body line only starting with the terminator, a redirect with no blank, a file of its own
            "cat > notes.md <<'EOF'\nEOF is the marker\nrun: git push | tail -3\nEOF",
            "cat <<EOF\n  EOF\ngit push | tail\nEOF",
            "git push 2>&1 | tail -1>log; cat log",
            "git push 2>&1 | tail -n 20 /tmp/prepush.log",
            "git push | tail < log",
            "echo \":((\"; cat > notes.md <<'EOF'\nrun: git push | tail -3\nEOF",
        ] {
            assert_eq!(drop_cuts(c), None, "{c}");
            assert!(hook(c).is_none(), "{c}");
        }
    }

    /// Done-when 2: the rewritten call keeps the other commands of the same call, a gh pr comment after the push included.
    #[dash_test]
    fn rewrite_keeps_the_rest_of_the_call() {
        let out = hook("git push 2>&1 | tail -3 && gh pr comment 5 --body 'see | tail'")
            .expect("an answer");
        assert_eq!(
            out["hookSpecificOutput"]["updatedInput"]["command"],
            "git push 2>&1 && gh pr comment 5 --body 'see | tail'"
        );
        assert_eq!(
            drop_cuts("git push 2>&1 | tail; echo done").as_deref(),
            Some("git push 2>&1 ; echo done")
        );
        assert_eq!(
            drop_cuts("git push | tail -n $(echo 5); ls").as_deref(),
            Some("git push ; ls")
        );
    }

    /// Done-when 3: dash task add or update piped into tail runs with the stage dropped, so every style problem shows.
    #[dash_test]
    fn task_add_or_update_into_tail_drops_the_stage() {
        let out = hook("private/dashboard/bin/dash task add x --title t 2>&1 | tail -5")
            .expect("an answer");
        assert_eq!(
            out["hookSpecificOutput"]["updatedInput"]["command"],
            "private/dashboard/bin/dash task add x --title t 2>&1 "
        );
        for (c, want) in [
            (
                "dash task add x --why - <<'EOF' 2>&1 | tail -5\nwhy | tail\nEOF",
                "dash task add x --why - <<'EOF' 2>&1 \nwhy | tail\nEOF",
            ),
            (
                "private/dashboard/bin/dash task update x --title t | head -3; git log | head -1",
                "private/dashboard/bin/dash task update x --title t ; git log | head -1",
            ),
            (
                "for t in a b; do dash task add $t | tail -3; done",
                "for t in a b; do dash task add $t ; done",
            ),
        ] {
            assert_eq!(drop_cuts(c).as_deref(), Some(want), "{c}");
        }
        for c in [
            "dash task show x | tail",
            "dash task list | head",
            "echo dash task add | tail",
            "id=`dash task add x | head -1`",
        ] {
            assert_eq!(drop_cuts(c), None, "{c}");
        }
    }

    /// Done-when 1, for dash ship: a tail or head stage after `dash ship` is dropped, the rest of the call kept.
    #[dash_test]
    fn dash_ship_into_tail_drops_the_stage() {
        let out =
            hook("private/dashboard/bin/dash ship \"m\" a b 2>&1 | tail -3").expect("an answer");
        assert_eq!(
            out["hookSpecificOutput"]["updatedInput"]["command"],
            "private/dashboard/bin/dash ship \"m\" a b 2>&1 "
        );
        assert_eq!(
            drop_cuts("dash ship m a | head -5; git log | head -1").as_deref(),
            Some("dash ship m a ; git log | head -1")
        );
        assert_eq!(drop_cuts("dash shipx m a | tail"), None);
    }

    /// Each rewrite adds one `pipe_cut` line to the firings file `dash gate firings` counts; a call left as typed adds none.
    #[dash_test]
    fn each_rewrite_is_counted_as_a_firing() {
        let dir = TestDir::new("pipe-cut-firings");
        let file = dir.0.join("run/gate-firings.tsv");
        let event = |cmd: &str| json!({"tool_input": {"command": cmd}});
        assert!(answer(&event("git log | tail"), Some(&file)).is_none());
        assert!(!file.exists(), "a call left as typed is no firing");
        assert!(answer(&event("git push | tail"), Some(&file)).is_some());
        assert!(answer(&event("dash ship m a | head"), Some(&file)).is_some());
        let text = std::fs::read_to_string(&file).expect("the firings");
        let lines: Vec<_> = text.lines().collect();
        assert_eq!(lines.len(), 2, "{text}");
        for line in lines {
            let (rule, day) = line.split_once('\t').expect("a tab");
            assert_eq!(rule, "pipe_cut");
            assert!(day.parse::<u64>().expect("a day number") > 20_000, "{day}");
        }
    }
}
