//! The latest-message pass (task rust-latest-reevaluates, the drafts plugin's `latest`): his mail over JMAP and the leads'
//! timelines give each contact, lead and draft address its latest message, kept in the drafts' file; each waiting draft
//! older than its latest gets Jev's five questions, and the verdict acts through the older app (`dash draft status|set`),
//! the answers kept beside it. `dash orch` runs it every 5 minutes (`watchers::LATEST`); `dash latest` runs it once.
use crate::watchers;
use dash_drafts::latest::{self, Held, Message, Verdict, Waiting};
use dash_drafts::{self as drafts, Draft};
use serde_json::{Map, Value};
use serde_json::{Value as Json, json};
use std::collections::BTreeMap;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::time::Duration;

/// The newest mails read a pass: a person's latest is among the last 200 mails of every folder, or older than any waiting
/// draft's day it would re-judge.
const RECENT: u64 = 200;
/// The drafts plugin's file under `dir`, as the host opens it.
pub(crate) fn drafts_file(dir: &Path) -> Result<dash_store::Connection, String> {
    let file = dir.join(dash_drafts::FILE);
    dash_store::plugin::open(&file, &dash_drafts::SCHEMAS, crate::tasks::now_ms())
}

/// `run` on the drafts' file in one transaction, its changeset kept, as the host's `transact`.
pub(crate) fn written(
    db: &dash_store::Connection,
    run: &mut dyn FnMut(&dash_store::Connection) -> Result<(), String>,
) -> Result<(), String> {
    dash_store::plugin::transact(db, crate::tasks::now_ms(), run)
}

/// Jev's calls a pass, at most: a first pass over a full store never runs up a bill, the rest go at the next.
const CALLS: usize = 10;
const OLDER_CALL: Duration = Duration::from_secs(60);

/// What a pass reads and writes outside its data folder: the mail, Jev, the older app's draft writes.
pub(crate) trait World {
    fn mail(&self) -> Result<Vec<dash_mail::Brief>, String>;
    fn ask(&self, state: Json) -> Result<Json, String>;
    fn older(&self, words: &[String]) -> Result<(), String>;
}

/// The world as it is, Jev asked through the host.
struct Live<'a>(&'a dyn dash_core::Host);

impl World for Live<'_> {
    fn mail(&self) -> Result<Vec<dash_mail::Brief>, String> {
        let http = crate::mail::signed_in(dash_mail::OWN)?;
        dash_mail::recent(&http, RECENT)
    }

    fn ask(&self, state: Json) -> Result<Json, String> {
        let questions: Json =
            serde_json::from_str(latest::QUESTIONS).expect("the questions are JSON");
        crate::jev::ask(self.0, &questions, state, "site_perso/latest")
    }

    fn older(&self, words: &[String]) -> Result<(), String> {
        let words: Vec<&str> = words.iter().map(String::as_str).collect();
        dash_process::run(&crate::tasks::older_app().bin, &words, None, OLDER_CALL).map(drop)
    }
}

/// How one run goes: a replay of one draft (any status), messages up to a time, nothing written.
#[derive(Default)]
pub(crate) struct Asked<'a> {
    pub draft: Option<&'a str>,
    pub until: Option<&'a str>,
    pub dry: bool,
}

/// The messages, and why the mail is missing when its read failed: the timelines still go on (/code-review of PR 1528).
fn messages(world: &dyn World, older_db: &Path) -> Result<(Vec<Message>, Option<String>), String> {
    let (mail, unread) = match world.mail() {
        Ok(m) => (m, None),
        Err(e) => (vec![], Some(e)),
    };
    let mail = mail.into_iter().map(|b| Message {
        id: format!("mail:{}", b.id),
        channel: "email".into(),
        at: b.at,
        from: b.from,
        to: b.to,
        parties: vec![],
        subject: b.subject,
        preview: b.preview,
    });
    let events = dash_store::older_drafts::timeline_events(older_db)?;
    let events = events
        .into_iter()
        .map(|(lead, id, at, channel, by, text)| Message {
            id,
            channel,
            at,
            // his or an agent's send on the lead, else theirs, by no address
            from: if by.is_empty() {
                String::new()
            } else {
                latest::HIS[0].into()
            },
            to: vec![],
            parties: vec![format!("lead:{lead}")],
            subject: String::new(),
            preview: text,
        });
    Ok((mail.chain(events).collect(), unread))
}

/// One pass: the latest kept, each due draft judged and acted on. Its line, and one line a judgement.
pub(crate) fn pass(
    dir: &Path,
    older_db: &Path,
    world: &dyn World,
    asked: &Asked,
) -> Result<(String, Vec<String>), String> {
    let drafts = dash_store::older_drafts::drafts(older_db, asked.draft)?;
    let waiting: Vec<Waiting> = (drafts.as_array().into_iter().flatten())
        .filter_map(|d| match asked.draft {
            // a replay judges the draft as it waited, whatever it is now
            Some(_) => {
                let mut d = d.clone();
                d["status"] = "pending".into();
                Waiting::of(&d)
            }
            None => Waiting::of(d),
        })
        .collect();
    if asked.draft.is_some() && waiting.is_empty() {
        return Err(format!("no draft {}", asked.draft.unwrap_or_default()));
    }
    let rows = dash_store::older_drafts::parties(older_db)?;
    let parties = latest::parties(&rows, &waiting);
    let (mut all, unread) = messages(world, older_db)?;
    if let Some(until) = asked.until {
        all.retain(|m| m.at.as_str() <= until);
    }
    // a replay never writes: it judges a draft as it stood (review of PR: a replay forced a sent draft moot)
    let dry = asked.dry || asked.draft.is_some() || asked.until.is_some();
    // a dry run on a folder with no store opens none, so creates none
    let log = match dry && !dir.join(dash_drafts::FILE).exists() {
        true => None,
        false => Some(drafts_file(dir)?),
    };
    let mut held = log.as_ref().map_or(Ok(Held::default()), latest::load)?;
    // a replay up to a time never takes a kept latest from after it (/code-review of PR 1528)
    if asked.until.is_some() {
        held.latest.clear();
    }
    let now = latest::newer(&held.latest, latest::latest(&parties, &all));
    let new = match (&log, dry) {
        (Some(log), false) => {
            let mut new = vec![];
            written(log, &mut |db| {
                new = latest::save(db, &held, &now)?;
                Ok(())
            })?;
            new
        }
        _ => vec![],
    };
    let mut held = Held {
        latest: now,
        judged: held.judged,
    };
    if asked.draft.is_some() {
        held.judged.clear();
    }
    let due = latest::due(&waiting, &held);
    let (mut lines, mut counts) = (vec![], BTreeMap::<String, usize>::new());
    for (w, m) in due.iter().take(CALLS) {
        let (line, record) = match judge(world, w, m, dry) {
            Ok((line, record)) => {
                let verdict = record["verdict"].as_str().unwrap_or("kept").to_string();
                *counts.entry(verdict).or_default() += 1;
                (line, record)
            }
            // tried again on the next pass, up to `latest::TRIES` times on this message, then left for a newer one
            Err(e) => {
                *counts.entry("failed".into()).or_default() += 1;
                (
                    format!("{}: not judged: {e}", w.id),
                    latest::failed(&held, &w.id, m, &e),
                )
            }
        };
        lines.push(line);
        if let (Some(log), false) = (&log, dry) {
            // its acts ran: a record that does not land is said, the rest of the pass goes on
            if let Err(e) = written(log, &mut |db| latest::judged(db, &w.id, record.clone())) {
                lines.push(format!("{}: its judgement not kept: {e}", w.id));
            }
        }
    }
    let left = due.len().saturating_sub(CALLS);
    let said: Vec<String> = counts.iter().map(|(k, n)| format!("{n} {k}")).collect();
    let line = format!(
        "{}{} parties hold a latest message, {} new; {} drafts judged{}{}",
        unread.map_or(String::new(), |e| format!(
            "mail not read ({e}), timelines only; "
        )),
        held.latest.len(),
        new.len(),
        due.len().min(CALLS) - counts.get("failed").copied().unwrap_or(0),
        if said.is_empty() {
            String::new()
        } else {
            format!(": {}", said.join(", "))
        },
        if left > 0 {
            format!("; {left} left for the next pass")
        } else {
            String::new()
        },
    );
    Ok((line, lines))
}

/// Jev's answers on `w` and `m`, the verdict and its acts in the older app (none on a dry run): its line and its record.
fn judge(world: &dyn World, w: &Waiting, m: &Message, dry: bool) -> Result<(String, Json), String> {
    let answer = world.ask(latest::state(w, m))?;
    let mut answers = BTreeMap::new();
    // a score Jev left out is no opinion, 0.5 in `latest::verdict`
    for q in ["still_latest", "change", "update", "discard", "answered"] {
        if let Ok(v) = crate::jev::noul(&answer, q) {
            answers.insert(q.to_string(), v);
        }
    }
    let verdict = latest::verdict(&answers);
    let words = latest::acts(w, verdict, m);
    let mut acted = vec![];
    for words in &words {
        let line = words.join(" ");
        match dry {
            true => acted.push(format!("would run dash {line}")),
            // an act that fails fails the judgement, so the draft is judged and acted on again (both acts are idempotent)
            false => match world.older(words) {
                Ok(()) => acted.push(format!("dash {line}")),
                Err(e) => return Err(format!("{verdict:?}: dash {line}: {e}")),
            },
        }
    }
    let name = match verdict {
        Verdict::Moot => "moot",
        Verdict::Update => "update",
        Verdict::Keep => "kept",
    };
    let scores: Vec<String> = answers.iter().map(|(k, v)| format!("{k} {v:.2}")).collect();
    let line = format!(
        "{}: {name} on {} {} ({}); {}",
        w.id,
        m.channel,
        m.at,
        scores.join(", "),
        if acted.is_empty() {
            "nothing to do".into()
        } else {
            acted.join("; ")
        }
    );
    let record = json!({
        "at_ms": crate::tasks::now_ms(), "message": {"id": m.id, "at": m.at, "from": m.from, "subject": m.subject},
        "answers": answers, "verdict": name, "acted": acted, "model": crate::jev::MODEL,
        "input_tokens": crate::jev::input_tokens(&answer)
    });
    Ok((line, record))
}

/// A pass every `watchers::LATEST.every_ms` for as long as `dash orch` runs, its line on /watchers.
pub fn watch(host: &'static dyn dash_core::Host) {
    let sleep = Duration::from_millis(watchers::LATEST.every_ms as u64);
    watchers::run(host, &watchers::LATEST, sleep, |host| {
        let dir = &host.data();
        // the older app's drafts first, so the pass and the plugin's pages read them from drafts.sqlite
        if let Err(e) = import(dir) {
            eprintln!("latest: the drafts' import: {e}");
        }
        let ran = pass(
            dir,
            &crate::tasks::older_db(),
            &Live(host),
            &Asked::default(),
        );
        for l in ran.iter().flat_map(|(_, lines)| lines) {
            eprintln!("latest: {l}");
        }
        let (ok, said) = match &ran {
            Ok((line, _)) => (true, line.clone()),
            Err(e) => (false, format!("latest: {e}")),
        };
        watchers::told(dir, &watchers::LATEST, crate::tasks::now_ms(), ok, &said);
        ran.map(|(line, _)| line)
    });
}

/// One of the older app's reads as JSON: `dash model drafts`, for its outbox facts alone (task rust-outbox-rules ports them).
fn read(args: &[&str]) -> Result<Value, String> {
    let text = dash_process::run(&crate::tasks::older_app().bin, args, None, OLDER_CALL)?;
    serde_json::from_str(&text).map_err(|e| format!("dash {}: {e}", args.join(" ")))
}

/// The older app's drafts, where drafts are written today, read from its SQLite file (task rust-draft-page-cached), Jev's
/// log and the labelled files, as `dash draft import` reads them from an export. Which go now is still `dash model drafts`'s
/// outbox facts, read here alone, off any request: a failed read leaves the list whole (task rust-outbox-rules).
/// ponytail: reads Jev's whole log (43 MB on 2026-11-09) each time, off the routing thread and at most every five minutes;
/// a read from the log's last offset if its time ever shows.
fn exported() -> Result<Vec<Draft>, String> {
    let repo = crate::tasks::repo();
    // its file before the move while it stands, then the log; neither is an error, as a missing log was
    let mut jev: Option<Box<dyn std::io::Read>> = None;
    for p in dash_hooks::outbound::jev_logs(&repo) {
        if p.exists() {
            let file = File::open(&p).map_err(|e| format!("{}: {e}", p.display()))?;
            jev = Some(match jev {
                // a newline between: an old file whose last line has none never joins the log's first (review of PR 1520)
                Some(read) => {
                    let read = std::io::Read::chain(read, &b"\n"[..]);
                    Box::new(std::io::Read::chain(read, file))
                }
                None => Box::new(file),
            });
        }
    }
    let jev = jev.ok_or(format!(
        "{}: no Jev log",
        repo.join(dash_hooks::outbound::JEV_LOG).display()
    ))?;
    let db = crate::tasks::older_db();
    let leads = dash_store::older_drafts::leads(&db, None)?;
    let export = dash_store::older_drafts::drafts(&db, None)?;
    let all = drafts::from_store(&export, &leads, BufReader::new(jev))?;
    Ok(drafts::command::with_labels(all, &repo.join("private/evidence/drafts"))?.0)
}

/// `dash outbox` (task rust-daily-today): the older app's export read now, never a page's last read, since a draft sent a minute
/// ago must not be listed to send again.
pub fn outbox_lines() -> Result<String, String> {
    Ok(drafts::outbox::outbox(&read(&["model", "drafts"])?)?.lines())
}

/// Draft `id`'s page facts as the older app holds them now, read from its SQLite file with no call to the app (task
/// rust-draft-page-cached): whether Claude sends it, what it answers and its state, and each store read that failed; null
/// when it holds none of that id. `postings`, the postings table once read: one read a pass, never one a draft (/code-review).
fn facts(
    db: &Path,
    id: &str,
    postings: &mut Option<Result<Value, String>>,
) -> Result<Value, String> {
    let all = dash_store::older_drafts::drafts(db, Some(id))?;
    let Some(d) = all.as_array().and_then(|a| a.first()) else {
        return Ok(Value::Null);
    };
    // each store read only when the draft names what it finds; its lead alone, by its id
    let lead = d["opportunity"].as_str();
    let postings = (d["posting"].is_string() || lead.is_some())
        .then(|| (postings.get_or_insert_with(|| dash_store::older_drafts::postings(db))).clone());
    let leads = lead.map(|l| dash_store::older_drafts::leads(db, Some(l)));
    let (mut answers, mut unread) = answers(d, postings, leads);
    // the steps it waits on, from the older app's database as its own page read them; unread, the page says it is not
    // known, never « waits on nothing » (/code-review)
    let waits = dash_store::older::waits(db, &format!("drafts:{id}"))
        .map_err(|e| unread.push(format!("waits: {e}")))
        .ok();
    answers.extend(stated(d, drafts::by_claude(d), waits.as_deref()));
    Ok(json!({"by_claude": drafts::by_claude(d), "answers": answers, "unread": unread}))
}

/// Draft `id`'s facts read at `read_ms`, `to_send` its place in the outbox's « now » (false: not now; null: not read): a
/// facts read that failed is named in its `unread`, never the import's failure.
fn facts_at(
    db: &Path,
    id: &str,
    (read_ms, to_send): (i64, Value),
    postings: &mut Option<Result<Value, String>>,
) -> Value {
    let mut f =
        facts(db, id, postings).unwrap_or_else(|e| json!({"unread": [format!("its record: {e}")]}));
    if f.is_null() {
        f = json!({});
    }
    f["read_ms"] = read_ms.into();
    f["to_send"] = to_send;
    f
}

/// The older app's drafts into drafts.sqlite under `dir` (task drafts-genuinely-good, part 3): each with the versions the
/// table kept and its page's facts, its place in the outbox's « now » (its rules the older app's: ready, a day out, banned
/// phrases, approval order) among them; one the export no longer holds is kept, marked `gone`, off the list (data is gold).
/// A draft read again since this import began (`import_one`, his write) keeps its newer read. The latest pass runs it every
/// five minutes. Says how many drafts it wrote.
pub fn import(dir: &Path) -> Result<usize, String> {
    let (db, read_ms) = (crate::tasks::older_db(), crate::tasks::now_ms());
    let mut all = exported()?;
    let now = read(&["model", "drafts"])
        .and_then(|e| drafts::outbox::outbox(&e))
        .map(|o| o.now.into_iter().map(|r| r.id).collect::<Vec<_>>());
    if let Err(e) = &now {
        eprintln!("latest: the outbox did not read, which drafts go now is not known: {e}");
    }
    let mut postings = None;
    for d in &mut all {
        let place = now.as_ref().map_or(Value::Null, |ids| {
            ids.iter()
                .position(|i| *i == d.id)
                .map_or(json!(false), |p| json!(p))
        });
        d.facts = facts_at(&db, &d.id, (read_ms, place), &mut postings);
    }
    let file = drafts_file(dir)?;
    let mut n = 0;
    written(&file, &mut |db| {
        let before = drafts::command::kept(db)?;
        let newer = |id: &str| {
            let at = before
                .drafts
                .get(id)
                .and_then(|k| k.facts["read_ms"].as_i64());
            at.is_some_and(|at| at > read_ms)
        };
        let mut merged: Vec<Draft> = (all.iter().filter(|d| !newer(&d.id)).cloned())
            .map(|d| unread_again(before.drafts.get(&d.id), d))
            .collect();
        // one `import_one` added while this pass read is newer than its snapshot, never gone (/code-review)
        let gone = (before.drafts.values()).filter(|k| {
            !all.iter().any(|d| d.id == k.id) && k.facts["gone"] != true && !newer(&k.id)
        });
        merged.extend(gone.cloned().map(|mut k| {
            k.facts["gone"] = true.into();
            k
        }));
        n = drafts::save(db, &before, &merged)?;
        drafts::latest::passed(db, read_ms)
    })?;
    Ok(n)
}

/// `d` as read now, its kept versions before its own, keeping the kept read's `read_ms` when nothing else changed: the pass's
/// own read time (`drafts::latest::passed`) dates it, so an unchanged pass writes one row, never every draft (/code-review:
/// 288 whole-table changesets a day).
fn unread_again(kept: Option<&Draft>, d: Draft) -> Draft {
    let mut d = drafts::with_kept_versions(kept, d);
    if let Some(k) = kept {
        let at = d.facts["read_ms"].clone();
        d.facts["read_ms"] = k.facts["read_ms"].clone();
        if d != *k {
            d.facts["read_ms"] = at;
        }
    }
    d
}

/// Draft `id` as the older app holds it now into drafts.sqlite under `dir`: a write of his through this app (his edit, a
/// status, a day out) shows on its page at once, never at the next pass. Its place in the outbox stays the last pass's.
pub fn import_one(dir: &Path, id: &str) -> Result<(), String> {
    let db = crate::tasks::older_db();
    let record = dash_store::older_drafts::drafts(&db, Some(id))?;
    let Some(d) = drafts::from_store(&record, &json!({}), &b""[..])?.pop() else {
        return Ok(());
    };
    // its labelled file's sender, receiver and goal, as the pass reads them (/code-review)
    let labelled = drafts::command::with_labels(
        vec![d],
        &crate::tasks::repo().join("private/evidence/drafts"),
    )?
    .0;
    let Some(d) = labelled.into_iter().find(|l| l.id == id) else {
        return Ok(());
    };
    let read_ms = crate::tasks::now_ms();
    written(&drafts_file(dir)?, &mut |tx| {
        let before = drafts::command::kept(tx)?;
        let kept = before.drafts.get(id);
        let place = kept.map_or(Value::Null, |k| k.facts["to_send"].clone());
        let mut d = drafts::with_kept_versions(kept, d.clone());
        // its reply is the pass's, which reads every lead and Jev's log: never lost to a read of one record (/code-review)
        d.reply = kept.and_then(|k| k.reply.clone());
        d.facts = facts_at(&db, id, (read_ms, place), &mut None);
        drafts::save(tx, &before, &[d]).map(drop)
    })
}

/// Draft `id` read again from the older app into the drafts plugin's file of the served data folder, a failure said.
pub fn read_in(dir: &Path, id: &str) {
    let read = import_one(dir, id);
    if let Err(e) = read {
        eprintln!("dash serve: draft {id} not read again: {e}");
    }
}

// a draft the drafts' page misses, read in from the older app's file: one it wrote since the last pass, never a 404 for five
// minutes (/code-review of PR 1662); the page asks by key, the router names no plugin
inventory::submit! { dash_core::ContextProvider { key: "drafts/read-in", lines: |host, id| {
    import_one(&host.data(), id).map(|()| vec![]).inspect_err(|e| eprintln!("dash: draft {id} not read in: {e}"))
} } }

/// `POST /d/<id>/<write>`: his write of a draft the older app answers (his edit, its status).
pub fn a_draft_write(line: &str) -> Option<String> {
    let mut words = line.split_whitespace();
    (words.next() == Some("POST")).then_some(())?;
    let rest = words.next()?.split('?').next()?.strip_prefix("/d/")?;
    let (id, _) = rest.split_once('/')?;
    (!id.is_empty()).then(|| crate::pages::segment_decoded(id))
}

/// The drafts' line's key: first in « today », since drafts to send come first (the task's title: « drafts to send first »).
pub const SLOT: &str = "today/10-drafts";

/// The drafts plugin's line in the daily's « today » (rfc-rust-plugins-ui section 3, the Slot port): how many go now, from
/// drafts.sqlite, as /drafts?show=to-send lists them.
pub fn slot(dir: &Path) -> dash_core::registry::Contribution {
    let dir = dir.to_path_buf();
    let count = move || -> Result<Option<usize>, String> {
        let db = drafts_file(&dir)?;
        let kept: Vec<Draft> = drafts::load(&db)?.drafts.into_values().collect();
        let sends = dash_drafts::card::states(&db)?;
        Ok(drafts::pages::to_send(&kept, &sends).map(|ids| ids.len()))
    };
    let line: Box<dyn dash_core::registry::Slot> = Box::new(drafts::outbox::ToSend(count));
    dash_core::registry::Contribution {
        slots: vec![(SLOT.into(), line)],
        ..Default::default()
    }
}

/// The days its decisions were made, as State lists them, in the older page's order.
const DATED: [(&str, &str); 4] = [
    (
        "approved-pending-changes-on",
        "Approved with his corrections",
    ),
    ("approved-on", "Approved"),
    ("rejected-on", "Rejected"),
    ("sent-on", "Sent"),
];

/// What Why this text, State and Waits show of draft record `d` (task rust-draft-state-edit), as the older page read it:
/// `waits`, the steps it waits on still open; `None`, not read.
fn stated(d: &Value, by_claude: bool, waits: Option<&[String]>) -> Map<String, Value> {
    let text = |k: &str| d[k].as_str().unwrap_or_default().trim().to_string();
    let why: Vec<&str> = match &d["context"] {
        Value::Array(all) => all.iter().filter_map(Value::as_str).collect(),
        one => one.as_str().into_iter().collect(),
    };
    let why: Vec<&str> = why
        .into_iter()
        .map(str::trim)
        .filter(|w| !w.is_empty())
        .collect();
    let dates: Vec<Value> = (DATED.iter())
        .filter_map(|(k, label)| Some(json!({"label": label, "value": d[*k].as_str()?})))
        .collect();
    let waits_unread = waits.is_none();
    let waits: Vec<Value> = (waits.unwrap_or_default().iter())
        .map(|w| json!({"label": w, "value": w, "href": step_href(w)}))
        .collect();
    let meanwhile = d["waits-on"]["in-flight"].as_str().unwrap_or_default();
    let props = json!({
        "stated": true, "id": text("id"), "copy": !by_claude, "why": why, "where": text("where"),
        "bold": drafts::bold(d, by_claude), "dates": dates, "send_on": text("send-on"), "waits": waits, "meanwhile": meanwhile,
        "waits_unread": waits_unread, "keep_bold": d["channel"] == "form" && d["form-bold"] == true,
    });
    props.as_object().cloned().unwrap_or_default()
}

/// A step's page, as the older app's `Steps.href/1`: a draft, a lead, a task; else none.
fn step_href(step: &str) -> Option<String> {
    let (table, id) = step.split_once(':')?;
    match table {
        "drafts" => Some(drafts::list::href(id)),
        "leads" => Some(format!("/o/{}", dash_comments::segment(id))),
        "tasks" => Some(dash_surface::task_href(id, None)),
        _ => None,
    }
}

/// `POST /drafts/<id>/set`: State's Save on draft `id`'s page.
pub fn set_posted(line: &str) -> Option<String> {
    let mut words = line.split_whitespace();
    (words.next() == Some("POST")).then_some(())?;
    let id = words
        .next()?
        .split('?')
        .next()?
        .strip_prefix("/drafts/")?
        .strip_suffix("/set")?;
    let id = crate::pages::segment_decoded(id);
    (!id.is_empty() && !id.contains('/') && !id.starts_with('-')).then_some(id)
}

/// Draft `id` goes out on `day` (an ISO date; empty: with the next outbox), written through the older app's `dash draft set
/// <id> --send-on <day>`, which refuses a draft already closed. Err: the status to answer and why.
pub fn send_on(id: &str, day: &str) -> Result<(), (&'static str, String)> {
    let day = day.trim();
    if !day.is_empty() && !(day.len() == 10 && dash_core::days(day).is_some()) {
        let why = format!("{day}: a day is written 2026-11-15, or left empty");
        return Err(("400 Bad Request", why));
    }
    let args = ["draft", "set", id, "--send-on", day];
    dash_process::run(&crate::tasks::older_app().bin, &args, None, OLDER_CALL)
        .map(drop)
        .map_err(|e| ("502 Bad Gateway", e))
}

/// What a draft answers, as its template's props, and each store read that failed, named: the ad from its archive, its
/// Application form and its lead's last events, its search note among them (task rust-draft-posting).
type Answers = (Map<String, Value>, Vec<String>);

/// `draft`'s ad, form and lead's events from `postings` and `leads` (`dash store files postings|opportunities`), `None` where
/// not read. A read that failed leaves its own parts out alone: the ad shows with no lead, the lead with no ad (/code-review).
fn answers(
    draft: &Value,
    postings: Option<Result<Value, String>>,
    leads: Option<Result<Value, String>>,
) -> Answers {
    let mut unread = vec![];
    let mut got = |read: Option<Result<Value, String>>, what: &str| match read? {
        Ok(v) => Some(v),
        Err(e) => {
            unread.push(format!("{what}: {e}"));
            None
        }
    };
    let (postings, leads) = (got(postings, "postings"), got(leads, "leads"));
    let lead =
        (leads.as_ref()).and_then(|l| drafts::posting::lead(l, draft["opportunity"].as_str()?));
    let mut props = Map::new();
    let ad = postings
        .as_ref()
        .and_then(|p| drafts::posting::archive_of(draft, p));
    if let Some(p) = ad.map(drafts::posting::parse) {
        let mut facts: Vec<Value> = (p.facts.iter())
            .map(|(label, value)| json!({"label": label, "value": value}))
            .collect();
        facts.extend(
            p.url
                .map(|u| json!({"label": "Source", "value": u, "href": u})),
        );
        props.insert("posting".into(), facts.into());
        props.insert("ad".into(), p.text.into());
    }
    props.insert(
        "apply".into(),
        drafts::posting::apply(draft, lead.as_ref()).into(),
    );
    let today = drafts::date(crate::tasks::now_ms().div_euclid(86_400_000));
    let recent = (lead.as_ref()).map_or(vec![], |l| drafts::posting::recent(l, &today, 3));
    let recent: Vec<Value> = (recent.into_iter())
        .map(|(day, words, source)| json!({"label": day, "value": words, "href": source}))
        .collect();
    props.insert("recent".into(), recent.into());
    (props, unread)
}

const USAGE: &str = "dash latest --data <dir> [--draft <id>] [--until <ISO time>] [--dry-run] (each contact's and lead's latest message kept, each waiting draft older than it judged by Jev and acted on; --draft replays one draft whatever its status, --until reads the messages up to then, a replay always dry, --dry-run writes and acts on nothing)";

fn command(host: &dyn dash_core::Host, args: &[&str], out: &mut dyn std::io::Write) {
    let usage = || -> ! {
        eprintln!("usage: {USAGE}");
        std::process::exit(2);
    };
    let mut asked = Asked::default();
    let mut words = args.iter();
    while let Some(w) = words.next() {
        match *w {
            "--draft" => asked.draft = Some(*words.next().unwrap_or_else(|| usage())),
            "--until" => asked.until = Some(*words.next().unwrap_or_else(|| usage())),
            "--dry-run" => asked.dry = true,
            _ => usage(),
        }
    }
    // the folder `--data` names, read once by the binary: none named is the usage
    let dir = &host.switched().unwrap_or_else(|| usage());
    host.arm(dir, "latest");
    match pass(dir, &crate::tasks::older_db(), &Live(host), &asked) {
        Ok((line, lines)) => {
            for l in lines.iter().chain([&line]) {
                writeln!(out, "{l}").unwrap_or_default();
            }
        }
        Err(e) => {
            eprintln!("dash latest: {e}");
            std::process::exit(1);
        }
    }
}

inventory::submit! {
    crate::usage::Command { usage: USAGE, words: &["latest"], run: Some(command) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::{TestDir, dash_test};
    use std::cell::RefCell;

    /// An unchanged pass keeps a draft's kept read time, so `drafts::save` writes nothing; any other change takes the new one.
    #[dash_test]
    fn an_unchanged_draft_keeps_its_read_time_and_a_changed_one_takes_the_new() {
        let at = |status: &str, ms: i64| {
            let j = json!({"id": "d", "status": status, "versions": [], "facts": {"read_ms": ms, "to_send": false}});
            drafts::from_json(&j).expect("a draft")
        };
        let kept = at("pending", 1);
        assert_eq!(unread_again(Some(&kept), at("pending", 2)), kept);
        assert_eq!(
            unread_again(Some(&kept), at("approved", 2)),
            at("approved", 2)
        );
        assert_eq!(unread_again(None, at("pending", 2)), at("pending", 2));
    }

    /// The older app's tables, as much of them as a pass reads.
    const OLDER: &str = "
      CREATE TABLE drafts (collection TEXT, pos INTEGER, key TEXT, id TEXT, stage TEXT, data TEXT NOT NULL, body TEXT,
        \"to\" TEXT, channel TEXT, created TEXT, opportunity TEXT, cold INTEGER);
      CREATE TABLE leads (collection TEXT, pos INTEGER, key TEXT, id TEXT, stage TEXT, data TEXT NOT NULL, body TEXT,
        title TEXT);
      CREATE TABLE links (\"from\" TEXT, kind TEXT, \"to\" TEXT, pos INTEGER DEFAULT 0 NOT NULL);
      CREATE TABLE contacts (id TEXT, name TEXT, email TEXT);
      CREATE TABLE companies (id TEXT, name TEXT);
      CREATE TABLE events (source TEXT, pos INTEGER, key TEXT, about TEXT, at TEXT, data TEXT, body TEXT);";

    /// A stand-in world: its mail, Jev's answers by draft, and each older-app call it was asked.
    struct Stand {
        mail: Vec<dash_mail::Brief>,
        answers: BTreeMap<&'static str, [f64; 5]>,
        asked: RefCell<Vec<String>>,
        older: RefCell<Vec<String>>,
        /// The older app refuses every write.
        refuses: bool,
    }

    impl World for Stand {
        fn mail(&self) -> Result<Vec<dash_mail::Brief>, String> {
            Ok(self.mail.clone())
        }

        fn ask(&self, state: Json) -> Result<Json, String> {
            let text = state["draft"]["text"]
                .as_str()
                .unwrap_or_default()
                .to_string();
            self.asked.borrow_mut().push(text.clone());
            let (_, a) = (self.answers.iter())
                .find(|(k, _)| text.contains(*k))
                .ok_or("no answer for this draft")?;
            let names = ["still_latest", "change", "update", "discard", "answered"];
            let answers: serde_json::Map<String, Json> = (names.iter().zip(a))
                .map(|(n, v)| (n.to_string(), json!({"noul": v})))
                .collect();
            Ok(json!({"answers": answers, "usage": {"input_tokens": 900}}))
        }

        fn older(&self, words: &[String]) -> Result<(), String> {
            self.older.borrow_mut().push(words.join(" "));
            match self.refuses {
                true => Err("the older app answered 502".into()),
                false => Ok(()),
            }
        }
    }

    fn brief(id: &str, at: &str, from: &str, to: &str, preview: &str) -> dash_mail::Brief {
        dash_mail::Brief {
            id: id.into(),
            thread: "t".into(),
            at: at.into(),
            from: from.into(),
            to: vec![to.into()],
            subject: "Re: Grant application".into(),
            preview: preview.into(),
        }
    }

    /// The Hostman case, its names and ID changed (this crate is mirrored in public): a pending draft promising to send the
    /// account's ID once made; his own mail sent that ID at 10:16, then their answer at 10:31. Another draft on a lead, its
    /// LinkedIn timeline newer than it; a third written after its latest.
    fn older(dir: &Path) -> std::path::PathBuf {
        let db = dir.join("older.db");
        let conn = dash_store::Connection::open(&db).expect("db");
        conn.execute_batch(OLDER).expect("tables");
        conn.execute_batch(
            "INSERT INTO contacts VALUES ('ana', 'Ana B.', 'ana@acme.example');
             INSERT INTO links VALUES ('leads:acme-lead', 'contact', 'contacts:ana', 0);
             INSERT INTO events VALUES ('timeline:acme-lead', 0, NULL, NULL, '2026-11-10',
               '{\"date\":\"2026-11-10\",\"channel\":\"~:linkedin\",\"event\":\"Ana asks for a call on Thursday\"}', NULL);
             INSERT INTO events VALUES ('timeline:acme-lead', 1, NULL, NULL, '2026-11-11',
               '{\"date\":\"2026-11-11\",\"channel\":\"~:other\",\"by\":\"~:claude\",\"event\":\"sweep note\"}', NULL);",
        )
        .expect("rows");
        // each field a marker of its column, as the older app stores a draft
        let col = dash_store::older::marked("⟦col⟧");
        let data = json!({"id": col, "status": dash_store::older::marked("⟦stage⟧"), "created": col, "to": col, "opportunity": col,
                          "context": "Goal: get the test account topped up, then the grant."})
        .to_string();
        let sql = "INSERT INTO drafts (collection, pos, key, id, stage, data, body, \"to\", channel, created, opportunity)
                   VALUES ('drafts', ?1, ?2, ?2, ?3, ?4, ?5, ?6, '~:email', ?7, ?8)";
        for (pos, id, stage, to, created, opp, body) in [
            (
                0,
                "grant-test-account-reply",
                "pending",
                "Sam B. (Example cloud grants), sam.b@cloud.example",
                "2026-11-09",
                None,
                "Thank you, the test account works for me. I will create the account and send you its ID.",
            ),
            (
                1,
                "acme-intro",
                "approved",
                "Ana B. (Acme), ana@acme.example",
                "2026-11-09",
                Some("acme-lead"),
                "Hello Ana, here is my CV.",
            ),
            (
                2,
                "fresh-reply",
                "pending",
                "Sam B., sam.b@cloud.example",
                "2026-11-12",
                None,
                "A reply written today.",
            ),
            (
                3,
                "old-sent",
                "sent",
                "Sam B., sam.b@cloud.example",
                "2026-11-01",
                None,
                "Sent long ago.",
            ),
        ] {
            conn.execute(sql, (pos, id, stage, &data, body, to, created, opp))
                .expect("draft");
        }
        db
    }

    fn stand() -> Stand {
        Stand {
            mail: vec![
                brief(
                    "m0",
                    "2026-11-08T09:00:00Z",
                    "ana@acme.example",
                    "nicolas@klar.im",
                    "Thanks for applying.",
                ),
                brief(
                    "m1",
                    "2026-11-09T06:46:00Z",
                    "sam.b@cloud.example",
                    "nicolas@klar.im",
                    "I can open the testing account with EUR 30. Create the account, send me its ID and I will top it.",
                ),
                brief(
                    "m2",
                    "2026-11-09T10:16:00Z",
                    "nicolas@klar.im",
                    "sam.b@cloud.example",
                    "That makes sense. My account ID is ab123456.",
                ),
                brief(
                    "m3",
                    "2026-11-09T10:31:00Z",
                    "sam.b@cloud.example",
                    "nicolas@klar.im",
                    "Found the account. May I ask to create a business type instead of personal?",
                ),
            ],
            // still_latest, change, update, discard, answered
            answers: BTreeMap::from([
                ("send you its ID", [0.1, 0.9, 0.6, 0.92, 0.85]),
                ("my CV", [0.3, 0.8, 0.8, 0.2, 0.1]),
            ]),
            asked: RefCell::new(vec![]),
            older: RefCell::new(vec![]),
            refuses: false,
        }
    }

    /// Done-when 1: each contact and lead holds its latest message from every channel pulled (mail, then the leads'
    /// timelines), and each address a waiting draft writes to; a second pass with nothing new writes no latest again.
    #[dash_test]
    fn each_contact_and_lead_holds_its_latest_message_from_every_channel_pulled() {
        let dir = TestDir::new("latest-holds");
        let db = older(&dir.0);
        let world = Stand {
            answers: BTreeMap::new(),
            ..stand()
        };
        let (line, _) = pass(
            &dir.0,
            &db,
            &world,
            &Asked {
                dry: false,
                ..Default::default()
            },
        )
        .expect("a pass");
        assert!(
            line.starts_with("4 parties hold a latest message, 4 new"),
            "{line}"
        );
        let held = latest::load(&drafts_file(&dir.0).expect("log")).expect("held");
        assert_eq!(
            held.latest["address:sam.b@cloud.example"].id, "mail:m3",
            "the newest, theirs"
        );
        assert_eq!(
            held.latest["contact:ana"].id, "mail:m0",
            "a contact by its address"
        );
        assert_eq!(
            held.latest["lead:acme-lead"].id, "timeline:acme-lead:0",
            "its LinkedIn event, newer than its contact's mail"
        );
        assert!(
            !held.latest.contains_key("timeline:acme-lead:1"),
            "a sweep note is no message"
        );
        let again = pass(&dir.0, &db, &world, &Asked::default())
            .expect("again")
            .0;
        assert!(
            again.starts_with("4 parties hold a latest message, 0 new"),
            "{again}"
        );
    }

    /// Done-when 2, 3 and the Hostman case replayed with a stand-in Jev: a new latest asks Jev the five questions of each
    /// waiting draft older than it; the draft whose promise his own mail kept goes moot with no word from him, the approved
    /// one goes back to pending flagged for update, the one written after its latest is never asked, the sent one never;
    /// each judgement is kept with Jev's answers, and the next pass asks nothing again.
    #[dash_test]
    fn a_new_latest_asks_jev_and_each_verdict_acts_and_is_logged() {
        let dir = TestDir::new("latest-acts");
        let db = older(&dir.0);
        let world = stand();
        let (line, lines) = pass(&dir.0, &db, &world, &Asked::default()).expect("a pass");
        assert!(
            line.ends_with("2 drafts judged: 1 moot, 1 update"),
            "{line}"
        );
        assert_eq!(
            world.asked.borrow().len(),
            2,
            "fresh-reply and old-sent never asked"
        );
        assert_eq!(
            *world.older.borrow(),
            [
                "draft status acme-intro pending",
                "draft set acme-intro --why.update rewrite for the latest message, Jev's verdict: linkedin on 2026-11-10: \
                 their « Ana asks for a call on Thursday » (timeline:acme-lead:0)",
                "draft status grant-test-account-reply moot",
            ]
        );
        assert!(
            lines[1].starts_with("grant-test-account-reply: moot on email 2026-11-09T10:31:00Z"),
            "{lines:?}"
        );
        let held = latest::load(&drafts_file(&dir.0).expect("log")).expect("held");
        let kept = &held.judged["grant-test-account-reply"];
        assert_eq!(
            (
                kept["verdict"].as_str(),
                kept["answers"]["discard"].as_f64()
            ),
            (Some("moot"), Some(0.92))
        );
        assert_eq!(kept["message"]["id"], "mail:m3");
        pass(&dir.0, &db, &world, &Asked::default()).expect("again");
        assert_eq!(world.asked.borrow().len(), 2, "judged once on its latest");
    }

    /// The replay: one draft, whatever its status, judged on the messages up to a time, nothing written nor acted on.
    #[dash_test]
    fn a_dry_replay_judges_one_draft_up_to_a_time_and_writes_nothing() {
        let dir = TestDir::new("latest-replay");
        let db = older(&dir.0);
        let world = stand();
        // no --dry-run: a replay never writes all the same (/code-review)
        let asked = Asked {
            draft: Some("grant-test-account-reply"),
            until: Some("2026-11-09T10:20:00Z"),
            dry: false,
        };
        let (_, lines) = pass(&dir.0, &db, &world, &asked).expect("a replay");
        assert!(
            lines[0].contains("moot on email 2026-11-09T10:16:00Z"),
            "{lines:?}"
        );
        assert!(
            lines[0].ends_with("would run dash draft status grant-test-account-reply moot"),
            "{lines:?}"
        );
        assert!(world.older.borrow().is_empty());
        assert!(!dir.0.join("drafts.sqlite").exists(), "no store made");
        assert_eq!(
            latest::load(&drafts_file(&dir.0).expect("log")).expect("held"),
            Held::default()
        );
        let asked = Asked {
            draft: Some("nothing-here"),
            ..asked
        };
        assert_eq!(
            pass(&dir.0, &db, &world, &asked).map(drop),
            Err("no draft nothing-here".into())
        );
    }

    /// An act the older app refuses keeps no verdict: the draft is judged and acted on again at the next pass (/code-review).
    #[dash_test]
    fn a_refused_act_is_tried_again_at_the_next_pass() {
        let dir = TestDir::new("latest-refused");
        let db = older(&dir.0);
        let world = Stand {
            refuses: true,
            ..stand()
        };
        let (line, lines) = pass(&dir.0, &db, &world, &Asked::default()).expect("a pass");
        assert!(
            line.ends_with("0 drafts judged: 2 failed"),
            "a failed call is no judgement: {line}"
        );
        assert!(lines.iter().any(|l| l.contains("not judged: Moot: dash draft status grant-test-account-reply moot: the older app answered 502")), "{lines:?}");
        pass(&dir.0, &db, &world, &Asked::default()).expect("again");
        assert_eq!(world.asked.borrow().len(), 4, "asked again");
    }
}
