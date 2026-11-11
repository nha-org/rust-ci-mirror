//! The drafts plugin (rfc-rust-unified section 20b): each draft kept with its goal, sender, receiver, every version with
//! Jev's scores, and its outcome, so what gets replies can be read (@nha, 2026-11-07: « do we save past drafts ?
//! success/fails ? we should. data is gold. »). It depends on the core alone: it writes and reads through the op log port,
//! and the binary opens its table with `SCHEMA`. The older app stays where a draft is written (its `dash draft`):
//! `from_store` reads its export.
pub mod outbox;
pub mod posting;

pub use dash_core::date;
use dash_core::{FieldWrite, OpLog, Value, days};
use serde_json::{Map, Value as Json, json};
use std::collections::BTreeMap;
use std::io::BufRead;
use std::path::Path;

/// One row a draft, its record one JSON field.
/// ponytail: one field, so two replicas writing one draft keep the later record whole, never a merge of their fields.
pub const SCHEMA: &str =
    "CREATE TABLE IF NOT EXISTS drafts (id TEXT PRIMARY KEY, record TEXT) WITHOUT ROWID;";
const TABLE: &str = "drafts";
const FIELD: &str = "record";
/// A reply counts until this many days after the send; past it with none, the draft got none.
pub const REPLY_DAYS: i64 = 7;
/// The channels a person answers on, as `Dashboard.Sales` reads a lead's timeline.
const PERSON: [&str; 4] = ["linkedin", "email", "call", "phone"];
const PAGE: i64 = 1000;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Version {
    /// Empty where no source kept it: a labelled file can name a version Jev's log alone holds.
    pub text: String,
    /// `claude`, or `nicolas` for his edit.
    pub by: String,
    /// The day Jev first scored it, else the draft's day.
    pub on: String,
    /// Each Jev question's answer: a fraction, or a score.
    pub jev: BTreeMap<String, f64>,
    /// What was asked or made for it, as a labelled file says.
    pub change: String,
    /// His edit that made this version, line by line, as a labelled file says.
    pub edit: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Reply {
    pub on: String,
    pub what: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Draft {
    pub id: String,
    pub goal: String,
    pub sender: String,
    pub receiver: String,
    pub channel: String,
    pub lead: Option<String>,
    /// The older app's status: pending, approved, sent, rejected, moot.
    pub status: String,
    /// He approved it (approved, sent, or an approval day), he rejected it, or `None`: not judged yet, or moot unjudged.
    pub approved: Option<bool>,
    /// Made only of his approved blocks; `None` where the older app never said.
    pub from_blocks: Option<bool>,
    pub sent_on: Option<String>,
    /// The day the older app wrote it, what /drafts' « oldest » reads; empty where no source said (a record kept before it).
    pub created: String,
    pub versions: Vec<Version>,
    /// The lead's first answer after this send and before our next send on that lead.
    pub reply: Option<Reply>,
}

impl Draft {
    pub fn his_rewrite(&self) -> bool {
        self.versions.iter().any(|v| v.by == "nicolas")
    }

    /// His last edit's size: the words it changed (taken out plus put in) and the words of the version it edited. `None`
    /// with no version of his after one of ours, or a text no source kept.
    pub fn his_edit(&self) -> Option<(usize, usize)> {
        let pairs = self
            .versions
            .windows(2)
            .filter(|w| w[1].by == "nicolas" && w[0].by != "nicolas");
        let (before, after) = pairs
            .filter(|w| !w[0].text.is_empty() && !w[1].text.is_empty())
            .map(|w| (&w[0], &w[1]))
            .next_back()?;
        Some((
            words_changed(&before.text, &after.text),
            before.text.split_whitespace().count(),
        ))
    }

    /// `unsent`; `no lead`: sent with no lead to read a reply on; `no send day`: its day is no YYYY-MM-DD; `reply`: by day
    /// 7; `none`: none by day 7; `waiting`: sent, day 7 not over and no reply yet.
    pub fn outcome(&self, today: &str) -> &'static str {
        let Some(sent) = self.sent_on.as_deref() else {
            return "unsent";
        };
        if self.lead.is_none() {
            return "no lead";
        }
        let Some(by) = plus_days(sent, REPLY_DAYS) else {
            return "no send day";
        };
        match &self.reply {
            Some(r) if r.on <= by => "reply",
            _ if today > by.as_str() => "none",
            _ => "waiting",
        }
    }
}

fn text(m: &Map<String, Json>, key: &str) -> Option<String> {
    m.get(key).and_then(Json::as_str).map(str::to_string)
}

/// A field a record from before it was added does not hold: empty.
fn text_or(m: &Map<String, Json>, key: &str) -> String {
    text(m, key).unwrap_or_default()
}

/// The older app's keywords are « ~:name ».
fn keyword(m: &Map<String, Json>, key: &str) -> Option<String> {
    text(m, key).map(|s| s.strip_prefix("~:").unwrap_or(&s).to_string())
}

/// Two texts equal but for their spaces and line breaks: Jev's log holds a document with its lines joined.
fn same(a: &str, b: &str) -> bool {
    a.split_whitespace().eq(b.split_whitespace())
}

fn version_json(v: &Version) -> Json {
    json!({"text": v.text, "by": v.by, "on": v.on, "jev": v.jev, "change": v.change, "edit": v.edit})
}

fn to_json(d: &Draft) -> Json {
    let reply = d
        .reply
        .as_ref()
        .map(|r| json!({"on": r.on, "what": r.what}));
    let versions: Vec<Json> = d.versions.iter().map(version_json).collect();
    json!({"id": d.id, "goal": d.goal, "sender": d.sender, "receiver": d.receiver, "channel": d.channel, "lead": d.lead,
        "status": d.status, "approved": d.approved, "from_blocks": d.from_blocks, "sent_on": d.sent_on, "created": d.created, "versions": versions, "reply": reply})
}

fn from_json(j: &Json) -> Option<Draft> {
    let m = j.as_object()?;
    let owned = |key: &str| text(m, key).unwrap_or_default();
    let versions = m.get("versions")?.as_array()?.iter().map(|v| {
        let v = v.as_object()?;
        let jev = v.get("jev")?.as_object()?;
        let jev = jev
            .iter()
            .filter_map(|(q, x)| Some((q.clone(), x.as_f64()?)));
        let (text, by, on) = (text(v, "text")?, text(v, "by")?, text(v, "on")?);
        let edit = v.get("edit").and_then(Json::as_array).into_iter().flatten();
        Some(Version {
            text,
            by,
            on,
            jev: jev.collect(),
            change: text_or(v, "change"),
            edit: edit
                .filter_map(|l| l.as_str().map(str::to_string))
                .collect(),
        })
    });
    let reply = m.get("reply").and_then(Json::as_object).map(|r| Reply {
        on: text(r, "on").unwrap_or_default(),
        what: text(r, "what").unwrap_or_default(),
    });
    Some(Draft {
        id: text(m, "id")?,
        goal: owned("goal"),
        sender: owned("sender"),
        receiver: owned("receiver"),
        channel: owned("channel"),
        lead: text(m, "lead"),
        status: owned("status"),
        approved: m.get("approved").and_then(Json::as_bool),
        from_blocks: m.get("from_blocks").and_then(Json::as_bool),
        sent_on: text(m, "sent_on"),
        created: owned("created"),
        versions: versions.collect::<Option<_>>()?,
        reply,
    })
}

/// What the table holds: each draft as its row's last record says, and the rows whose last record does not read (a newer
/// build's, a hand edit): left out and named, so `show` and `rates` still answer and an import writes them again.
#[derive(Default)]
pub struct Kept {
    pub drafts: BTreeMap<String, Draft>,
    pub unread: std::collections::BTreeSet<String>,
}

/// The table, read from the log.
/// ponytail: the whole log is read each time, fine at a few hundred drafts; a read of the table once the core has a read port.
pub fn load(log: &dyn OpLog) -> Result<Kept, String> {
    let mut kept = Kept::default();
    let mut after: Option<(i64, String)> = None;
    loop {
        let ops = log.ops_after(after.as_ref().map(|(s, w)| (*s, w.as_str())), PAGE)?;
        let Some(last) = ops.last() else {
            return Ok(kept);
        };
        after = Some((last.stamp, last.writer.clone()));
        for w in ops.iter().map(|op| &op.write) {
            if let (TABLE, FIELD, Value::Text(t)) = (w.table.as_str(), w.field.as_str(), &w.value) {
                match serde_json::from_str(t).ok().and_then(|j| from_json(&j)) {
                    Some(d) => {
                        kept.unread.remove(&w.row);
                        kept.drafts.insert(w.row.clone(), d);
                    }
                    None => {
                        kept.drafts.remove(&w.row);
                        kept.unread.insert(w.row.clone());
                    }
                }
            }
        }
    }
}

/// Writes each draft whose record differs from `kept` (what `load` read), and says how many: an import run twice writes
/// nothing the second time.
pub fn save(log: &mut dyn OpLog, kept: &Kept, drafts: &[Draft]) -> Result<usize, String> {
    let writes: Vec<FieldWrite> = drafts
        .iter()
        .filter(|d| kept.drafts.get(&d.id) != Some(d))
        .map(|d| FieldWrite {
            table: TABLE.into(),
            row: d.id.clone(),
            field: FIELD.into(),
            value: Value::Text(to_json(d).to_string()),
        })
        .collect();
    if !writes.is_empty() {
        log.write(&writes)?;
    }
    Ok(writes.len())
}

/// `d` as read now, with every version `kept` (the table's record of it) holds that `d` lacks, before its own: a text he
/// asked corrections on stays readable once the agent rewrites it, though the older app keeps only the last body (task
/// rust-draft-approve). A version of the same words is the table's, with the text as written now and the newer read's Jev
/// scores when it has any (second /code-review); `d`'s current text, its last version, stays last, so a text set back to an
/// earlier one is the current again.
/// ponytail: no lock between /d/<id>'s save and /drafts' refresh; two writes of one draft in the same instant keep the later.
pub fn with_kept_versions(kept: Option<&Draft>, mut d: Draft) -> Draft {
    let Some(kept) = kept else {
        return d;
    };
    let mut versions = kept.versions.clone();
    let current = d.versions.last().map(|v| v.text.clone());
    for v in d.versions {
        match versions.iter_mut().find(|k| same(&k.text, &v.text)) {
            Some(k) => {
                k.text = v.text;
                if !v.jev.is_empty() {
                    k.jev = v.jev;
                }
            }
            None => versions.push(v),
        }
    }
    if let Some(at) = current.and_then(|t| versions.iter().position(|k| same(&k.text, &t))) {
        let v = versions.remove(at);
        versions.push(v);
    }
    d.versions = versions;
    d
}

/// One Jev answer: the text it scored and each question's number.
#[derive(Clone)]
struct Scored {
    ts: String,
    text: String,
    jev: BTreeMap<String, f64>,
    /// Jev read only his own sentences of the message (`draft score`'s part « mine »): a part, never a version.
    mine: bool,
}

/// Jev's log, private/ats/jev.jsonl: the lines about a draft, by its file's name, and the lines about a document, by its
/// path. A line that is no JSON, an error or no answer is left out: the log is everything Jev was asked.
type ByKey = BTreeMap<String, Vec<Scored>>;

fn jev_lines(jev: impl BufRead) -> Result<(ByKey, ByKey), String> {
    let (mut drafts, mut docs) = (ByKey::new(), ByKey::new());
    for line in jev.lines() {
        let line = line.map_err(|e| format!("jev log: {e}"))?;
        let Ok(v) = serde_json::from_str::<Json>(&line) else {
            continue;
        };
        let state = v.pointer("/request/body/state").and_then(Json::as_object);
        let answers = v.pointer("/response/answers").and_then(Json::as_object);
        let (Some(e), Some(state), Some(answers)) = (v.as_object(), state, answers) else {
            continue;
        };
        let numbers = answers.iter().filter_map(|(q, a)| {
            let n = a.get("noul").or_else(|| a.get("score"))?.as_f64()?;
            Some((q.clone(), n))
        });
        let scored = |said: Option<String>| {
            Some(Scored {
                ts: text(e, "ts")?,
                text: said?,
                jev: numbers.collect(),
                mine: text(e, "part").as_deref() == Some("mine"),
            })
        };
        let (into, key, scored) = if let Some(file) = text(e, "draft") {
            let name = Path::new(&file)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned());
            (&mut drafts, name, scored(text(state, "message")))
        } else {
            (&mut docs, text(e, "doc"), scored(text(state, "doc")))
        };
        if let (Some(key), Some(scored)) = (key, scored) {
            into.entry(key).or_default().push(scored);
        }
    }
    Ok((drafts, docs))
}

/// One version per text, in the order Jev first saw each, each question's latest answer kept. A part « mine » follows the
/// whole message it was cut from in the same run: its answers join the version just scored, as `mine.<question>`; with
/// none before it, the first version; with no version at all, they are answered apart, for the body's.
fn versions(mut scored: Vec<Scored>) -> (Vec<Version>, BTreeMap<String, f64>) {
    scored.sort_by(|a, b| a.ts.cmp(&b.ts));
    let mut out: Vec<Version> = vec![];
    let (mut last, mut apart): (Option<usize>, _) = (None, BTreeMap::new());
    for s in scored {
        if s.mine {
            let mine = s.jev.into_iter().map(|(q, x)| (format!("mine.{q}"), x));
            match last {
                Some(i) => out[i].jev.extend(mine),
                None => apart.extend(mine),
            }
            continue;
        }
        let at = out.iter().position(|v| same(&v.text, &s.text));
        let v = match at {
            Some(i) => &mut out[i],
            None => {
                let on = s.ts.get(..10).unwrap_or_default().to_string();
                out.push(Version {
                    text: s.text,
                    by: "claude".into(),
                    on,
                    ..Default::default()
                });
                out.last_mut().expect("pushed")
            }
        };
        v.jev.extend(s.jev);
        last = Some(at.unwrap_or(out.len() - 1));
    }
    // A part scored before any whole message was cut from the earliest text, not the last.
    if let Some(first) = out.first_mut() {
        first.jev.append(&mut apart);
    }
    (out, apart)
}

/// An event someone else wrote: not ours, on a channel a person answers on. `Dashboard.Sales`'s ours?, its regexes as words.
fn person_event(e: &Map<String, Json>) -> bool {
    let event = text(e, "event").unwrap_or_default();
    let t = text(e, "summary").unwrap_or_else(|| event.clone());
    let lower = t.to_lowercase();
    let starts = [
        "message",
        "relance",
        "invitation",
        "accord",
        "cv",
        "candidature",
        "réponse envoyée",
    ];
    let has = ["envoy", "transfer", "transfèr", "demandé à", "republi"];
    let looks_ours = text(e, "ref").is_some_and(|r| r.starts_with("private/store/drafts/"))
        || starts.iter().any(|w| lower.starts_with(w))
        || has.iter().any(|w| lower.contains(w));
    let answers =
        lower.starts_with("relance de ") || lower.contains("répond") || quotes_someone(&t);
    let by = e
        .get("by")
        .is_some_and(|b| !b.is_null() && b != &Json::Bool(false));
    let ours = by || event.starts_with("sent draft") || (looks_ours && !answers);
    !ours && keyword(e, "channel").is_some_and(|c| PERSON.contains(&c.as_str()))
}

/// « Name : « … »: the event quotes what someone wrote.
fn quotes_someone(t: &str) -> bool {
    let c: Vec<char> = t.chars().collect();
    let lower = |x: char| x.is_ascii_lowercase() || ('à'..='ÿ').contains(&x);
    let upper = |x: char| x.is_ascii_uppercase() || ('À'..='Ý').contains(&x);
    let back = |mut j: usize| {
        while j > 0 && c[j - 1].is_whitespace() {
            j -= 1;
        }
        j
    };
    (0..c.len()).filter(|&i| c[i] == '«').any(|i| {
        let j = back(i);
        if j == 0 || c[j - 1] != ':' {
            return false;
        }
        let (end, mut j) = (back(j - 1), back(j - 1));
        while j > 0 && lower(c[j - 1]) {
            j -= 1;
        }
        end > j && j > 0 && upper(c[j - 1])
    })
}

/// A place on a lead's timeline: its day, then its line.
type At = (String, i64);

/// Each lead's timeline, by its id.
fn timelines(leads: &Json) -> Result<BTreeMap<String, Vec<Map<String, Json>>>, String> {
    let files = leads.as_object().ok_or("leads: not a map of files")?;
    let mut out = BTreeMap::new();
    for (file, body) in files {
        let lead: Json = serde_json::from_str(body.as_str().unwrap_or_default())
            .map_err(|e| format!("lead {file}: {e}"))?;
        let Some(lead) = lead.as_object() else {
            continue;
        };
        let Some(id) = keyword(lead, "id") else {
            continue;
        };
        let events = lead
            .get("timeline")
            .and_then(Json::as_array)
            .into_iter()
            .flatten();
        let events = events.filter_map(|e| e.as_object().cloned()).collect();
        out.insert(id, events);
    }
    Ok(out)
}

/// How much two texts share: the words in both over the words in either, case and marks apart.
fn overlap(a: &str, b: &str) -> f64 {
    let words = |t: &str| -> std::collections::BTreeSet<String> {
        t.split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
            .map(str::to_lowercase)
            .collect()
    };
    let (a, b) = (words(a), words(b));
    let either = a.union(&b).count();
    if either == 0 {
        0.0
    } else {
        a.intersection(&b).count() as f64 / either as f64
    }
}

/// A text shared at least this much is the same message edited.
const SAME_MESSAGE: f64 = 0.6;

/// Which draft each document Jev read (`dash doc ready`) belongs to: the one with a day, on or before the document's first
/// score, whose body is closest to the document's last text. One document, one draft at most. The body may have changed
/// after the last score (the Hostman form was edited before he sent it), so exact text cannot be the link.
/// ponytail: a word overlap, no edit distance; a short draft whose words a document also holds could take it.
fn documents_by_draft(
    docs: ByKey,
    list: &[(String, String, String)],
) -> BTreeMap<String, Vec<Scored>> {
    let mut out: BTreeMap<String, Vec<Scored>> = BTreeMap::new();
    for g in docs.into_values() {
        let first = g.iter().map(|s| s.ts.get(..10).unwrap_or_default()).min();
        let last = g
            .iter()
            .max_by(|a, b| a.ts.cmp(&b.ts))
            .map(|s| s.text.as_str());
        let (Some(first), Some(last)) = (first, last) else {
            continue;
        };
        let close = list
            .iter()
            .filter(|(_, created, _)| !created.is_empty() && created.as_str() <= first);
        let close = close
            .map(|(id, created, body)| (overlap(last, body), created, id))
            .filter(|(o, ..)| *o >= SAME_MESSAGE);
        // Two drafts alike (a template sent twice): the one written last before the score.
        let closest = close.max_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(b.1)));
        if let Some((.., id)) = closest {
            out.entry(id.clone()).or_default().extend(g);
        }
    }
    out
}

/// The older app's export: `dash model drafts` (a list), `dash store files opportunities` (each lead file's name to its
/// JSON text) and Jev's log. A sent draft's reply is its lead's first person event after it and before our next send on
/// that lead (`Dashboard.Followup` reads replies from the same timelines).
/// ponytail: a reply goes to the lead's last send before it whoever it answers; `Dashboard.Sales.answered?` also matches the
/// first name, worth porting once two people written to on one lead skew a rate.
pub fn from_store(drafts: &Json, leads: &Json, jev: impl BufRead) -> Result<Vec<Draft>, String> {
    let list = drafts.as_array().ok_or("drafts: not a list")?;
    let timelines = timelines(leads)?;
    let (mut by_draft, by_doc) = jev_lines(jev)?;
    let bodies: Vec<(String, String, String)> = list
        .iter()
        .filter_map(Json::as_object)
        .map(|m| {
            let f = |k| text(m, k).unwrap_or_default();
            (f("id"), f("created"), f("body"))
        })
        .collect();
    let mut by_document = documents_by_draft(by_doc, &bodies);
    let (mut out, mut files) = (vec![], vec![]);
    for d in list {
        let m = d.as_object().ok_or("drafts: an entry that is not a map")?;
        let id = text(m, "id").ok_or("drafts: an entry with no id")?;
        let body = text(m, "body").unwrap_or_default();
        let created = text(m, "created").unwrap_or_default();
        let file = text(m, "file").unwrap_or_default();
        let name = Path::new(&file)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned());
        let mut scored = name
            .as_ref()
            .and_then(|n| by_draft.remove(n))
            .unwrap_or_default();
        files.push(name.unwrap_or_default());
        scored.extend(by_document.remove(&id).unwrap_or_default());
        let who = text(m, "written-by").unwrap_or_else(|| "claude".into());
        let (mut versions, apart) = versions(scored);
        for v in versions.iter_mut().filter(|v| same(&v.text, &body)) {
            (v.by, v.text) = (who.clone(), body.clone());
        }
        if !versions.iter().any(|v| same(&v.text, &body)) {
            versions.push(Version {
                text: body.clone(),
                by: who,
                on: created.clone(),
                ..Default::default()
            });
        }
        if let Some(v) = versions.iter_mut().find(|v| same(&v.text, &body)) {
            v.jev.extend(apart);
        }
        let context = text(m, "context").unwrap_or_default();
        let goal = context
            .find("Goal: ")
            .map_or(context.as_str(), |at| &context[at + 6..]);
        let status = text(m, "status").unwrap_or_default();
        out.push(Draft {
            goal: goal.to_string(),
            sender: text(m, "from").unwrap_or_else(|| "Nicolas Ha".into()),
            receiver: text(m, "to").unwrap_or_default(),
            channel: text(m, "channel").unwrap_or_default(),
            lead: text(m, "opportunity"),
            // A rejection after an approval is his last word.
            approved: match status.as_str() {
                "rejected" => Some(false),
                "approved" | "sent" => Some(true),
                _ if text(m, "approved-on").is_some() => Some(true),
                _ => None,
            },
            from_blocks: m.get("from-approved-blocks").and_then(Json::as_bool),
            sent_on: day(text(m, "sent-on")).filter(|_| status == "sent"),
            created: day(Some(created)).unwrap_or_default(),
            status,
            versions,
            reply: None,
            id,
        });
    }
    join_replies(&mut out, &files, &timelines);
    Ok(out)
}

/// A date's day: an event written « 2026-10-08T09:00 » is on 2026-10-08, the day a send is compared with.
fn day(date: Option<String>) -> Option<String> {
    date.map(|d| d.get(..10).unwrap_or(&d).to_string())
}

/// `files`: each draft's file name, beside it. A send sits at the timeline's line that names it (« sent draft <id> », or
/// its file in a ref or a text); with none, at the end of its day, so an event that day is never taken for its reply: it
/// may be the message it answered (on 2026-11-07, at most 29 of 110 sends had no such line, 13 of them an event that
/// day). A rate errs low, never high.
fn join_replies(
    drafts: &mut [Draft],
    files: &[String],
    timelines: &BTreeMap<String, Vec<Map<String, Json>>>,
) {
    let names = |e: &Map<String, Json>, id: &str, file: &str| {
        let said = [text(e, "event"), text(e, "summary")].into_iter().flatten();
        let mut said = said.map(|t| format!("{t} "));
        let draft = [" ", ",", ")", ";", ":"].map(|end| format!("draft {id}{end}"));
        text(e, "ref").is_some_and(|r| !file.is_empty() && r.ends_with(&format!("/{file}")))
            || said.any(|t| {
                (!file.is_empty() && t.contains(file))
                    || draft.iter().any(|d| t.contains(d.as_str()))
            })
    };
    for (lead, events) in timelines {
        let mut sends: Vec<(At, usize)> = (0..drafts.len())
            .filter(|&i| drafts[i].sent_on.is_some() && drafts[i].lead.as_ref() == Some(lead))
            .map(|i| {
                let line = events
                    .iter()
                    .position(|e| names(e, &drafts[i].id, &files[i]));
                let on = drafts[i].sent_on.clone().unwrap_or_default();
                ((on, line.map_or(i64::MAX, |l| l as i64)), i)
            })
            .collect();
        sends.sort();
        let answers: Vec<(At, &Map<String, Json>)> = (0..events.len())
            .filter(|&i| person_event(&events[i]))
            .map(|i| {
                (
                    (day(text(&events[i], "date")).unwrap_or_default(), i as i64),
                    &events[i],
                )
            })
            .collect();
        for (k, (from, i)) in sends.iter().enumerate() {
            let until = sends.get(k + 1).map(|(at, _)| at);
            let first = answers
                .iter()
                .filter(|(at, _)| at > from && until.is_none_or(|u| at < u));
            let first = first.min_by(|a, b| a.0.cmp(&b.0));
            drafts[*i].reply = first.map(|((on, _), e)| Reply {
                on: on.clone(),
                what: text(e, "summary")
                    .or_else(|| text(e, "event"))
                    .unwrap_or_default(),
            });
        }
    }
}

/// The words one text changed into the other: those taken out plus those put in, by the longest run they share in order.
/// ponytail: a table of both lengths' product, fine at a message's few hundred words.
pub fn words_changed(before: &str, after: &str) -> usize {
    let (a, b): (Vec<&str>, Vec<&str>) = (
        before.split_whitespace().collect(),
        after.split_whitespace().collect(),
    );
    let mut row = vec![0usize; b.len() + 1];
    for x in &a {
        let mut diagonal = 0;
        for (j, y) in b.iter().enumerate() {
            let above = row[j + 1];
            row[j + 1] = if x == y {
                diagonal + 1
            } else {
                above.max(row[j])
            };
            diagonal = above;
        }
    }
    a.len() + b.len() - 2 * row[b.len()]
}

/// What a labelled file says of a draft (private/evidence/drafts/<day>-<id>.md): its goal, sender and receiver, each
/// version's change and Jev score, the texts it keeps, and his edit with the version it made.
#[derive(Debug, Default, PartialEq)]
pub struct Labels {
    pub id: String,
    pub goal: String,
    pub sender: String,
    pub receiver: String,
    /// In the table's order.
    pub versions: Vec<Labelled>,
    /// The version his edit made, and its lines.
    pub edit: Option<(String, Vec<String>)>,
}

/// One row of a labelled file's table.
#[derive(Debug, Default, PartialEq)]
pub struct Labelled {
    /// v1, v2…
    pub name: String,
    /// What was asked or made.
    pub change: String,
    /// The Jev question its table names, and the score, when it was run.
    pub score: Option<(String, f64)>,
    /// Its text, when the file keeps it.
    pub text: Option<String>,
}

/// Reads a labelled file: « Draft `<id>` », « N. **Goal:** », « **Sender:** », « **Receiver:** », a table under « ##
/// Versions » with a « Jev <question> » column, « ## His edit, vA to vB » and its bullets, and each « ## vN » with a text block.
/// ponytail: the one file of this shape so far, written by hand; a schema once a second writer makes them.
pub fn labels(md: &str) -> Result<Labels, String> {
    let after = |at: &str, end: char| {
        md.split_once(at)
            .and_then(|(_, rest)| rest.split(end).next())
            .map(str::trim)
    };
    let field = |name: &str| {
        after(&format!("**{name}:**"), '\n')
            .unwrap_or_default()
            .to_string()
    };
    let mut l = Labels {
        id: after("Draft `", '`')
            .ok_or("no « Draft `<id>` » line")?
            .to_string(),
        goal: field("Goal"),
        sender: field("Sender"),
        receiver: field("Receiver"),
        ..Default::default()
    };
    // A fenced block is text, never headings or bullets; only a version's section keeps it.
    let (mut question, mut section, mut block) = (None, String::new(), None::<Vec<&str>>);
    for line in md.lines() {
        if let Some(lines) = block.as_mut() {
            if line.starts_with("```") {
                let text = lines.join("\n");
                if let Some(v) = l.versions.iter_mut().find(|v| v.name == section) {
                    v.text = Some(text);
                } else if section.starts_with('v') {
                    return Err(format!(
                        "text of {section}, a version its table does not name"
                    ));
                }
                block = None;
            } else {
                lines.push(line);
            }
            continue;
        }
        if let Some(heading) = line.strip_prefix("## ") {
            section = heading
                .split([',', ' '])
                .next()
                .unwrap_or_default()
                .to_string();
            if let Some(range) = heading.strip_prefix("His edit, ") {
                let made = range
                    .split_whitespace()
                    .nth(2)
                    .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()));
                let made = made
                    .filter(|w| !w.is_empty())
                    .ok_or("« ## His edit, vA to vB »: no vB")?;
                l.edit = Some((made.to_string(), vec![]));
                section = "His edit".into();
            }
        } else if line.starts_with("```") {
            block = Some(vec![]);
        } else if let Some(bullet) = line.strip_prefix("- ").filter(|_| section == "His edit") {
            if let Some((_, lines)) = l.edit.as_mut() {
                lines.push(bullet.to_string());
            }
        } else if line.starts_with('|') && section == "Versions" {
            let cells: Vec<&str> = line.trim_matches('|').split('|').map(str::trim).collect();
            if cells.first() == Some(&"Version") {
                question = cells
                    .iter()
                    .position(|c| c.starts_with("Jev "))
                    .map(|i| (i, cells[i][4..].to_string()));
            } else if cells.first().is_some_and(|c| c.starts_with('v')) {
                let name = cells[0]
                    .split([',', ' '])
                    .next()
                    .unwrap_or_default()
                    .to_string();
                let score = question
                    .as_ref()
                    .and_then(|(i, q)| Some((q.clone(), cells.get(*i)?.parse().ok()?)));
                let change = cells.get(1).unwrap_or(&"").to_string();
                l.versions.push(Labelled {
                    name,
                    change,
                    score,
                    text: None,
                });
            }
        }
    }
    if block.is_some() {
        return Err(format!(
            "a text block under « ## {section} » with no closing fence"
        ));
    }
    if l.versions.is_empty() {
        return Err("no version table under « ## Versions »".into());
    }
    Ok(l)
}

/// A draft with a labelled file's word: its goal, sender, receiver and versions. Version n keeps what `base` (the store's,
/// or the kept one) knows of its n-th version, its text, day and Jev's log scores; the file adds its change, its score
/// where the log has none, the text it keeps, and his edit. The rest (lead, status, sends, reply) stays the base's.
pub fn labelled(base: Option<Draft>, l: Labels) -> Draft {
    let mut d = base.unwrap_or_else(|| Draft {
        id: l.id.clone(),
        goal: String::new(),
        sender: String::new(),
        receiver: String::new(),
        channel: String::new(),
        lead: None,
        status: String::new(),
        approved: None,
        from_blocks: None,
        sent_on: None,
        created: String::new(),
        versions: vec![],
        reply: None,
    });
    let known = std::mem::take(&mut d.versions);
    // A row with a text takes the known version of that text; one without, the known version at its place, if no row took
    // it. No known version goes to two rows, and none is dropped: those no row takes stay, after the file's.
    let mut taken = vec![false; known.len()];
    let mut at: Vec<Option<usize>> = vec![None; l.versions.len()];
    for (row, slot) in l.versions.iter().zip(at.iter_mut()) {
        let Some(t) = row.text.as_deref() else {
            continue;
        };
        *slot = (0..known.len()).find(|&k| !taken[k] && same(t, &known[k].text));
        slot.inspect(|&k| taken[k] = true);
    }
    for (row, slot) in at.iter_mut().enumerate() {
        if slot.is_none() && l.versions[row].text.is_none() && taken.get(row) == Some(&false) {
            (*slot, taken[row]) = (Some(row), true);
        }
    }
    let rows = l.versions.into_iter().zip(at);
    d.versions = rows
        .map(
            |(
                Labelled {
                    name,
                    change,
                    score,
                    text,
                },
                k,
            )| {
                let mut v = k.map(|k| known[k].clone()).unwrap_or_default();
                if let Some(t) = text {
                    v.text = t;
                }
                if let Some((q, x)) = score {
                    v.jev.entry(q).or_insert(x);
                }
                if let Some((_, lines)) = l.edit.as_ref().filter(|(made, _)| *made == name) {
                    (v.by, v.edit) = ("nicolas".into(), lines.clone());
                }
                if v.by.is_empty() {
                    v.by = "claude".into();
                }
                v.change = change;
                v
            },
        )
        .collect();
    let left = known.into_iter().zip(taken).filter(|(_, t)| !t);
    d.versions.extend(left.map(|(mut v, _)| {
        v.change = "not in the labelled file".into();
        v
    }));
    (d.goal, d.sender, d.receiver) = (l.goal, l.sender, l.receiver);
    d
}

/// The proxy until replies are linked (@nha, 2026-11-07: « the proxy is what I approve for now »), by channel: what he
/// approved of what he judged, and how much his edits changed the drafts he approved. Then replies by day 7, by channel
/// and by whether the message was made only of his approved blocks.
pub fn rates(drafts: &[Draft], today: &str) -> String {
    let pct = |n: usize, of: usize| (n * 100).checked_div(of).unwrap_or(0);
    let mut channels: BTreeMap<&str, [usize; 5]> = BTreeMap::new();
    for d in drafts {
        let row = channels.entry(d.channel.as_str()).or_default();
        if let Some(yes) = d.approved {
            row[0] += 1;
            row[1] += usize::from(yes);
        }
        if let (Some(true), Some((changed, of))) = (d.approved, d.his_edit()) {
            row[2] += 1;
            row[3] += changed;
            row[4] += of;
        }
    }
    let mut lines: Vec<String> = channels
        .iter()
        .map(|(channel, [judged, approved, edited, changed, of])| {
            format!(
                "approval {channel}: {approved} of {judged} judged approved ({}%); his edit on {edited}, \
                 {changed} of {of} words changed ({}%)",
                pct(*approved, *judged),
                pct(*changed, *of)
            )
        })
        .collect();
    let mut rows: BTreeMap<(&str, String), [usize; 3]> = BTreeMap::new();
    for d in drafts.iter().filter(|d| d.sent_on.is_some()) {
        let outcome = d.outcome(today);
        let blocks = match d.from_blocks {
            Some(true) => "yes",
            Some(false) => "no",
            None => "unknown",
        };
        for key in [
            ("channel", d.channel.clone()),
            ("from approved blocks", blocks.into()),
        ] {
            let row = rows.entry(key).or_default();
            row[0] += 1;
            row[1] += usize::from(matches!(outcome, "reply" | "none"));
            row[2] += usize::from(outcome == "reply");
        }
    }
    lines.extend(rows.iter().map(|((by, key), [sent, decided, replies])| {
        let p = pct(*replies, *decided);
        format!("{by} {key}: {replies} of {decided} replied by day 7 ({p}%), {sent} sent")
    }));
    lines.join("\n")
}

/// A draft as `dash draft show` prints it, each version whole with its scores.
pub fn show(d: &Draft, today: &str) -> String {
    let yes = |b: bool| if b { "yes" } else { "no" };
    let blocks = d.from_blocks.map_or("unknown", yes);
    let edit = match d.his_edit() {
        Some((changed, of)) => format!("{changed} of {of} words changed"),
        None if d.versions.first().is_some_and(|v| v.by == "nicolas") => "his own text".into(),
        None if d.his_rewrite() => "his, its size unknown: a text is not kept".into(),
        None => "none".into(),
    };
    let outcome = match (d.outcome(today), &d.reply) {
        ("reply", Some(r)) => format!("reply on {}: {}", r.on, r.what),
        (o, _) => o.to_string(),
    };
    let mut lines = vec![
        d.id.clone(),
        format!("goal: {}", d.goal),
        format!("sender: {}", d.sender),
        format!("receiver: {}", d.receiver),
        format!(
            "channel: {}, lead: {}",
            d.channel,
            d.lead.as_deref().unwrap_or("none")
        ),
        format!(
            "status: {}, sent on: {}",
            d.status,
            d.sent_on.as_deref().unwrap_or("not sent")
        ),
        format!(
            "approved: {}; outcome: {outcome}; his edit: {edit}; from approved blocks: {blocks}",
            d.approved.map_or("not judged", yes),
        ),
        format!("versions: {}", d.versions.len()),
    ];
    for (n, v) in d.versions.iter().enumerate() {
        let jev: Vec<String> = v.jev.iter().map(|(q, x)| format!("{q} {x}")).collect();
        let jev = if jev.is_empty() {
            "not scored".into()
        } else {
            jev.join(", ")
        };
        lines.push(format!("v{} on {} by {}: {jev}", n + 1, v.on, v.by));
        if !v.change.is_empty() {
            lines.push(format!("  change: {}", v.change));
        }
        lines.extend(v.edit.iter().map(|l| format!("  his edit: {l}")));
        if v.text.is_empty() {
            lines.push("  (its text is not kept)".into());
        }
        lines.extend(v.text.lines().map(|l| format!("  {l}")));
    }
    lines.join("\n")
}

/// The statuses a draft is open in (`Dashboard.Sales`' @open_draft), in the order /drafts lists them: his corrections in
/// progress, then those to review, then those approved and not sent yet (task rust-drafts-list).
pub const OPEN: [&str; 3] = ["approved-pending-changes", "pending", "approved"];
/// A goal's line on /drafts is at most this many characters; `dash draft show` prints it whole.
const GOAL_LINE: usize = 140;

/// The open drafts in /drafts' order: by status as `OPEN` lists them, each status oldest first: the day the older app wrote
/// it, else its first version's (Jev may first score a draft days after it was written).
pub fn open(drafts: &[Draft]) -> Vec<&Draft> {
    let first = |d: &Draft| match d.created.as_str() {
        "" => d.versions.iter().map(|v| v.on.clone()).min(),
        day => Some(day.to_string()),
    };
    let mut open: Vec<_> = drafts
        .iter()
        .filter_map(|d| Some(((OPEN.iter().position(|s| *s == d.status)?, first(d)), d)))
        .collect();
    open.sort_by(|(a, x), (b, y)| (a, &x.id).cmp(&(b, &y.id)));
    open.into_iter().map(|(_, d)| d).collect()
}

impl Draft {
    /// Its goal's first line, cut at `GOAL_LINE` characters; « no goal » when it has none, never a blank.
    pub fn goal_line(&self) -> String {
        let line = self.goal.lines().map(str::trim).find(|l| !l.is_empty());
        let Some(line) = line else {
            return "no goal".into();
        };
        match line.char_indices().nth(GOAL_LINE) {
            Some((at, _)) => format!("{}…", &line[..at]),
            None => line.into(),
        }
    }

    /// Jev's answers on its last version scored, each question and its number, « to rescore » when a later version came
    /// after it; « not scored » with none. A part « mine » is left out: it scored his sentences alone, not the draft.
    pub fn jev_line(&self) -> String {
        let said = |v: &Version| -> Vec<String> {
            let answers = v.jev.iter().filter(|(q, _)| !q.starts_with("mine."));
            answers.map(|(q, x)| format!("{q} {x}")).collect()
        };
        let mut scored = self.versions.iter().map(said).enumerate().rev();
        match scored.find(|(_, s)| !s.is_empty()) {
            None => "not scored".into(),
            Some((at, s)) if at + 1 < self.versions.len() => {
                format!("{} (an earlier version: to rescore)", s.join(", "))
            }
            Some((_, s)) => s.join(", "),
        }
    }
}

fn plus_days(on: &str, n: i64) -> Option<String> {
    days(on).map(|d| date(d + n))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::memory::TestBudget;

    use dash_core::MemoryLog as Log;

    fn lead(id: &str, timeline: Json) -> (String, Json) {
        let text = json!({"id": format!("~:{id}"), "timeline": timeline}).to_string();
        (format!("{id}.edn"), Json::String(text))
    }

    fn export() -> Vec<Draft> {
        let drafts = json!([
            {"id": "a-first", "file": "private/store/drafts/2026-10-01-a-first.md", "to": "Ana (Acme)", "channel": "linkedin",
             "opportunity": "acme", "status": "sent", "sent-on": "2026-10-01", "from-approved-blocks": true,
             "created": "2026-10-01", "context": "First message.", "body": "Hi Ana,\nhello"},
            {"id": "a-relance", "file": "private/store/drafts/2026-10-05-a-relance.md", "to": "Ana (Acme)",
             "channel": "linkedin", "opportunity": "acme", "status": "sent", "sent-on": "2026-10-05",
             "from-approved-blocks": false, "created": "2026-10-05", "body": "Ana, any news?", "written-by": "nicolas"},
            {"id": "form-x", "file": "private/store/drafts/2026-11-07-form-x.md", "to": "a reviewer", "channel": "form",
             "status": "pending", "created": "2026-11-07", "from": "Klar",
             "context": "Credits for Klar. Goal: host Klar on an EU cloud.", "body": "Klar is a filter.\n- one\n- two\n- three"},
            {"id": "b-cold", "file": "x/b-cold.md", "to": "Bo", "channel": "email", "opportunity": "bee",
             "status": "sent", "sent-on": "2026-10-01", "created": "2026-10-01", "body": "Bo"},
            {"id": "form-y", "file": "x/form-y.md", "channel": "form", "status": "pending", "created": "2026-11-06",
             "body": "Klar is a filter.\n- one\n- two\n- three"},
            {"id": "form-z", "file": "x/form-z.md", "channel": "form", "status": "pending",
             "body": "Klar is a filter.\n- one\n- two\n- three"},
        ]);
        let leads: Map<String, Json> = [
            lead("acme", json!([
                {"date": "2026-10-01", "channel": "~:linkedin", "by": "~:claude", "event": "sent draft a-first to Ana"},
                {"date": "2026-10-02", "channel": "~:linkedin", "summary": "relance envoyée"},
                {"date": "2026-10-05", "channel": "~:linkedin", "event": "sent draft a-relance to Ana"},
                {"date": "2026-10-06", "channel": "~:other", "summary": "Ana viewed the profile"},
                {"date": "2026-10-09", "channel": "~:linkedin", "summary": "Ana répond : « send your CV »"},
            ])),
            lead("bee", json!([
                {"date": "2026-10-01T08:00", "channel": "~:email", "summary": "Bo: « who is free? »"},
                {"date": "2026-10-20", "channel": "~:email", "summary": "Bo: « later »"},
            ])),
        ]
        .into_iter()
        .collect();
        let jev = [
            json!({"ts": "2026-10-01T08:00:00Z", "draft": "old/drafts/2026-10-01-a-first.md",
                   "request": {"body": {"state": {"message": "Hi Ana, hello there"}}},
                   "response": {"answers": {"reads_ai_written": {"noul": 0.9}}}}),
            json!({"ts": "2026-10-01T09:00:00Z", "draft": "private/store/drafts/2026-10-01-a-first.md",
                   "request": {"body": {"state": {"message": "Hi Ana, hello"}}},
                   "response": {"answers": {"reads_ai_written": {"noul": 0.2}, "too_long": {"noul": 0.1}}}}),
            json!({"ts": "2026-10-01T09:00:01Z", "draft": "x/2026-10-01-a-first.md", "part": "mine",
                   "request": {"body": {"state": {"message": "hello"}}},
                   "response": {"answers": {"reads_ai_written": {"noul": 0.3}}}}),
            json!({"ts": "2026-10-05T09:00:00Z", "draft": "x/2026-10-05-a-relance.md", "part": "mine",
                   "request": {"body": {"state": {"message": "any news?"}}},
                   "response": {"answers": {"too_long": {"noul": 0.05}}}}),
            json!({"ts": "2026-11-07T21:00:00Z", "doc": "/tmp/s/x.txt", "request": {"body": {"state": {"doc": "Klar is AI."}}},
                   "response": {"answers": {"unexplained_terms": {"noul": 0.78}}}}),
            json!({"ts": "2026-11-07T21:01:00Z", "doc": "/tmp/s/x.txt",
                   "request": {"body": {"state": {"doc": "Klar is a filter. - one - two"}}},
                   "response": {"answers": {"unexplained_terms": {"noul": 0.64}}}}),
            json!({"ts": "2026-11-07T21:02:00Z", "doc": "/tmp/s/other.txt", "request": {"body": {"state": {"doc": "Else"}}},
                   "response": {"answers": {"q": {"noul": 0.5}}}}),
            json!({"ts": "2026-11-07T21:03:00Z", "status": "error", "draft": "x/b-cold.md"}),
        ];
        let jev: String = jev.iter().map(|l| format!("{l}\nnot json\n")).collect();
        from_store(&drafts, &Json::Object(leads), jev.as_bytes()).expect("imports")
    }

    #[test]
    fn a_reply_goes_to_the_last_send_before_it_and_counts_by_day_7() {
        let _budget = TestBudget::new();
        let d = export();
        let (first, relance, form, cold) = (&d[0], &d[1], &d[2], &d[3]);
        assert_eq!(first.reply, None, "our own relance line is no reply");
        assert_eq!(first.outcome("2026-10-20"), "none");
        let reply = relance.reply.as_ref().expect("Ana answered the relance");
        assert_eq!(
            (reply.on.as_str(), relance.outcome("2026-10-20")),
            ("2026-10-09", "reply")
        );
        assert_eq!(form.outcome("2026-11-07"), "unsent");
        assert_eq!(
            cold.reply.as_ref().map(|r| r.on.as_str()),
            Some("2026-10-20"),
            "a send no line names sits at its day's end, so its day's message is no reply; a late reply is kept"
        );
        let mut no_day = cold.clone();
        no_day.sent_on = Some("10/01/2026".into());
        assert_eq!(no_day.outcome("2026-10-21"), "no send day");
        assert_eq!(cold.outcome("2026-10-08"), "waiting");
        assert_eq!(cold.outcome("2026-10-21"), "none", "it came after day 7");
        let mut no_lead = cold.clone();
        no_lead.lead = None;
        assert_eq!(no_lead.outcome("2026-10-21"), "no lead");
    }

    #[test]
    fn each_text_jev_scored_is_a_version_and_his_rewrite_is_named() {
        let _budget = TestBudget::new();
        let d = export();
        let first = &d[0];
        let header = (
            first.goal.as_str(),
            first.sender.as_str(),
            first.receiver.as_str(),
            first.channel.as_str(),
        );
        assert_eq!(
            header,
            ("First message.", "Nicolas Ha", "Ana (Acme)", "linkedin"),
            "no « from »: him"
        );
        let sent = (
            first.status.as_str(),
            first.sent_on.as_deref(),
            first.lead.as_deref(),
        );
        assert_eq!(sent, ("sent", Some("2026-10-01"), Some("acme")));
        assert_eq!(
            (d[2].status.as_str(), d[2].sent_on.as_deref()),
            ("pending", None),
            "unsent has no send day"
        );
        assert_eq!(first.versions.len(), 2, "an older path names the same file");
        assert_eq!(
            first.versions[1].text, "Hi Ana,\nhello",
            "the body keeps its lines"
        );
        assert_eq!(first.versions[1].jev.get("too_long"), Some(&0.1));
        assert_eq!(
            first.versions[1].jev.get("mine.reads_ai_written"),
            Some(&0.3),
            "a part joins its whole"
        );
        assert!(!first.his_rewrite());
        assert!(d[1].his_rewrite(), "written by him");
        let relance = &d[1].versions;
        assert_eq!(relance.len(), 1, "a part alone is no version");
        assert_eq!(
            relance[0].jev.get("mine.too_long"),
            Some(&0.05),
            "it scores the body"
        );

        let form = &d[2];
        let scores: Vec<_> = form
            .versions
            .iter()
            .map(|v| v.jev.get("unexplained_terms"))
            .collect();
        assert_eq!(
            scores,
            [Some(&0.78), Some(&0.64), None],
            "the document Jev read is its own, edited since; the other file is not it"
        );
        assert_eq!(
            form.versions[2].text,
            "Klar is a filter.\n- one\n- two\n- three"
        );
        assert_eq!(
            (form.goal.as_str(), form.sender.as_str()),
            ("host Klar on an EU cloud.", "Klar")
        );
        assert_eq!(d[3].versions.len(), 1, "an error line scores nothing");
        let scored = |i: usize| d[i].versions.iter().any(|v| !v.jev.is_empty());
        assert!(
            !scored(4),
            "one document goes to one draft, the closest with a day"
        );
        assert!(!scored(5), "a draft with no day takes no document");
    }

    #[test]
    fn a_kept_draft_reads_back_and_an_unchanged_import_writes_nothing() {
        let _budget = TestBudget::new();
        let mut log = Log::default();
        let d = export();
        assert_eq!(save(&mut log, &Kept::default(), &d), Ok(6));
        let kept = load(&log).expect("reads");
        assert_eq!(save(&mut log, &kept, &d), Ok(0));
        let mut changed = d[0].clone();
        changed.status = "moot".into();
        let kept = load(&log).expect("reads");
        assert_eq!(save(&mut log, &kept, &[changed.clone()]), Ok(1));
        let bad = FieldWrite {
            table: TABLE.into(),
            row: "form-x".into(),
            field: FIELD.into(),
            value: Value::Text("{".into()),
        };
        log.write(&[bad]).expect("written");
        let kept = load(&log).expect("one bad record stops no read");
        assert_eq!((kept.drafts.len(), &kept.drafts["a-first"]), (5, &changed));
        assert_eq!(kept.unread.iter().collect::<Vec<_>>(), ["form-x"]);
        assert_eq!(
            save(&mut log, &kept, &d[2..3]),
            Ok(1),
            "an import writes it again"
        );
        let kept = load(&log).expect("reads").drafts;
        assert_eq!(kept["form-x"], d[2]);
        let shown = show(&kept["form-x"], "2026-11-07");
        assert!(
            shown.contains("versions: 3\nv1 on 2026-11-07 by claude: unexplained_terms 0.78"),
            "{shown}"
        );
    }

    #[test]
    fn rates_count_only_sends_with_a_verdict() {
        let _budget = TestBudget::new();
        let mut drafts = export();
        let mut edited = labelled(Some(drafts[2].clone()), labels(LABELLED).expect("reads"));
        edited.approved = Some(true);
        drafts[2] = edited;
        assert_eq!(
            rates(&drafts, "2026-10-10"),
            "approval email: 1 of 1 judged approved (100%); his edit on 0, 0 of 0 words changed (0%)\n\
             approval form: 1 of 1 judged approved (100%); his edit on 1, 2 of 8 words changed (25%)\n\
             approval linkedin: 2 of 2 judged approved (100%); his edit on 0, 0 of 0 words changed (0%)\n\
             channel email: 0 of 1 replied by day 7 (0%), 1 sent\n\
             channel linkedin: 1 of 2 replied by day 7 (50%), 2 sent\n\
             from approved blocks no: 1 of 1 replied by day 7 (100%), 1 sent\n\
             from approved blocks unknown: 0 of 1 replied by day 7 (0%), 1 sent\n\
             from approved blocks yes: 0 of 1 replied by day 7 (0%), 1 sent"
        );
    }

    const LABELLED: &str = "# x\n\nDraft `form-x` (store) holds the sent text only.\n\n1. **Goal:** credits.\n\
        2. **Sender:** Nicolas.\n3. **Receiver:** a reviewer.\n\n## Versions\n\n\
        | Version | Change asked or made | Jev unexplained_terms |\n|---|---|---|\n\
        | v1 | engine detail | 0.78 |\n| v2 | cut | not run |\n| v3, sent | his own edit of v2 | not run |\n\n\
        ## His edit, v2 to v3 (what went)\n\n- Added « three ».\n\n## v3, sent 2026-11-07\n\n\
        ```text\nKlar is a filter.\n- one\n- two\n- three\n```\n";

    #[test]
    fn a_labelled_file_names_versions_and_his_edit_on_the_stores_record() {
        let _budget = TestBudget::new();
        let md = LABELLED;
        let l = labels(md).expect("reads");
        assert_eq!(
            (l.id.as_str(), l.goal.as_str(), l.receiver.as_str()),
            ("form-x", "credits.", "a reviewer.")
        );
        assert_eq!(l.edit, Some(("v3".into(), vec!["Added « three ».".into()])));
        let d = labelled(export().into_iter().nth(2), l);
        let v = &d.versions;
        assert_eq!(v.len(), 3);
        assert_eq!(
            (v[0].jev["unexplained_terms"], v[1].jev["unexplained_terms"]),
            (0.78, 0.64),
            "the log's scores kept"
        );
        assert_eq!(
            (v[2].by.as_str(), v[2].edit.len(), v[1].change.as_str()),
            ("nicolas", 1, "cut")
        );
        assert_eq!(
            d.his_edit(),
            Some((2, 8)),
            "« - three » put in, of v2's 8 words"
        );
        assert_eq!(d.status, "pending", "the store's fields stay");
        let alone = labelled(None, labels(md).expect("reads"));
        assert_eq!(
            (
                alone.versions[0].text.as_str(),
                alone.versions[0].jev["unexplained_terms"]
            ),
            ("", 0.78)
        );
        assert!(labels("no table").is_err());
    }

    #[test]
    fn a_row_takes_one_version_none_is_lost_and_an_odd_file_is_refused_not_a_panic() {
        let _budget = TestBudget::new();
        let v = |text: &str, x: f64| Version {
            text: text.into(),
            by: "claude".into(),
            jev: [("q".to_string(), x)].into(),
            ..Default::default()
        };
        let mut base = export().remove(2);
        base.versions = vec![v("one", 0.1), v("three", 0.3), v("sent", 0.0)];
        let md = "Draft `form-x`\n## Versions\n| Version | Change | Jev q |\n| v1 | a | 0.1 |\n| v2 | b | not run |\n\
                  | v3 | c | 0.3 |\n## His edit, v2 to v3, sent\n- cut\n## Notes\n```text\n## v9\n- inside\n```\n\
                  ## v3\n```text\nthree\n```\n";
        let d = labelled(Some(base), labels(md).expect("reads"));
        let texts: Vec<_> = d
            .versions
            .iter()
            .map(|v| (v.text.as_str(), v.jev.get("q")))
            .collect();
        assert_eq!(
            texts,
            [
                ("one", Some(&0.1)),
                ("", None),
                ("three", Some(&0.3)),
                ("sent", Some(&0.0))
            ],
            "v2 has no text and takes no other version's; the sent body no row named stays, last"
        );
        assert_eq!(d.versions[3].change, "not in the labelled file");
        assert_eq!(
            (d.versions[2].by.as_str(), d.versions[2].edit.as_slice()),
            ("nicolas", &["cut".to_string()][..])
        );
        assert!(show(&d, "2026-11-07").contains("his edit: his, its size unknown"));
        let odd =
            "Draft `x`\n## Versions\n| Version | Change |\n| v1 | a |\n## His notes\n- a note\n";
        assert_eq!(
            labels(odd)
                .expect("a bullet under another heading is no edit")
                .edit,
            None
        );
        assert!(
            labels("Draft `x`\n## Versions\n| Version | C |\n| v1 | a |\n## v1\n```text\nno end")
                .is_err()
        );
        let mut drafts =
            json!([{"id": "r", "status": "rejected", "approved-on": "2026-10-01", "body": "x"}]);
        let r = from_store(&drafts.take(), &json!({}), &b""[..]).expect("imports");
        assert_eq!(
            r[0].approved,
            Some(false),
            "a rejection after an approval is his last word"
        );
    }

    #[test]
    fn an_edit_counts_the_words_out_and_in() {
        let _budget = TestBudget::new();
        assert_eq!(words_changed("a b c", "a x c"), 2);
        assert_eq!(words_changed("a b", "a b"), 0);
        assert_eq!(words_changed("", "a b"), 2);
        let d = export();
        assert_eq!(
            (d[0].approved, d[2].approved),
            (Some(true), None),
            "sent is approved, pending is not judged"
        );
        assert_eq!(d[1].his_edit(), None, "his own text edits nothing of ours");
    }

    #[test]
    fn a_week_on_crosses_months_and_years() {
        let _budget = TestBudget::new();
        for (from, to) in [
            ("2026-02-25", "2026-03-04"),
            ("2024-02-25", "2024-03-03"),
            ("2026-12-28", "2027-01-04"),
        ] {
            assert_eq!(plus_days(from, 7).as_deref(), Some(to));
        }
    }

    #[test]
    fn an_event_quoting_someone_is_theirs() {
        let _budget = TestBudget::new();
        assert!(quotes_someone("Message de Sophie : « oui »"));
        assert!(!quotes_someone("sent : « x »"));
        let ev = |s: &str| {
            json!({"channel": "~:email", "summary": s})
                .as_object()
                .cloned()
                .unwrap()
        };
        assert!(person_event(&ev("Candidature reçue, Léa : « merci »")));
        assert!(!person_event(&ev("candidature envoyée")));
    }

    /// The older app's export of one draft a status, the oldest pending last in the file; Jev scored two.
    fn waiting() -> Vec<Draft> {
        let drafts = json!([
            {"id": "sent-1", "to": "Sam", "status": "sent", "sent-on": "2026-11-01", "created": "2026-10-01", "body": "s"},
            {"id": "new-pending", "to": "Nia (Acme)", "status": "pending", "created": "2026-11-07",
             "context": "Lead acme. Goal: ask for the call.\nSecond line.", "body": "n", "file": "x/new-pending.md"},
            {"id": "approved-1", "to": "Al", "status": "approved", "created": "2026-10-20", "context": "Goal: apply", "body": "a"},
            {"id": "corrected", "to": "Cy", "status": "approved-pending-changes", "created": "2026-11-08",
             "context": "Goal: thank them", "body": "c"},
            {"id": "rejected-1", "to": "Ro", "status": "rejected", "created": "2026-09-01", "body": "r"},
            {"id": "old-pending", "to": "Olu", "status": "pending", "created": "2026-10-24", "context": "", "body": "o",
             "file": "x/old-pending.md"},
        ]);
        let jev = json!({"ts": "2026-11-07T10:00:00Z", "draft": "y/new-pending.md",
            "request": {"body": {"state": {"message": "n"}}},
            "response": {"answers": {"too_long": {"noul": 0.49}, "reads_ai_written": {"noul": 0.32}}}});
        // Jev first scored the oldest draft after the newest was written: « oldest » is the day it was written.
        let late = json!({"ts": "2026-11-09T10:00:00Z", "draft": "y/old-pending.md",
            "request": {"body": {"state": {"message": "o"}}}, "response": {"answers": {"too_long": {"noul": 0.1}}}});
        let log = format!("{jev}\n{late}\n");
        from_store(&drafts, &json!({}), log.as_bytes()).expect("imports")
    }

    /// Done-when 1 of rust-drafts-list: his corrections first, then pending oldest first, then approved; the record left out.
    #[test]
    fn the_open_drafts_list_corrections_first_then_pending_oldest_first() {
        let _budget = TestBudget::new();
        let all = waiting();
        let ids: Vec<&str> = open(&all).iter().map(|d| d.id.as_str()).collect();
        assert_eq!(
            ids,
            ["corrected", "old-pending", "new-pending", "approved-1"]
        );
    }

    /// Done-when 3 of rust-draft-approve: once the agent rewrites the text he asked corrections on, the old version stays.
    #[test]
    fn a_rewritten_draft_keeps_its_earlier_version_readable() {
        let _budget = TestBudget::new();
        let all = waiting();
        let before = all.iter().find(|d| d.id == "new-pending").expect("kept");
        let mut now = before.clone();
        now.status = "approved-pending-changes".into();
        now.versions = vec![Version {
            text: "n, corrected".into(),
            by: "claude".into(),
            ..Default::default()
        }];
        let merged = with_kept_versions(Some(before), now.clone());
        let texts: Vec<&str> = merged.versions.iter().map(|v| v.text.as_str()).collect();
        assert_eq!(texts, ["n", "n, corrected"]);
        assert_eq!(merged.versions[0].jev.get("too_long"), Some(&0.49));
        assert_eq!(merged.status, "approved-pending-changes");
        // read again, nothing doubles; with no record kept, the draft as read
        assert_eq!(with_kept_versions(Some(&merged), now.clone()), merged);
        assert_eq!(with_kept_versions(None, now.clone()), now);
        // /drafts' refresh, Jev's later score with it, keeps the earlier version too (/code-review finding 1)
        let mut scored = now.clone();
        scored.versions[0].jev = [("too_long".to_string(), 0.2)].into();
        let refreshed = with_kept_versions(Some(&merged), scored);
        assert_eq!(refreshed.versions.len(), 2);
        assert_eq!(refreshed.versions[1].jev.get("too_long"), Some(&0.2));
        // the text set back to the first: the current version is last again (/code-review finding 7)
        let mut back = now.clone();
        back.versions = vec![before.versions[0].clone()];
        let texts: Vec<String> = with_kept_versions(Some(&refreshed), back)
            .versions
            .into_iter()
            .map(|v| v.text)
            .collect();
        assert_eq!(texts, ["n, corrected", "n"]);
        // the same words reflowed: the text as written now; a read with no score keeps the scores, one with scores replaces
        // them whole, a question Jev dropped with them (second /code-review)
        let mut reflowed = now.clone();
        reflowed.versions[0].text = "n,\ncorrected".into();
        let after = with_kept_versions(Some(&refreshed), reflowed);
        assert_eq!(after.versions[1].text, "n,\ncorrected");
        assert_eq!(after.versions[1].jev.get("too_long"), Some(&0.2));
        let mut rescored = now.clone();
        rescored.versions[0].jev = [("vague".to_string(), 0.1)].into();
        let jev = &with_kept_versions(Some(&refreshed), rescored).versions[1].jev;
        assert_eq!(jev.keys().collect::<Vec<_>>(), ["vague"]);
    }

    /// Done-when 2 of rust-drafts-list: a goal is one line, cut when long; none says « no goal ».
    #[test]
    fn a_goal_is_one_line_and_none_says_no_goal() {
        let _budget = TestBudget::new();
        let all = waiting();
        let by = |id: &str| all.iter().find(|d| d.id == id).expect(id);
        assert_eq!(by("new-pending").goal_line(), "ask for the call.");
        assert_eq!(by("old-pending").goal_line(), "no goal");
        let mut long = by("corrected").clone();
        long.goal = format!("  \n{}", "é".repeat(200));
        assert_eq!(long.goal_line(), format!("{}…", "é".repeat(140)));
    }

    /// Done-when 2 of rust-drafts-list: Jev's answers on the last version, his part left out; none says « not scored ».
    #[test]
    fn a_row_s_score_is_jev_s_answers_or_not_scored() {
        let _budget = TestBudget::new();
        let all = waiting();
        let by = |id: &str| all.iter().find(|d| d.id == id).expect(id);
        assert_eq!(
            by("new-pending").jev_line(),
            "reads_ai_written 0.32, too_long 0.49"
        );
        assert_eq!(by("corrected").jev_line(), "not scored");
        let mut mine = by("new-pending").clone();
        mine.versions[0].jev = [("mine.too_long".to_string(), 0.1)].into();
        assert_eq!(mine.jev_line(), "not scored");
        // an edit Jev has not scored yet keeps the last score, marked (the older page's « to rescore »)
        let mut edited = by("new-pending").clone();
        edited.versions.push(Version::default());
        assert_eq!(
            edited.jev_line(),
            "reads_ai_written 0.32, too_long 0.49 (an earlier version: to rescore)"
        );
    }
}
