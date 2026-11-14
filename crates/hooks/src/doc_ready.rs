//! Is this document ready? doc_ready.py's ask, ported (task rust-jev-tool, done-when 5; @nha, 2026-10-19: « JEV is cheap -
//! use it. "is this document ready?" »). The jev tool, called through `Registry::call` alone, scores each question of
//! private/ats/doc-questions.json; a score over its gate fails the document. A text too long for one read is judged by
//! section, the worst score per question standing for it; each h2 section is judged too, shown under its heading. Every ask
//! goes through the host's Jev, the core's tool, and nothing else (rfc-rust-unified#tools: the tool is the one door, the
//! core keeps every call): an unchanged text's second ask is the cache's, and pays nothing. The tool's side writes a fresh
//! answer's line of Jev's log (`jev::log_row`), where the doc pass (`jev::judged`) and the older app's Readiness read it.
//! `dash doc ready <doc>... [--reader <who>]`, `dash doc ready --controls`; doc_ready.py only hands the frozen app's three
//! callers to it (@nha, 2026-11-14: « every Jev ask, the frozen app's three included, then goes through the tool »).
use regex::Regex;
use serde_json::{Value, json};
use std::path::Path;
use std::sync::LazyLock;

pub const QUESTIONS: &str = "private/ats/doc-questions.json";
pub const CONTROLS: &str = ".claude/skills/nicolas-daily/doc-ready-controls";
/// What the tool's typed answer `too_long` means here, and the words a failure that stops on it carries.
pub const TOO_LONG: &str = "too long for one read";
/// The section name of a document's preamble in Jev's log, as Readiness looks for it.
pub const INTRO: &str = "(before the first section)";
const BY_SECTION: &str = "too long for one read: judged by section";
/// The User-Agent's caller.
const AGENT: &str = "site_perso/doc-ready";

/// Noncharacters, so no text forges them (doc_ready.py's): LINE ends a line while the tags go; LI and H stand for a list item
/// and a heading level until the lines are joined, so a line that only starts with « - » or « # » is never taken for one.
const LINE: char = '\u{fffe}';
const LI: char = '\u{fdd0}';
const H: char = '\u{fdd1}';

macro_rules! re {
    ($p:expr) => {{
        static R: LazyLock<Regex> = LazyLock::new(|| Regex::new($p).expect("a pattern"));
        &*R
    }};
}

/// Writer notes, styles, scripts and comments, which no reader reads.
const HIDDEN: &str = r#"(?si)\x3cdetails class="wn">.*?\x3c/details>|\x3cstyle.*?\x3c/style>|\x3c(?:script).*?\x3c/(?:script)>|\x3c!--.*?-->"#;

/// The reading text, one line per block, headings as « # », list items as « - »: writer notes, styles, scripts, diagrams
/// (an SVG's labels are a picture's) and comments dropped. Readiness.text/1 makes the same text from doc-ready-text.html.
pub fn text(raw: &str) -> String {
    let raw = re!(r"(?si)\x3csvg.*?\x3c/svg>").replace_all(raw, " ");
    let raw = re!(HIDDEN).replace_all(&raw, " ");
    // a point link (<a class="pt" href="#p-…"></a>) is empty until shell.js numbers it: its point's name, never a blank
    let raw = re!(r"(?i)\x3ca\b([^>]*)>\s*\x3c/a>").replace_all(&raw, |c: &regex::Captures| {
        let pt = re!(r#"(?i)\sclass=["'](?:[^"']*\s)?pt["'\s]"#).is_match(&c[1]);
        match re!(r#"(?i)\bhref=["']#p-([^"']+)["']"#).captures(&c[1]) {
            Some(href) if pt => format!("« {} »", href[1].replace('-', " ")),
            _ => c[0].to_string(),
        }
    });
    let raw = re!(r"(?i)\x3ch([1-6])\b[^>]*>").replace_all(&raw, |c: &regex::Captures| {
        let level: usize = c[1].parse().unwrap_or(1);
        format!("{LINE}{} ", H.to_string().repeat(level))
    });
    let raw = re!(r"(?i)\x3cli\b[^>]*>").replace_all(&raw, format!("{LINE}{LI} ").as_str());
    const BLOCK: &str = r"(?i)\x3c/?(?:p|div|ul|ol|li|table|thead|tbody|tr|section|article|header|footer|nav|main|aside|h[1-6]|br|hr|blockquote|pre|figure|figcaption|details|summary|dl|dt|dd|title|caption)\b[^>]*>";
    let raw = re!(BLOCK).replace_all(&raw, LINE.to_string().as_str());
    let raw = unescape(&re!(r"\x3c[^>]+>").replace_all(&raw, " "));
    let lines: Vec<String> = raw
        .split(LINE)
        .map(collapse)
        .filter(|l| !l.is_empty())
        .collect();
    // an item alone on its line takes the line after it, unless that one is an item or a heading
    let mut joined: Vec<String> = vec![];
    let mut at = 0;
    while at < lines.len() {
        let next = lines.get(at + 1).filter(|n| !n.starts_with([LI, H]));
        match next {
            Some(n) if lines[at] == LI.to_string() => {
                joined.push(format!("{LI} {n}"));
                at += 2;
            }
            _ => {
                joined.push(lines[at].clone());
                at += 1;
            }
        }
    }
    // an item or a heading still alone is empty and goes
    let kept = joined
        .into_iter()
        .filter(|l| *l != LI.to_string() && !(l.chars().all(|c| c == H)));
    let kept: Vec<String> = kept.collect();
    kept.join("\n").replace(LI, "-").replace(H, "#")
}

/// A Markdown document's reading text: its own lines, blank ones dropped, spaces collapsed, notes and tags gone.
pub fn md_text(raw: &str) -> String {
    let raw = re!(HIDDEN).replace_all(raw, " ");
    let raw = unescape(&re!(r"\x3c[^>]+>").replace_all(&raw, " "));
    let lines: Vec<String> = raw
        .lines()
        .map(collapse)
        .filter(|l| !l.is_empty())
        .collect();
    lines.join("\n")
}

fn collapse(line: &str) -> String {
    line.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Python's html.unescape, one pass: a reference to a noncharacter or a control is dropped, so no page forges a marker.
/// ponytail: 0x80 to 0x9f are dropped, where Python maps them to cp1252; map them if a page ever writes one.
fn unescape(s: &str) -> String {
    let refs = re!(r"&(#[xX][0-9a-fA-F]+|#[0-9]+|[A-Za-z][A-Za-z0-9]{0,31});?");
    refs.replace_all(s, |c: &regex::Captures| {
        let name = &c[1];
        let number = match name.strip_prefix("#x").or(name.strip_prefix("#X")) {
            Some(hex) => u32::from_str_radix(hex, 16).ok(),
            None => name
                .strip_prefix('#')
                .map(|d| d.parse().unwrap_or(u32::MAX)),
        };
        let Some(n) = number else {
            return html_escape::decode_html_entities(&c[0]).into_owned();
        };
        let dropped = matches!(n, 1..=8 | 0xb | 0xe..=0x1f | 0x7f..=0x9f | 0xfdd0..=0xfdef)
            || n & 0xfffe == 0xfffe;
        match char::from_u32(n).filter(|_| n != 0) {
            _ if dropped => String::new(),
            Some(ch) => ch.to_string(),
            None => '\u{fffd}'.to_string(),
        }
    })
    .into_owned()
}

/// The audience line the rfc skill puts under every title, else a generic reader.
pub fn reader_of(raw: &str) -> String {
    let m = re!(r"(?:Audience|Pour qui|Lecteur)\s*:?\s*\x3c/b>\s*([^\x3c]{3,300})").captures(raw);
    m.map_or("the person this document is written for".into(), |m| {
        m[1].trim().to_string()
    })
}

/// Each h2 section of the reading text, its « ## » line first, down to the next h2; a heading with nothing under it is none.
/// ponytail: a paragraph that itself starts with « ## » opens a section too, as in doc_ready.py.
pub fn sections(doc: &str) -> Vec<String> {
    parts(doc)
        .into_iter()
        .skip(1)
        .filter(|p| p.contains('\n'))
        .collect()
}

/// What comes before the first h2 section.
fn preamble(doc: &str) -> String {
    parts(doc).swap_remove(0)
}

/// The text cut before each line that starts « ## », each part's trailing newlines dropped: the first is the preamble.
fn parts(doc: &str) -> Vec<String> {
    let mut parts = vec![String::new()];
    for line in doc.split('\n') {
        if line.starts_with("## ") {
            parts.push(String::new());
        }
        let part = parts.last_mut().expect("one part");
        part.push_str(line);
        part.push('\n');
    }
    parts
        .iter()
        .map(|p| p.trim_end_matches('\n').to_string())
        .collect()
}

fn noul(answer: &Value) -> Option<f64> {
    answer["noul"].as_f64()
}

/// Each question over its gate, with its reason when the spec gives one; a gated question Jev left unanswered fails.
pub fn failures(answers: &Value, gate: &Value, reasons: &Value) -> Vec<String> {
    let gate = gate.as_object().cloned().unwrap_or_default();
    let mut bad: Vec<String> = gate
        .keys()
        .filter_map(|k| k.strip_suffix("_max"))
        .filter(|q| noul(&answers[*q]).is_none())
        .map(|q| format!("{q} not answered"))
        .collect();
    for (q, a) in answers.as_object().into_iter().flatten() {
        let (Some(max), Some(n)) = (gate.get(&format!("{q}_max")), noul(a)) else {
            continue;
        };
        // a gate that is not a number fails closed
        if max.as_f64().is_none_or(|m| n > m) {
            let why = reasons[q]
                .as_str()
                .map(|r| format!(": {r}"))
                .unwrap_or_default();
            bad.push(format!("{q} {n:.2} > {max}{why}"));
        }
    }
    bad
}

/// What a judge asks with: the Jev tool's whole answer to (questions, state, agent), `cached` set when the cache gave it;
/// the unscoped banned phrases in a text.
pub struct Judge<'a> {
    pub ask: &'a dyn Fn(&Value, Value, &str) -> Result<Value, String>,
    pub banned: &'a dyn Fn(&str) -> Result<Vec<String>, String>,
    /// When the judge started: one document's asks stop at `ASK`, as doc_ready.py's process was ended there.
    pub started: std::time::Instant,
}

/// The doc pass's own bound on one document, never a call's (the tool holds those): a section is about 2 s, and the
/// longest RFC has some 40.
const ASK: std::time::Duration = std::time::Duration::from_secs(600);

impl Judge<'_> {
    /// The tool's answers on one text of `rel`, `section` named when it is one: the host's jev tool is this module's one
    /// door to the service, and the core caches, counts, charges and keeps the call; the log's line is the tool's side
    /// (`jev::log_row`). Err(TOO_LONG) when the tool answers that the text is too long.
    fn ask_one(
        &self,
        spec: &Value,
        who: &str,
        part: &str,
        rel: &str,
        section: Option<&str>,
    ) -> Result<Value, String> {
        if self.started.elapsed() > ASK {
            return Err(format!(
                "past its {} minutes, no further part asked",
                ASK.as_secs() / 60
            ));
        }
        // the path keys the cache and names the log's row: a renamed or copied document, a section two documents share,
        // gets its own (/code-review)
        let state = json!({"doc": part, "reader": who, "_for": [rel, section]});
        let answer = (self.ask)(&spec["questions"], state, AGENT)?;
        match answer["too_long"] == true {
            true => Err(TOO_LONG.into()),
            false => Ok(answer["answers"].clone()),
        }
    }

    /// The document at `path`, judged for `reader` (its audience line, else a generic reader) under its path on main, `rel`;
    /// `parts` judges each section too. What it says goes to `out`; the failures, none when ready. A French document
    /// carrying a banned phrase fails before Jev is asked, with no verdict line.
    pub fn judge(
        &self,
        path: &Path,
        rel: &str,
        reader: Option<&str>,
        spec: &Value,
        parts: bool,
        out: &mut String,
    ) -> Result<Vec<String>, String> {
        let raw = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let md = rel.ends_with(".md");
        let doc = if md { md_text(&raw) } else { text(&raw) };
        let who = reader.map_or_else(|| reader_of(&raw), str::to_string);
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        if re!(r#"\x3chtml[^>]*\blang="fr""#).is_match(&raw) {
            let hits = (self.banned)(&doc)?;
            if !hits.is_empty() {
                let quoted: Vec<String> = hits.iter().map(|h| format!("'{h}'")).collect();
                out.push_str(&format!("{name}: banned phrase {}, Jev not asked (private/dashboard/bin/dash store text banned-phrases.edn)\n", quoted.join(", ")));
                return Ok(hits
                    .iter()
                    .map(|h| format!("banned phrase '{h}'"))
                    .collect());
            }
        }
        let questions: Vec<String> = spec["questions"]
            .as_object()
            .into_iter()
            .flatten()
            .map(|(q, _)| q.clone())
            .collect();
        let score = |a: &Value, q: &str| noul(&a[q]).map_or("None".into(), |n| n.to_string());
        let whole = match self.ask_one(spec, &who, &doc, rel, None) {
            Err(e) if e == TOO_LONG => None,
            other => Some(other?),
        };
        for q in whole.iter().flat_map(|_| &questions) {
            out.push_str(&format!(
                "  {q}: {}\n",
                score(whole.as_ref().expect("whole"), q)
            ));
        }
        let split = match (parts || whole.is_none()) && !md {
            true => sections(&doc),
            false => vec![],
        };
        if whole.is_none() && split.is_empty() {
            return Err(format!("{TOO_LONG}, and it has no section to judge by"));
        }
        // judged by section, the preamble is read too, as its own part: the verdict stands on every word
        let intro = whole
            .is_none()
            .then(|| preamble(&doc))
            .filter(|p| !p.is_empty());
        let mut scored = vec![];
        for (i, part) in intro.iter().chain(&split).enumerate() {
            let heading = match (i, &intro) {
                (0, Some(_)) => INTRO.to_string(),
                _ => part.lines().next().unwrap_or_default()[3..].to_string(),
            };
            match self.ask_one(spec, &who, part, rel, Some(&heading)) {
                Ok(got) => {
                    let said: Vec<String> = questions
                        .iter()
                        .map(|q| format!("{q} {}", score(&got, q)))
                        .collect();
                    out.push_str(&format!("  ## {heading}: {}\n", said.join(", ")));
                    scored.push(got);
                }
                // the verdict needs every section when it stands on them
                Err(e) if whole.is_none() => return Err(e),
                Err(e) => {
                    out.push_str(&format!(
                        "  ## {heading}: Jev not asked ({e}), no further section asked\n"
                    ));
                    break;
                }
            }
        }
        let answers = whole.unwrap_or_else(|| {
            out.push_str(&format!(
                "  {BY_SECTION}: {} sections, worst score per question\n",
                scored.len()
            ));
            let worst = questions.iter().filter_map(|q| {
                let n = scored.iter().filter_map(|g| noul(&g[q])).reduce(f64::max)?;
                Some((q.clone(), json!({"noul": n, "type": "noul"})))
            });
            Value::Object(worst.collect())
        });
        let bad = failures(&answers, &spec["gate"], &spec["reasons"]);
        let verdict = if bad.is_empty() {
            "ready".into()
        } else {
            format!("NOT READY: {}", bad.join("; "))
        };
        out.push_str(&format!("{name} (reader: {who}): {verdict}\n"));
        Ok(bad)
    }
}

/// One plain sentence for a failure, never a trace: what the chip and the last line of `dash doc ready` show. The tool says
/// why it could not answer in its own words, a spent or refused key included.
pub fn meaning(e: &str) -> String {
    if e.contains(TOO_LONG) {
        return "too long for one read, even one section at a time".into();
    }
    let first: String = e.lines().next().unwrap_or("").chars().take(140).collect();
    format!("Jev could not answer: {first}")
}

/// A control's name says what it must give: ready-<x>.html passes the whole gate; not-ready-<reason>-<x>.html fails
/// not_ready_<reason> with a score, never « not answered ».
pub fn control_ok(name: &str, bad: &[String]) -> Result<bool, String> {
    let m = re!(r"^(?:not-ready-(placeholder|gap)|ready)-").captures(name);
    let m = m.ok_or(format!(
        "{name}: a control is ready-<x>.html or not-ready-(placeholder|gap)-<x>.html"
    ))?;
    Ok(match m.get(1) {
        Some(reason) => {
            let scored =
                Regex::new(&format!(r"^not_ready_{} \d", reason.as_str())).expect("a pattern");
            bad.iter().any(|b| scored.is_match(b))
        }
        None => bad.is_empty(),
    })
}

/// The document's path in its own checkout: a worktree's document gets its row under the path it has on main once merged.
pub fn rel_of(path: &Path, repo: &Path) -> String {
    let full = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let mut git = std::process::Command::new("git");
    git.arg("-C")
        .arg(full.parent().unwrap_or(Path::new(".")))
        .args(["rev-parse", "--show-toplevel"]);
    // never an inherited GIT_DIR: git would name its top, not the document's
    for (k, _) in std::env::vars_os().filter(|(k, _)| k.to_string_lossy().starts_with("GIT_")) {
        git.env_remove(k);
    }
    let top = git.output().ok().filter(|o| o.status.success());
    let top = top.map_or_else(
        || std::fs::canonicalize(repo).unwrap_or(repo.to_path_buf()),
        |o| Path::new(String::from_utf8_lossy(&o.stdout).trim()).to_path_buf(),
    );
    full.strip_prefix(&top)
        .unwrap_or(&full)
        .to_string_lossy()
        .into_owned()
}

fn host_ask(questions: &Value, state: Value, agent: &str) -> Result<Value, String> {
    (crate::host().jev_ask)(questions, state, agent)
}

fn host_banned(text: &str) -> Result<Vec<String>, String> {
    let rows = (crate::host().older)(&["store", "text", "banned-phrases.edn"])?;
    crate::outbound::banned_unscoped(&rows, text)
}

/// The host's judge: its Jev tool, and its banned phrases from the older app's model.
fn host_judge() -> Judge<'static> {
    Judge {
        ask: &host_ask,
        banned: &host_banned,
        started: std::time::Instant::now(),
    }
}

fn spec(repo: &Path) -> Result<Value, String> {
    let path = repo.join(QUESTIONS);
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("{QUESTIONS}: {e}"))
}

/// The doc pass's ask: Ok once Jev gave a verdict, ready or not, else what stopped it.
pub fn ask(repo: &Path, rel: &str) -> Result<(), String> {
    let mut out = String::new();
    let judged =
        spec(repo).and_then(|s| host_judge().judge(&repo.join(rel), rel, None, &s, true, &mut out));
    judged.map_err(|e| meaning(&e))?;
    match out.contains(" (reader: ") {
        true => Ok(()),
        false => Err(out.lines().last().unwrap_or("no verdict").to_string()),
    }
}

pub const USAGE: &str = "dash doc ready <doc>... [--reader <who>] | --controls (is it ready for its reader? Jev answers, an unchanged text free; exit 1 when not, 2 when Jev was not asked)";

/// `dash doc ready`: its exit code and what it says.
pub fn command(args: &[&str]) -> (i32, String) {
    let repo = (crate::host().repo)();
    let mut out = String::new();
    let run = |out: &mut String| -> Result<bool, String> {
        let spec = spec(&repo)?;
        if args == ["--controls"] {
            let mut names: Vec<String> = std::fs::read_dir(repo.join(CONTROLS))
                .map_err(|e| format!("{CONTROLS}: {e}"))?
                .filter_map(|e| e.ok()?.file_name().into_string().ok())
                .filter(|n| n.ends_with(".html"))
                .collect();
            names.sort();
            let mut wrong = vec![];
            for n in names {
                let path = repo.join(CONTROLS).join(&n);
                let bad = host_judge().judge(
                    &path,
                    &format!("{CONTROLS}/{n}"),
                    None,
                    &spec,
                    false,
                    out,
                )?;
                if !control_ok(&n, &bad)? {
                    wrong.push(n);
                }
            }
            out.push_str(&match wrong.is_empty() {
                true => "controls: all as expected\n".into(),
                false => format!("controls: WRONG {}\n", wrong.join(", ")),
            });
            return Ok(wrong.is_empty());
        }
        let at = args.iter().position(|a| *a == "--reader");
        let reader = at.and_then(|i| args.get(i + 1)).copied();
        let docs: Vec<&str> = args
            .iter()
            .enumerate()
            .filter(|(i, _)| at.is_none_or(|r| *i != r && *i != r + 1))
            .map(|(_, a)| *a)
            .collect();
        if docs.is_empty() || (at.is_some() && reader.is_none()) {
            return Err(format!("usage: {USAGE}"));
        }
        let mut ready = true;
        for d in docs {
            let path = Path::new(d);
            // a judge a document: its 10 minutes are its own, never the documents' before it (/code-review)
            ready &= host_judge()
                .judge(path, &rel_of(path, &repo), reader, &spec, true, out)?
                .is_empty();
        }
        Ok(ready)
    };
    let code = match run(&mut out) {
        Ok(ready) => i32::from(!ready),
        Err(e) if e.starts_with("usage: ") => {
            out.push_str(&format!("{e}\n"));
            2
        }
        Err(e) => {
            out.push_str(&format!("Jev not asked: {}\n", meaning(&e)));
            2
        }
    };
    (code, out)
}
