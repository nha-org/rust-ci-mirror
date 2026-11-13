//! What waits on him, `dash handoff` and /handoff, from the older app's SQLite file: `Dashboard.Handoff.report/0` ported
//! (task rust-handoff-port; its `dash handoff` waited 66 s on `Dashboard.Store.Lock`, 2026-11-13). Everything is read in
//! one read transaction, a WAL snapshot no writer blocks; the steps a check closed are then ticked each in its own short
//! write, which a writer holding the store skips until the next run (rust/CLAUDE.md rule 5: no lock across a scan).
//!
//! The steps are handoff.md's, `handoff_steps` rows put back into their collection's shell. Each carries one JSON line
//! saying what closes it (`{"check": {...}}`, `"task"`, `"until"`, `"from"`, `"waits"`, `"judged"`); the module doc of
//! `Dashboard.Handoff` lists them. The rest is computed: pending drafts with new wording, the `new` blocks they use, the
//! facts to confirm of the evidence files, the sandbox approval, the requests waiting on his word, his comments.
//!
//! ponytail: what the older app read that this does not, each named where it was: a host step's dry run
//! (`Approvals.dry_run`, its « step held » warning), the outbox's « on no step » warning and `Model.go?` (task
//! rust-outbox-rules).
use crate::older::{Piece, STEP, opened, pieces};
use regex::Regex;
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Map, Value, json};
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::Duration;

const STALE_DAYS: i64 = 7;
const PAGE: &str = "http://100.72.158.115:7788/handoff";
const FILE: &str = "private/store/handoff.md";
const CLOSED_DRAFT: [&str; 3] = ["sent", "moot", "rejected"];
const TERMINAL: [&str; 3] = ["won", "lost", "dropped"];
const OPEN_LEAD: [&str; 2] = ["lead", "considering"];
/// His own mailboxes (`Dashboard.Mail`'s `@own`): a mail he sends from one is no approval he owes.
const OWN: [&str; 4] = [
    "hanicolas74@gmail.com",
    "nicolas.ha.pro@gmail.com",
    "maltsevaha@gmail.com",
    "nicolas@klar.im",
];

/// What Claude tried before leaving each computed part to him (@nha, 2026-09-27: « for each you must add a one bullet
/// about what you tried »).
const TRIED_SANDBOX: &str = "Essayé : la règle qui manquait est écrite en .candidate, à côté du refus qu'elle corrige, et `make sandbox/check` passe ; mais le profil est en lecture seule pour Claude, et sandbox-apply tourne hors du bac à sable, exprès. Les scripts qui appliquent ou lancent le profil (bin/sandbox-apply, bin/approved-only, sandbox/*.sh) ne tournent que tels que tu les as approuvés : la même approbation couvre les deux ; le reste du code hôte suit master sans toi.";
const TRIED_DRAFTS: &str = "Essayé : chaque brouillon est assemblé des blocs, passé à sepia-review et au gate Jev ; ceux-ci contiennent une formulation nouvelle (bloc :new ou texte libre), donc ils attendent ton accord. Un message fait seulement de blocs approuvés part sans relecture (ta règle du 2026-10-03).";
const TRIED_BLOCKS: &str = "Essayé : la phrase est écrite et utilisée dans un brouillon en attente ; une formulation nouvelle en ton nom doit être validée une fois par toi (blocks.edn, :status :new).";
const TRIED_QUESTIONS: &str = "Essayé : les fichiers de preuves, les notes WhatsApp, l'historique des dépôts et le CV ; aucune source ne tranche, et un fait sur ton passé ne s'invente pas.";
const TRIED_INBOX: &str = "Essayé : la demande est déposée dans le to_process du dépôt ; mais elle élargit ses permissions (bac à sable, hooks qui coûtent des jetons), et une session refuse à juste titre cet élargissement sur la parole d'une autre (klar, 2026-09-27).";

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("a valid pattern")
}

macro_rules! regex {
    ($name:ident, $pattern:expr) => {
        static $name: LazyLock<Regex> = LazyLock::new(|| re($pattern));
    };
}

regex!(BOX, r"^- \[([ xX])\] (.*)$");
regex!(INDENTED, r"^\s{2,}\S");
regex!(META, r#"^\{[":}]"#);
regex!(OU, r"^Où :\s*(.*)$");
regex!(
    PLACE,
    r"(?i)^(?:(?:sur ton |sur le |depuis (?:le |ton )?)?(?:i?phone|t[ée]l[ée]phone|Mac(?: mini)?)\b(?:\s+ou\s+|\s*\|\s*)?)+[\s,.;:]*"
);
regex!(PLACED, r"^\s+Où :");
regex!(OWN_DRAFT_LINK, r"\]\(/d/([\w-]+)");
regex!(LINK_TARGET, r"\]\(([^)\s]+)\)");
regex!(UNLINK, r"\]\([^)]*\)");
regex!(TITLE, r"^\*\*(.+?)\*\*");
regex!(MAC_WHY, r"(?i)\s*\(Mac\s*[:,)][^)]*\)?");
regex!(HAS_LINK, r"\]\(|https?://|`[^`]+\.(md|edn|pdf|html|svg)`");
regex!(PROSE_SKIP, r"^(@nha:|\*\*?(Fait|Noté))");
regex!(EMAIL, r"[\w.+-]+@[\w-]+(?:\.[\w-]+)+");
regex!(
    EVENT,
    r"(?i)\b(entretiens?|interviews?|appels?|calls?|visios?|rendez-vous|rdv|réunions?|meetings?|rencontres?|deadlines?|échéances?|date limite)\b"
);
regex!(
    DATED,
    r"(?i)\b\d{1,2}/(0[1-9]|1[0-2])\b|\b\d{4}-\d{2}-\d{2}\b|\b\d{1,2}(er)?\s(janv(ier)?|févr(ier)?|mars|avr(il)?|mai|juin|juil(let)?|août|sept(embre)?|oct(obre)?|nov(embre)?|déc(embre)?|jan(uary)?|feb(ruary)?|mar(ch)?|apr(il)?|may|june?|july?|aug(ust)?|sep(t|tember)?|dec(ember)?)\b|\b(janv(ier)?|févr(ier)?|mars|avr(il)?|mai|juin|juil(let)?|août|sept(embre)?|oct(obre)?|nov(embre)?|déc(embre)?|jan(uary)?|feb(ruary)?|mar(ch)?|apr(il)?|may|june?|july?|aug(ust)?|sep(t|tember)?|dec(ember)?)\.?\s\d{1,2}\b|pas encore de date"
);
regex!(CODE_OR_URL, r"`[^`]*`|https?://\S+");
regex!(FLAG, r"(?i)à confirmer|à lui faire préciser");
regex!(POINTER, r"(?i)«\s*à confirmer\s*»");
regex!(
    ANSWERED,
    r"(?i)\b(répondu|answered)\b|plus rien à lui demander"
);
regex!(CONFIRM_SECTION, r"(?i)^##+ .*à confirmer");
regex!(
    AVAILABLE,
    r"(?i)\b(dispo\w*|available|availability|libre|free)\b"
);
regex!(
    MONTH_PART,
    r"(?i)\b(début|mi|fin|early|mid|late)[ -]([a-zéû]+)\b"
);
regex!(WAITING, r"(?m)^\*\*Waiting [^*]*\*\*\s*(.*)$");
regex!(DONE_LINE, r"(?m)^\*\*Done ");
regex!(DAY_IN_NAME, r"\d{4}-\d\d-\d\d");

// ---------------------------------------------------------------- days

/// Days since 1970-01-01 of an exact `YYYY-MM-DD` that is a real day, as `Date.from_iso8601/1` reads one.
pub fn day(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let num = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    let (y, m, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let days = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    if !(1..=12).contains(&m) || d < 1 || d > days[(m - 1) as usize] {
        return None;
    }
    // Howard Hinnant's days_from_civil
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    Some(era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468)
}

// POSIX's tzset, which the libc crate does not bind on every target.
unsafe extern "C" {
    fn tzset();
}

/// `t` on this Mac's clock. tzset first: localtime_r need not read TZ again, and `dash serve` and `dash orch` run for days,
/// across a change of hour. tzset writes libc's zone globals, so one caller at a time.
pub fn local_tm(t: libc::time_t) -> libc::tm {
    static TZ: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _one = TZ.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    // SAFETY: localtime_r writes only into `tm`, which outlives the call.
    unsafe {
        tzset();
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        tm
    }
}

/// `secs` since the epoch on this Mac's clock, `YYYY-MM-DDTHH:MM:SS`, as the older app's `NaiveDateTime.to_iso8601`.
pub fn local_iso(secs: i64) -> String {
    let tm = local_tm(secs as libc::time_t);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec
    )
}

/// His local day now, as the steps are dated, from the store's clock port.
pub fn today() -> String {
    use dash_core::WallClock;
    local_iso(crate::oplog::SystemClock.now_ms() / 1000)[..10].to_string()
}

/// An ISO datetime with its zone (`Z` or `±HH:MM`, a fraction allowed) as seconds since the epoch.
fn utc_secs(at: &str) -> Option<i64> {
    let d = day(at.get(0..10)?)?;
    if at.as_bytes().get(10) != Some(&b'T') {
        return None;
    }
    let num = |r: std::ops::Range<usize>| at.get(r)?.parse::<i64>().ok();
    let (h, mi, s) = (num(11..13)?, num(14..16)?, num(17..19)?);
    let mut rest = at.get(19..)?;
    if let Some(f) = rest.strip_prefix('.') {
        rest = f.trim_start_matches(|c: char| c.is_ascii_digit());
    }
    let offset = match rest {
        "Z" => 0,
        _ if rest.len() == 6 && (rest.starts_with('+') || rest.starts_with('-')) => {
            let sign = if rest.starts_with('-') { -1 } else { 1 };
            sign * (rest.get(1..3)?.parse::<i64>().ok()? * 3600
                + rest.get(4..6)?.parse::<i64>().ok()? * 60)
        }
        _ => return None,
    };
    Some(d * 86_400 + h * 3600 + mi * 60 + s - offset)
}

/// `d` after `since` at `since`'s precision: a day is strict, a datetime lets a later moment that day pass.
fn after(d: &str, since: Option<&str>) -> bool {
    match since {
        None => true,
        Some(a) => {
            let cut: String = d.chars().take(a.chars().count()).collect();
            cut.as_str() > a
        }
    }
}

// ---------------------------------------------------------------- text

fn unbold(s: &str) -> String {
    s.replace("**", "")
}

fn unlink(l: &str) -> String {
    UNLINK.replace_all(l, "]").into_owned()
}

fn head(s: &str) -> String {
    s.chars().take(60).collect()
}

/// A text lowercased, its letters, digits and `<placeholders>` kept, one space between words.
pub fn plain(text: &str) -> String {
    let lower = unbold(text).to_lowercase();
    let mut out = String::with_capacity(lower.len());
    let mut gap = false;
    for c in lower.chars() {
        if c.is_alphanumeric() || c == '<' || c == '>' {
            if gap && !out.is_empty() {
                out.push(' ');
            }
            gap = false;
            out.push(c);
        } else {
            gap = true;
        }
    }
    out
}

/// `text` cut into sentences, each `plain`'s.
pub fn sentences(text: &str) -> Vec<String> {
    split_sentences(text)
        .iter()
        .map(|p| plain(p))
        .filter(|p| !p.is_empty())
        .collect()
}

/// `text` cut after `.`, `?` or `!` before a blank, and at each line break, as the older app's `(?<=[.?!])\s+|\n+`.
fn split_sentences(text: &str) -> Vec<String> {
    let mut parts = vec![];
    let mut cur = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\n' {
            parts.push(std::mem::take(&mut cur));
            continue;
        }
        cur.push(c);
        if matches!(c, '.' | '?' | '!') && chars.peek().is_some_and(|n| n.is_whitespace()) {
            parts.push(std::mem::take(&mut cur));
        }
    }
    parts.push(cur);
    parts
}

/// A negation a placeholder never stands for: « je suis <dispo> pour » is not said by « je suis pas disponible pour ».
const NEGATIONS: [&str; 7] = ["pas", "plus", "jamais", "aucun", "not", "no", "never"];

/// Whether sentence `t` (`plain`'s, a `<placeholder>` standing for one to four words) is said in `body` (`plain`'s):
/// `None` for a sentence too thin to tell (under 8 letters outside its placeholders).
pub fn said(t: &str, body: &str) -> Option<bool> {
    let mut parts = vec![];
    let mut rest = t;
    while let Some(open) = rest.find('<') {
        match rest[open..].find('>') {
            Some(close) => {
                parts.push(&rest[..open]);
                rest = &rest[open + close + 1..];
            }
            None => break,
        }
    }
    parts.push(rest);
    if parts.concat().replace(' ', "").chars().count() < 8 {
        return None;
    }
    let words: Vec<&str> = parts
        .iter()
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .collect();
    if !words.iter().all(|w| body.contains(w)) {
        return Some(false);
    }
    let slot = r"((?:\w+ ?){1,4}?)";
    let joined = parts
        .iter()
        .map(|p| regex::escape(p))
        .collect::<Vec<_>>()
        .join(slot);
    let pattern = format!(r"(?:^|[^\w])({joined})(?:$|[^\w])");
    let Ok(found) = Regex::new(&pattern) else {
        return Some(false);
    };
    let free = |c: &regex::Captures| {
        c.iter().skip(2).flatten().all(|m| {
            !m.as_str()
                .split_whitespace()
                .any(|w| NEGATIONS.contains(&w))
        })
    };
    // each start tried, a match with a negation in a slot skipped to the next character: the older app's lookahead in the
    // slot, which never lets a negated match hide a good one that overlaps it (/code-review)
    let mut at = 0;
    while let Some(c) = found.captures_at(body, at) {
        if free(&c) {
            return Some(true);
        }
        let start = c.get(0).map_or(body.len(), |m| m.start());
        at = start + body[start..].chars().next().map_or(1, char::len_utf8);
        if at > body.len() {
            break;
        }
    }
    Some(false)
}

/// Whether every sentence of `t` that can tell is said in `body` (`plain`'s), and one at least.
fn says(t: &str, body: &str) -> bool {
    let hits: Vec<bool> = sentences(t).iter().filter_map(|x| said(x, body)).collect();
    !hits.is_empty() && hits.iter().all(|h| *h)
}

// ---------------------------------------------------------------- the steps of handoff.md

/// A `- [ ]` or `- [x]` item: its line (0-based), its box, its text, its indented lines less two spaces (its check line
/// out of them, read into `meta`), and whether one said where it is done (« Où : »).
#[derive(Clone, Debug)]
pub struct Step {
    pub line: usize,
    pub done: bool,
    pub text: String,
    pub more: Vec<String>,
    pub meta: Meta,
    pub placed: bool,
}

#[derive(Clone, Debug, Default)]
pub enum Meta {
    #[default]
    None,
    Read(Map<String, Value>),
    Unreadable(String),
}

impl Meta {
    fn get(&self, k: &str) -> Option<&Value> {
        match self {
            Meta::Read(m) => m.get(k),
            _ => None,
        }
    }

    fn str(&self, k: &str) -> Option<String> {
        self.get(k).and_then(text_of)
    }
}

/// A JSON value as the older app's `to_string`: a string as is, a number or a boolean written, nothing else.
fn text_of(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// A value written in the model's tagged form (`"~:x"`) as its plain name, all through.
fn plain_value(v: Value) -> Value {
    match v {
        Value::String(s) => Value::String(s.strip_prefix("~:").map(String::from).unwrap_or(s)),
        Value::Array(xs) => Value::Array(xs.into_iter().map(plain_value).collect()),
        Value::Object(kvs) => {
            Value::Object(kvs.into_iter().map(|(k, x)| (k, plain_value(x))).collect())
        }
        other => other,
    }
}

fn lines(text: &str) -> Vec<&str> {
    let mut ls: Vec<&str> = text.split('\n').collect();
    if ls.last() == Some(&"") {
        ls.pop();
    }
    ls
}

/// An old « Où : » line keeps only its words past the place; a Mac-only step says why: « (Mac : why) » (steps-no-where).
fn unplace(line: &str) -> String {
    let Some(c) = OU.captures(line) else {
        return line.to_string();
    };
    let rest = c.get(1).map_or("", |m| m.as_str());
    let place = PLACE.find(rest).map_or("", |m| m.as_str());
    let why = &rest[place.len()..];
    let r = if place.is_empty() || place.to_lowercase().contains("phone") {
        why.to_string()
    } else if why.is_empty() {
        "(Mac)".into()
    } else {
        format!("(Mac : {why})")
    };
    let mut cs = r.chars();
    match cs.next() {
        Some(f) => f.to_uppercase().collect::<String>() + cs.as_str(),
        None => String::new(),
    }
}

/// Every step of `text`.
pub fn steps(text: &str) -> Vec<Step> {
    let ls = lines(text);
    let mut out = vec![];
    for (i, l) in ls.iter().enumerate() {
        let Some(c) = BOX.captures(l) else { continue };
        let raw: Vec<&str> = ls[i + 1..]
            .iter()
            .take_while(|x| INDENTED.is_match(x))
            .copied()
            .collect();
        let more: Vec<String> = raw
            .iter()
            .map(|x| unplace(x.strip_prefix("  ").unwrap_or(x).trim_end()))
            .filter(|x| !x.is_empty())
            .collect();
        let (metas, more): (Vec<String>, Vec<String>) =
            more.into_iter().partition(|x| META.is_match(x));
        let meta = match metas.first() {
            None => Meta::None,
            Some(m) => match serde_json::from_str::<Value>(m).map(plain_value) {
                Ok(Value::Object(o)) => Meta::Read(o),
                _ => Meta::Unreadable(m.clone()),
            },
        };
        out.push(Step {
            line: i,
            done: &c[1] != " ",
            text: c[2].to_string(),
            more,
            meta,
            placed: raw.iter().any(|x| PLACED.is_match(x)),
        });
    }
    out
}

fn step_end(ls: &[&str], line: usize) -> usize {
    line + 1
        + ls[line + 1..]
            .iter()
            .take_while(|x| INDENTED.is_match(x))
            .count()
}

/// `text` with the step at `line` ticked and « **Fait <today> :** <evidence> » under its lines.
pub fn done(text: &str, line: usize, today: &str, evidence: &str) -> String {
    let ls = lines(text);
    let end = step_end(&ls, line);
    let mut out: Vec<String> = ls.iter().map(|l| l.to_string()).collect();
    out[line] = out[line].replacen("- [ ]", "- [x]", 1);
    out.insert(end, format!("  **Fait {today} :** {evidence}"));
    out.join("\n") + "\n"
}

fn task_of(st: &Step) -> Option<String> {
    st.meta
        .str("task")
        .filter(|t| !t.is_empty())
        .map(|t| t.trim_start_matches(':').to_string())
}

fn answered(st: &Step) -> bool {
    st.more
        .last()
        .is_some_and(|l| l.trim().starts_with("@nha:"))
}

fn step_title(st: &Step) -> Option<String> {
    TITLE.captures(&st.text).map(|c| c[1].to_string())
}

/// A step's lines he reads: not a command (indented), not his answer, not Claude's reply.
fn prose(more: &[String]) -> Vec<&String> {
    more.iter()
        .filter(|l| !(l.starts_with("    ") || PROSE_SKIP.is_match(l.trim())))
        .collect()
}

/// The repo whose CI runner a step's command starts: herdr tries it first (task runner-start-herdr).
fn herdr_repo(st: &Step) -> Option<String> {
    static RUNNER: LazyLock<Regex> =
        LazyLock::new(|| re(r"runner-restart\.sh|\.action-runner/run\.sh"));
    static REPO: LazyLock<Regex> = LazyLock::new(|| re(r"~/repo/([\w.-]+)"));
    st.more.iter().find_map(|l| {
        (l.starts_with(char::is_whitespace) && !l.trim().is_empty() && RUNNER.is_match(l))
            .then(|| REPO.captures(l).map(|c| c[1].to_string()))
            .flatten()
    })
}

// ---------------------------------------------------------------- what closes a step

/// What a step is on a day: closed (the evidence), later (a day or what it waits on), stale (when last judged), answered
/// (his word is the last), refused (why), or open.
#[derive(Clone, Debug, PartialEq)]
pub enum Verdict {
    Closed(String),
    Later(String),
    Stale(String),
    Answered,
    Refused(String),
    Open,
}

/// What a verdict reads besides the step: the model's rows and the files the checks name. No network.
#[derive(Default)]
pub struct Facts {
    /// klar.im's Sent Items as `dash mail` last indexed them: `to`, `subject`, `date`.
    pub sent: Vec<Value>,
    /// Every draft, oldest first, as `dash model drafts` lists them.
    pub drafts: Vec<Value>,
    pub leads: HashMap<String, Value>,
    /// Each block by id: `status`, `short`, `short-en`.
    pub blocks: HashMap<String, Value>,
    /// Each task's status and owner.
    pub tasks: HashMap<String, (String, String)>,
    /// `~/.claude/logins.jsonl`'s lines.
    pub logins: Vec<Value>,
    /// The repo a `file-changed` check's path is under, and the home a `~/` one is.
    pub root: PathBuf,
    pub home: PathBuf,
    /// klar-mail's folder: a step waiting on a reply reads its mails' heads.
    pub mail: PathBuf,
    /// Whether herdr's runner pass tries a runner step first.
    pub runners: bool,
}

impl Facts {
    fn draft(&self, id: &str) -> Option<&Value> {
        self.drafts.iter().find(|d| d["id"].as_str() == Some(id))
    }
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}

/// The evidence that `check` is met, as a short French line written under the step.
fn check_met(c: &Map<String, Value>, f: &Facts) -> Option<String> {
    let get = |k: &str| c.get(k).and_then(text_of);
    if let Some(to) = get("sent-to") {
        let (to_l, subject) = (to.to_lowercase(), get("subject").map(|x| x.to_lowercase()));
        return f.sent.iter().find_map(|m| {
            let date = text_of(&m["date"]).unwrap_or_default();
            (s(m, "to").to_lowercase().contains(&to_l)
                && subject
                    .as_ref()
                    .is_none_or(|x| s(m, "subject").to_lowercase().contains(x))
                && after(&date, get("after").as_deref()))
            .then(|| {
                format!(
                    "envoyé à {to} le {} (klar.im, Envoyés)",
                    date.chars().take(10).collect::<String>()
                )
            })
        });
    }
    if let Some(id) = get("lead") {
        let o = f.leads.get(&id)?;
        let (status, applied) = (
            s(o, "status"),
            text_of(&o["applied"]).filter(|a| !a.is_empty()),
        );
        return (applied.is_some() || !OPEN_LEAD.contains(&status)).then(|| {
            format!("la fiche {id} est {status}")
                + &applied
                    .map(|a| format!(", candidature le {a}"))
                    .unwrap_or_default()
        });
    }
    if let Some(id) = get("lead-closed") {
        let status = s(f.leads.get(&id)?, "status");
        return TERMINAL
            .contains(&status)
            .then(|| format!("la fiche {id} est close ({status})"));
    }
    if let Some(file) = get("file-changed") {
        let path = match file.strip_prefix("~/") {
            Some(rest) => f.home.join(rest),
            None => f.root.join(&file),
        };
        let mtime = std::fs::metadata(path).ok()?.modified().ok()?;
        let secs = mtime.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs() as i64;
        let d = local_iso(secs);
        return after(&d, get("after").as_deref())
            .then(|| format!("{file} modifié le {}", &d[..10]));
    }
    if let Some(id) = get("draft") {
        let id = id.trim_start_matches(':');
        let status = f.draft(id).map(|d| s(d, "status")).unwrap_or("");
        return CLOSED_DRAFT
            .contains(&status)
            .then(|| format!("le brouillon {id} est {status}"));
    }
    if let Some(id) = get("block") {
        let status = s(f.blocks.get(&id)?, "status");
        return (status != "new").then(|| format!("le bloc {id} est validé"));
    }
    if let Some(site) = get("signin") {
        let since = get("after")?;
        return f.logins.iter().find_map(|e| {
            if s(e, "site") != site || s(e, "event") != "signin" {
                return None;
            }
            let local = local_iso(utc_secs(s(e, "at"))?);
            after(&local, Some(&since)).then(|| {
                format!(
                    "{site} connecté le {} à {} (bin/logins)",
                    &local[..10],
                    &local[11..16]
                )
            })
        });
    }
    None
}

/// A closed task closes its steps: done or dropped, what the step was for is over.
fn task_closed(st: &Step, f: &Facts) -> Option<String> {
    let id = task_of(st)?;
    let (status, _) = f.tasks.get(&id)?;
    ["done", "dropped"]
        .contains(&status.as_str())
        .then(|| format!("la tâche {id} est close ({status})"))
}

/// The draft a step links went out after the step was last judged, itself or carried by a later mail (half its
/// sentences or more, two at least): what the step asked is done.
fn sent_again(st: &Step, f: &Facts) -> Option<String> {
    let id = OWN_DRAFT_LINK.captures(&st.text)?[1].to_string();
    let judged = st.meta.str("judged")?;
    let d = f.draft(&id)?;
    let own: Vec<String> = sentences(s(d, "body"))
        .into_iter()
        .filter(|t| t.chars().count() >= 25)
        .collect();
    f.drafts.iter().find_map(|y| {
        let sent_on = text_of(&y["sent-on"]).unwrap_or_default();
        let same = s(y, "id") == id;
        let carries = || {
            let b = plain(s(y, "body"));
            let n = own.iter().filter(|t| b.contains(t.as_str())).count();
            !own.is_empty() && n >= 2 && 2 * n >= own.len()
        };
        (s(y, "status") == "sent" && sent_on > judged && (same || carries())).then(|| {
            let carried = if same {
                String::new()
            } else {
                format!(" dans {}", s(y, "id"))
            };
            format!("{id} est parti{carried} le {sent_on}")
        })
    })
}

/// Each address in `text`, lower case.
fn addresses(text: &str) -> Vec<String> {
    EMAIL
        .find_iter(text)
        .map(|m| m.as_str().to_lowercase())
        .collect()
}

/// A step waiting on someone's reply: their reply's name until a mail from that address dated after that day is in.
fn unreplied(w: &Map<String, Value>, f: &Facts) -> Option<String> {
    let from = w.get("reply-from").and_then(Value::as_str)?;
    let since = w.get("after").and_then(Value::as_str);
    let wanted = from.to_lowercase();
    let names = std::fs::read_dir(&f.mail).into_iter().flatten().flatten();
    let came = names.into_iter().any(|e| {
        let name = e.file_name().to_string_lossy().into_owned();
        let Some(d) = DAY_IN_NAME.find(&name).map(|m| m.as_str().to_string()) else {
            return false;
        };
        name.ends_with(".eml")
            && !name.starts_with("other-sent-")
            && after(&d, since)
            && mail_from(&e.path()).is_some_and(|h| addresses(&h).contains(&wanted))
    });
    (!came).then(|| format!("{from}'s reply"))
}

/// A mail's From, unfolded: its header lines up to the first blank one.
fn mail_from(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    let text = String::from_utf8_lossy(&bytes);
    let head = text.split("\r\n\r\n").next()?.split("\n\n").next()?;
    let unfolded = head
        .replace("\r\n", "\n")
        .replace("\n ", " ")
        .replace("\n\t", " ");
    unfolded.lines().find_map(|l| {
        let (k, v) = l.split_once(':')?;
        k.eq_ignore_ascii_case("from").then(|| v.trim().to_string())
    })
}

fn stale(judged: Option<&str>, today: &str) -> bool {
    match (judged.and_then(day), day(today)) {
        (Some(j), Some(t)) => t - j > STALE_DAYS,
        _ => true,
    }
}

/// What step `st` is on `today`.
pub fn verdict(st: &Step, today: &str, f: &Facts) -> Verdict {
    shut(st, &st.meta, today, f).unwrap_or_else(|| waiting(st, today, f))
}

fn shut(st: &Step, m: &Meta, today: &str, f: &Facts) -> Option<Verdict> {
    if st.done {
        return Some(Verdict::Closed("coché".into()));
    }
    let empty = Map::new();
    let map = match m {
        Meta::Unreadable(l) => {
            return Some(Verdict::Refused(format!("unreadable check line: {l}")));
        }
        Meta::Read(map) => map,
        Meta::None => &empty,
    };
    if let Some(Value::Object(c)) = map.get("check")
        && c.contains_key("signin")
        && !c.contains_key("after")
    {
        return Some(Verdict::Refused(
            r#"a {"signin": …} check needs "after""#.into(),
        ));
    }
    if let Some(w) = map.get("waits") {
        let read = w
            .get("reply-from")
            .and_then(Value::as_str)
            .zip(w.get("after").and_then(Value::as_str));
        return match read {
            Some((from, a)) => {
                if addresses(from) == [from.to_lowercase()] && a.len() == 10 && day(a).is_some() {
                    let mut rest = map.clone();
                    rest.remove("waits");
                    shut(st, &Meta::Read(rest), today, f)
                } else {
                    Some(Verdict::Refused(
                        r#"a {"waits": …} needs "reply-from" one address and "after" a day"#.into(),
                    ))
                }
            }
            None => Some(Verdict::Refused(
                r#"a {"waits": …} needs "reply-from" and "after", two strings"#.into(),
            )),
        };
    }
    let met = || {
        let check = match map.get("check") {
            Some(Value::Object(c)) => check_met(c, f),
            _ => None,
        };
        check
            .or_else(|| task_closed(st, f))
            .or_else(|| sent_again(st, f))
    };
    let until = map.get("until").and_then(text_of);
    if !map.contains_key("check") && until.is_none() && task_of(st).is_none() {
        return Some(Verdict::Refused("no :check nor :until".into()));
    }
    if let Some(u) = until.filter(|u| u.as_str() < today) {
        return Some(Verdict::Closed(format!("expiré le {u}")));
    }
    met().map(Verdict::Closed)
}

fn waiting(st: &Step, today: &str, f: &Facts) -> Verdict {
    let tried = st.more.iter().any(|l| l.starts_with("herdr:"));
    let from = st.meta.str("from").or_else(|| match st.meta.get("check") {
        Some(Value::Object(c)) => c.get("from").and_then(text_of),
        _ => None,
    });
    let waits = match st.meta.get("waits") {
        Some(Value::Object(w)) => unreplied(w, f),
        _ => None,
    };
    if answered(st) {
        Verdict::Answered
    } else if f.runners && herdr_repo(st).is_some() && !tried {
        Verdict::Later("herdr's try".into())
    } else if let Some(w) = waits {
        Verdict::Later(w)
    } else if let Some(fr) = from.filter(|fr| fr.as_str() > today) {
        Verdict::Later(fr)
    } else if stale(st.meta.str("judged").as_deref(), today) {
        Verdict::Stale(st.meta.str("judged").unwrap_or_else(|| "jamais".into()))
    } else {
        Verdict::Open
    }
}

/// Every step with its verdict, sorted for the page: `top` the three open ones he sees (an `until` within 14 days first,
/// the soonest first, then the file's order), `later`, `stale`, `answered`, `closed` (met, not ticked yet), `refused`.
#[derive(Default)]
pub struct Judged {
    pub top: Vec<(Step, Verdict)>,
    pub later: Vec<(Step, Verdict)>,
    pub stale: Vec<(Step, Verdict)>,
    pub answered: Vec<(Step, Verdict)>,
    pub closed: Vec<(Step, Verdict)>,
    pub refused: Vec<(Step, Verdict)>,
}

pub fn judge(text: &str, today: &str, f: &Facts) -> Judged {
    let mut j = Judged::default();
    let mut open = vec![];
    for st in steps(text) {
        let v = verdict(&st, today, f);
        match &v {
            Verdict::Open => open.push((st, v)),
            Verdict::Later(_) => j.later.push((st, v)),
            Verdict::Stale(_) => j.stale.push((st, v)),
            Verdict::Answered => j.answered.push((st, v)),
            Verdict::Closed(_) if !st.done => j.closed.push((st, v)),
            Verdict::Closed(_) => {}
            Verdict::Refused(_) => j.refused.push((st, v)),
        }
    }
    let soon = |st: &Step| {
        let u = st.meta.str("until").unwrap_or_default();
        match (day(&u), day(today)) {
            (Some(a), Some(b)) if a - b <= 14 => u,
            _ => "9999".into(),
        }
    };
    open.sort_by_key(|(st, _)| soon(st));
    let rest = open.split_off(open.len().min(3));
    j.top = open;
    j.later = rest.into_iter().chain(j.later).collect();
    j
}

// ---------------------------------------------------------------- the store's rows

/// handoff.md as the older store holds it: its shell, each step a row in its place.
pub struct Rows {
    pub text: String,
    /// Each step row: its pos, its body as stored, the line it starts on in `text`.
    pub rows: Vec<(i64, String, usize)>,
}

/// handoff.md's text from `db`'s `collections` shell and its `handoff_steps` rows, each marker read (the box's stage
/// « x » or « », a name, a link's target), as `Store.text("handoff.md")` gives it.
pub fn rows(db: &Connection) -> rusqlite::Result<Rows> {
    let shell: Option<String> = db
        .query_row(
            "SELECT shell FROM collections WHERE name = 'handoff.md'",
            [],
            |r| r.get(0),
        )
        .optional()?
        .flatten();
    let shell = shell
        .map(|s| serde_json::from_str::<String>(&s).unwrap_or(s))
        .unwrap_or_default();
    let names = crate::older_drafts::names(db)?;
    let mut q = db.prepare(
        "SELECT pos, id, stage, body FROM handoff_steps WHERE collection = 'handoff.md' ORDER BY pos",
    )?;
    let got = q.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, Option<String>>(1)?,
            r.get::<_, Option<String>>(2)?,
            r.get::<_, Option<String>>(3)?,
        ))
    })?;
    let mut bodies = vec![];
    for row in got {
        let (pos, id, stage, body) = row?;
        let body = body.unwrap_or_default();
        let box_ = if stage.as_deref() == Some("open") {
            " "
        } else {
            "x"
        };
        let from = format!("handoff_steps:{}", id.unwrap_or_default());
        let text: String = pieces(&body)
            .into_iter()
            .map(|p| match p {
                Piece::Text(t) | Piece::Stray(t) => t.to_string(),
                Piece::Other("stage") => box_.to_string(),
                Piece::Mention(r) => names.get(r).cloned().unwrap_or_else(|| r.to_string()),
                Piece::Link(r) => {
                    crate::older_drafts::link_target(db, &from, r).unwrap_or_default()
                }
                Piece::Other(r) => r.to_string(),
            })
            .collect();
        bodies.push((pos, body, text));
    }
    let mut out = vec![];
    let mut rows = vec![];
    let mut next = bodies.into_iter();
    let mut line = 0;
    for l in shell.split('\n') {
        match (
            l.chars().eq([STEP]),
            l.chars().eq([STEP]).then(|| next.next()).flatten(),
        ) {
            (true, Some((pos, raw, text))) => {
                rows.push((pos, raw, line));
                line += text.split('\n').count();
                out.push(text);
            }
            _ => {
                line += 1;
                out.push(l.to_string());
            }
        }
    }
    // a row the shell keeps no place for still shows, at the end, never lost (/code-review)
    for (pos, raw, text) in next {
        rows.push((pos, raw, line));
        line += text.split('\n').count();
        out.push(text);
    }
    Ok(Rows {
        text: out.join("\n"),
        rows,
    })
}

// ---------------------------------------------------------------- the computed parts

fn ids(d: &Value, key: &str) -> Vec<String> {
    let one = |v: &Value| text_of(v).map(|t| t.trim_start_matches(':').to_string());
    match d.get(key) {
        Some(Value::Array(xs)) => xs.iter().filter_map(one).collect(),
        Some(Value::Null) | None => vec![],
        Some(v) => one(v).into_iter().collect(),
    }
}

fn new_parts(d: &Value) -> Vec<String> {
    ids(d, "new-parts")
}

/// The drafts an open step names (`{"check": {"draft": id}}`): Claude never sends one while a step holds it.
pub fn held_drafts(text: &str) -> BTreeSet<String> {
    static UNREAD: LazyLock<Regex> = LazyLock::new(|| re(r#"[:"]draft"?:?\s+"?:?([^"\s}]+)"#));
    steps(text)
        .iter()
        .filter(|st| !st.done)
        .filter_map(|st| match &st.meta {
            Meta::Read(m) => m
                .get("check")
                .and_then(|c| c.get("draft"))
                .and_then(text_of),
            Meta::Unreadable(l) => UNREAD.captures(l).map(|c| c[1].to_string()),
            Meta::None => None,
        })
        .map(|d| d.trim_start_matches(':').to_string())
        .collect()
}

/// Whether block `b` is approved and `body` (`plain`'s) says one of its texts.
fn said_approved(b: Option<&Value>, body: &str) -> bool {
    let Some(Value::Object(b)) = b else {
        return false;
    };
    b.get("status").and_then(Value::as_str) == Some("approved")
        && b.iter()
            .any(|(k, t)| k.starts_with("short") && t.as_str().is_some_and(|t| says(t, body)))
}

/// Whether draft `d` says only approved wording: marked so, or each new part a block approved since that its message
/// says, with no line of its own; its screening answers too.
fn approved_wording(d: &Value, bs: &HashMap<String, Value>) -> bool {
    let body = plain(s(d, "body"));
    let parts = new_parts(d);
    let own_line = match d.get("new-text") {
        Some(Value::Array(xs)) => xs.iter().any(|x| match x {
            Value::Array(p) if p.len() == 2 => p[0].is_null() || p[0].as_str() == Some(""),
            _ => false,
        }),
        _ => false,
    };
    let from_approved = d["from-approved-blocks"] == Value::Bool(true)
        || (!parts.is_empty()
            && !own_line
            && parts.iter().all(|p| said_approved(bs.get(p), &body)));
    let answers = match d.get("answers") {
        Some(Value::Array(xs)) => xs.clone(),
        None | Some(Value::Null) => vec![],
        Some(v) => vec![v.clone()],
    };
    from_approved
        && answers
            .iter()
            .all(|a| match a.get("answer").and_then(Value::as_str) {
                Some(t) => said_approved(
                    a.get("block").and_then(text_of).and_then(|b| bs.get(&b)),
                    &plain(t),
                ),
                None => false,
            })
}

/// The « début octobre » phrases of a sentence about his availability whose time is over on `today`.
fn past_months(text: &str, today: &str) -> bool {
    const MONTHS: [&str; 24] = [
        "janvier",
        "février",
        "mars",
        "avril",
        "mai",
        "juin",
        "juillet",
        "août",
        "septembre",
        "octobre",
        "novembre",
        "décembre",
        "january",
        "february",
        "march",
        "april",
        "may",
        "june",
        "july",
        "august",
        "september",
        "october",
        "november",
        "december",
    ];
    let (month, mday) = (
        today[5..7].parse::<i64>().unwrap_or(1),
        today[8..10].parse::<i64>().unwrap_or(1),
    );
    text.split(['.', '\n'])
        .filter(|x| AVAILABLE.is_match(x))
        .any(|sentence| {
            MONTH_PART.captures_iter(sentence).any(|c| {
                let Some(at) = MONTHS.iter().position(|m| *m == c[2].to_lowercase()) else {
                    return false;
                };
                let back = (month - (at as i64 % 12 + 1)).rem_euclid(12);
                (1..=6).contains(&back)
                    || (back == 0
                        && ["début", "early"].contains(&c[1].to_lowercase().as_str())
                        && mday > 5)
            })
        })
}

/// Whether pending draft `d` goes without him: a mail he sends himself from his own mailbox, or only approved wording,
/// handed over or taken by the outbox (`Sales.pre_approved?`).
fn not_his(d: &Value, f: &Facts, held: &BTreeSet<String>, today: &str) -> bool {
    let by_claude = match s(d, "channel") {
        "linkedin" => true,
        "email" => d["from"].is_null() || s(d, "from") == "nicolas@klar.im",
        "form" => s(d, "where").to_lowercase().contains("free-work"),
        _ => false,
    };
    let from = s(d, "from").to_lowercase();
    let himself = s(d, "channel") == "email"
        && d["handover"].is_null()
        && !by_claude
        && OWN.iter().any(|o| from.contains(o));
    let lead = f.leads.get(s(d, "opportunity"));
    let pre_approved = || {
        s(d, "status") == "pending"
            && d["reopened-on"].is_null()
            && d["waits-on"].is_null()
            && !held.contains(s(d, "id"))
            && lead.is_some_and(|l| !TERMINAL.contains(&s(l, "status")))
            && !past_months(s(d, "body"), today)
            // a draft handed over goes by the outer test; `Model.go?` (strong fit) is task rust-outbox-rules'
            && d["handover"].is_null()
    };
    himself || (approved_wording(d, &f.blocks) && (!d["handover"].is_null() || pre_approved()))
}

/// Each `new` block a pending draft uses, with the drafts using it, in the blocks' order.
fn new_blocks(
    pending: &[&Value],
    order: &[String],
    bs: &HashMap<String, Value>,
) -> Vec<(String, String)> {
    let news: Vec<&String> = order
        .iter()
        .filter(|id| s(&bs[*id], "status") == "new")
        .collect();
    let stem = |t: &str| {
        unbold(t)
            .split('<')
            .next()
            .unwrap_or("")
            .chars()
            .take(40)
            .collect::<String>()
    };
    let used: Vec<Vec<String>> = pending
        .iter()
        .map(|d| {
            let blocks = ids(d, "blocks");
            let body = unbold(s(d, "body"));
            if blocks.is_empty() {
                let starts = news.iter().filter(|id| {
                    bs[**id].as_object().is_some_and(|b| {
                        b.iter().any(|(k, v)| {
                            k.starts_with("short")
                                && v.as_str().is_some_and(|v| {
                                    let st = stem(v);
                                    st.chars().count() > 15 && body.contains(&st)
                                })
                        })
                    })
                });
                new_parts(d)
                    .into_iter()
                    .chain(starts.map(|id| id.to_string()))
                    .collect()
            } else {
                blocks.into_iter().chain(new_parts(d)).collect()
            }
        })
        .collect();
    news.iter()
        .filter(|id| used.iter().any(|u| u.contains(id)))
        .map(|id| (id.to_string(), s(&bs[*id], "short").to_string()))
        .collect()
}

/// The facts only he can confirm in one evidence file: the bullets under an « ## À confirmer » heading, and any other line
/// flagging one inline, a fact answered left out. Each `(file:line, text)`.
fn questions(rel: &str, text: &str) -> Vec<(String, String)> {
    let ls = lines(text);
    let n = ls.len();
    let mut inside = BTreeSet::new();
    for i in 0..n {
        if CONFIRM_SECTION.is_match(ls[i]) {
            inside.extend((i + 1..n).take_while(|j| !ls[*j].starts_with('#')));
        }
    }
    let mut out = vec![];
    for (i, l) in ls.iter().enumerate() {
        let asks = if inside.contains(&i) {
            l.starts_with("- ")
        } else {
            !CONFIRM_SECTION.is_match(l) && FLAG.is_match(l) && !POINTER.is_match(l)
        };
        if !asks || ANSWERED.is_match(l) {
            continue;
        }
        let text = if inside.contains(&i) {
            bullet(&ls, i)
        } else {
            flagged_sentence(&ls, i)
        };
        out.push((format!("{rel}:{}", i + 1), text));
    }
    out
}

fn bullet(ls: &[&str], i: usize) -> String {
    let rest = ls[i + 1..]
        .iter()
        .take_while(|l| !l.trim().is_empty() && l.starts_with(char::is_whitespace));
    let joined = std::iter::once(ls[i])
        .chain(rest.copied())
        .map(str::trim)
        .collect::<Vec<_>>()
        .join(" ");
    joined
        .strip_prefix("- ")
        .map(String::from)
        .unwrap_or(joined)
}

/// The whole sentence carrying the flag on line `i`, read across its paragraph's wrapped lines.
fn flagged_sentence(ls: &[&str], i: usize) -> String {
    let blank = |j: usize| ls[j].trim().is_empty();
    let from = (0..=i)
        .rev()
        .find(|&j| j == 0 || blank(j - 1) || ls[j - 1].starts_with('#'))
        .unwrap_or(0);
    let to = (i..ls.len())
        .find(|&j| j == ls.len() - 1 || blank(j + 1))
        .unwrap_or(ls.len() - 1);
    let mut para = ls[from..=to]
        .iter()
        .map(|l| l.trim())
        .collect::<Vec<_>>()
        .join(" ");
    // one start cut, « * » or « - », as the older app's single anchored pass
    if let Some(p) = para.strip_prefix('*').or_else(|| para.strip_prefix("- ")) {
        para = p.to_string();
    }
    if let Some(p) = para.strip_suffix('*') {
        para = p.to_string();
    }
    let para = para.split_whitespace().collect::<Vec<_>>().join(" ");
    // a split after « . », « ) » or « » » before a blank and a capital, « « » or « * »
    let mut sentences = vec![];
    let cs: Vec<char> = para.chars().collect();
    let mut start = 0;
    for k in 1..cs.len().saturating_sub(1) {
        if cs[k].is_whitespace()
            && matches!(cs[k - 1], '.' | ')' | '»')
            && (cs[k + 1].is_uppercase() || matches!(cs[k + 1], '«' | '*'))
        {
            sentences.push(cs[start..k].iter().collect::<String>());
            start = k + 1;
        }
    }
    sentences.push(cs[start..].iter().collect::<String>());
    let found = sentences
        .iter()
        .enumerate()
        .find(|(_, c)| FLAG.is_match(c))
        .map(|(k, cur)| {
            let lower = cur.to_lowercase();
            match k.checked_sub(1) {
                Some(p) if lower.starts_with("à confirmer") => format!("{} {cur}", sentences[p]),
                _ => cur.clone(),
            }
        });
    let t = found.unwrap_or(para);
    // a lone « * » (italics) dropped, « ** » kept
    let cs: Vec<char> = t.chars().collect();
    let kept: String = cs
        .iter()
        .enumerate()
        .filter(|(k, c)| **c != '*' || cs.get(k + 1) == Some(&'*') || (*k > 0 && cs[k - 1] == '*'))
        .map(|(_, c)| *c)
        .collect();
    kept.trim().to_string()
}

/// A fact asked for leads that are all closed asks nothing; one naming no lead stays his.
fn moot(text: &str, leads: &HashMap<String, Value>) -> bool {
    let word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '-');
    let named: Vec<&str> = leads
        .iter()
        .filter(|(id, _)| {
            text.match_indices(id.as_str()).any(|(at, _)| {
                !word(text[..at].chars().last()) && !word(text[at + id.len()..].chars().next())
            })
        })
        .map(|(_, l)| s(l, "status"))
        .collect();
    !named.is_empty() && named.iter().all(|st| TERMINAL.contains(st))
}

/// A request from another repo waiting on his word here, or one of ours waiting on it there: (who, title).
fn requests(root: &Path, repos: &Path) -> Vec<(String, String)> {
    let read = |p: &Path| request_text(p);
    let files = |dir: PathBuf, prefix: &str| -> Vec<(PathBuf, String)> {
        let mut fs: Vec<(PathBuf, String)> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| (e.path(), e.file_name().to_string_lossy().into_owned()))
            .filter(|(_, n)| n.starts_with(prefix) && n.ends_with(".md"))
            .collect();
        fs.sort();
        fs
    };
    let title = |t: &str| {
        t.split('\n')
            .next()
            .unwrap_or("")
            .trim_start_matches('#')
            .trim_start()
            .to_string()
    };
    let mut out = vec![];
    for (p, n) in files(root.join("to_process"), "from-") {
        let Some(t) = read(&p) else { continue };
        let Some(why) = WAITING.captures(&t).map(|c| c[1].to_string()) else {
            continue;
        };
        if DONE_LINE.is_match(&t) {
            continue;
        }
        static SENDER: LazyLock<Regex> =
            LazyLock::new(|| re(r"^from-([\w-]+?)-\d{4}|^from-([^-.]+)"));
        let from = SENDER
            .captures(&n)
            .and_then(|c| c.get(1).or(c.get(2)))
            .map(|m| m.as_str().to_string());
        let who = from.map_or("site_perso".to_string(), |f| format!("ici, de {f}"));
        out.push((who, format!("{} : {why}", title(&t))));
    }
    let mut repos_dirs: Vec<PathBuf> = std::fs::read_dir(repos)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .collect();
    repos_dirs.sort();
    for repo in repos_dirs {
        let ours = files(repo.join("to_process"), "from-site_perso-");
        let day = |n: &str| {
            DAY_IN_NAME
                .find(n)
                .map_or(String::new(), |m| m.as_str().to_string())
        };
        for (p, n) in &ours {
            let Some(t) = read(p) else { continue };
            if !t.to_lowercase().contains("needs nicolas's word") || DONE_LINE.is_match(&t) {
                continue;
            }
            let withdrawn = ours.iter().any(|(o, on)| {
                o != p
                    && day(on) >= day(n)
                    && read(o).is_some_and(|x| {
                        x.to_lowercase().contains("withdraw") && x.contains(n.as_str())
                    })
            });
            if !withdrawn {
                let name = repo
                    .file_name()
                    .map_or(String::new(), |x| x.to_string_lossy().into_owned());
                out.push((name, title(&t)));
            }
        }
    }
    out
}

/// A request file's text when it can matter (a Waiting line, his word asked, a withdrawal), else none, read again only when
/// its size or time changed: a run in `dash serve` stats some 2,000 files where it read them (0.1 s against 0.8 to 2 s cold
/// at a load of 60, 2026-11-13). ponytail: an edit keeping both its size and its second is missed until the next change.
fn request_text(p: &Path) -> Option<String> {
    type Seen = HashMap<PathBuf, (Duration, u64, Option<String>)>;
    static SEEN: LazyLock<std::sync::Mutex<Seen>> = LazyLock::new(Default::default);
    let m = std::fs::metadata(p).ok()?;
    let stamp = (
        m.modified()
            .ok()?
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?,
        m.len(),
    );
    let seen = |s: &mut Seen| {
        s.get(p)
            .filter(|(t, l, _)| (*t, *l) == stamp)
            .map(|(_, _, x)| x.clone())
    };
    if let Some(kept) = seen(&mut SEEN.lock().unwrap_or_else(|e| e.into_inner())) {
        return kept;
    }
    let text = std::fs::read_to_string(p).ok()?;
    let lower = text.to_lowercase();
    let matters = text.contains("**Waiting")
        || lower.contains("needs nicolas's word")
        || lower.contains("withdraw");
    let kept = matters.then_some(text);
    let mut all = SEEN.lock().unwrap_or_else(|e| e.into_inner());
    all.insert(p.to_path_buf(), (stamp.0, stamp.1, kept.clone()));
    kept
}

// ---------------------------------------------------------------- the report

/// What waits on him: `text`, `dash handoff`'s words, `exit` 1 when the page breaks a rule, and `parts`, /handoff's.
pub struct Report {
    pub exit: i32,
    pub text: String,
    pub parts: Value,
    /// The steps a check closed this run that a writer kept from being ticked: ticked on a later run.
    pub unticked: usize,
}

/// Where the report reads besides the store.
pub struct Places {
    pub db: PathBuf,
    /// The repo: its `to_process/`, the evidence files, `bin/sandbox-apply`, klar-mail's folder under `private/store`.
    pub root: PathBuf,
    /// His repos, each with its `to_process/`.
    pub repos: PathBuf,
    pub home: PathBuf,
    pub runners: bool,
}

impl Places {
    /// This Mac's: the repo's older database, `~/repo`, `$HOME`.
    pub fn here(root: &Path) -> Places {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default();
        Places {
            db: root.join("private/dashboard/dashboard_dev.db"),
            root: root.to_path_buf(),
            repos: home.join("repo"),
            home,
            runners: true,
        }
    }
}

/// `dash handoff`'s words; `bin/dash` hands this binary those two forms alone, its others (`count`, `rejudge`, `pages`)
/// still the older app's.
pub const USAGE: &str = "dash handoff [--db <the older app's database, a copy>]: what waits on him, read from the store";

/// What waits on him today in repo `root`, as this Mac holds it; `write` as `report`'s.
pub fn here(root: &Path, write: bool) -> Result<Report, String> {
    report(&Places::here(root), &today(), write)
}

/// `dash handoff [--db <file>]` in repo `root`: the report on `out`, exit 1 when the page breaks a rule, 2 on other words.
pub fn command(root: &Path, args: &[&str], mut out: impl std::io::Write) -> ! {
    let mut p = Places::here(root);
    let said = match args {
        [] => report(&p, &today(), true),
        ["--db", db] => {
            p.db = PathBuf::from(db);
            report(&p, &today(), true)
        }
        _ => {
            eprintln!("usage: {USAGE}");
            std::process::exit(2)
        }
    };
    match said {
        Ok(r) => {
            out.write_all(r.text.as_bytes()).unwrap_or_default();
            std::process::exit(r.exit)
        }
        Err(why) => {
            eprintln!("dash handoff: {why}");
            std::process::exit(1)
        }
    }
}

/// Each task's status and owner. A task done while its PR waits for his review is « in-review » until its last event says
/// met or merged (`Tasks.in_review?/2`): the step he still has to take stays open.
const TASKS: &str = "SELECT t.id,
    CASE WHEN t.stage = 'done' AND json_extract(t.data, '$.review') = 1
      AND coalesce((SELECT e.status FROM task_events e WHERE e.task = t.id ORDER BY e.id DESC LIMIT 1), '')
        NOT IN ('met', 'merged')
    THEN 'in-review' ELSE coalesce(t.stage, '') END,
    coalesce(t.owner, 'claude')
  FROM tasks t WHERE t.collection = 'tasks.edn' AND t.id IS NOT NULL";

/// What the read transaction gave: the text, the rows behind it, the model's facts, and the parts the store alone holds.
struct Read {
    rows: Rows,
    facts: Facts,
    block_order: Vec<String>,
    evidence: Vec<String>,
    comments: Vec<(String, String, String, String)>,
    read_line: Option<(String, String)>,
}

fn read(p: &Places) -> Result<Read, String> {
    let named = |e: rusqlite::Error| format!("{}: {e}", p.db.display());
    let db = opened(&p.db)?;
    // one snapshot for every table below: WAL keeps it while writers go on, so the steps and the drafts are one version
    db.execute_batch("BEGIN DEFERRED").map_err(named)?;
    let rows = rows(&db).map_err(named)?;
    let mut drafts = match crate::older_drafts::drafts_in(&db, None).map_err(named)? {
        Value::Array(xs) => xs,
        _ => vec![],
    };
    drafts.sort_by_key(|d| {
        (
            text_of(&d["created"]).unwrap_or_default(),
            s(d, "file").to_string(),
        )
    });
    let leads = crate::older_drafts::leads_in(&db)
        .map_err(named)?
        .into_iter()
        .map(|l| (s(&l, "id").to_string(), l))
        .collect();
    let mut q = db
        .prepare("SELECT id, stage, short, short_en FROM blocks WHERE collection = 'blocks.edn' AND id IS NOT NULL ORDER BY pos")
        .map_err(named)?;
    let mut blocks = HashMap::new();
    let mut block_order = vec![];
    let got = q
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, Option<String>>(3)?,
            ))
        })
        .map_err(named)?;
    for row in got {
        let (id, stage, short, short_en) = row.map_err(named)?;
        block_order.push(id.clone());
        blocks.insert(
            id,
            json!({"status": stage, "short": short, "short-en": short_en}),
        );
    }
    let mut q = db.prepare(TASKS).map_err(named)?;
    let tasks = q
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                (r.get::<_, String>(1)?, r.get::<_, String>(2)?),
            ))
        })
        .map_err(named)?
        .collect::<rusqlite::Result<HashMap<_, _>>>()
        .map_err(named)?;
    let evidence: Option<String> = db
        .query_row(
            "SELECT data FROM records WHERE collection = 'profile.edn' AND key IN ('evidence', ':evidence', '~:evidence')",
            [],
            |r| r.get(0),
        )
        .optional()
        .map_err(named)?;
    // in the profile's order, which a map read would sort: each value of the object as written, existing, once
    static PAIR: LazyLock<Regex> =
        LazyLock::new(|| re(r#""(?:[^"\\]|\\.)*"\s*:\s*"((?:[^"\\]|\\.)*)""#));
    let mut evidence: Vec<String> = PAIR
        .captures_iter(&evidence.unwrap_or_default())
        .map(|c| c[1].to_string())
        .filter(|f| p.root.join(f).is_file())
        .collect();
    let mut once = BTreeSet::new();
    evidence.retain(|f| once.insert(f.clone()));
    // his comments still waiting on Claude: open, his word the last, newest first
    let mut q = db
        .prepare("SELECT id, coalesce(file, ''), coalesce(quote, ''), coalesce(text, ''), coalesce(date, ''), data FROM comments WHERE coalesce(stage, '') != 'resolved' AND id IS NOT NULL ORDER BY pos")
        .map_err(named)?;
    let got = q
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
            ))
        })
        .map_err(named)?;
    let mut waiting = vec![];
    for (i, row) in got.enumerate() {
        let (id, file, quote, text, date, data) = row.map_err(named)?;
        let replies = serde_json::from_str::<Value>(&data)
            .ok()
            .and_then(|d| d.get("replies").cloned());
        let last = replies
            .as_ref()
            .and_then(Value::as_array)
            .and_then(|r| r.last())
            .cloned();
        if last.as_ref().is_some_and(|l| s(l, "by") == "claude") {
            continue;
        }
        let at = last
            .as_ref()
            .map(|l| s(l, "date").to_string())
            .filter(|d| !d.is_empty())
            .unwrap_or(date);
        waiting.push((at, i, (file, quote, text, id)));
    }
    waiting.sort_by(|a, b| (&b.0, b.1).cmp(&(&a.0, a.1)));
    let comments = waiting.into_iter().map(|(_, _, c)| c).collect();
    let read_line: Option<(String, String)> = {
        let mut q = db
            .prepare("SELECT data FROM events WHERE source = 'weekly-read.jsonl'")
            .map_err(named)?;
        let all = q
            .query_map([], |r| r.get::<_, Option<String>>(0))
            .map_err(named)?;
        let mut best: Option<(String, i32, Value)> = None;
        for d in all {
            let Some(v) = d
                .map_err(named)?
                .and_then(|d| serde_json::from_str::<Value>(&d).ok())
            else {
                continue;
            };
            let key = (
                [s(&v, "week"), s(&v, "through")]
                    .into_iter()
                    .find(|x| !x.is_empty())
                    .unwrap_or("")
                    .to_string(),
                i32::from(!v["week"].is_null()),
            );
            if best
                .as_ref()
                .is_none_or(|b| (key.0.as_str(), key.1) > (b.0.as_str(), b.1))
            {
                best = Some((key.0, key.1, v));
            }
        }
        best.map(|(_, _, r)| {
            let label = s(&r, "text")
                .split('\n')
                .next()
                .unwrap_or("")
                .replace("**", "");
            let at = if r["history"].as_bool().unwrap_or(false) || r["history"].is_string() {
                "history"
            } else {
                "read"
            };
            (label, format!("/insights#{at}"))
        })
    };
    db.execute_batch("COMMIT").map_err(named)?;
    let sent = std::fs::read_to_string(p.root.join("private/store/mail/klar/.sent.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default();
    let logins = std::fs::read_to_string(p.home.join(".claude/logins.jsonl"))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            serde_json::from_str::<Value>(l)
                .ok()
                .filter(Value::is_object)
        })
        .collect();
    let facts = Facts {
        sent,
        drafts,
        leads,
        blocks,
        tasks,
        logins,
        root: p.root.clone(),
        home: p.home.clone(),
        mail: p.root.join("private/store/mail/klar"),
        runners: p.runners,
    };
    Ok(Read {
        rows,
        facts,
        block_order,
        evidence,
        comments,
        read_line,
    })
}

/// Ticks each step `closed` holds in its own row, its evidence under it, only while the row is as read: his tick or
/// answer written meanwhile is kept, and a writer holding the store leaves it for the next run. How many stayed unticked.
fn tick(p: &Places, rows: &Rows, closed: &[(Step, Verdict)], today: &str) -> usize {
    if closed.is_empty() {
        return 0;
    }
    let db = match rusqlite::Connection::open_with_flags(
        &p.db,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE,
    ) {
        Ok(db) => db,
        Err(_) => return closed.len(),
    };
    // a writer's lock: give way at once, the steps stay closed on the page and the next run ticks them
    let _ = db.busy_timeout(Duration::from_millis(200));
    let sql = "UPDATE handoff_steps SET stage = 'done', body = ?1
      WHERE collection = 'handoff.md' AND pos = ?2 AND body = ?3 AND stage = 'open'";
    // the rows' steps: every step is a row's first line (`Store`'s handoff_split), so one with none is no tick to try
    let ticks: Vec<(&i64, &String, &String)> = closed
        .iter()
        .filter_map(|(st, v)| {
            let Verdict::Closed(evidence) = v else {
                return None;
            };
            let row = rows.rows.iter().find(|(_, _, at)| *at == st.line);
            row.map(|(pos, raw, _)| (pos, raw, evidence))
        })
        .collect();
    for (i, (pos, raw, evidence)) in ticks.iter().enumerate() {
        let body = format!("{raw}\n  **Fait {today} :** {evidence}");
        // the first refusal ends the run: the rest would each wait out the same writer (/code-review)
        if db.execute(sql, params![body, pos, raw]).is_err() {
            return ticks.len() - i;
        }
    }
    0
}

/// What waits on him, read from the store at `p` on `today`; with `write`, the steps a check closed ticked first. A page's
/// read never writes: `dash handoff` and the inbox pass tick, as the older app's report did.
pub fn report(p: &Places, today: &str, write: bool) -> Result<Report, String> {
    let r = read(p)?;
    let f = &r.facts;
    let closed = judge(&r.rows.text, today, f).closed;
    let unticked = if write {
        tick(p, &r.rows, &closed, today)
    } else {
        0
    };
    // the page reads the text as ticked, whether or not the write landed: a met step is never his
    let mut text = r.rows.text.clone();
    let mut sorted: Vec<&(Step, Verdict)> = closed.iter().collect();
    sorted.sort_by_key(|(st, _)| std::cmp::Reverse(st.line));
    for (st, v) in sorted {
        if let Verdict::Closed(e) = v {
            text = done(&text, st.line, today, e);
        }
    }
    Ok(summarize(&text, &r, &p.repos, today, unticked))
}

/// A part of `dash handoff`'s words: its title and count, what Claude tried, an item a line; nothing when empty.
fn section(out: &mut Vec<String>, title: &str, tried: Option<&str>, xs: Vec<String>) {
    if xs.is_empty() {
        return;
    }
    out.push(format!("\n## {title} ({})", xs.len()));
    if let Some(t) = tried {
        out.push(format!("  {t}"));
    }
    out.extend(xs.into_iter().map(|x| format!("- {x}")));
}

fn summarize(text: &str, r: &Read, repos: &Path, today: &str, unticked: usize) -> Report {
    let f = &r.facts;
    let j = judge(text, today, f);
    let held = held_drafts(text);
    let pending: Vec<&Value> = f
        .drafts
        .iter()
        .filter(|d| s(d, "status") == "pending")
        .collect();
    let drafts: Vec<&Value> = pending
        .iter()
        .copied()
        .filter(|d| !not_his(d, f, &held, today))
        .collect();
    let blocks = new_blocks(&pending, &r.block_order, &f.blocks);
    let questions: Vec<(String, String)> = r
        .evidence
        .iter()
        .filter_map(|rel| {
            std::fs::read_to_string(f.root.join(rel))
                .ok()
                .map(|t| questions(rel, &t))
        })
        .flatten()
        .filter(|(_, t)| !moot(t, &f.leads))
        .collect();
    let sandbox = sandbox_pending(&f.root);
    let inbox = requests(&f.root, repos);
    let open: Vec<Step> = steps(text).into_iter().filter(|st| !st.done).collect();
    let count = j.top.len()
        + drafts.len()
        + blocks.len()
        + questions.len()
        + usize::from(sandbox)
        + inbox.len();

    let mut out: Vec<String> = vec![format!(
        "{count} things wait on Nicolas. Page: {PAGE}. File: {FILE}"
    )];
    if let Some((label, href)) = &r.read_line {
        out.push(format!("To read: {label} http://100.72.158.115:7788{href}"));
    }
    let step_text = |st: &Step| unbold(&st.text);
    section(
        &mut out,
        "His three steps (À toi)",
        None,
        j.top.iter().map(|(st, _)| step_text(st)).collect(),
    );
    section(
        &mut out,
        "Later (Plus tard)",
        None,
        j.later
            .iter()
            .map(|(st, v)| {
                let when = match v {
                    Verdict::Later(w) if day(w).is_some() => format!("  (from {w})"),
                    Verdict::Later(w) => format!("  ({w})"),
                    _ => String::new(),
                };
                step_text(st) + &when
            })
            .collect(),
    );
    section(
        &mut out,
        &format!(
            "Stale, re-judge each: dash handoff rejudge <line> keep|drop ({STALE_DAYS} days without a :judged)"
        ),
        None,
        j.stale
            .iter()
            .map(|(st, v)| {
                let when = if let Verdict::Stale(w) = v {
                    w.as_str()
                } else {
                    ""
                };
                format!("line {}: {}  (judged: {when})", st.line + 1, step_text(st))
            })
            .collect(),
    );
    let answers: Vec<&Step> = open.iter().filter(|st| answered(st)).collect();
    if !answers.is_empty() {
        out.push(format!(
            "\n## His answers waiting on Claude ({}): act, then write « **Fait <date> :** » under it",
            answers.len()
        ));
        for st in &answers {
            out.push(format!(
                "- line {}: {}\n    {}",
                st.line + 1,
                step_text(st),
                st.more.last().map_or("", |l| l.trim())
            ));
        }
    }
    let warnings = warnings(&open, f);
    out.extend(warnings.iter().cloned());
    let sandbox_file = "sandbox.sb.candidate, .safehouse.candidate ou un script du bac à sable (sandbox/*.sh, bin/sandbox-apply)";
    section(
        &mut out,
        "Sandbox to apply",
        Some(TRIED_SANDBOX),
        if sandbox {
            vec![format!(
                "{sandbox_file}: make -C ~/repo/site_perso sandbox/apply"
            )]
        } else {
            vec![]
        },
    );
    section(
        &mut out,
        "Drafts to review",
        Some(TRIED_DRAFTS),
        drafts
            .iter()
            .map(|d| format!("{} -> {}", s(d, "id"), s(d, "to")))
            .collect(),
    );
    section(
        &mut out,
        "Blocks to validate once",
        Some(TRIED_BLOCKS),
        blocks
            .iter()
            .map(|(id, short)| format!("{id}: {short}"))
            .collect(),
    );
    section(
        &mut out,
        "Facts to confirm",
        Some(TRIED_QUESTIONS),
        questions
            .iter()
            .map(|(file, t)| format!("{} ({file})", unbold(t)))
            .collect(),
    );
    section(
        &mut out,
        "Your word in another repo's session",
        Some(TRIED_INBOX),
        inbox.iter().map(|(who, t)| format!("{who}: {t}")).collect(),
    );
    if !r.comments.is_empty() {
        out.push(format!(
            "\n## Comments waiting on Claude ({}): answer in the document, then resolve",
            r.comments.len()
        ));
        for (file, quote, text, id) in &r.comments {
            out.push(format!("- {file}: « {quote} » -> {text}  [{id}]"));
        }
    }
    // a line of its own kind, never prose after a part, which the inbox's card reader would join to the last item
    if unticked > 0 {
        out.push(format!("WARNING: {unticked} met steps stay unticked (a writer held the store, or it is read-only here): the next run ticks them."));
    }
    let faults = faults(text, f, today);
    out.extend(faults.iter().map(|x| format!("ERROR: {x}")));
    // one thing on the page: its title, a line under it, where it opens
    let item = |title: String, line: String, href: Option<String>| json!({"title": title, "line": line, "href": href});
    let step_item = |st: &Step, line: String| {
        let title = unbold(&unlink(&st.text)).replace(['[', ']', '`'], "");
        let href = LINK_TARGET.captures(&st.text).map(|c| c[1].to_string());
        item(title, line, href)
    };
    let mut tasks: Vec<Value> = j
        .top
        .iter()
        .map(|(st, _)| step_item(st, "your step".into()))
        .collect();
    tasks.extend(
        answers
            .iter()
            .map(|st| step_item(st, "you answered: Claude acts next".into())),
    );
    tasks.extend(
        j.stale
            .iter()
            .map(|(st, _)| step_item(st, format!("line {}: Claude re-judges it", st.line + 1))),
    );
    tasks.extend(
        questions
            .iter()
            .map(|(file, t)| item(unbold(t), format!("a fact to confirm, {file}"), None)),
    );
    tasks.extend(
        inbox
            .iter()
            .map(|(who, t)| item(t.clone(), format!("your word in {who}"), None)),
    );
    tasks.extend(r.comments.iter().map(|(file, quote, text, id)| {
        item(
            text.clone(),
            format!("« {quote} », waiting on Claude"),
            Some(format!("/v/{file}#c-{id}")),
        )
    }));
    let later: Vec<Value> = j
        .later
        .iter()
        .map(|(st, v)| {
            step_item(
                st,
                if let Verdict::Later(w) = v {
                    if day(w).is_some() {
                        format!("from {w}")
                    } else {
                        w.clone()
                    }
                } else {
                    String::new()
                },
            )
        })
        .collect();
    let mut draft_items: Vec<Value> = drafts
        .iter()
        .map(|d| {
            let parts = new_parts(d);
            let line = if parts.is_empty() {
                "hand-written, read it whole".to_string()
            } else {
                format!("new: {}", parts.join(", "))
            };
            item(
                format!("{} to {}", s(d, "id"), s(d, "to")),
                line,
                Some(format!("/drafts/{}", s(d, "id"))),
            )
        })
        .collect();
    draft_items.extend(
        blocks
            .iter()
            .map(|(id, short)| item(id.clone(), short.clone(), Some("/review".into()))),
    );
    let sandbox_items: Vec<Value> = if sandbox {
        vec![item(
            "Yes, approve the host change".into(),
            sandbox_file.into(),
            Some("/approve".into()),
        )]
    } else {
        vec![]
    };
    let notes: String = lines(text)
        .into_iter()
        .filter(|l| !META.is_match(l.trim_start()))
        .collect::<Vec<_>>()
        .join("\n");
    let parts = json!({
        "count": count,
        "read": r.read_line.as_ref().map(|(label, href)| json!({"label": label, "href": href})),
        "sandbox": sandbox_items,
        "drafts": draft_items,
        "tasks": tasks,
        "later": later,
        "notes": notes,
        "rules": warnings.iter().map(|w| w.trim().to_string()).chain(faults.iter().map(|x| format!("ERROR: {x}"))).collect::<Vec<_>>(),
    });
    Report {
        exit: i32::from(!faults.is_empty()),
        text: out.join("\n") + "\n",
        parts,
        unticked,
    }
}

/// A sandbox change waits on his approval: `bin/sandbox-apply --pending` exits 0 (one apply covers the profile and its
/// scripts). No script, nothing pending.
fn sandbox_pending(root: &Path) -> bool {
    type Seen = HashMap<PathBuf, (Vec<Option<(Duration, u64)>>, bool)>;
    static SEEN: LazyLock<std::sync::Mutex<Seen>> = LazyLock::new(Default::default);
    // what --pending reads, as the older app stamped it: the script, the main checkout's HEAD log, the approved commit,
    // the candidates and what the script sources; its answer kept while none of them changed (0.3 s a run, /code-review)
    let stamp: Vec<Option<(Duration, u64)>> = [
        "bin/sandbox-apply",
        ".git/logs/HEAD",
        ".claude/run/approved",
        "sandbox.sb.candidate",
        ".safehouse.candidate",
        "bin/approved-only",
        "sandbox/jobs.conf",
    ]
    .iter()
    .map(|f| {
        let m = std::fs::metadata(root.join(f)).ok()?;
        Some((
            m.modified()
                .ok()?
                .duration_since(std::time::UNIX_EPOCH)
                .ok()?,
            m.len(),
        ))
    })
    .collect();
    // a checkout whose HEAD log is not there (a worktree's .git is a file) is asked each time
    let kept = stamp[1].is_some();
    if kept
        && let Some((was, pending)) = SEEN.lock().unwrap_or_else(|e| e.into_inner()).get(root)
        && *was == stamp
    {
        return *pending;
    }
    let pending = sandbox_asked(root);
    if kept {
        SEEN.lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(root.to_path_buf(), (stamp, pending));
    }
    pending
}

fn sandbox_asked(root: &Path) -> bool {
    let script = root.join("bin/sandbox-apply");
    script.is_file()
        && std::process::Command::new(&script)
            .arg("--pending")
            .current_dir(root)
            .env("SITE_PERSO_REPO", root)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
}

/// The page's rules an open step breaks (@nha, 2026-10-08: « handoff is WAY too verbose and big. [...] Executive-style
/// please. »): no « Où : » line, two lines at most, 200 characters a line, a 50-character title, a link, and a step on a
/// task he owns.
fn warnings(open: &[Step], f: &Facts) -> Vec<String> {
    let mut w = vec![];
    for st in open.iter().filter(|st| st.placed) {
        w.push(format!(
            "WARNING: « Où : » line, drop it (a Mac-only step says why in its action): {}",
            head(&st.text)
        ));
    }
    for st in open.iter().filter(|st| prose(&st.more).len() > 1) {
        w.push(format!(
            "WARNING: step over two lines, keep the action and its link, move the rest out: {}",
            head(&st.text)
        ));
    }
    for st in open {
        let all: Vec<String> = std::iter::once(st.text.clone())
            .chain(st.more.iter().cloned())
            .collect();
        for l in prose(&all) {
            if unlink(l).chars().count() > 200 {
                w.push(format!(
                    "WARNING: step line over 200 characters, split it: {}",
                    head(l)
                ));
            }
        }
    }
    for st in open {
        if let Some(t) = step_title(st)
            && MAC_WHY.replace_all(&unlink(&t), "").chars().count() > 50
        {
            w.push(format!(
                "WARNING: step title over 50 characters, keep the action, move the rest below: {t}"
            ));
        }
    }
    for st in open {
        let all = std::iter::once(&st.text)
            .chain(&st.more)
            .cloned()
            .collect::<String>();
        if !HAS_LINK.is_match(&all) {
            w.push(format!(
                r#"WARNING: step without a link (@nha, 2026-10-01: "LINK."): {}"#,
                head(&st.text)
            ));
        }
    }
    for st in open {
        let Some(id) = task_of(st) else { continue };
        let Some((_, owner)) = f.tasks.get(&id) else {
            continue;
        };
        if owner != "nicolas" {
            w.push(format!(
                "WARNING: step on {id}, a task {owner} owns, not him: file his part as a task (dash task add <id> --owner nicolas ...) and point the step's :task to it; only when all of {id} is his act, dash task update {id} --owner nicolas: {}",
                head(&st.text)
            ));
        }
    }
    w
}

/// Why the page fails: an open step nothing would close, a duplicate, a step naming no task or one the list lacks, or
/// an open step naming an interview, call or deadline without its date.
pub fn faults(text: &str, f: &Facts, today: &str) -> Vec<String> {
    let ss = steps(text);
    let bare = Facts {
        root: f.root.clone(),
        home: f.home.clone(),
        mail: PathBuf::new(),
        ..Facts::default()
    };
    let mut out = vec![];
    for st in &ss {
        if let Verdict::Refused(why) = verdict(st, today, &bare) {
            out.push(format!(
                r#"handoff.md line {}: {why} (a JSON line  {{"check": …}} or {{"until": …}} under it): {}"#,
                st.line + 1,
                head(&st.text)
            ));
        }
    }
    let key = |st: &Step| {
        let t = step_title(st).unwrap_or_default().to_lowercase();
        t.split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
            .collect::<Vec<_>>()
            .join(" ")
    };
    let mut seen = BTreeSet::new();
    for st in ss.iter().filter(|st| !st.done) {
        if !seen.insert(key(st)) {
            out.push(format!(
                "handoff.md line {}: duplicate of another open step: {}",
                st.line + 1,
                head(&st.text)
            ));
        }
    }
    for st in ss
        .iter()
        .filter(|st| !matches!(st.meta, Meta::Unreadable(_)))
    {
        let why = match task_of(st) {
            None => Some("no :task names the task it belongs to".to_string()),
            Some(id) if !f.tasks.contains_key(&id) => {
                Some(format!("its :task {id} is not in tasks.edn"))
            }
            Some(_) => None,
        };
        if let Some(why) = why {
            out.push(format!(
                "handoff.md line {}: {why}: {}",
                st.line + 1,
                head(&st.text)
            ));
        }
    }
    for st in ss.iter().filter(|st| !st.done && undated(st)) {
        out.push(format!(
            "handoff.md line {}: names an interview, call or deadline without its date (« le 23/10 », « 2026-10-23 ») or « pas encore de date »: {}",
            st.line + 1,
            head(&st.text)
        ));
    }
    out
}

/// A step naming an event says when in the same sentence, or that nobody knows yet (task step-names-its-facts).
fn undated(st: &Step) -> bool {
    let all: Vec<String> = std::iter::once(st.text.clone())
        .chain(st.more.iter().cloned())
        .collect();
    let read = prose(&all)
        .iter()
        .map(|l| unlink(l))
        .collect::<Vec<_>>()
        .join("\n");
    let read = CODE_OR_URL.replace_all(&read, " ");
    split_sentences(&read).iter().any(|x| {
        let event = EVENT.find_iter(x).any(|m| {
            let after = &x[m.end()..];
            !after.starts_with('-')
                && !(m.as_str().to_lowercase().starts_with("appel")
                    && after.starts_with(" d'offre"))
        });
        event && !DATED.is_match(x)
    })
}

#[cfg(test)]
mod tests;
