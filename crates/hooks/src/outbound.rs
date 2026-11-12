//! The one gate every message to a person passes before it leaves: nothing banned, nothing that reads as AI. `dash hook
//! outbound-gate` (Claude Code's PreToolUse on the LinkedIn sends, the browser's typed or filled text, any Gmail send) and
//! `dash outbound check` (klar-mail's and freework_apply.py's send, a text a session is about to paste). Ported from
//! .claude/hooks/outbound_gate.py (task rust-hooks-rest-port; @nha, 2026-11-10: « all these python scripts are also very
//! brittle. we want to move all logic to rust and have it well tested. »), which keeps only what the frozen Elixir runs.
//!
//! (@nha, 2026-10-21: « This is yours to own. Banned lines never ever appear in ourside comms and this is enforced by tests.
//! All comms run throug JEV for readability and robotic score or any other external or internal tool we can find »). On the
//! exact text that leaves:
//! 1. banned-phrases.edn with its scopes: :cold rows for a cold message, :first rows for one sent before they replied. A text
//!    no draft stands behind is judged cold and first, the strictest. A row this regex engine cannot read refuses.
//! 2. Jev's verdicts (private/ats/draft-questions.json): reads_ai_written and too_long, from its log private/ats/jev.jsonl,
//!    else asked now. `:long-ok` lifts too_long only. No verdict, no send.
//! 3. Local tells, free: priv/ai_isms.json (the em-dash first), LIX over READABLE_MAX on a message of 40 words or more.
//!
//! Every failure refuses: a gate that cannot judge lets nothing through.
use crate::send::truthy;
use regex::Regex;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::Duration;
use unicode_normalization::UnicodeNormalization;

/// A literal regex, compiled once a process: the hook judges each tool call (/code-review).
macro_rules! re {
    ($s:expr) => {{
        static R: std::sync::LazyLock<Regex> =
            std::sync::LazyLock::new(|| Regex::new($s).expect($s));
        &*R
    }};
}

/// LIX (Björnsson): words per sentence plus the share of words over six letters; over 60 is « very hard » on its scale.
const READABLE_MAX: f64 = 60.0;
const READABLE_WORDS: usize = 40;
const LINKEDIN: &str = "mcp__mcp-server-linkedin__";
const CHROME: &str = "mcp__claude-in-chrome__";
const LINKEDIN_SENDS: [&str; 5] = [
    "send_message",
    "reply_in_thread",
    "message_job_poster",
    "connect_with_person",
    "easy_apply",
];
/// A browser text this long is a message, not a search or a field.
const FORM_MIN: usize = 200;
const GATED: [(&str, &str); 2] = [
    ("reads_ai_written", "reads_ai_written_max"),
    ("too_long", "too_long_max"),
];
/// The paid repeats `ats.py repeat` logs beside a key.
const REPEATS: usize = 5;
/// The hook's own bound, under its 200 s in .claude/settings.json: a hook that times out lets the call through.
pub(crate) const HOOK_DEADLINE: Duration = Duration::from_secs(150);
const GMAIL_REFUSED: &str = "Outbound gate: nothing sent. The Gmail connector skips the gate and writes from the wrong mailbox: private/dashboard/bin/dash mail send <id>.";

/// The checkout the gate reads: the session's (CLAUDE_PROJECT_DIR), else dash's repo.
pub(crate) fn root() -> PathBuf {
    crate::project_dir()
        .map(PathBuf::from)
        .unwrap_or_else(crate::host().repo)
}

/// Jev's (answers, gate) for a text, or why it gave none.
pub(crate) type Verdict = Result<(Value, Value), String>;

/// A banned-phrases.edn row: a regex or a plain match, its scope (cold, first, or none).
pub(crate) struct Row {
    /// none for a plain match; a regex this engine cannot read is its error
    pattern: Option<Result<Regex, String>>,
    s: String,
    label: String,
    scope: Option<String>,
    instead: String,
}

/// An AI tell of priv/ai_isms.json.
pub(crate) struct Ism(Regex, String, String);

/// What the gate judges with: the tables, the open drafts, and Jev (a stub in tests).
pub(crate) struct Gate<'a> {
    pub rows: Vec<Row>,
    pub isms: Vec<Ism>,
    pub drafts: Vec<Value>,
    pub ask: &'a dyn Fn(&str) -> Verdict,
}

/// A value of the model's tagged JSON as its bare name: "~:x" and "~$x" lose the tag, "~~x" its escaping ~, a tagged set or
/// list is its list (europass.untag).
fn untag(v: &Value) -> Value {
    if let Some(m) = v.as_object()
        && m.len() == 1
        && let Some(inner) = m.get("~#set").or_else(|| m.get("~#list"))
    {
        return inner.clone();
    }
    match v.as_str() {
        Some(s) if s.starts_with("~~") => Value::String(s[1..].into()),
        Some(s) if s.starts_with("~:") || s.starts_with("~$") => Value::String(s[2..].into()),
        _ => v.clone(),
    }
}

fn text_of(v: &Value) -> Option<String> {
    untag(v)
        .as_str()
        .filter(|s| !s.is_empty())
        .map(String::from)
}

/// banned-phrases.edn's rows from its JSON (`dash store text banned-phrases.edn`). A table of no row refuses, as a missing
/// one does: it would pass every phrase.
pub(crate) fn rows(text: &str) -> Result<Vec<Row>, String> {
    let all: Value = serde_json::from_str(text).map_err(|e| format!("banned-phrases.edn: {e}"))?;
    let all = untag(&all);
    let mut out = vec![];
    for row in all
        .as_array()
        .into_iter()
        .flatten()
        .filter(|r| r.is_object())
    {
        let Some((kind, raw)) = ["match", "regex"]
            .into_iter()
            .find_map(|k| row[k].as_str().map(|s| (k, s)))
        else {
            continue;
        };
        let s = text_of(&Value::String(raw.into())).unwrap_or_default();
        let pattern = (kind == "regex").then(|| Regex::new(&s).map_err(|e| e.to_string()));
        out.push(Row {
            pattern,
            label: text_of(&row["label"]).unwrap_or_else(|| s.clone()),
            scope: text_of(&row["scope"]),
            instead: text_of(&row["instead"]).unwrap_or_default(),
            s,
        });
    }
    if out.is_empty() {
        return Err("banned-phrases.edn reads as no row".into());
    }
    Ok(out)
}

/// priv/ai_isms.json's tells, case-insensitive; a checkout without it still checks the em-dash.
pub(crate) fn isms(root: &Path) -> Result<Vec<Ism>, String> {
    let path = root.join("private/dashboard/priv/ai_isms.json");
    let rows = match std::fs::read_to_string(&path) {
        Ok(t) => serde_json::from_str::<Value>(&t)
            .map_err(|e| format!("{}: {e}", path.display()))?["rows"]
            .clone(),
        Err(_) => {
            json!([{"regex": "—", "label": "an em-dash", "instead": "a period, a comma, a colon or parentheses"}])
        }
    };
    rows.as_array()
        .into_iter()
        .flatten()
        .map(|r| {
            let rx = r["regex"].as_str().unwrap_or_default();
            let rx = Regex::new(&format!("(?i){rx}"))
                .map_err(|e| format!("ai_isms.json « {rx} »: {e}"))?;
            Ok(Ism(
                rx,
                r["label"].as_str().unwrap_or_default().into(),
                r["instead"].as_str().unwrap_or_default().into(),
            ))
        })
        .collect()
}

/// Lowercase, one apostrophe, entities decoded, and LinkedIn's Unicode bold (`dash bold`) back to letters.
/// An entity with no `;` decoded too, as Python's html.unescape and a browser read `caf&eacute ` (/code-review of PR 1480).
/// ponytail: only one followed by a non-alphanumeric; html.unescape's longest legacy prefix (`&eacutex`) if it ever matters.
fn norm(text: &str) -> String {
    let semi = re!(r"&(#[0-9]+|[A-Za-z][A-Za-z0-9]*)(?:[^A-Za-z0-9;]|$)");
    let closed = semi.replace_all(text, |c: &regex::Captures| {
        let (all, name) = (&c[0], &c[1]);
        let entity = format!("&{name};");
        let decoded = html_escape::decode_html_entities(&entity);
        match decoded == entity {
            true => all.to_string(),
            false => format!("{decoded}{}", &all[1 + name.len()..]),
        }
    });
    html_escape::decode_html_entities(&closed)
        .nfkc()
        .collect::<String>()
        .to_lowercase()
        .replace('’', "'")
}

fn flat(text: &str) -> String {
    norm(text)
        .replace("**", "")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// (LIX, words)
fn lix(text: &str) -> (f64, usize) {
    let word = re!(r"[^\W\d_]+(?:['’-][^\W\d_]+)*");
    let stop = re!(r"[.!?]+(?:\s|$)|\n+");
    let has_word = re!(r"\w");
    let cleaned = text.replace("**", "");
    let words: Vec<&str> = word.find_iter(&cleaned).map(|m| m.as_str()).collect();
    let sentences = stop
        .split(text)
        .filter(|s| has_word.is_match(s))
        .count()
        .max(1);
    let n = words.len();
    let long = words.iter().filter(|w| w.chars().count() > 6).count();
    (
        n as f64 / sentences as f64 + 100.0 * long as f64 / n.max(1) as f64,
        n,
    )
}

/// `d.get(k, default)`'s truth
fn flag(d: &Value, k: &str, default: bool) -> bool {
    d.get(k).map_or(default, truthy)
}

impl Gate<'_> {
    fn banned(&self, text: &str, cold: bool, first: bool) -> Vec<String> {
        let t = norm(text);
        let mut hits = vec![];
        for r in &self.rows {
            match r.scope.as_deref() {
                Some("cold") if !cold => continue,
                Some("first") if !(first || cold) => continue,
                None | Some("cold") | Some("first") => {}
                Some(_) => continue,
            }
            let hit = match &r.pattern {
                Some(Ok(rx)) => rx.is_match(&t),
                None => t.contains(&norm(&r.s)),
                // fails closed: a row this engine cannot read would pass every text
                Some(Err(e)) => {
                    hits.push(format!(
                        "banned phrase row « {} » cannot be read here ({e}), so nothing leaves",
                        r.label
                    ));
                    continue;
                }
            };
            if hit {
                let scope = r
                    .scope
                    .as_ref()
                    .map(|s| format!(" (scope {s})"))
                    .unwrap_or_default();
                hits.push(format!(
                    "banned phrase « {} »{scope}: {}",
                    r.label, r.instead
                ));
            }
        }
        hits
    }

    fn local(&self, text: &str) -> Vec<String> {
        let mut out: Vec<String> = self
            .isms
            .iter()
            .filter(|Ism(rx, _, _)| rx.is_match(text))
            .map(|Ism(_, label, instead)| format!("an AI tell, {label}: write {instead}"))
            .collect();
        let (score, n) = lix(text);
        if n >= READABLE_WORDS && score > READABLE_MAX {
            out.push(format!(
                "hard to read: LIX {score:.0} > {READABLE_MAX} (long sentences, long words): shorter sentences"
            ));
        }
        out
    }

    /// Why text may not leave; [] when it may. Jev is asked only once nothing local stops it.
    pub fn reasons(&self, text: &str, cold: bool, first: bool, long_ok: bool) -> Vec<String> {
        let text = text.trim();
        if text.is_empty() {
            return vec![];
        }
        let mut out = self.banned(text, cold, first);
        out.extend(self.local(text));
        if !out.is_empty() {
            return out;
        }
        match (self.ask)(text) {
            Err(e) => vec![format!("Jev gave no verdict ({e}), so nothing leaves")],
            Ok((answers, gate)) => failures(&answers, &gate)
                .into_iter()
                .filter(|f| !(long_ok && f.starts_with("too_long")))
                .map(|f| format!("Jev: {f}"))
                .collect(),
        }
    }

    /// The open draft whose body is this text, or none.
    fn draft_for(&self, text: &str) -> Option<&Value> {
        let t = flat(text);
        self.drafts.iter().find(|d| {
            matches!(d["status"].as_str(), Some("pending" | "approved"))
                && flat(d["body"].as_str().unwrap_or_default()) == t
        })
    }

    /// reasons() with the flags of the draft behind text; none found, the strictest.
    pub fn judge(&self, text: &str) -> Vec<String> {
        let none = json!({});
        let d = self.draft_for(text).unwrap_or(&none);
        self.reasons(
            text,
            flag(d, "cold", true),
            flag(d, "first", true),
            flag(d, "long-ok", false),
        )
    }
}

/// Jev's failed gates: each GATED question over its bound, or with no score.
fn failures(answers: &Value, gate: &Value) -> Vec<String> {
    GATED
        .iter()
        .filter_map(|(k, g)| {
            let n = answers[k]["noul"].as_f64();
            let bound = &gate[g];
            let over = match (n, bound.as_f64()) {
                (Some(n), Some(b)) => n > b,
                _ => true,
            };
            over.then(|| {
                let shown = n.map_or("-".into(), |n| format!("{n:.2}"));
                format!("{k} {shown} > {bound}")
            })
        })
        .collect()
}

/// Python's `json.dumps(v, sort_keys=True)`: ", " and ": " between, keys sorted, every non-ASCII character escaped.
fn py_dumps(v: &Value) -> String {
    match v {
        Value::Object(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            let inner: Vec<String> = keys
                .iter()
                .map(|k| {
                    format!(
                        "{}: {}",
                        py_dumps(&Value::String((*k).clone())),
                        py_dumps(&m[*k])
                    )
                })
                .collect();
            format!("{{{}}}", inner.join(", "))
        }
        Value::Array(a) => format!(
            "[{}]",
            a.iter().map(py_dumps).collect::<Vec<_>>().join(", ")
        ),
        Value::String(_) => {
            // ponytail: numbers print as serde_json does; the questions' spec holds none
            let mut out = String::new();
            for c in v.to_string().chars() {
                if c.is_ascii() {
                    out.push(c);
                } else {
                    for unit in c.encode_utf16(&mut [0; 2]) {
                        out.push_str(&format!("\\u{unit:04x}"));
                    }
                }
            }
            out
        }
        other => other.to_string(),
    }
}

fn sha256(s: &str) -> String {
    Sha256::digest(s.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// draft_ai.key_of: the log key of the questions and the message, the same as the Python's.
pub(crate) fn key_of(spec: &Value, message: &str) -> String {
    let ident =
        json!({"model": spec["model"], "questions": spec["questions"], "message": sha256(message)});
    sha256(&py_dumps(&ident))
}

/// The log's answered rows for `key` and its repeats, key -> answers, the fixtures' first so a later live row wins
/// (ats.read_log, ats.fold). Only a line naming one of those keys is parsed: the live log is 58 MB (/simplify of PR 1480).
fn fold(root: &Path, key: &str) -> std::collections::HashMap<String, Value> {
    let keys: Vec<String> = std::iter::once(key.to_string())
        .chain((1..REPEATS).map(|i| sha256(&format!("{key}:repeat:{i}"))))
        .collect();
    let mut out = std::collections::HashMap::new();
    for p in ["private/ats/fixtures/jev.jsonl", "private/ats/jev.jsonl"] {
        for line in std::fs::read_to_string(root.join(p))
            .unwrap_or_default()
            .lines()
            .filter(|l| keys.iter().any(|k| l.contains(k.as_str())))
        {
            let Ok(r) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            if r["status"] == 200
                && let Some(a) = r["response"].get("answers")
                && let Some(k) = r["key"].as_str()
            {
                out.insert(k.to_string(), a.clone());
            }
        }
    }
    out
}

/// The main answer and its paid repeats, one answer set: each gated score the samples' mean (ats.samples, mean_answers).
fn logged(cache: &std::collections::HashMap<String, Value>, key: &str) -> Option<Value> {
    let mut samples: Vec<&Value> = cache.get(key).into_iter().collect();
    samples.extend((1..REPEATS).filter_map(|i| cache.get(&sha256(&format!("{key}:repeat:{i}")))));
    let mut out = (*samples.first()?).clone();
    if samples.len() > 1 {
        for (k, _) in GATED {
            let vals: Vec<f64> = samples
                .iter()
                .filter_map(|a| a[k]["noul"].as_f64())
                .collect();
            if out[k].get("noul").is_some() && !vals.is_empty() {
                out[k] = json!({"noul": vals.iter().sum::<f64>() / vals.len() as f64});
            }
        }
    }
    Some(out)
}

/// (answers, gate) for exactly text: the log's, else Jev asked now and its answer logged as ats.post logs it.
pub(crate) fn jev_verdict(root: &Path, text: &str) -> Result<(Value, Value), String> {
    let path = root.join("private/ats/draft-questions.json");
    let spec: Value = serde_json::from_str(
        &std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?,
    )
    .map_err(|e| format!("{}: {e}", path.display()))?;
    let key = key_of(&spec, text);
    if let Some(a) = logged(&fold(root, &key), &key) {
        return Ok((a, spec["gate"].clone()));
    }
    if spec["model"] != crate::host().jev_model {
        return Err(format!(
            "draft-questions.json asks {}, this dash's Jev client {}",
            spec["model"],
            crate::host().jev_model
        ));
    }
    let state = json!({"message": text});
    let started = std::time::Instant::now();
    let response = (crate::host().jev_ask)(
        &spec["questions"],
        state.clone(),
        "site_perso/outbound-gate",
    )?;
    let row = json!({"ts": (crate::host().now_ms)(), "part": "gate", "key": key, "attempt": 1,
        "request": {"url": "/v1/systemone", "method": "POST",
                    "body": {"state": state, "model": spec["model"], "questions": spec["questions"]}},
        "status": 200, "response": response, "headers": {}, "wall_ms": started.elapsed().as_millis() as u64});
    use std::io::Write;
    let log = root.join("private/ats/jev.jsonl");
    // a verdict not logged is asked again next time: never a refusal
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log)
    {
        let _ = writeln!(f, "{row}");
    }
    Ok((response["answers"].clone(), spec["gate"].clone()))
}

/// The older app's `dash <args>` as JSON through dash's bridge (`Host::older`, rfc-rust-unified#self-hosting), or none when
/// it does not answer.
pub(crate) fn app(args: &[&str]) -> Option<Value> {
    let out = (crate::host().older)(args).ok()?;
    serde_json::from_str(&out).ok()
}

/// The gate as the live data has it: the model's banned rows, the tells, the open drafts, Jev.
/// The gate's tables; `drafts`: the model's drafts too (the hook's), never for `check`, which takes its flags.
pub(crate) fn load<'a>(
    root: &Path,
    ask: &'a dyn Fn(&str) -> Verdict,
    drafts: bool,
) -> Result<Gate<'a>, String> {
    let text = (crate::host().older)(&["store", "text", "banned-phrases.edn"])
        .map_err(|why| format!("the dashboard's model gave no banned-phrases.edn: {why}"))?;
    let drafts = if drafts {
        app(&["model", "drafts"])
            .and_then(|v| v.as_array().cloned())
            .unwrap_or_default()
    } else {
        vec![]
    };
    Ok(Gate {
        rows: rows(&text)?,
        isms: isms(root)?,
        drafts,
        ask,
    })
}

/// A JS literal's text: escapes decoded, a pair of surrogates whole, a lone one replaced.
fn js_unescape(raw: &str) -> String {
    let esc = re!(r"(?s)\\(u\{[0-9a-fA-F]+\}|u[0-9a-fA-F]{4}|x[0-9a-fA-F]{2}|.)");
    let mut units: Vec<u16> = vec![];
    let mut at = 0;
    for c in esc.captures_iter(raw) {
        let (all, e) = (c.get(0).expect("0"), &c[1]);
        units.extend(raw[at..all.start()].encode_utf16());
        at = all.end();
        if e.chars().count() == 1 {
            let s = match e {
                "n" => "\n",
                "t" => "\t",
                "r" => "\r",
                "b" => "\u{8}",
                "f" => "\u{c}",
                "v" => "\u{b}",
                "0" => "\0",
                "\n" => "",
                other => other,
            };
            units.extend(s.encode_utf16());
            continue;
        }
        let code =
            u32::from_str_radix(e.trim_matches(|c| "ux{}".contains(c)), 16).unwrap_or(u32::MAX);
        match code {
            0xD800..=0xDFFF => units.push(code as u16),
            c if c <= 0x10FFFF => units.extend(
                char::from_u32(c)
                    .map(String::from)
                    .unwrap_or_default()
                    .encode_utf16(),
            ),
            _ => units.extend(all.as_str().encode_utf16()),
        }
    }
    units.extend(raw[at..].encode_utf16());
    String::from_utf16_lossy(&units)
}

/// The template literal opening at `start`: its raw text and the index after it. A template nested in ${} is part of it.
/// ponytail: a quote holding a brace inside ${} miscounts it; the text then runs on, read rather than dropped.
fn js_template(s: &str, start: usize) -> (String, usize) {
    let b = s.as_bytes();
    let (mut i, mut depth) = (start + 1, 0);
    while i < b.len() {
        match b[i] {
            b'\\' => i += 1 + s[i + 1..].chars().next().map_or(0, char::len_utf8),
            b'`' if depth == 0 => return (s[start + 1..i].into(), i + 1),
            b'`' => i = js_template(s, i).1,
            b'$' if b.get(i + 1) == Some(&b'{') => {
                depth += 1;
                i += 2
            }
            c => {
                if depth > 0 && c == b'{' {
                    depth += 1
                } else if depth > 0 && c == b'}' {
                    depth -= 1
                }
                i += 1
            }
        }
    }
    (s[start + 1..].into(), s.len())
}

/// A script's long strings with a space, what fillMerge or a `.value =` would type; its code is not read as prose.
/// Literals of one expression joined by +, with names between them or by +=, count as one. ponytail: a text from a join()
/// or from variables alone goes unread, an oops guard's ceiling.
fn js_texts(script: &str) -> Vec<String> {
    let token = re!(r#"(?s)//[^\n]*|/\*.*?\*/|"((?:\\.|[^"\\\n])*)"|'((?:\\.|[^'\\\n])*)'|\w+|\S"#);
    let regex = re!(r"/(?:\\.|\[(?:\\.|[^\]\n])*\]|[^/\\\n])+/[a-z]*");
    // a slash after one of these opens a regex, after anything else it divides. ponytail: `if (x) /re/` reads as a division
    let before_regex = |t: &str| {
        (t.chars().count() == 1 && "(,=:[!&|?{};+-*%<>~^".contains(t))
            || [
                "return", "typeof", "void", "case", "in", "of", "delete", "throw", "new", "else",
                "do", "yield", "await",
            ]
            .contains(&t)
    };
    let chain_ends = |t: &str| t.chars().count() == 1 && ";,=:{}[]".contains(t);
    let (mut parts, mut before, mut last, mut chain, mut i): (
        Vec<String>,
        Option<String>,
        Option<String>,
        bool,
        usize,
    ) = (vec![], None, None, false, 0);
    while let Some(m) = token.captures_at(script, i) {
        let all = m.get(0).expect("0");
        let tok = all.as_str();
        i = all.end();
        if tok.starts_with("//") || tok.starts_with("/*") {
            continue;
        }
        let postfix = matches!(last.as_deref(), Some("+" | "-")) && before == last;
        let lit = m.get(1).or(m.get(2));
        let re = (tok == "/" && !postfix && last.as_deref().is_none_or(before_regex))
            .then(|| {
                regex
                    .find_at(script, all.start())
                    .filter(|r| r.start() == all.start())
            })
            .flatten();
        if let Some(r) = re {
            i = r.end();
            before = last.take();
            last = Some("regex".into());
        } else if tok == "`" || lit.is_some() {
            let raw = if tok == "`" {
                let (raw, end) = js_template(script, all.start());
                i = end;
                raw
            } else {
                lit.expect("lit").as_str().into()
            };
            let t = js_unescape(&raw);
            let joins = (chain && last.as_deref() == Some("+"))
                || (before.as_deref() == Some("+") && last.as_deref() == Some("="));
            match parts.last_mut() {
                Some(p) if joins => p.push_str(&t),
                _ => parts.push(t),
            }
            before = last.take();
            last = Some("\"".into());
            chain = true;
        } else {
            chain = chain && !chain_ends(tok);
            before = last.take();
            last = Some(tok.into());
        }
    }
    parts
        .into_iter()
        .filter(|t| t.chars().count() >= FORM_MIN && t.contains(' '))
        .collect()
}

/// The LinkedIn tool's short name when this call sends, else none: a preview types nothing that leaves.
fn linkedin_send<'a>(tool: &'a str, args: &Value) -> Option<&'a str> {
    let name = tool.strip_prefix(LINKEDIN)?;
    let confirm = if name == "easy_apply" {
        "confirm_submit"
    } else {
        "confirm_send"
    };
    (LINKEDIN_SENDS.contains(&name) && (name == "connect_with_person" || args[confirm] == true))
        .then_some(name)
}

/// What a tool call would send to a person: the LinkedIn sends' message or note, a browser's long typed text.
fn texts(tool: &str, args: &Value) -> Vec<String> {
    if let Some(name) = tool.strip_prefix(LINKEDIN)
        && LINKEDIN_SENDS.contains(&name)
    {
        let field = if name == "connect_with_person" {
            "note"
        } else {
            "message"
        };
        return match linkedin_send(tool, args) {
            Some(_) => args[field].as_str().map(String::from).into_iter().collect(),
            None => vec![],
        };
    }
    if tool == format!("{CHROME}javascript_tool") {
        return js_texts(args["text"].as_str().unwrap_or_default());
    }
    if !tool.starts_with(CHROME) {
        return vec![];
    }
    fn walk(v: &Value, found: &mut Vec<String>) {
        match v {
            Value::Object(m) => {
                if m.get("name")
                    .and_then(Value::as_str)
                    .is_some_and(|n| n.ends_with("javascript_tool"))
                {
                    found.extend(js_texts(v["input"]["text"].as_str().unwrap_or_default()));
                    return;
                }
                for (k, x) in m {
                    match x.as_str() {
                        Some(s)
                            if (k == "text" || k == "value")
                                && s.chars().count() >= FORM_MIN
                                && m.get("action").and_then(Value::as_str) != Some("key") =>
                        {
                            found.push(s.into())
                        }
                        _ => walk(x, found),
                    }
                }
            }
            Value::Array(a) => a.iter().for_each(|x| walk(x, found)),
            _ => {}
        }
    }
    let mut found = vec![];
    walk(args, &mut found);
    found
}

/// A Gmail tool that sends, whatever its server is called: a name that does not say it only reads is refused.
fn gmail_send(tool: &str) -> bool {
    if !tool.starts_with("mcp__") || !tool.to_lowercase().contains("gmail") {
        return false;
    }
    let own = tool.rsplit("__").next().unwrap_or_default();
    let own = re!(r"(?i)^gmail[_-]?").replace(own, "");
    let words: Vec<String> = re!(r"[A-Z]?[a-z]+|[A-Z]+")
        .find_iter(&own)
        .map(|m| m.as_str().to_lowercase())
        .collect();
    let sends = |w: &String| {
        w.starts_with("send")
            || w.starts_with("reply")
            || (w.starts_with("forward") && !w.starts_with("forwarding"))
    };
    words.iter().any(sends)
        && !["get", "list", "search", "read", "find", "count"].contains(&words[0].as_str())
        && !["settings", "list"].contains(&words[words.len() - 1].as_str())
}

/// The PreToolUse refusal for one call, or none. `gate` loads the tables only when the call carries a text.
pub(crate) fn hook<'a>(
    event: &Value,
    gate: impl FnOnce() -> Result<Gate<'a>, String>,
) -> Option<String> {
    let tool = event["tool_name"].as_str().unwrap_or_default();
    if gmail_send(tool) {
        return Some(GMAIL_REFUSED.into());
    }
    let inp = match &event["tool_input"] {
        Value::Null => json!({}),
        v if v.is_object() => v.clone(),
        _ => {
            return Some(
                "Outbound gate failed (its tool_input is no object), so nothing is sent.".into(),
            );
        }
    };
    let ts = texts(tool, &inp);
    if ts.is_empty() {
        return None;
    }
    let gate = match gate() {
        Ok(g) => g,
        Err(e) => return Some(format!("Outbound gate failed ({e}), so nothing is sent.")),
    };
    let mut why: Vec<String> = ts.iter().flat_map(|t| gate.judge(t)).collect();
    // a draft held for its :send-on goes out on that day, not before
    why.extend(ts.iter().filter_map(|t| {
        let d = gate.draft_for(t)?;
        let day = d["held-until"].as_str().filter(|s| !s.is_empty())?;
        Some(format!(
            "{} goes out on {day} (:send-on), not before",
            d["id"].as_str().unwrap_or_default()
        ))
    }));
    (!why.is_empty()).then(|| format!("Outbound gate: nothing sent. {}", why.join("; ")))
}

/// `dash hook outbound-gate`: the call on stdin, a deny on stdout; bounded at HOOK_DEADLINE, past which it denies.
pub fn hook_main(input: &str) -> Option<String> {
    let Ok(event) = serde_json::from_str::<Value>(input) else {
        return Some("Outbound gate failed (its input is no JSON), so nothing is sent.".into());
    };
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let root = root();
        let ask = |t: &str| jev_verdict(&root, t);
        let _ = tx.send(hook(&event, || load(&root, &ask, true)));
    });
    rx.recv_timeout(HOOK_DEADLINE).unwrap_or_else(|e| {
        Some(match e {
            std::sync::mpsc::RecvTimeoutError::Timeout => format!(
                "Outbound gate failed (no answer in {} s), so nothing is sent.",
                HOOK_DEADLINE.as_secs()
            ),
            // the judging thread panicked: its message is on stderr
            std::sync::mpsc::RecvTimeoutError::Disconnected => {
                "Outbound gate failed (it crashed, see its stderr), so nothing is sent.".into()
            }
        })
    })
}

/// `dash outbound check [--draft <id>] [--not-cold] [--not-first] [--long-ok] [<file>]`: PASS, or each FAIL line and exit 1.
/// With --draft, the flags of that draft while it is open (`dash model draft <id>`); else the flags given, cold and first
/// by default, the strictest.
pub fn check(args: &[&str]) -> (i32, String) {
    let mut draft = None;
    let (mut cold, mut first, mut long_ok, mut file) = (true, true, false, None);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match *a {
            "--draft" => draft = it.next().copied(),
            "--not-cold" => cold = false,
            "--not-first" => first = false,
            "--long-ok" => long_ok = true,
            f => file = Some(f),
        }
    }
    let text = match file {
        Some(f) => std::fs::read_to_string(f).map_err(|e| format!("{f}: {e}")),
        None => {
            let mut s = String::new();
            std::io::Read::read_to_string(&mut std::io::stdin(), &mut s)
                .map(|_| s)
                .map_err(|e| e.to_string())
        }
    };
    let root = root();
    let ask = |t: &str| jev_verdict(&root, t);
    let why = text.and_then(|text| {
        let gate = load(&root, &ask, false)?;
        if let Some(id) = draft {
            let d = app(&["model", "draft", id])
                .ok_or(format!("no draft {id}, or the app does not answer"))?;
            let open = matches!(d["status"].as_str(), Some("pending" | "approved"));
            (cold, first, long_ok) = if open {
                (
                    flag(&d, "cold", true),
                    flag(&d, "first", true),
                    flag(&d, "long-ok", false),
                )
            } else {
                (true, true, false)
            };
        }
        Ok(gate.reasons(&text, cold, first, long_ok))
    });
    let why =
        why.unwrap_or_else(|e| vec![format!("Outbound gate failed ({e}), so nothing is sent")]);
    if why.is_empty() {
        (0, "PASS\n".into())
    } else {
        (1, why.iter().map(|w| format!("FAIL {w}\n")).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::memory::TestBudget;

    // three rows of his banned-phrases.edn, copied 2026-10-27: the tests prove the gate, never read his live model
    const ROWS: &str = r#"[
      {"regex": "flexibilit[ée] du client|rythme pr[ée]sentiel|combien de jours (sur site|de pr[ée]sentiel|en pr[ée]sentiel|de t[ée]l[ée]travail)|how many days on site",
       "label": "a conditions question (on-site, rate, duration) before they replied", "scope": "~:first", "instead": "Drop the question"},
      {"regex": "\\btjm\\b|day rate|\\b\\d{3,4} ?(€|eur\\b|euros\\b)", "label": "a rate", "scope": "~:cold", "instead": "Leave the rate"},
      {"regex": "(?i)\\band that stays\\b|\\bet j'y reste\\b", "label": "« and that stays », a robotic aside", "instead": "« I'm based in France. »"}
    ]"#;
    const BANNED: &str =
        "Bonjour Paul,\nJe travaille sur Kafka depuis 2019, and that stays.\nBonne journée";
    const ROBOTIC: &str =
        "Bonjour Paul,\nVotre mission m'intéresse beaucoup.\nOn en discute cette semaine ?";
    const CLEAN: &str = "Bonjour Paul,\nJ'ai vu votre mission Kafka.\nOn en parle cette semaine ?";

    /// Jev, planted: the robotic text reads as AI, every other as typed.
    fn stub(text: &str) -> Result<(Value, Value), String> {
        let ai = if text.starts_with(ROBOTIC) { 0.9 } else { 0.1 };
        let long = if text.contains("LONG") { 0.8 } else { 0.1 };
        Ok((
            json!({"reads_ai_written": {"noul": ai}, "too_long": {"noul": long}}),
            json!({"reads_ai_written_max": 0.5, "too_long_max": 0.6}),
        ))
    }

    fn gate(drafts: Vec<Value>) -> Gate<'static> {
        let table = json!([{"regex": "—", "label": "an em-dash", "instead": "a period"},
                          {"regex": "\\b(stays|remains) with (him|her)\\b", "label": "« stays with him »", "instead": "say who"},
                          {"regex": "\\bload-bearing\\b", "label": "« load-bearing »", "instead": "say what"}]);
        let dir = std::env::temp_dir().join(format!("outbound-isms-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("private/dashboard/priv")).expect("dir");
        std::fs::write(
            dir.join("private/dashboard/priv/ai_isms.json"),
            json!({"rows": table}).to_string(),
        )
        .expect("isms");
        Gate {
            rows: rows(ROWS).expect("rows"),
            isms: isms(&dir).expect("isms"),
            drafts,
            ask: &stub,
        }
    }

    fn deny(tool: &str, input: Value, drafts: Vec<Value>) -> Option<String> {
        hook(&json!({"tool_name": tool, "tool_input": input}), || {
            Ok(gate(drafts))
        })
    }

    /// An entity missing its `;` hides no banned phrase, and a lone `&` stays one (R&D).
    #[test]
    fn an_entity_without_its_semicolon_is_decoded() {
        let _budget = TestBudget::new();
        assert_eq!(norm("caf&eacute flexibilit&#233 du"), "café flexibilité du");
        assert_eq!(norm("R&D &amp; ops&lt"), "r&d & ops<");
    }

    #[test]
    fn banned_rows_keep_their_scopes_and_unicode_bold_hides_nothing() {
        let _budget = TestBudget::new();
        let g = gate(vec![]);
        assert!(
            g.reasons(BANNED, true, true, false)[0]
                .starts_with("banned phrase « « and that stays », a robotic aside »")
        );
        assert!(
            !g.reasons("𝗮𝗻𝗱 𝘁𝗵𝗮𝘁 𝘀𝘁𝗮𝘆𝘀", true, true, false).is_empty(),
            "LinkedIn bold"
        );
        assert!(
            !g.reasons("Mon TJM est 650 €.", true, true, false)
                .is_empty()
        );
        assert!(
            g.reasons("Mon TJM est 650 €.", false, false, false)
                .is_empty()
        );
        assert!(
            !g.reasons("Combien de jours sur site ?", false, true, false)
                .is_empty()
        );
        assert!(
            g.reasons("Combien de jours sur site ?", false, false, false)
                .is_empty()
        );
        let r = rows(r#"[1, {"match": "~~ ok", "label": "~~x"}]"#).expect("rows");
        assert_eq!(
            (r[0].s.as_str(), r[0].label.as_str(), r[0].scope.is_none()),
            ("~ ok", "~x", true)
        );
        assert!(rows("[]").is_err(), "a table of no row refuses");
    }

    #[test]
    fn a_row_this_engine_cannot_read_refuses_every_text() {
        let _budget = TestBudget::new();
        let g = Gate {
            rows: rows(r#"[{"regex": "(?<=x)y", "label": "lookbehind"}]"#).expect("rows"),
            ..gate(vec![])
        };
        assert!(
            g.reasons(CLEAN, true, true, false)[0].contains("lookbehind » cannot be read here")
        );
    }

    #[test]
    fn local_tells_jev_and_long_ok_lift_only_the_length_verdict() {
        let _budget = TestBudget::new();
        let g = gate(vec![]);
        assert_eq!(
            g.reasons(ROBOTIC, true, true, false),
            ["Jev: reads_ai_written 0.90 > 0.5"]
        );
        assert!(g.reasons(CLEAN, true, true, false).is_empty());
        assert_eq!(
            g.reasons(&format!("{CLEAN} LONG"), true, true, false),
            ["Jev: too_long 0.80 > 0.6"]
        );
        assert!(
            g.reasons(&format!("{CLEAN} LONG"), false, false, true)
                .is_empty()
        );
        assert!(
            !g.reasons(&format!("{BANNED} LONG"), false, false, true)
                .is_empty()
        );
        assert!(
            !g.reasons(&format!("{ROBOTIC} LONG"), false, false, true)
                .is_empty()
        );
        assert!(
            g.reasons("Un texte — avec un tiret", true, true, false)[0]
                .starts_with("an AI tell, an em-dash")
        );
        assert!(
            g.local("It's load-bearing.")
                .iter()
                .any(|r| r.contains("load-bearing"))
        );
        let hard = "Concernant l'implémentation architecturale, l'industrialisation systématique des infrastructures ".repeat(6);
        assert!(
            g.reasons(&hard, true, true, false)
                .iter()
                .any(|r| r.starts_with("hard to read"))
        );
    }

    #[test]
    fn no_verdict_refuses() {
        let _budget = TestBudget::new();
        let down = |_: &str| Err("no network".to_string());
        let g = Gate {
            ask: &down,
            ..gate(vec![])
        };
        assert!(g.reasons(CLEAN, true, true, false)[0].starts_with("Jev gave no verdict"));
    }

    #[test]
    fn each_linkedin_send_is_judged_and_a_preview_passes() {
        let _budget = TestBudget::new();
        for (tool, field) in [
            ("send_message", "message"),
            ("reply_in_thread", "message"),
            ("message_job_poster", "message"),
            ("connect_with_person", "note"),
            ("easy_apply", "message"),
        ] {
            let yes = if tool == "easy_apply" {
                "confirm_submit"
            } else {
                "confirm_send"
            };
            for (planted, why) in [(BANNED, "banned phrase"), (ROBOTIC, "reads_ai_written")] {
                let out = deny(
                    &format!("{LINKEDIN}{tool}"),
                    json!({field: planted, yes: true}),
                    vec![],
                );
                assert!(out.is_some_and(|o| o.contains(why)), "{tool}");
            }
            assert!(
                deny(
                    &format!("{LINKEDIN}{tool}"),
                    json!({field: CLEAN, yes: true}),
                    vec![]
                )
                .is_none(),
                "{tool}"
            );
        }
        assert!(
            deny(
                &format!("{LINKEDIN}send_message"),
                json!({"message": BANNED, "confirm_send": false}),
                vec![]
            )
            .is_none()
        );
        let rate = json!({"message": "TJM : 650 € par jour.", "confirm_send": true});
        assert!(
            deny(&format!("{LINKEDIN}reply_in_thread"), rate.clone(), vec![]).is_some(),
            "no draft: judged cold"
        );
        let d = json!({"status": "approved", "body": "TJM : **650 €** par jour.", "cold": false, "first": false});
        assert!(deny(&format!("{LINKEDIN}reply_in_thread"), rate, vec![d]).is_none());
        let held = json!({"id": "x", "status": "approved", "body": CLEAN, "cold": false, "first": false, "held-until": "2026-10-27"});
        let out = deny(
            &format!("{LINKEDIN}send_message"),
            json!({"message": CLEAN, "confirm_send": true}),
            vec![held],
        );
        assert!(out.is_some_and(|o| o.contains("x goes out on 2026-10-27")));
        assert!(
            deny(
                &format!("{LINKEDIN}send_message"),
                json!("not an object"),
                vec![]
            )
            .is_some(),
            "fails closed"
        );
    }

    #[test]
    fn form_text_is_judged_in_type_form_input_and_batch() {
        let _budget = TestBudget::new();
        let pad = format!(" {}", "Je reste dispo cette semaine. ".repeat(8));
        for (planted, why) in [(BANNED, "banned phrase"), (ROBOTIC, "reads_ai_written")] {
            let planted = format!("{planted}{pad}");
            for (tool, input) in [
                ("computer", json!({"action": "type", "text": planted})),
                ("form_input", json!({"ref": "r1", "value": planted})),
                (
                    "browser_batch",
                    json!({"actions": [{"action": "type", "text": planted}]}),
                ),
            ] {
                assert!(
                    deny(&format!("{CHROME}{tool}"), input, vec![])
                        .is_some_and(|o| o.contains(why)),
                    "{tool}"
                );
            }
        }
        assert!(
            deny(
                &format!("{CHROME}computer"),
                json!({"action": "type", "text": "kafka freelance"}),
                vec![]
            )
            .is_none()
        );
    }

    #[test]
    fn javascript_tool_reads_the_fills_text_never_fill_merges_code() {
        let _budget = TestBudget::new();
        // the skill's file is outside rust/: the public mirror stops here
        let Some(top) = dash_meter::monorepo() else {
            return;
        };
        let lib = std::fs::read_to_string(top.join(".claude/skills/browser-apply/fill-merge.js"))
            .expect("fill-merge.js");
        let planted = format!("{BANNED} {}", "Je reste dispo cette semaine. ".repeat(8));
        let q = serde_json::to_string(&planted).expect("q");
        let script = format!("{lib}\nfillMerge([{{field: \"#why\", value: {q}}}])");
        let js = |s: &str| texts(&format!("{CHROME}javascript_tool"), &json!({"text": s}));
        assert_eq!(
            js(&script),
            std::slice::from_ref(&planted),
            "only the fill's text"
        );
        assert!(js(&lib).is_empty(), "the library alone is no message");
        let batch = json!({"actions": [{"name": "javascript_tool", "input": {"action": "javascript_exec", "text": script}}]});
        assert_eq!(
            texts(&format!("{CHROME}browser_batch"), &batch),
            std::slice::from_ref(&planted)
        );
        assert!(
            deny(&format!("{CHROME}browser_batch"), batch, vec![])
                .is_some_and(|o| o.contains("banned phrase"))
        );
        for full in ["javascript_tool", "mcp__claude-in-chrome__javascript_tool"] {
            assert!(
                texts(
                    &format!("{CHROME}browser_batch"),
                    &json!({"actions": [{"name": full, "input": {"text": lib}}]})
                )
                .is_empty()
            );
        }
        // a regex's // or quote hides nothing, pieces joined by + are one text
        for before in [
            "x.replace(/https?:\\/\\//,\"\"); ",
            "x.replace(/'/g,\"-\"); ",
            "y = a / b; z = c / d; ",
            "s => /\"/.test(s); ",
            "return /\"/.test(s); ",
            "a + /\"/.source; ",
            "void /\"/; ",
            "x = a\n/ 2; ",
        ] {
            assert_eq!(
                js(&format!("{before}fillMerge([{{value: {q}}}])")),
                std::slice::from_ref(&planted),
                "{before}"
            );
        }
        let half = planted
            .char_indices()
            .nth(planted.chars().count() / 2)
            .expect("half")
            .0;
        let (a, b) = (
            serde_json::to_string(&planted[..half]).expect("a"),
            serde_json::to_string(&planted[half..]).expect("b"),
        );
        assert_eq!(
            js(&format!("fillMerge([{{value: {a} + {b}}}])")),
            std::slice::from_ref(&planted),
            "joined by +"
        );
        assert_eq!(
            js(&format!("fillMerge([{{value: {a} + /* more */ {b}}}])")),
            std::slice::from_ref(&planted),
            "a comment in the join"
        );
        assert_eq!(
            js(&format!(
                "p = i++ / 2; fillMerge([{{value: {q}}}]); r = n / 3;"
            )),
            std::slice::from_ref(&planted),
            "a postfix"
        );
        assert_eq!(
            js(&format!(
                "p = i-- / 2; fillMerge([{{value: {q}}}]); r = n / 3;"
            )),
            std::slice::from_ref(&planted),
            "a postfix"
        );
        let nested = js(&format!(
            "fillMerge([{{value: `Bonjour ${{a ? `x` : `y`}}, {planted} J'ai vu.`}}]); done('x')"
        ));
        assert!(
            nested.len() == 1 && nested[0].contains(BANNED) && nested[0].ends_with("J'ai vu."),
            "{nested:?}"
        );
        assert_eq!(
            js(&format!(
                "x = `a ${{b ? `c ${{d}} e` : {{f: 1}}.f}} {planted}`"
            )),
            [format!("a ${{b ? `c ${{d}} e` : {{f: 1}}.f}} {planted}")]
        );
        assert_eq!(
            js(&format!("let m = {a}; m += {b}; fillMerge([{{value: m}}])")),
            std::slice::from_ref(&planted),
            "+="
        );
        assert_eq!(
            js(&format!("fillMerge([{{value: {a} + name + {b}}}])")),
            std::slice::from_ref(&planted),
            "a name between"
        );
        assert_eq!(
            js(&format!("fillMerge([{{value: {a} + f(x) + {b}}}])")),
            std::slice::from_ref(&planted),
            "a call between"
        );
        assert!(
            js(&format!("a({a}); b = c + {b}")).is_empty(),
            "another statement's literal is another text"
        );
        assert_eq!(
            js(&format!("x += {q}")),
            std::slice::from_ref(&planted),
            "+= on the first literal"
        );
        assert_eq!(
            js_unescape(r"\u{110000}"),
            r"\u{110000}",
            "out of range stays as written"
        );
        assert_eq!(
            js_unescape(r"a\u{2014}b—c\x41\n😀"),
            "a\u{2014}b\u{2014}cA\n\u{1f600}"
        );
    }

    #[test]
    fn gmail_sends_are_refused_and_its_reads_pass() {
        let _budget = TestBudget::new();
        for name in [
            "mcp__claude_ai_Gmail__send_message",
            "mcp__claude_ai_Gmail__reply",
            "mcp__claude_ai_Gmail__forward",
            "mcp__claude_ai_Gmail__send_draft",
            "mcp__google-workspace__gmail_send_message",
            "mcp__GMAIL__send",
            "mcp__gmail__SendEmail",
            "mcp__GMAIL__Reply",
            "mcp__gmail__users_messages_send",
            "mcp__gmail__batch_send",
            "mcp__gmail__create_and_send_draft",
            "mcp__google-workspace__gmail_quick_reply",
            "mcp__gmail__sendmail",
            "mcp__gmail__send_and_get_id",
        ] {
            assert!(
                deny(name, json!({"body": CLEAN}), vec![])
                    .is_some_and(|o| o.contains("dash mail send")),
                "{name}"
            );
        }
        for name in [
            "search_threads",
            "list_send_as",
            "get_reply_settings",
            "list_forwarding_addresses",
            "forwarding_settings",
            "reply_settings",
            "send_as_list",
            "forward_settings",
        ] {
            assert!(
                deny(&format!("mcp__claude_ai_Gmail__{name}"), json!({}), vec![]).is_none(),
                "{name}"
            );
        }
        assert!(deny("mcp__gmail-sender__list_threads", json!({}), vec![]).is_none());
    }

    #[test]
    fn key_of_is_draft_ai_key_of_byte_for_byte() {
        let _budget = TestBudget::new();
        // python3 -I -c 'import hashlib,json; m="Bonjour — été 😀"; s={"model":"jev-1.13.0","questions":{"too_long":
        // {"type":"noul","instructions":"Is `message` long? « x »"}}}; i={"model":s["model"],"questions":s["questions"],
        // "message":hashlib.sha256(m.encode()).hexdigest()}; print(hashlib.sha256(json.dumps(i,sort_keys=True).encode()).hexdigest())'
        let spec = json!({"model": "jev-1.13.0", "questions": {"too_long": {"type": "noul", "instructions": "Is `message` long? « x »"}}});
        assert_eq!(key_of(&spec, "Bonjour — été 😀"), PYTHON_KEY);
    }
    const PYTHON_KEY: &str = "fa96da700b84939a4df3f4be6c9a44a74c41540e094f2e82a84e682b3f8250e9";

    #[test]
    fn the_log_answers_a_known_text_and_repeats_are_averaged() {
        let _budget = TestBudget::new();
        let dir = std::env::temp_dir().join(format!("outbound-log-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("private/ats/fixtures")).expect("dir");
        let spec = json!({"model": "jev-1.13.0", "gate": {"reads_ai_written_max": 0.5, "too_long_max": 0.6},
                          "questions": {"too_long": {"type": "noul", "instructions": "x"}}});
        std::fs::write(
            dir.join("private/ats/draft-questions.json"),
            spec.to_string(),
        )
        .expect("spec");
        let key = key_of(&spec, CLEAN);
        let row = |k: &str, n: f64| {
            json!({"key": k, "status": 200, "response": {"answers":
            {"reads_ai_written": {"noul": n}, "too_long": {"noul": 0.1}}}})
            .to_string()
        };
        let lines = [
            row(&key, 0.2),
            row(&sha256(&format!("{key}:repeat:1")), 0.6),
        ]
        .join("\n");
        std::fs::write(dir.join("private/ats/jev.jsonl"), lines).expect("log");
        let (a, gate) = jev_verdict(&dir, CLEAN).expect("logged");
        assert_eq!(
            (
                a["reads_ai_written"]["noul"].as_f64(),
                gate["too_long_max"].as_f64()
            ),
            (Some(0.4), Some(0.6))
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
