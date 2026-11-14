//! The audit port's adapter (task rust-audit-log): every tool call the core makes, one row, and its sums by tool.
//! Its functions take the connection, so this module needs nothing above it; `SqliteStore` calls them.
use dash_core::registry::Call;
use rusqlite::{Connection, OptionalExtension, params};

pub(crate) const TABLE: &str = "CREATE TABLE IF NOT EXISTS tool_calls (id INTEGER PRIMARY KEY, tool TEXT NOT NULL,
  origin TEXT NOT NULL, asked TEXT NOT NULL, answer TEXT NOT NULL, ok INTEGER NOT NULL, at_ms INTEGER NOT NULL,
  ms INTEGER NOT NULL, cost REAL NOT NULL);
CREATE INDEX IF NOT EXISTS tool_calls_at ON tool_calls (at_ms);
CREATE INDEX IF NOT EXISTS tool_calls_tool ON tool_calls (tool, at_ms);";

/// What a row keeps of a question or an answer. ponytail: a cut, not the whole answer; a tool whose answers matter past it
/// keeps them in its own store. ponytail: no row is ever pruned (8 KB at most a call); add a kept window, as automations.rs's
/// `KEPT_MS`, once `dash settings` shows the table past a few MB.
pub(crate) const KEPT_BYTES: usize = 4000;

fn cut(text: &str) -> &str {
    let mut end = text.len().min(KEPT_BYTES);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// One tool's calls over a period: what worked, what failed, what they cost and took, and its last failure.
#[derive(Clone, Debug, PartialEq)]
pub struct Sum {
    pub tool: String,
    pub worked: i64,
    pub failed: i64,
    pub cost: f64,
    pub ms: i64,
    pub last_failure: Option<String>,
}

/// One call, its question and answer cut to `KEPT_BYTES`.
pub(crate) fn record(db: &Connection, call: &Call) -> Result<(), String> {
    let (answer, ok) = match &call.answer {
        Ok(a) => (a, true),
        Err(e) => (e, false),
    };
    db.execute(
        "INSERT INTO tool_calls (tool, origin, asked, answer, ok, at_ms, ms, cost)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            call.tool,
            call.origin,
            cut(&call.asked),
            cut(answer),
            ok,
            call.at_ms,
            call.ms,
            call.cost
        ],
    )
    .map(drop)
    .map_err(|e| e.to_string())
}

/// A store `open_read` found without the table holds no call yet.
fn kept(db: &Connection) -> Result<bool, String> {
    db.query_row(
        "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'tool_calls'",
        [],
        |_| Ok(()),
    )
    .optional()
    .map(|found| found.is_some())
    .map_err(|e| e.to_string())
}

/// Each tool's sum since `since_ms`, by name; `tool` alone when named.
pub(crate) fn sums(db: &Connection, tool: Option<&str>, since_ms: i64) -> Result<Vec<Sum>, String> {
    if !kept(db)? {
        return Ok(vec![]);
    }
    let mut statement = db
            .prepare(
                "SELECT tool, sum(ok), sum(1 - ok), sum(cost), sum(ms),
                   (SELECT answer FROM tool_calls f WHERE f.tool = c.tool AND f.ok = 0 AND f.at_ms >= ?1
                    ORDER BY at_ms DESC, id DESC LIMIT 1)
                 FROM tool_calls c WHERE at_ms >= ?1 AND (?2 IS NULL OR tool = ?2) GROUP BY tool ORDER BY tool",
            )
            .map_err(|e| e.to_string())?;
    let rows = statement
        .query_map(params![since_ms, tool], |r| {
            Ok(Sum {
                tool: r.get(0)?,
                worked: r.get(1)?,
                failed: r.get(2)?,
                cost: r.get(3)?,
                ms: r.get(4)?,
                last_failure: r.get(5)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<_, _>>().map_err(|e| e.to_string())
}

/// Every call since `since_ms`, oldest first.
pub(crate) fn calls(db: &Connection, since_ms: i64) -> Result<Vec<Call>, String> {
    if !kept(db)? {
        return Ok(vec![]);
    }
    let mut statement = db
            .prepare(
                "SELECT tool, origin, asked, answer, ok, at_ms, ms, cost FROM tool_calls WHERE at_ms >= ?1
                 ORDER BY at_ms, id",
            )
            .map_err(|e| e.to_string())?;
    let rows = statement
        .query_map(params![since_ms], |r| {
            let answer: String = r.get(3)?;
            Ok(Call {
                tool: r.get(0)?,
                origin: r.get(1)?,
                asked: r.get(2)?,
                answer: if r.get(4)? { Ok(answer) } else { Err(answer) },
                at_ms: r.get(5)?,
                ms: r.get(6)?,
                cost: r.get(7)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<_, _>>().map_err(|e| e.to_string())
}
