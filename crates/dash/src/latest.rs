//! The latest-message pass (task rust-latest-reevaluates, the drafts plugin's `latest`): his mail over JMAP and the leads'
//! timelines give each contact, lead and draft address its latest message, kept on the drafts' op log; each waiting draft
//! older than its latest gets Jev's five questions, and the verdict acts through the older app (`dash draft status|set`),
//! the answers kept beside it. `dash orch` runs it every 5 minutes (`watchers::LATEST`); `dash latest` runs it once.
use crate::watchers;
use dash_drafts::latest::{self, Held, Message, Verdict, Waiting};
use serde_json::{Value as Json, json};
use std::collections::BTreeMap;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The newest mails read a pass: a person's latest is among the last 200 mails of every folder, or older than any waiting
/// draft's day it would re-judge.
const RECENT: u64 = 200;
/// Jev's calls a pass, at most: a first pass over a full store never runs up a bill, the rest go at the next.
const CALLS: usize = 10;
const OLDER_CALL: Duration = Duration::from_secs(60);

/// What a pass reads and writes outside its data folder: the mail, Jev, the older app's draft writes.
pub(crate) trait World {
    fn mail(&self) -> Result<Vec<dash_mail::Brief>, String>;
    fn ask(&self, state: Json) -> Result<Json, String>;
    fn older(&self, words: &[String]) -> Result<(), String>;
}

struct Live;

impl World for Live {
    fn mail(&self) -> Result<Vec<dash_mail::Brief>, String> {
        let http = crate::mail::signed_in(dash_mail::OWN)?;
        dash_mail::recent(&http, RECENT)
    }

    fn ask(&self, state: Json) -> Result<Json, String> {
        let questions: Json =
            serde_json::from_str(latest::QUESTIONS).expect("the questions are JSON");
        crate::jev::ask(&questions, state, "site_perso/latest")
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
    let mut log = match dry && !dir.join("drafts.sqlite").exists() {
        true => None,
        false => Some(crate::drafts_log::log(dir)?),
    };
    let mut held = log
        .as_ref()
        .map_or(Ok(Held::default()), |l| latest::load(l))?;
    // a replay up to a time never takes a kept latest from after it (/code-review of PR 1528)
    if asked.until.is_some() {
        held.latest.clear();
    }
    let now = latest::newer(&held.latest, latest::latest(&parties, &all));
    let new = match (&mut log, dry) {
        (Some(log), false) => latest::save(log, &held, &now)?,
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
        if let (Some(log), false) = (&mut log, dry) {
            // its acts ran: a record that does not land is said, the rest of the pass goes on
            if let Err(e) = latest::judged(log, &w.id, record) {
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
pub fn watch(dir: PathBuf) {
    let sleep = Duration::from_millis(watchers::LATEST.every_ms as u64);
    watchers::run(dir, &watchers::LATEST, sleep, |dir| {
        let ran = pass(dir, &crate::tasks::older_db(), &Live, &Asked::default());
        for l in ran.iter().flat_map(|(_, lines)| lines) {
            eprintln!("latest: {l}");
        }
        let (ok, said) = match &ran {
            Ok((line, _)) => (true, line.clone()),
            Err(e) => (false, format!("latest: {e}")),
        };
        if let Err(e) = watchers::beat(dir, &watchers::LATEST, crate::tasks::now_ms(), ok, &said) {
            eprintln!("latest: the beat: {e}");
        }
        ran.map(|(line, _)| line)
    });
}

const USAGE: &str = "dash latest --data <dir> [--draft <id>] [--until <ISO time>] [--dry-run] (each contact's and lead's latest message kept, each waiting draft older than it judged by Jev and acted on; --draft replays one draft whatever its status, --until reads the messages up to then, a replay always dry, --dry-run writes and acts on nothing)";

fn command(args: &[&str], mut out: File) {
    let usage = || -> ! {
        eprintln!("usage: {USAGE}");
        std::process::exit(2);
    };
    let (mut dir, mut asked) = (None, Asked::default());
    let mut words = args.iter();
    while let Some(w) = words.next() {
        match *w {
            "--data" => dir = Some(*words.next().unwrap_or_else(|| usage())),
            "--draft" => asked.draft = Some(*words.next().unwrap_or_else(|| usage())),
            "--until" => asked.until = Some(*words.next().unwrap_or_else(|| usage())),
            "--dry-run" => asked.dry = true,
            _ => usage(),
        }
    }
    let dir = crate::reporting::armed(dir.unwrap_or_else(|| usage()), "latest");
    match pass(dir, &crate::tasks::older_db(), &Live, &asked) {
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
    fn older(dir: &Path) -> PathBuf {
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
        let col = dash_store::older::COL;
        let data = json!({"id": col, "status": dash_store::older::STAGE, "created": col, "to": col, "opportunity": col,
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
        let held = latest::load(&crate::drafts_log::log(&dir.0).expect("log")).expect("held");
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
        let held = latest::load(&crate::drafts_log::log(&dir.0).expect("log")).expect("held");
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
            latest::load(&crate::drafts_log::log(&dir.0).expect("log")).expect("held"),
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
