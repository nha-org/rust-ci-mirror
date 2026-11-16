//! A field write refuses a value that reads as an EDN or JSON list or map (task field-list-refused; @nha, 2026-10-22, CLAUDE.md:
//! « A store write takes each field as a flag, never EDN »): on 2026-10-30 `dash draft new … --blocks '["a" "b"]'` saved one
//! string on a draft. A list is the flag given once per item, a map one `--<field>.<key>` per key, as the older app's
//! `Records.fields/1` reads them. Checked at the front door, before the older app writes anything.

/// The commands whose words after their id are `--<field> <text>` pairs: every caller of `Records.fields/1` and `task_flags/1`.
const WRITES: [(&str, &str); 10] = [
    ("task", "add"),
    ("task", "update"),
    ("block", "add"),
    ("block", "set"),
    ("contact", "add"),
    ("contact", "set"),
    ("draft", "new"),
    ("draft", "set"),
    ("store", "set"),
    ("store", "conj"),
];

/// A flag as the older app's `fields/1` reads a field's name (`field_pair/3`: `--`, a letter or digit, then name characters),
/// which holds every word its `flag?/1` reads as one, so both walks pair the same words. Prose that opens on `--`
/// (« --force is refused ») and a `--<field>=<text>` word are texts, never flags (/code-review medium).
fn is_flag(w: &str) -> bool {
    w.strip_prefix("--").is_some_and(|n| {
        n.starts_with(|c: char| c.is_ascii_alphanumeric())
            && (n.chars()).all(|c| c.is_ascii_alphanumeric() || "_.?!-".contains(c))
    })
}

/// Each flag of `words` with the text after it, none when the next word is a flag or the words end: the one walk the door's
/// checks read a write's fields by (`refused` here, `dedupe::values`; /code-review medium: each had its own reading of a flag).
pub fn pairs(words: &[String]) -> Vec<(&str, Option<&str>)> {
    let mut out = vec![];
    let mut at = 0;
    while at < words.len() {
        let value = words.get(at + 1).filter(|v| !is_flag(v));
        if is_flag(&words[at]) {
            out.push((words[at].as_str(), value.map(String::as_str)));
            at += usize::from(value.is_some());
        }
        at += 1;
    }
    out
}

/// A text that is one whole list or map: its opening `[` or `{` closes, on its own kind, on its last character. Prose that
/// opens on a bracket (« [WIP] port [ci] », « [Prénom], merci ») is no list (/code-review medium; the task's title: « a value
/// that looks like an EDN or JSON list »). A bracket inside a "quoted" item is text, so `["a]" "b"]` is still one list.
fn listed(v: &str) -> bool {
    let v = v.trim();
    let (mut open, mut quoted, mut escaped) = (vec![], false, false);
    for (at, c) in v.char_indices() {
        match c {
            _ if escaped => escaped = false,
            '\\' if quoted => escaped = true,
            '"' => quoted = !quoted,
            _ if quoted => {}
            '[' => open.push(']'),
            '{' => open.push('}'),
            ']' | '}' if open.pop() != Some(c) => return false,
            _ => {}
        }
        if open.is_empty() {
            return at > 0 && at + c.len_utf8() == v.len();
        }
    }
    false
}

/// One line per flag of a field write whose text is a list or map, naming the repeat-the-flag form; none for any other
/// command. A `-` is `stdin`, the text bin/dash read for it. A draft's `--body` is its message and no field, given once, so
/// it is never read here, piped or inline (/code-review medium). The line quotes the text's first 60 characters.
pub fn refused(args: &[String], stdin: &str) -> Vec<String> {
    let [noun, verb, rest @ ..] = args else {
        return vec![];
    };
    if !WRITES.contains(&(noun.as_str(), verb.as_str())) {
        return vec![];
    }
    // a task's flags take no `.<key>` (`task_flags/1`)
    let keyed = |flag: &str| match noun.as_str() {
        "task" => String::new(),
        _ => format!(", or one --{flag}.<key> <text> per key"),
    };
    let mut out = vec![];
    for (flag, value) in pairs(rest) {
        let v = match value {
            _ if (noun.as_str(), flag) == ("draft", "--body") => continue,
            Some("-") => stdin.trim(),
            Some(v) => v.trim(),
            None => continue,
        };
        if listed(v) {
            let flag = &flag[2..];
            let shown: String = v.chars().take(60).collect();
            let cut = if shown.len() < v.len() { "…" } else { "" };
            out.push(format!(
                "--{flag} {shown}{cut}: refused, a list or map is never one flag's text (it would be saved as one string); \
                 repeat the flag once per item, --{flag} <item> --{flag} <item>{}",
                keyed(flag)
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;

    fn words(ws: &[&str]) -> Vec<String> {
        ws.iter().map(|w| w.to_string()).collect()
    }

    /// Task field-list-refused, done-when 1 (the Records test: `Records.fields/1`'s callers, at Rust's door): draft new and
    /// every field write refuse a text that is a list or map, naming the repeat-the-flag form; prose, ids and stdin pass.
    #[dash_test]
    fn a_field_write_refuses_a_list_or_map_and_names_the_repeated_flag() {
        for (noun, verb) in WRITES {
            let args = words(&[
                noun,
                verb,
                "x",
                "--blocks",
                r#"["a" "b"]"#,
                "--why",
                "{:a 1}",
            ]);
            let said = refused(&args, "");
            assert_eq!(said.len(), 2, "{noun} {verb}: {said:?}");
            assert!(
                said[0].contains("--blocks <item> --blocks <item>"),
                "{said:?}"
            );
            assert!(said[1].starts_with("--why {:a 1}: refused"), "{said:?}");
            // a map's key form only where the write reads one
            let keyed = said[1].contains("--why.<key> <text>");
            assert_eq!(keyed, noun != "task", "{noun} {verb}: {said:?}");
        }
        let ok = words(&[
            "draft", "new", "a.md", "--blocks", "a", "--blocks", "b", "--body", "-",
        ]);
        assert_eq!(refused(&ok, "[a message]"), Vec::<String>::new());
        // a field's `-` is read as the stdin it stands for
        let piped = words(&["contact", "set", "c", "--notes", "-"]);
        assert_eq!(refused(&piped, "{:a 1}\n").len(), 1);
        assert_eq!(refused(&piped, "[2026-11-01] call"), Vec::<String>::new());
        // prose opening on a bracket, a bracket inside, a flag with no text before the next, ids: no list
        let ok = words(&[
            "task",
            "update",
            "a",
            "b",
            "--dry",
            "--title",
            "[WIP] x [y]",
        ]);
        assert_eq!(refused(&ok, ""), Vec::<String>::new());
        let ok = words(&[
            "block",
            "set",
            "b1",
            "--short",
            "[Prénom], merci",
            "--why",
            "{a} and {b}.",
        ]);
        assert_eq!(refused(&ok, ""), Vec::<String>::new());
        // a `---` text is no flag, so the list after its flag is still read as that flag's
        let after = words(&["draft", "set", "d", "--sig", "---", "--blocks", "[a]"]);
        assert_eq!(refused(&after, "").len(), 1, "{:?}", refused(&after, ""));
        // a draft's message is no field, piped or inline; another write's --body is one
        let body = words(&["draft", "set", "d", "--body", "[Prénom]"]);
        assert_eq!(refused(&body, ""), Vec::<String>::new());
        let body = words(&["store", "set", "f", "--body", "[a]"]);
        assert_eq!(refused(&body, "").len(), 1);
        // a bracket inside a quoted item, a field named on a digit: each still a list
        let quoted = words(&[
            "draft",
            "new",
            "a.md",
            "--blocks",
            r#"["a]" "b\"["]"#,
            "--2fa",
            "[a]",
        ]);
        assert_eq!(refused(&quoted, "").len(), 2, "{:?}", refused(&quoted, ""));
        // a close of the other kind, prose that opens on `--`, an open quote: no list
        let ok = words(&[
            "block",
            "set",
            "b",
            "--short",
            "[a}",
            "--why",
            "--force is refused",
            "--x",
            r#"[5" wide]"#,
        ]);
        assert_eq!(refused(&ok, ""), Vec::<String>::new());
        // a long text is quoted by its first 60 characters
        let long = format!("[{}]", "a".repeat(200));
        let said = refused(&words(&["contact", "set", "c", "--notes", "-"]), &long);
        let start = format!("--notes [{}…: refused", "a".repeat(59));
        assert!(said[0].starts_with(&start), "{said:?}");
        // a read is no write
        assert_eq!(
            refused(&words(&["store", "text", "--x", "[a]"]), ""),
            Vec::<String>::new()
        );
    }
}
