//! The older app's drafts, leads and postings for Rust, read from its SQLite file, read-only (task rust-draft-page-cached):
//! what `dash model drafts`, `dash store files opportunities` and `dash store files postings` gave, so no draft page calls
//! the app (/drafts/<id> waited 9.4 s on it, 2026-11-11). Each row as `Model.rows/4` makes it: a field kept in its column
//! read back, then its markers replaced (the stage, a name, a link by its place, a lead's timeline).
//! ponytail: `types` is not read, so a value stays the plain JSON it is stored as (a lead's `:fit`, a map of other keys,
//! stays its pairs; a keyword its name); no draft page reads one. Which drafts go now stays `dash model drafts`'s until
//! task rust-outbox-rules ports its rules.
use rusqlite::Connection;
use rusqlite::types::Value as Cell;
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::path::Path;

/// The fields a row keeps in a boolean column, 0 or 1 there: `Row`'s `@typed`.
const BOOLS: [&str; 3] = ["from-approved-blocks", "cold", "from-france-ok"];
/// The link kinds a marker names: `Model`'s `@kinds`.
const KINDS: &str = "'waits-on', 'about', 'contact'";
const EVENTS: &str = "\u{E000}events\u{E001}";

/// A row of `drafts` or `leads`: its own fields, and every column by name.
struct Row {
    from: String,
    key: String,
    stage: Option<String>,
    data: Value,
    body: Option<String>,
    cols: HashMap<String, Cell>,
}

/// What markers point at: the names by ref (`companies:x`), the links by their row, kind and place, the timelines by lead.
#[derive(Default)]
struct Marked {
    names: HashMap<String, String>,
    links: HashMap<(String, String, i64), String>,
    timelines: HashMap<String, Vec<Value>>,
}

fn named(db: &Path) -> impl Fn(rusqlite::Error) -> String + '_ {
    move |e| format!("{}: {e}", db.display())
}

/// The rows of `table`'s `collection` in their order, `id`'s alone when given.
fn rows(
    db: &Connection,
    table: &str,
    collection: &str,
    id: Option<&str>,
) -> rusqlite::Result<Vec<Row>> {
    let sql = format!(
        "SELECT * FROM {table} WHERE collection = ?1 AND id IS NOT NULL AND (?2 IS NULL OR id = ?2) ORDER BY pos"
    );
    let mut q = db.prepare(&sql)?;
    let names: Vec<String> = q.column_names().map(String::from).collect();
    let got = q.query_map(rusqlite::params![collection, id], |r| {
        let mut cols = HashMap::new();
        for (i, n) in names.iter().enumerate() {
            cols.insert(n.clone(), r.get::<_, Cell>(i)?);
        }
        let text = |k: &str| match cols.get(k) {
            Some(Cell::Text(s)) => Some(s.clone()),
            _ => None,
        };
        let row = format!("{table} {}", text("id").unwrap_or_default());
        let at = names.iter().position(|n| n == "data").unwrap_or_default();
        let data = json(at, &row, &text("data").unwrap_or_default())?;
        Ok(Row {
            from: format!("{table}:{}", text("id").unwrap_or_default()),
            key: text("key").unwrap_or_default(),
            stage: text("stage"),
            body: text("body"),
            data,
            cols,
        })
    })?;
    got.collect()
}

/// A JSON cell, read: one that does not read is the read's error, naming its row (`drafts x`), as `Row.decode/1` raises,
/// never a value shown empty (/code-review).
fn json(at: usize, row: &str, cell: &str) -> rusqlite::Result<Value> {
    serde_json::from_str(cell).map_err(|e| {
        let said = format!("{row}: its data does not read: {e}");
        rusqlite::Error::FromSqlConversionFailure(at, rusqlite::types::Type::Text, said.into())
    })
}

/// The names, the links of `table`'s rows (`id`'s alone when given) and, for leads, their timelines.
fn marked(db: &Connection, table: &str, id: Option<&str>) -> rusqlite::Result<Marked> {
    let mut m = Marked::default();
    let names = "SELECT 'companies:' || id, name FROM companies WHERE name IS NOT NULL
      UNION ALL SELECT 'contacts:' || id, name FROM contacts WHERE name IS NOT NULL";
    let mut q = db.prepare(names)?;
    for r in q.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))? {
        let (k, v) = r?;
        m.names.insert(k, v);
    }
    // a row's own, or the table's: `from` is « drafts:<id> », so it sorts between « drafts: » and « drafts; »
    let own = |col: &str| {
        format!(
            "(?2 IS NOT NULL AND {col} = ?1 || ':' || ?2 OR ?2 IS NULL AND {col} >= ?1 || ':' AND {col} < ?1 || ';')"
        )
    };
    let links = format!(
        "SELECT \"from\", kind, pos, \"to\" FROM links WHERE kind IN ({KINDS}) AND {}",
        own("\"from\"")
    );
    let mut q = db.prepare(&links)?;
    let got = q.query_map(rusqlite::params![table, id], |r| {
        Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get::<_, String>(3)?))
    })?;
    for r in got {
        let (f, k, p, to) = r?;
        let id = to
            .split_once(':')
            .map_or(to.as_str(), |(_, id)| id)
            .to_string();
        m.links.insert((f, k, p), id);
    }
    if table == "leads" {
        let sql = format!(
            "SELECT about, data FROM events WHERE source LIKE 'timeline:%' AND {} ORDER BY source, pos",
            own("about")
        );
        let mut q = db.prepare(&sql)?;
        let got = q.query_map(rusqlite::params![table, id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
        })?;
        for r in got {
            let (about, data) = r?;
            let event = match data {
                Some(d) => json(1, &format!("event of {about}"), &d)?,
                None => Value::Null,
            };
            m.timelines.entry(about).or_default().push(event);
        }
    }
    Ok(m)
}

/// `row`'s entry: its data with each column's field read back, then every marker replaced.
fn entry(row: &Row, m: &Marked) -> Value {
    let mut data = row.data.clone();
    if let Value::Object(fields) = &mut data {
        for (k, v) in fields.iter_mut() {
            if v == "\u{E000}col\u{E001}" {
                *v = column(
                    row.cols.get(&k.replace('-', "_")),
                    BOOLS.contains(&k.as_str()),
                );
            }
        }
    }
    unmark(data, row, m)
}

fn column(cell: Option<&Cell>, boolean: bool) -> Value {
    match cell {
        Some(Cell::Integer(n)) if boolean => Value::Bool(*n == 1),
        Some(Cell::Integer(n)) => (*n).into(),
        Some(Cell::Real(x)) => (*x).into(),
        Some(Cell::Text(s)) => s.clone().into(),
        _ => Value::Null,
    }
}

/// Each string's markers replaced, as `Model.unmark/3`: a lead's timeline whole, else each marker in place. Where the older
/// app raised, a page shows the draft: a name not held stays its ref, a link not held is nothing, so a value that was that
/// link alone is none (/code-review: its marker read as the draft's lead).
fn unmark(v: Value, row: &Row, m: &Marked) -> Value {
    match v {
        Value::String(s) if s == EVENTS => {
            Value::Array(m.timelines.get(&row.from).cloned().unwrap_or_default())
        }
        Value::String(s) => match text(&s, row, m) {
            t if t.is_empty() && !s.is_empty() => Value::Null,
            t => Value::String(t),
        },
        Value::Array(xs) => Value::Array(xs.into_iter().map(|x| unmark(x, row, m)).collect()),
        Value::Object(kvs) => Value::Object(
            kvs.into_iter()
                .map(|(k, x)| (k, unmark(x, row, m)))
                .collect(),
        ),
        other => other,
    }
}

fn text(s: &str, row: &Row, m: &Marked) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(at) = rest.find('\u{E000}') {
        out.push_str(&rest[..at]);
        let after = &rest[at + '\u{E000}'.len_utf8()..];
        let Some(end) = after.find('\u{E001}') else {
            out.push_str(&rest[at..]);
            return out;
        };
        let r = &after[..end];
        let link = || {
            let (k, p) = r.strip_prefix('@')?.split_once('/')?;
            m.links
                .get(&(row.from.clone(), k.to_string(), p.parse().ok()?))
        };
        let said = match r {
            "stage" => row.stage.clone().unwrap_or_default(),
            _ if r.starts_with("m:") => m
                .names
                .get(&r[2..])
                .cloned()
                .unwrap_or_else(|| r[2..].to_string()),
            _ if r.starts_with('@') => link().cloned().unwrap_or_default(),
            _ => r.to_string(),
        };
        out.push_str(&said);
        rest = &after[end + '\u{E001}'.len_utf8()..];
    }
    out.push_str(rest);
    out
}

/// The drafts as `dash model drafts` lists them, its outbox facts apart (`Crm.drafts/0`): each record's fields, its
/// `status` its stage, its `body` trimmed and its `file`, oldest first; `id`'s alone when given.
pub fn drafts(db: &Path, id: Option<&str>) -> Result<Value, String> {
    let conn = crate::older::opened(db)?;
    let read = || -> rusqlite::Result<Vec<Value>> {
        let rows = rows(&conn, "drafts", "drafts", id)?;
        let m = marked(&conn, "drafts", id)?;
        let mut all: Vec<Value> = (rows.iter())
            .filter_map(|r| {
                let Value::Object(mut d) = entry(r, &m) else {
                    return None;
                };
                let body = r
                    .body
                    .as_deref()
                    .map(|b| text(b, r, &m))
                    .unwrap_or_default();
                d.insert("body".into(), body.trim().into());
                d.insert(
                    "file".into(),
                    format!("private/store/drafts/{}", r.key).into(),
                );
                Some(Value::Object(d))
            })
            .collect();
        let at = |d: &Value| {
            (
                d["created"].as_str().unwrap_or_default().to_string(),
                d["id"].as_str().unwrap_or_default().to_string(),
            )
        };
        all.sort_by_key(at);
        Ok(all)
    };
    read().map(Value::Array).map_err(named(db))
}

/// His name and mails, profile.edn's `candidate` record (`dash store text profile.edn`): what an application form asks first
/// (task rust-apply-greenhouse).
pub fn candidate(db: &Path) -> Result<Value, String> {
    let conn = crate::older::opened(db)?;
    let sql = "SELECT data FROM records WHERE collection = 'profile.edn' AND id = 'candidate'";
    let data: String = conn.query_row(sql, [], |r| r.get(0)).map_err(named(db))?;
    serde_json::from_str(&data).map_err(|e| format!("profile.edn candidate: {e}"))
}

/// The leads as `dash store files opportunities` gives them: each file's name and its entry as JSON text, its timeline
/// in it; `id`'s alone when given.
pub fn leads(db: &Path, id: Option<&str>) -> Result<Value, String> {
    let conn = crate::older::opened(db)?;
    let read = || -> rusqlite::Result<Map<String, Value>> {
        let rows = rows(&conn, "leads", "opportunities", id)?;
        let m = marked(&conn, "leads", id)?;
        Ok((rows.iter())
            // a row with no file name keyed by its id, never all of them on one empty key (/code-review)
            .map(|r| {
                let key = match r.key.as_str() {
                    "" => format!("{}.edn", &r.from["leads:".len()..]),
                    k => k.to_string(),
                };
                (key, entry(r, &m).to_string().into())
            })
            .collect())
    };
    read().map(Value::Object).map_err(named(db))
}

/// The archived ads as `dash store files postings` gives them: each file's name and its text.
pub fn postings(db: &Path) -> Result<Value, String> {
    let conn = crate::older::opened(db)?;
    let read = || -> rusqlite::Result<Map<String, Value>> {
        let mut q = conn.prepare("SELECT key, body FROM events WHERE source = 'postings' AND key IS NOT NULL ORDER BY pos")?;
        let got = q.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?.into(),
            ))
        })?;
        got.collect()
    };
    read().map(Value::Object).map_err(named(db))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::memory::TestBudget;
    use serde_json::json;

    /// The older app's tables, as much of them as the read takes.
    const OLDER: &str = "
      CREATE TABLE drafts (collection TEXT, pos INTEGER, key TEXT, id TEXT, stage TEXT, data TEXT NOT NULL, body TEXT,
        \"to\" TEXT, channel TEXT, created TEXT, opportunity TEXT, cold INTEGER);
      CREATE TABLE leads (collection TEXT, pos INTEGER, key TEXT, id TEXT, stage TEXT, data TEXT NOT NULL, body TEXT,
        title TEXT);
      CREATE TABLE links (\"from\" TEXT, kind TEXT, \"to\" TEXT, pos INTEGER DEFAULT 0 NOT NULL);
      CREATE TABLE contacts (id TEXT, name TEXT);
      CREATE TABLE companies (id TEXT, name TEXT);
      CREATE TABLE events (source TEXT, pos INTEGER, key TEXT, about TEXT, data TEXT, body TEXT);";

    /// One draft on one lead, a mention in its `to` and its body, its lead a link, its status the stage; the lead's
    /// timeline two events; one archived ad.
    const ROWS: &str = "
      INSERT INTO contacts VALUES ('ana', 'Ana B.');
      INSERT INTO companies VALUES ('acme', 'Acme');
      INSERT INTO drafts VALUES ('drafts', 0, '2026-11-02-b.md', 'b', 'approved',
        '{\"id\":\"\u{E000}col\u{E001}\",\"to\":\"\u{E000}col\u{E001}\",\"channel\":\"\u{E000}col\u{E001}\",
          \"created\":\"\u{E000}col\u{E001}\",\"opportunity\":\"\u{E000}@about/0\u{E001}\",
          \"status\":\"\u{E000}stage\u{E001}\",\"cold\":\"\u{E000}col\u{E001}\",\"answers\":[{\"q\":\"\u{E000}m:companies:acme\u{E001}?\"}]}',
        ' Hi \u{E000}m:contacts:ana\u{E001},\n ', '\u{E000}m:contacts:ana\u{E001} (\u{E000}m:companies:acme\u{E001})',
        'form', '2026-11-02', '\u{E000}@about/0\u{E001}', 1);
      INSERT INTO drafts VALUES ('drafts', 1, '2026-11-01-a.md', 'a', 'pending',
        '{\"id\":\"\u{E000}col\u{E001}\",\"created\":\"\u{E000}col\u{E001}\",\"status\":\"\u{E000}stage\u{E001}\"}',
        'Older', NULL, NULL, '2026-11-01', NULL, NULL);
      INSERT INTO links VALUES ('drafts:b', 'about', 'leads:acme-ai', 0);
      INSERT INTO leads VALUES ('opportunities', 0, '2026-11-acme-ai.edn', 'acme-ai', 'applied',
        '{\"id\":\"\u{E000}col\u{E001}\",\"title\":\"\u{E000}col\u{E001}\",\"status\":\"\u{E000}stage\u{E001}\",
          \"urls\":{\"apply\":\"https://acme.io/apply\"},\"timeline\":\"\u{E000}events\u{E001}\"}', NULL, 'AI engineer');
      INSERT INTO leads VALUES ('opportunities', 1, '2026-11-other.edn', 'other', 'open',
        '{\"timeline\":\"\u{E000}events\u{E001}\"}', NULL, NULL);
      INSERT INTO events VALUES ('timeline:acme-ai', 0, NULL, 'leads:acme-ai', '{\"date\":\"2026-11-01\",\"channel\":\"~:linkedin\",\"event\":\"found it\"}', NULL);
      INSERT INTO events VALUES ('timeline:acme-ai', 1, NULL, 'leads:acme-ai', '{\"date\":\"2026-11-03\",\"event\":\"sent draft b\"}', NULL);
      INSERT INTO events VALUES ('postings', 0, 'acme-ai.md', NULL, NULL, '# acme-ai' || char(10) || char(10) || '**Source:** https://acme.io/job');";

    fn older(name: &str) -> (dash_meter::TestDir, std::path::PathBuf) {
        let dir = dash_meter::TestDir::new(name);
        let db = dir.0.join("older.db");
        let conn = Connection::open(&db).expect("db");
        conn.execute_batch(OLDER).expect("tables");
        conn.execute_batch(ROWS).expect("rows");
        (dir, db)
    }

    /// Done-when 1 of rust-draft-page-cached: a draft read from the older app's file as `dash model drafts` gave it, each
    /// marker replaced: its column's fields, its stage as its status, a name, its lead by its link, its body trimmed.
    #[test]
    fn a_draft_reads_from_the_older_file_its_markers_replaced() {
        let _budget = TestBudget::new();
        let (_dir, db) = older("older-drafts");
        let all = drafts(&db, None).expect("drafts");
        let ids: Vec<&str> = (all.as_array().expect("a list").iter())
            .filter_map(|d| d["id"].as_str())
            .collect();
        assert_eq!(
            ids,
            ["a", "b"],
            "oldest first, as `dash model drafts` lists them"
        );
        let b = drafts(&db, Some("b")).expect("one");
        assert_eq!(
            b,
            json!([{
                "id": "b", "to": "Ana B. (Acme)", "channel": "form", "created": "2026-11-02", "opportunity": "acme-ai",
                "status": "approved", "cold": true, "answers": [{"q": "Acme?"}], "body": "Hi Ana B.,",
                "file": "private/store/drafts/2026-11-02-b.md",
            }])
        );
        assert_eq!(drafts(&db, Some("none")).expect("none"), json!([]));
        // its link gone: no lead, never the marker read as one (/code-review)
        let conn = Connection::open(&db).expect("db");
        conn.execute_batch("DELETE FROM links").expect("no link");
        let b = drafts(&db, Some("b")).expect("one");
        assert_eq!(b[0]["opportunity"], Value::Null, "{b}");
        // a data cell that does not read is the read's error, naming its row (/code-review)
        conn.execute_batch("UPDATE drafts SET data = '{' WHERE id = 'a'")
            .expect("broken");
        let said = drafts(&db, None).expect_err("no read");
        assert!(said.contains("drafts a: its data does not read"), "{said}");
    }

    /// Done-when 1 of rust-draft-page-cached: the leads as `dash store files opportunities` gave them, each file its entry
    /// as text with its own timeline; one lead alone when asked; the ads as `dash store files postings`.
    #[test]
    fn leads_and_postings_read_from_the_older_file() {
        let _budget = TestBudget::new();
        let (_dir, db) = older("older-leads");
        let all = leads(&db, None).expect("leads");
        let one: Value =
            serde_json::from_str(all["2026-11-acme-ai.edn"].as_str().expect("text")).expect("json");
        assert_eq!(
            one,
            json!({"id": "acme-ai", "title": "AI engineer", "status": "applied", "urls": {"apply": "https://acme.io/apply"},
                   "timeline": [{"date": "2026-11-01", "channel": "~:linkedin", "event": "found it"},
                                {"date": "2026-11-03", "event": "sent draft b"}]})
        );
        let other: Value =
            serde_json::from_str(all["2026-11-other.edn"].as_str().expect("text")).expect("json");
        assert_eq!(
            other,
            json!({"timeline": []}),
            "a lead's timeline is its own alone"
        );
        let alone = leads(&db, Some("acme-ai")).expect("one");
        // an event that does not read fails the read, named, never a null event (/code-review)
        let conn = Connection::open(&db).expect("db");
        conn.execute_batch(
            "UPDATE events SET data = 'x' WHERE pos = 1 AND source = 'timeline:acme-ai'",
        )
        .expect("broken");
        let said = leads(&db, None).expect_err("no read");
        assert!(
            said.contains("event of leads:acme-ai: its data does not read"),
            "{said}"
        );
        assert_eq!(alone.as_object().map(|m| m.len()), Some(1));
        assert_eq!(
            postings(&db).expect("ads"),
            json!({"acme-ai.md": "# acme-ai\n\n**Source:** https://acme.io/job"})
        );
        // a file that is not the older app's says so, named
        let gone = drafts(&db.with_file_name("none.db"), None).expect_err("no file");
        assert!(gone.contains("none.db"), "{gone}");
    }

    /// His name and mail for rust-apply-greenhouse's form: profile.edn's candidate record, read as it is stored.
    #[test]
    fn the_candidate_is_profile_edn_s_record() {
        let _budget = TestBudget::new();
        let dir = dash_meter::TestDir::new("older-candidate");
        let db = dir.0.join("older.db");
        let conn = Connection::open(&db).expect("db");
        conn.execute_batch(
            "CREATE TABLE records (collection TEXT, pos INTEGER, id TEXT, data TEXT NOT NULL);
             INSERT INTO records VALUES ('profile.edn', 0, 'candidate', '{\"name\":\"Nicolas HA\",\"email-pro\":\"nicolas@klar.im\"}');",
        )
        .expect("rows");
        let c = candidate(&db).expect("the candidate");
        assert_eq!(c["name"], "Nicolas HA");
        assert_eq!(c["email-pro"], "nicolas@klar.im");
    }
}
