//! A lead's chances judged as it lands (task rust-judge-on-arrival; @nha, 2026-11-09: « should not be used too much in
//! practice. judging is cheap. and fast. »): `dash orch` judges every lead of the older app's store each
//! `watchers::CHANCES.every_ms` and keeps each score in `chances.sqlite`, so a new lead is judged within that time and the
//! reading family's Judging state (/design) is brief. The judge is `Fields.odds/3` ported: base 5, then the ad's age,
//! the share of the skills asked he has, our rate against theirs, a human who answered, a named contact, full remote,
//! three days on site, the screening questions answered, kept between 0 and 10. Every lead is judged again each pass, so
//! a score follows its lead and the days going by (163 leads, a few milliseconds); only a change is written.
//! ponytail: `raise` (what would raise them) is not ported: no Rust page reads it yet.
use crate::automations::local_now;
use crate::tasks::now_ms;
use crate::watchers;
use dash_store::older::{self, Lead, Piece};
use serde_json::{Map, Value};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS chances (id TEXT PRIMARY KEY, score INTEGER NOT NULL, parts TEXT NOT NULL, \
                      judged_ms INTEGER NOT NULL) WITHOUT ROWID;";

/// What every lead's judgement reads besides the lead: profile.edn's contract and skills.edn.
#[derive(Default)]
pub struct Ctx {
    /// The day rate we ask, profile.edn's `:day-rate :target`, unless the lead names its own `:our-ask`.
    asked: Option<f64>,
    eur_per: Map<String, Value>,
    have: Map<String, Value>,
    partial: Map<String, Value>,
    alias: Map<String, Value>,
}

/// A source's map, written as an object or as `[key, value]` pairs (skills.edn's `:have`, `:alias`).
fn map_of(v: Option<&Value>) -> Map<String, Value> {
    match v {
        Some(Value::Object(m)) => m.clone(),
        Some(Value::Array(pairs)) => pairs
            .iter()
            .filter_map(|p| Some((p.get(0)?.as_str()?.to_string(), p.get(1)?.clone())))
            .collect(),
        _ => Map::new(),
    }
}

impl Ctx {
    fn read(db: &Path) -> Result<Ctx, String> {
        let source = |name| -> Result<Map<String, Value>, String> {
            let rows = older::records(db, name)?;
            Ok(rows
                .into_iter()
                .map(|(k, d)| (k, serde_json::from_str(&d).unwrap_or(Value::Null)))
                .collect())
        };
        let (profile, skills) = (source("profile.edn")?, source("skills.edn")?);
        let contract = profile.get("contract");
        Ok(Ctx {
            asked: contract
                .and_then(|c| c.pointer("/day-rate/target"))
                .and_then(Value::as_f64),
            eur_per: map_of(contract.and_then(|c| c.get("eur-per"))),
            have: map_of(skills.get("have")),
            partial: map_of(skills.get("partial")),
            alias: map_of(skills.get("alias")),
        })
    }

    /// How a word of the ad stands against skills.edn: have, partial or none (`Fields.skill_level/2`).
    fn level(&self, word: &Value) -> &'static str {
        let w = match word {
            Value::Object(o) => o.get("name").map(text).unwrap_or_default(),
            w => text(w),
        };
        // a mention the older store keeps for a name (« m:companies:docker » between its markers) is matched by its id, the
        // name's slug. ponytail: a name whose slug is not its lowercase (« Node JS », id node-js) matches none; read the
        // row's name then
        let id = match older::pieces(&w)[..] {
            [Piece::Mention(m)] => m.split_once(':').map(|(_, id)| id.to_string()),
            _ => None,
        };
        let w = id.unwrap_or(w);
        let w = w.trim().to_lowercase();
        let k = self.alias.get(&w).and_then(Value::as_str).unwrap_or(&w);
        if self.have.contains_key(k) {
            "have"
        } else if self.partial.contains_key(k) {
            "partial"
        } else {
            "none"
        }
    }
}

fn text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        v => v.to_string(),
    }
}

/// A lead's chances out of 10 and each point's reason, as `Fields.odds/3` gives them (its `parts`), on day `today`.
pub fn judge(lead: &Lead, ctx: &Ctx, today: i64) -> (i64, Vec<(i64, String)>) {
    let data: Value = serde_json::from_str(&lead.data).unwrap_or(Value::Null);
    let mut parts: Vec<(i64, String)> = vec![];
    let mut part = |p: i64, why: String| parts.push((p, why));
    // the ad's age
    match lead.posted.as_deref().and_then(dash_core::days) {
        Some(d) if today - d <= 2 => part(2, "ad under 48 h old".into()),
        Some(d) if today - d < 8 => part(0, format!("ad {} days old", today - d)),
        Some(d) => part(
            -2,
            format!("ad {} days old, the pipeline is already running", today - d),
        ),
        None => {}
    }
    // the share of the skills asked he has: as Claude read the ad, else its stack guessed against skills.edn
    let levels: Vec<&str> = match data.get("requirements") {
        Some(Value::Array(rs)) if !rs.is_empty() => rs
            .iter()
            .filter(|r| r.is_object())
            .map(|r| r.get("level").and_then(Value::as_str).unwrap_or("none"))
            .collect(),
        _ => match data.get("skills-required") {
            Some(Value::Array(ws)) => ws.iter().map(|w| ctx.level(w)).collect(),
            Some(Value::Null) | None => vec![],
            Some(w) => vec![ctx.level(w)],
        },
    };
    if !levels.is_empty() {
        let o = levels.iter().filter(|l| **l == "have").count() as f64 / levels.len() as f64;
        let p = (o * 100.0).trunc() as i64;
        match o {
            o if o >= 0.6 => part(2, format!("{p} % of the skills asked are his")),
            o if o > 0.3 => part(0, format!("{p} % of the skills asked")),
            _ => part(-2, format!("only {p} % of the skills asked")),
        }
    }
    // our rate against theirs, in whole EUR a day; an hourly or yearly rate is no day rate
    // `get_in(lead, [our-ask, target]) || target`: only nil or false falls back, and any other non-number asks no figure
    let asked = match data.pointer("/our-ask/target") {
        None | Some(Value::Null | Value::Bool(false)) => ctx.asked,
        Some(v) => v.as_f64(),
    };
    let rate = data.get("rate").filter(|r| r.is_object());
    let daily = rate
        .is_some_and(|r| matches!(r.get("unit"), None | Some(Value::Null)) || r["unit"] == "day");
    let fx = rate
        .filter(|_| daily)
        .and_then(|r| {
            ctx.eur_per
                .get(r.get("currency").and_then(Value::as_str).unwrap_or("EUR"))
        })
        .and_then(Value::as_f64);
    let eur = |k: &str| Some((rate?.get(k)?.as_f64()? * fx?).round() as i64);
    let (min, max) = (eur("min"), eur("max"));
    if let (Some(m), Some(a)) = (max, asked)
        && a > m as f64
    {
        part(
            -2,
            format!("we ask {}, above their stated maximum ({m})", num(a)),
        );
    }
    if let (Some(lo), Some(hi)) = (min, max)
        && lo == hi
        && asked != Some(lo as f64)
    {
        part(
            -1,
            format!("their day rate is fixed ({lo}): any other figure ends it"),
        );
    }
    if lead.human {
        part(
            2,
            "a human already answered: no longer an anonymous application".into(),
        );
    }
    if data
        .get("contacts")
        .and_then(Value::as_array)
        .is_some_and(|c| !c.is_empty())
    {
        part(1, "we have a named contact".into());
    }
    if lead.remote.as_deref() == Some("full") {
        part(-1, "full remote: the whole market applies".into());
    }
    let onsite = lead.onsite.as_deref().unwrap_or_default().to_lowercase();
    if ["3 jours", "trois jours", "3 days", "three days"]
        .iter()
        .any(|w| onsite.contains(w))
    {
        part(1, "three days on site: many candidates drop out".into());
    }
    if lead.answered {
        part(1, "the screening questions have answers".into());
    }
    let score = (5 + parts.iter().map(|p| p.0).sum::<i64>()).clamp(0, 10);
    (score, parts)
}

/// A figure as Elixir prints it: 600, or 612.5.
fn num(a: f64) -> String {
    if a.fract() == 0.0 {
        format!("{}", a as i64)
    } else {
        format!("{a}")
    }
}

fn open(dir: &Path) -> Result<dash_store::Connection, String> {
    let db = dash_store::connect(&dir.join("chances.sqlite")).map_err(|e| e.to_string())?;
    db.execute_batch(SCHEMA).map_err(|e| e.to_string())?;
    Ok(db)
}

/// One pass: every lead of `older_db` judged on day `today` at `now_ms`, a changed score written, a lead gone dropped. Says
/// how many it judged and how many it judged for the first time.
pub fn pass(dir: &Path, older_db: &Path, today: i64, now_ms: i64) -> Result<String, String> {
    let leads = older::leads(older_db)?;
    let ctx = Ctx::read(older_db)?;
    let mut db = open(dir)?;
    let tx = db.transaction().map_err(|e| e.to_string())?;
    let mut new = 0;
    {
        let had: HashSet<String> = tx
            .prepare("SELECT id FROM chances")
            .and_then(|mut q| q.query_map([], |r| r.get(0))?.collect())
            .map_err(|e| e.to_string())?;
        let mut put = tx
            .prepare(
                "INSERT INTO chances (id, score, parts, judged_ms) VALUES (?1, ?2, ?3, ?4) ON CONFLICT(id) DO UPDATE \
                 SET score = ?2, parts = ?3, judged_ms = ?4 WHERE score IS NOT ?2 OR parts IS NOT ?3",
            )
            .map_err(|e| e.to_string())?;
        for lead in &leads {
            let (score, parts) = judge(lead, &ctx, today);
            let parts = serde_json::to_string(&parts).map_err(|e| e.to_string())?;
            put.execute((&lead.id, score, &parts, now_ms))
                .map_err(|e| e.to_string())?;
            new += usize::from(!had.contains(&lead.id));
        }
        let ids: HashSet<&str> = leads.iter().map(|l| l.id.as_str()).collect();
        for gone in had.iter().filter(|id| !ids.contains(id.as_str())) {
            tx.execute("DELETE FROM chances WHERE id = ?1", [gone])
                .map_err(|e| e.to_string())?;
        }
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(format!("{} leads judged, {new} new", leads.len()))
}

inventory::submit! { crate::usage::Command::usage("dash chances --data <dir>") }
/// Each lead's chances as the last pass judged them, one per line: id, score, then each point and its reason. What an agent
/// reads (`dash chances --data <dir>`); the lead page reads the same table once it moves to Rust.
pub fn list(dir: &Path, mut out: impl std::io::Write) -> Result<(), String> {
    let db = open(dir)?;
    let mut q = db
        .prepare("SELECT id, score, parts FROM chances ORDER BY id")
        .map_err(|e| e.to_string())?;
    let rows = q
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .and_then(Iterator::collect::<Result<Vec<_>, _>>)
        .map_err(|e| e.to_string())?;
    for (id, score, parts) in rows {
        let parts: Vec<(i64, String)> = serde_json::from_str(&parts).map_err(|e| e.to_string())?;
        let why: Vec<String> = parts.iter().map(|(p, w)| format!("{p:+} {w}")).collect();
        writeln!(out, "{id}\t{score}/10\t{}", why.join("; ")).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// A pass and its beat on /watchers, whatever it said.
fn beating(dir: &Path, older_db: &Path) -> Result<String, String> {
    let today = local_now()
        .get(..10)
        .and_then(dash_core::days)
        .ok_or("no local date")?;
    watchers::beaten(
        dir,
        &watchers::CHANCES,
        pass(dir, older_db, today, now_ms()),
    )
}

/// A pass every `watchers::CHANCES.every_ms` on `older_db` for as long as `dash orch` runs; a line on stderr each time what
/// it says changes.
pub fn watch(dir: PathBuf, older_db: PathBuf) {
    let sleep = Duration::from_millis(watchers::CHANCES.every_ms as u64);
    watchers::run(dir, &watchers::CHANCES, sleep, move |dir| {
        beating(dir, &older_db)
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::TestDir;
    use dash_meter::dash_test;
    use std::time::Instant;

    /// The older app's tables, as much of them as a pass reads.
    const OLDER: &str = "CREATE TABLE leads (collection TEXT, pos INTEGER, id TEXT, posted TEXT, remote TEXT,
          onsite_expectation TEXT, data TEXT NOT NULL);
        CREATE TABLE events (source TEXT, pos INTEGER, data TEXT);
        CREATE TABLE drafts (lead TEXT, data TEXT NOT NULL);
        CREATE TABLE records (collection TEXT, pos INTEGER, key TEXT, id TEXT, data TEXT NOT NULL);
        INSERT INTO records VALUES ('profile.edn', 0, ':contract', NULL,
          '{\"day-rate\":{\"target\":600},\"eur-per\":{\"EUR\":1,\"GBP\":1.15}}');
        INSERT INTO records VALUES ('skills.edn', 0, 'have', 'have', '[[\"python\",{\"proof\":\"x\"}],[\"docker\",{}]]');
        INSERT INTO records VALUES ('skills.edn', 1, 'alias', 'alias', '[[\"py\",\"python\"]]');";

    fn lead(
        id: &str,
        posted: Option<&str>,
        remote: Option<&str>,
        onsite: Option<&str>,
        data: &str,
    ) -> Lead {
        let s = |v: Option<&str>| v.map(String::from);
        Lead {
            id: id.into(),
            posted: s(posted),
            remote: s(remote),
            onsite: s(onsite),
            data: data.into(),
            human: false,
            answered: false,
        }
    }

    fn older(dir: &Path) -> PathBuf {
        let db = dir.join("older.db");
        rusqlite_open(&db)
            .execute_batch(OLDER)
            .expect("the older tables");
        db
    }

    fn rusqlite_open(db: &Path) -> dash_store::Connection {
        dash_store::connect(db).expect("a database")
    }

    #[dash_test]
    fn a_lead_s_chances_are_the_older_app_s_odds() {
        let dir = TestDir::new("chances-judge");
        let ctx = Ctx::read(&older(&dir.0)).expect("the context");
        let today = dash_core::days("2026-10-20").expect("a day");
        // model_test.exs's case: 10 days old -2, one of three skills « have » 0, 600 over their 575 -2, full remote -1
        let data = r#"{"requirements":[{"ask":"Python","level":"have"},{"ask":"CDI","blocking":true},{"level":"partial"}],
                       "rate":{"min":500,"max":575,"currency":"EUR","unit":"day"},"contacts":[]}"#;
        let ad = lead("a", Some("2026-10-10"), Some("full"), None, data);
        let parts = |v: &[(i64, &str)]| {
            v.iter()
                .map(|(p, w)| (*p, w.to_string()))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            judge(&ad, &ctx, today),
            (
                0,
                parts(&[
                    (-2, "ad 10 days old, the pipeline is already running"),
                    (0, "33 % of the skills asked"),
                    (-2, "we ask 600, above their stated maximum (575)"),
                    (-1, "full remote: the whole market applies"),
                ])
            )
        );
        // a human who answered and a fresh ad lift it; the score stays within 0..10
        let fresh = Lead {
            human: true,
            answered: true,
            ..lead("b", Some("2026-10-19"), None, None, "{}")
        };
        assert_eq!(
            judge(&fresh, &ctx, today),
            (
                10,
                parts(&[
                    (2, "ad under 48 h old"),
                    (
                        2,
                        "a human already answered: no longer an anonymous application"
                    ),
                    (1, "the screening questions have answers"),
                ])
            )
        );
        assert_eq!(
            judge(&lead("c", None, None, None, "{}"), &ctx, today),
            (5, vec![])
        );
        // the on-site line whatever its case; a stack guessed against skills.edn through its alias; a fixed rate in GBP
        let onsite = lead(
            "d",
            None,
            None,
            Some("Trois jours par semaine sur site"),
            &older::marked(
                r#"{"skills-required":["py","cobol","⟦m:companies:docker⟧"],"rate":{"min":400,"max":400,"currency":"GBP"},"contacts":["@contact/0"]}"#,
            ),
        );
        assert_eq!(
            judge(&onsite, &ctx, today).1,
            parts(&[
                (2, "66 % of the skills asked are his"),
                (-2, "we ask 600, above their stated maximum (460)"),
                (
                    -1,
                    "their day rate is fixed (460): any other figure ends it"
                ),
                (1, "we have a named contact"),
                (1, "three days on site: many candidates drop out"),
            ])
        );
        // an ask written as no number asks no figure, as the older app reads it: no profile fallback, so no over-maximum line
        let text_ask =
            r#"{"our-ask":{"target":"650"},"rate":{"min":500,"max":500,"currency":"EUR"}}"#;
        assert_eq!(
            judge(&lead("f", None, None, None, text_ask), &ctx, today).1,
            parts(&[(
                -1,
                "their day rate is fixed (500): any other figure ends it"
            )])
        );
        // an hourly rate is no day rate: no rate line
        let hourly = lead(
            "e",
            None,
            None,
            None,
            r#"{"rate":{"min":50,"max":50,"unit":"hour"}}"#,
        );
        assert_eq!(judge(&hourly, &ctx, today), (5, vec![]));
    }

    /// The task's done-when: a lead that lands is judged within one minute, by the watcher `dash orch` starts.
    #[dash_test]
    fn a_new_lead_is_judged_within_a_minute_of_landing() {
        let dir = TestDir::new("chances-watch");
        let db = older(&dir.0);
        watch(dir.0.clone(), db.clone());
        let judged = |id: &str| -> Option<i64> {
            open(&dir.0)
                .ok()?
                .query_row("SELECT score FROM chances WHERE id = ?1", [id], |r| {
                    r.get(0)
                })
                .ok()
        };
        // the watcher's first pass, on a store with no lead yet
        let began = Instant::now();
        let beat = || -> Option<i64> {
            let sql = "SELECT at_ms FROM beat WHERE name = ?1";
            watchers::open(&dir.0)
                .ok()?
                .query_row(sql, [watchers::CHANCES.name], |r| r.get(0))
                .ok()
        };
        while beat().is_none() && began.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(beat().is_some(), "the watcher's first pass beats");
        let store = rusqlite_open(&db);
        store
            .execute(
                "INSERT INTO leads VALUES ('opportunities', 0, 'acme-platform', NULL, 'full', NULL, '{}')",
                [],
            )
            .expect("a lead lands");
        store
            .execute(
                "INSERT INTO events VALUES ('timeline:acme-platform', 0, '{\"channel\":\"~:call\"}')",
                [],
            )
            .expect("its call");
        let landed = Instant::now();
        while judged("acme-platform").is_none() && landed.elapsed() < Duration::from_secs(60) {
            std::thread::sleep(Duration::from_millis(100));
        }
        assert_eq!(
            judged("acme-platform"),
            Some(6),
            "full remote -1, a human who answered +2"
        );
        assert!(
            landed.elapsed() < Duration::from_secs(60),
            "judged {:?} after it landed",
            landed.elapsed()
        );
    }

    /// A pass writes each lead's score, an agent reads them as text, and a lead gone from the store leaves none behind.
    #[dash_test]
    fn a_pass_keeps_each_score_and_drops_a_lead_gone() {
        let dir = TestDir::new("chances-pass");
        let db = older(&dir.0);
        let store = rusqlite_open(&db);
        let land = "INSERT INTO leads VALUES ('opportunities', 0, 'acme-platform', NULL, 'full', NULL, '{}')";
        store.execute(land, []).expect("a lead lands");
        assert_eq!(
            pass(&dir.0, &db, 0, 1).expect("a pass"),
            "1 leads judged, 1 new"
        );
        let mut text = vec![];
        list(&dir.0, &mut text).expect("the list");
        assert_eq!(
            String::from_utf8(text).expect("utf-8"),
            "acme-platform\t4/10\t-1 full remote: the whole market applies\n"
        );
        assert_eq!(
            pass(&dir.0, &db, 0, 2).expect("a pass"),
            "1 leads judged, 0 new"
        );
        store.execute("DELETE FROM leads", []).expect("it goes");
        pass(&dir.0, &db, 0, 3).expect("a pass");
        let mut text = vec![];
        list(&dir.0, &mut text).expect("the list");
        assert!(text.is_empty(), "{}", String::from_utf8_lossy(&text));
    }
}
