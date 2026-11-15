//! `dash store text` and `dash store files` from the older app's SQLite file, read-only (task rust-forwarded-next; @nha,
//! 2026-11-14: « If you don't need it killing elixir should reclaim ram/cpu too. »): every collection as `Dashboard.Store`
//! renders it, byte for byte. A row as `Model.rows/4` and `Model.Row.join/2` make it (a field kept in its column read back,
//! its tags put back from `types`, then its markers replaced), a source as `Store.render/2` assembles it, its JSON as Jason
//! prints it: objects in their order, floats as Erlang's `float_to_binary(f, [:short])`.
//! ponytail: `store files` prints its files sorted by name, as the older app does for 32 files or fewer; past 32 its map
//! came out in the BEAM's hash order, which no reader relied on.
use crate::older::{COL, EVENTS, Piece, STEP, mention_names, opened, pieces};
use rusqlite::Connection;
use rusqlite::types::Value as Cell;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

/// A JSON value as Jason reads it with `objects: :ordered_objects`: an object keeps its keys' order, an integer its digits.
#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Int(String),
    Float(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

/// The JSON in `s`, all of it.
pub fn parse(s: &str) -> Result<Json, String> {
    let mut p = Parser {
        s: s.as_bytes(),
        at: 0,
    };
    let v = p.value()?;
    p.space();
    match p.at == p.s.len() {
        true => Ok(v),
        false => Err(format!("not JSON: text after the value at byte {}", p.at)),
    }
}

struct Parser<'a> {
    s: &'a [u8],
    at: usize,
}

impl Parser<'_> {
    fn space(&mut self) {
        while self.s.get(self.at).is_some_and(|b| b" \t\n\r".contains(b)) {
            self.at += 1;
        }
    }

    fn bad<T>(&self, what: &str) -> Result<T, String> {
        Err(format!("not JSON: {what} at byte {}", self.at))
    }

    fn eat(&mut self, word: &str, v: Json) -> Result<Json, String> {
        match self.s[self.at..].starts_with(word.as_bytes()) {
            true => {
                self.at += word.len();
                Ok(v)
            }
            false => self.bad("an unknown word"),
        }
    }

    fn value(&mut self) -> Result<Json, String> {
        self.space();
        match self.s.get(self.at) {
            Some(b'{') => {
                self.at += 1;
                let mut kvs = vec![];
                self.space();
                if self.s.get(self.at) == Some(&b'}') {
                    self.at += 1;
                    return Ok(Json::Obj(kvs));
                }
                loop {
                    self.space();
                    let k = self.string()?;
                    self.space();
                    if self.s.get(self.at) != Some(&b':') {
                        return self.bad("no colon");
                    }
                    self.at += 1;
                    kvs.push((k, self.value()?));
                    self.space();
                    match self.s.get(self.at) {
                        Some(b',') => self.at += 1,
                        Some(b'}') => {
                            self.at += 1;
                            return Ok(Json::Obj(kvs));
                        }
                        _ => return self.bad("an object not closed"),
                    }
                }
            }
            Some(b'[') => {
                self.at += 1;
                let mut xs = vec![];
                self.space();
                if self.s.get(self.at) == Some(&b']') {
                    self.at += 1;
                    return Ok(Json::Arr(xs));
                }
                loop {
                    xs.push(self.value()?);
                    self.space();
                    match self.s.get(self.at) {
                        Some(b',') => self.at += 1,
                        Some(b']') => {
                            self.at += 1;
                            return Ok(Json::Arr(xs));
                        }
                        _ => return self.bad("a list not closed"),
                    }
                }
            }
            Some(b'"') => self.string().map(Json::Str),
            Some(b't') => self.eat("true", Json::Bool(true)),
            Some(b'f') => self.eat("false", Json::Bool(false)),
            Some(b'n') => self.eat("null", Json::Null),
            Some(b'-' | b'0'..=b'9') => {
                let from = self.at;
                let number = |b: &u8| b.is_ascii_digit() || b"+-.eE".contains(b);
                while self.s.get(self.at).is_some_and(number) {
                    self.at += 1;
                }
                let n = std::str::from_utf8(&self.s[from..self.at]).unwrap_or_default();
                match n.contains(['.', 'e', 'E']) {
                    true => n
                        .parse()
                        .map(Json::Float)
                        .or_else(|_| self.bad("a bad number")),
                    // Jason reads « -0 » as the integer 0
                    false => match n.parse::<i128>() {
                        Ok(i) => Ok(Json::Int(i.to_string())),
                        // digits past an i128; a lone « - » is no number
                        Err(_)
                            if Some(n.strip_prefix('-').unwrap_or(n)).is_some_and(|d| {
                                !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit())
                            }) =>
                        {
                            Ok(Json::Int(n.to_string()))
                        }
                        Err(_) => self.bad("a bad number"),
                    },
                }
            }
            _ => self.bad("no value"),
        }
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let h = self
            .s
            .get(self.at..self.at + 4)
            .and_then(|h| std::str::from_utf8(h).ok());
        match h.and_then(|h| u32::from_str_radix(h, 16).ok()) {
            Some(n) => {
                self.at += 4;
                Ok(n)
            }
            None => self.bad("a bad \\u escape"),
        }
    }

    fn string(&mut self) -> Result<String, String> {
        if self.s.get(self.at) != Some(&b'"') {
            return self.bad("no string");
        }
        self.at += 1;
        let mut out: Vec<u8> = vec![];
        loop {
            match self.s.get(self.at) {
                None => return self.bad("a string not closed"),
                Some(b'"') => {
                    self.at += 1;
                    return String::from_utf8(out).or_else(|_| self.bad("a string not UTF-8"));
                }
                Some(b'\\') => {
                    self.at += 1;
                    let c = match self.s.get(self.at) {
                        Some(b'u') => {
                            self.at += 1;
                            let hi = self.hex4()?;
                            let code = match hi {
                                0xD800..=0xDBFF if self.s[self.at..].starts_with(b"\\u") => {
                                    self.at += 2;
                                    let lo = self.hex4()?;
                                    0x10000
                                        + ((hi - 0xD800) << 10)
                                        + (lo.wrapping_sub(0xDC00) & 0x3FF)
                                }
                                n => n,
                            };
                            let Some(c) = char::from_u32(code) else {
                                return self.bad("a lone surrogate");
                            };
                            let mut b = [0; 4];
                            out.extend_from_slice(c.encode_utf8(&mut b).as_bytes());
                            continue;
                        }
                        Some(b'b') => 8,
                        Some(b'f') => 12,
                        Some(b'n') => b'\n',
                        Some(b'r') => b'\r',
                        Some(b't') => b'\t',
                        Some(c @ (b'"' | b'\\' | b'/')) => *c,
                        _ => return self.bad("a bad escape"),
                    };
                    self.at += 1;
                    out.push(c);
                }
                Some(b) => {
                    out.push(*b);
                    self.at += 1;
                }
            }
        }
    }
}

/// A float as Erlang's `float_to_binary(f, [:short])` prints it, which Jason uses: the shortest digits, as a decimal or in
/// scientific form, whichever is shorter, the decimal on a tie; from 1.0e16 up always scientific.
pub fn erlang_float(f: f64) -> String {
    if f == 0.0 {
        return if f.is_sign_negative() { "-0.0" } else { "0.0" }.into();
    }
    let e = format!("{f:e}");
    let (mantissa, exp) = e.split_once('e').unwrap_or((&e, "0"));
    let exp: i32 = exp.parse().unwrap_or_default();
    let (sign, mantissa) = match mantissa.strip_prefix('-') {
        Some(m) => ("-", m),
        None => ("", mantissa),
    };
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    let (first, rest) = digits.split_at(1);
    let sci = format!(
        "{sign}{first}.{}e{exp}",
        if rest.is_empty() { "0" } else { rest }
    );
    if exp >= 16 {
        return sci;
    }
    let point = exp + 1;
    let n = digits.len() as i32;
    let dec = match point {
        p if p <= 0 => format!("{sign}0.{}{digits}", "0".repeat(-p as usize)),
        p if p >= n => format!("{sign}{digits}{}.0", "0".repeat((p - n) as usize)),
        p => format!("{sign}{}.{}", &digits[..p as usize], &digits[p as usize..]),
    };
    if dec.len() <= sci.len() { dec } else { sci }
}

fn quoted(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\u{c}' => out.push_str("\\f"),
            '\r' => out.push_str("\\r"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04X}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

/// `v` as `Jason.encode!/2` prints it: on one line, or `pretty` as its formatter lays it out (two spaces, « "k": v »).
pub fn encode(v: &Json, pretty: bool) -> String {
    let mut out = String::new();
    put(v, pretty, 0, &mut out);
    out
}

fn put(v: &Json, pretty: bool, depth: usize, out: &mut String) {
    let line = |out: &mut String, depth: usize| {
        if pretty {
            out.push('\n');
            out.push_str(&"  ".repeat(depth));
        }
    };
    match v {
        Json::Null => out.push_str("null"),
        Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Json::Int(i) => out.push_str(i),
        Json::Float(f) => out.push_str(&erlang_float(*f)),
        Json::Str(s) => quoted(s, out),
        Json::Arr(xs) if xs.is_empty() => out.push_str("[]"),
        Json::Obj(kvs) if kvs.is_empty() => out.push_str("{}"),
        Json::Arr(xs) => {
            out.push('[');
            for (i, x) in xs.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                line(out, depth + 1);
                put(x, pretty, depth + 1, out);
            }
            line(out, depth);
            out.push(']');
        }
        Json::Obj(kvs) => {
            out.push('{');
            for (i, (k, x)) in kvs.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                line(out, depth + 1);
                quoted(k, out);
                out.push_str(if pretty { ": " } else { ":" });
                put(x, pretty, depth + 1, out);
            }
            line(out, depth);
            out.push('}');
        }
    }
}

/// The table a collection's rows live in: `Model`'s `@tables`, `records` for the rest.
fn table(collection: &str) -> &'static str {
    match collection {
        "opportunities" => "leads",
        "contacts.edn" => "contacts",
        "drafts" => "drafts",
        "blocks.edn" => "blocks",
        "tasks.edn" => "tasks",
        "live/comments.edn" => "comments",
        "handoff.md" => "handoff_steps",
        "mail" => "mails",
        "docs" => "docs",
        _ => "records",
    }
}

/// The fields a table keeps in a boolean column, 0 or 1 there: `Row`'s `@typed`.
fn boolean(table: &str, field: &str) -> bool {
    matches!(
        (table, field),
        ("tasks", "review" | "plan-first")
            | ("leads", "from-france-ok")
            | ("drafts", "from-approved-blocks" | "cold")
    )
}

/// The collection kinds whose lines are journal events, never a table's rows: `Model`'s `@events_kinds`.
const EVENTS_KINDS: [&str; 2] = ["jsonl", "notes"];
/// The link kinds a marker names: `Model`'s `@kinds`.
const KINDS: &str = "'waits-on', 'about', 'contact'";

struct Collection {
    name: String,
    kind: String,
    rows_at: Vec<String>,
    shell: Json,
}

struct Row {
    key: Json,
    data: Json,
    body: Option<String>,
}

/// What markers point at, as `Model.ctx/1` gathers it.
#[derive(Default)]
struct Ctx {
    names: HashMap<String, String>,
    links: HashMap<(String, String, i64), String>,
    timelines: HashMap<String, Vec<Json>>,
}

/// The row a marker sits in: its `table:id` and its stage.
struct Place<'a> {
    table: &'a str,
    from: Option<String>,
    stage: Option<String>,
}

fn sql(e: rusqlite::Error) -> String {
    e.to_string()
}

fn collection(db: &Connection, name: &str) -> Result<Option<Collection>, String> {
    let got = db.query_row(
        "SELECT name, kind, rows_at, shell FROM collections WHERE name = ?1",
        [name],
        |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, Option<String>>(3)?,
            ))
        },
    );
    let (name, kind, rows_at, shell): (String, String, _, _) = match got {
        Ok(got) => got,
        Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(None),
        Err(e) => return Err(sql(e)),
    };
    let rows_at = match parse(rows_at.as_deref().unwrap_or("[]"))? {
        Json::Arr(xs) => xs
            .into_iter()
            .map(|x| match x {
                Json::Str(s) => Ok(s),
                _ => Err(format!("{name}: rows_at holds no key")),
            })
            .collect::<Result<_, _>>()?,
        _ => vec![],
    };
    let shell = shell
        .as_deref()
        .map(parse)
        .transpose()?
        .unwrap_or(Json::Null);
    Ok(Some(Collection {
        name,
        kind,
        rows_at,
        shell,
    }))
}

fn ctx(db: &Connection, table: &str) -> Result<Ctx, String> {
    let mut c = Ctx {
        names: mention_names(db).map_err(sql)?,
        ..Ctx::default()
    };
    let links = format!(
        "SELECT \"from\", kind, pos, \"to\" FROM links WHERE kind IN ({KINDS}) AND \"from\" >= ?1 || ':' AND \"from\" < ?1 || ';'"
    );
    let mut q = db.prepare(&links).map_err(sql)?;
    let got = q
        .query_map([table], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get::<_, String>(3)?))
        })
        .map_err(sql)?;
    for r in got {
        let (from, kind, pos, to) = r.map_err(sql)?;
        let id = to
            .split_once(':')
            .map_or(to.as_str(), |(_, id)| id)
            .to_string();
        c.links.insert((from, kind, pos), id);
    }
    if table == "leads" {
        let mut q = db
            .prepare("SELECT about, data FROM events WHERE source LIKE 'timeline:%' ORDER BY source, pos")
            .map_err(sql)?;
        let got = q
            .query_map([], |r| {
                Ok((
                    r.get::<_, Option<String>>(0)?,
                    r.get::<_, Option<String>>(1)?,
                ))
            })
            .map_err(sql)?;
        for r in got {
            let (about, data) = r.map_err(sql)?;
            let event = data
                .as_deref()
                .map(parse)
                .transpose()?
                .unwrap_or(Json::Null);
            c.timelines
                .entry(about.unwrap_or_default())
                .or_default()
                .push(event);
        }
    }
    Ok(c)
}

/// `untag/1`'s inverse, `Row.retag/2`: the keyword, symbol, list, set or map of other keys `types` says `v` held.
fn retag(v: Json, types: Option<&Json>) -> Json {
    match (v, types) {
        (Json::Str(s), Some(Json::Str(t))) if t == "k" => Json::Str(format!("~:{s}")),
        (Json::Str(s), Some(Json::Str(t))) if t == "s" => Json::Str(format!("~${s}")),
        (Json::Str(s), None) if s.starts_with('~') => Json::Str(format!("~{s}")),
        (Json::Arr(xs), Some(Json::Obj(t))) if t.len() == 1 && t[0].0 != "o" => {
            let (coll, inner) = &t[0];
            let inner = (!matches!(inner, Json::Null)).then_some(inner);
            Json::Obj(vec![(format!("~#{coll}"), retag(Json::Arr(xs), inner))])
        }
        (Json::Obj(kvs), t) => {
            let o = match t {
                Some(Json::Obj(t)) => t.iter().find(|(k, _)| k == "o").map(|(_, v)| v),
                _ => None,
            };
            let typed = |k: &str| match o {
                Some(Json::Obj(o)) => o.iter().find(|(f, _)| f == k).map(|(_, v)| v),
                _ => None,
            };
            Json::Obj(
                kvs.into_iter()
                    .map(|(k, v)| {
                        let t = typed(&k);
                        (k, retag(v, t))
                    })
                    .collect(),
            )
        }
        // Enum.zip_with stops at the shorter of the two
        (Json::Arr(xs), Some(Json::Arr(ts))) => Json::Arr(
            (xs.into_iter().zip(ts))
                .map(|(x, t)| retag(x, (!matches!(t, Json::Null)).then_some(t)))
                .collect(),
        ),
        (Json::Arr(xs), _) => Json::Arr(xs.into_iter().map(|x| retag(x, None)).collect()),
        (v, _) => v,
    }
}

/// `s` with each marker replaced by what it points at, as `Model.unmark/3` does; a marker it cannot read is an error.
fn unmarked(s: &str, c: &Ctx, at: &Place) -> Result<String, String> {
    let mut out = String::new();
    for p in pieces(s) {
        match p {
            Piece::Text(t) | Piece::Stray(t) => out.push_str(t),
            Piece::Mention(r) => match c.names.get(r) {
                Some(n) => out.push_str(n),
                None => return Err(format!("a mention of {r}, which no name holds")),
            },
            Piece::Link(kp) => {
                let target = kp.split_once('/').and_then(|(k, p)| {
                    let from = at.from.clone()?;
                    c.links.get(&(from, k.to_string(), p.parse().ok()?))
                });
                match target {
                    Some(t) => out.push_str(t),
                    None => {
                        return Err(format!("a link {kp} of {:?}, which no link holds", at.from));
                    }
                }
            }
            Piece::Other("stage") if at.table == "handoff_steps" => {
                out.push_str(if at.stage.as_deref() == Some("open") {
                    " "
                } else {
                    "x"
                })
            }
            Piece::Other("stage") => out.push_str(at.stage.as_deref().unwrap_or_default()),
            Piece::Other(o) => return Err(format!("a marker « {o} » no row reads")),
        }
    }
    Ok(out)
}

fn unmark(v: Json, c: &Ctx, at: &Place) -> Result<Json, String> {
    Ok(match v {
        Json::Str(s) if s == EVENTS => {
            let from = at.from.as_deref().unwrap_or_default();
            Json::Arr(c.timelines.get(from).cloned().unwrap_or_default())
        }
        Json::Str(s) => Json::Str(unmarked(&s, c, at)?),
        Json::Arr(xs) => Json::Arr(
            xs.into_iter()
                .map(|x| unmark(x, c, at))
                .collect::<Result<_, _>>()?,
        ),
        Json::Obj(kvs) => Json::Obj(
            kvs.into_iter()
                .map(|(k, v)| Ok((k, unmark(v, c, at)?)))
                .collect::<Result<_, String>>()?,
        ),
        v => v,
    })
}

fn rows(db: &Connection, c: &Collection) -> Result<Vec<Row>, String> {
    if EVENTS_KINDS.contains(&c.kind.as_str()) {
        let mut q = db
            .prepare("SELECT key, data, body FROM events WHERE source = ?1 ORDER BY pos, key")
            .map_err(sql)?;
        let got = q
            .query_map([&c.name], |r| {
                Ok((
                    r.get::<_, Option<String>>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get(2)?,
                ))
            })
            .map_err(sql)?;
        return got
            .map(|r| {
                let (key, data, body) = r.map_err(sql)?;
                Ok(Row {
                    key: key.map_or(Json::Null, Json::Str),
                    data: data
                        .as_deref()
                        .map(parse)
                        .transpose()?
                        .unwrap_or(Json::Null),
                    body,
                })
            })
            .collect();
    }
    let t = table(&c.name);
    let ctx = ctx(db, t)?;
    let mut q = db
        .prepare(&format!(
            "SELECT * FROM {t} WHERE collection = ?1 ORDER BY pos, key"
        ))
        .map_err(sql)?;
    let names: Vec<String> = q.column_names().map(String::from).collect();
    let mut got = q.query([&c.name]).map_err(sql)?;
    let mut out = vec![];
    while let Some(r) = got.next().map_err(sql)? {
        let mut cols = HashMap::new();
        for (i, n) in names.iter().enumerate() {
            cols.insert(n.as_str(), r.get::<_, Cell>(i).map_err(sql)?);
        }
        let text = |k: &str| match cols.get(k) {
            Some(Cell::Text(s)) => Some(s.clone()),
            _ => None,
        };
        let id = text("id");
        let data =
            parse(&text("data").unwrap_or_default()).map_err(|e| format!("{t} {id:?}: {e}"))?;
        let data = match data {
            Json::Obj(kvs) => Json::Obj(
                kvs.into_iter()
                    .map(|(k, v)| match v {
                        Json::Str(s) if s == COL => {
                            let cell = cols.get(k.replace('-', "_").as_str());
                            let v = match cell {
                                None => return Err(format!("{t} {id:?}: no column for {k}")),
                                Some(Cell::Null) => Json::Null,
                                Some(Cell::Integer(i)) if boolean(t, &k) => Json::Bool(*i == 1),
                                Some(Cell::Integer(i)) => Json::Int(i.to_string()),
                                Some(Cell::Real(f)) => Json::Float(*f),
                                Some(Cell::Text(s)) => Json::Str(s.clone()),
                                Some(Cell::Blob(_)) => {
                                    return Err(format!("{t} {id:?}: a blob in {k}"));
                                }
                            };
                            Ok((k, v))
                        }
                        v => Ok((k, v)),
                    })
                    .collect::<Result<_, String>>()?,
            ),
            other => other,
        };
        let types = text("types").as_deref().map(parse).transpose()?;
        let typed = |k: &str| match &types {
            Some(Json::Obj(t)) => t.iter().find(|(f, _)| f == k).map(|(_, v)| v),
            _ => None,
        };
        let key = text("key").map_or(Json::Null, |k| retag(Json::Str(k), typed("key")));
        let at = Place {
            table: t,
            from: id.as_ref().map(|id| format!("{t}:{id}")),
            stage: text("stage"),
        };
        let data = unmark(retag(data, typed("data")), &ctx, &at)?;
        let body = text("body").map(|b| unmarked(&b, &ctx, &at)).transpose()?;
        out.push(Row { key, data, body });
    }
    Ok(out)
}

fn key_text(k: &Json) -> &str {
    match k {
        Json::Str(s) => s,
        _ => "",
    }
}

/// `Store.assemble/3`: the file's value, its rows put back where the shell keeps them.
fn assemble(shell: &Json, at: &[String], rows: &[Row]) -> Result<Json, String> {
    let datas = || rows.iter().map(|r| r.data.clone()).collect::<Vec<_>>();
    match (shell, at) {
        (Json::Arr(xs), []) if xs.is_empty() => Ok(Json::Arr(datas())),
        (Json::Obj(kvs), [])
            if kvs.len() == 1 && matches!(kvs[0].0.as_str(), "~#list" | "~#set") =>
        {
            Ok(Json::Obj(vec![(kvs[0].0.clone(), Json::Arr(datas()))]))
        }
        (Json::Obj(kvs), []) if kvs.len() == 1 && kvs[0].0 == "~#map" => {
            let pairs = rows
                .iter()
                .map(|r| Json::Arr(vec![r.key.clone(), r.data.clone()]));
            Ok(Json::Obj(vec![(
                "~#map".into(),
                Json::Arr(pairs.collect()),
            )]))
        }
        (Json::Obj(kvs), []) if kvs.is_empty() => rows
            .iter()
            .map(|r| match key_text(&r.key).strip_prefix("~:") {
                Some(k) => Ok((k.to_string(), r.data.clone())),
                None => Err(format!("a row keyed {:?} in a map of keywords", r.key)),
            })
            .collect::<Result<_, _>>()
            .map(Json::Obj),
        (Json::Obj(kvs), [first, rest @ ..]) => {
            let inner = kvs.iter().rev().find(|(k, _)| k == first).map(|(_, v)| v);
            let inner = assemble(inner.unwrap_or(&Json::Null), rest, rows)?;
            let mut kvs = kvs.clone();
            if let Some(slot) = kvs.iter_mut().find(|(k, _)| k == first) {
                slot.1 = inner;
            }
            Ok(Json::Obj(kvs))
        }
        _ => Err("a shell the store cannot fill".into()),
    }
}

fn pretty(v: &Json) -> String {
    encode(v, true) + "\n"
}

/// `Store.render/2`: a source as its files, each `(path in the store, text)`.
fn render(c: &Collection, rows: &[Row]) -> Result<Vec<(String, String)>, String> {
    let rel = |r: &Row| format!("{}/{}", c.name, key_text(&r.key));
    Ok(match c.kind.as_str() {
        "file" => vec![(
            c.name.clone(),
            pretty(&assemble(&c.shell, &c.rows_at, rows)?),
        )],
        "dir" => rows.iter().map(|r| (rel(r), pretty(&r.data))).collect(),
        "drafts" => rows
            .iter()
            .map(|r| {
                let head = pretty(&r.data);
                (
                    rel(r),
                    match &r.body {
                        Some(b) => format!("{head}---\n{b}"),
                        None => head,
                    },
                )
            })
            .collect(),
        "handoff" => {
            let Json::Str(shell) = &c.shell else {
                return Err("handoff.md: its shell is no text".into());
            };
            let mut bodies = rows.iter().map(|r| r.body.clone().unwrap_or_default());
            let lines: Vec<String> = shell
                .split('\n')
                .map(
                    |l| match l.len() == STEP.len_utf8() && l.starts_with(STEP) {
                        true => bodies.next().unwrap_or_else(|| l.to_string()),
                        false => l.to_string(),
                    },
                )
                .collect();
            // `handoff_join` matched `{lines, []}`: a step row no marker places is an error, so the older app answers
            // and no step waiting on him is dropped (/code-review of PR 1604)
            if bodies.next().is_some() {
                return Err("handoff.md: more step rows than its shell's markers".into());
            }
            vec![(c.name.clone(), lines.join("\n"))]
        }
        "jsonl" => {
            let lines = rows
                .iter()
                .map(|r| r.body.clone().unwrap_or_else(|| encode(&r.data, false)) + "\n");
            vec![(c.name.clone(), lines.collect())]
        }
        _ => rows
            .iter()
            .map(|r| (rel(r), r.body.clone().unwrap_or_default()))
            .collect(),
    })
}

/// The sources `Store.known/3` names beside the top-level data files, with their kinds; the docs and Jev's journal are
/// `@outside`, never located.
const KNOWN: [(&str, &str); 25] = [
    ("live/comments.edn", "file"),
    ("live/task-journal.jsonl", "jsonl"),
    ("handoff.md", "handoff"),
    ("task-journal.jsonl", "jsonl"),
    ("outbound.jsonl", "jsonl"),
    ("revisions.jsonl", "jsonl"),
    ("feedback.jsonl", "jsonl"),
    ("waiting.jsonl", "jsonl"),
    ("app-memory.jsonl", "jsonl"),
    ("whatsapp.jsonl", "jsonl"),
    ("density.jsonl", "jsonl"),
    ("weekly-read.jsonl", "jsonl"),
    ("handoff-archive.jsonl", "jsonl"),
    ("prose.jsonl", "jsonl"),
    ("ao-journal.jsonl", "jsonl"),
    ("chat.jsonl", "jsonl"),
    ("demo-views.jsonl", "jsonl"),
    ("drafts", "drafts"),
    ("presentations", "drafts"),
    ("opportunities", "dir"),
    ("mail", "md"),
    ("interview/calls", "notes"),
    ("interview/calls/transcripts", "notes"),
    ("postings", "notes"),
    ("daily", "notes"),
];

/// A folder's entry: `Store.hit/3` for a folder, its file name when it has the folder's extension.
fn entry_of(kind: &str, base: &str) -> Option<String> {
    let ext = match kind {
        "dir" => ".edn",
        "docs" => ".html",
        _ => ".md",
    };
    static SHAPED: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"\A\w[\w.-]*\z").expect("an entry's shape"));
    (!matches!(kind, "file" | "handoff" | "jsonl")
        && base != "_template.md"
        && SHAPED.is_match(base)
        && base.ends_with(ext)
        && base.len() > ext.len())
    .then(|| base.to_string())
}

/// `Store.locate/1`: the source a path names, `(name, file in its folder)`; the path itself when none.
pub fn locate(path: &str) -> (String, Option<String>) {
    let known = |n: &str| KNOWN.iter().find(|(k, _)| *k == n).map(|(_, kind)| *kind);
    if known(path).is_some() {
        return (path.to_string(), None);
    }
    // the path under the CRM's folder, « . » and « .. » read as Path.expand reads them
    let mut parts: Vec<&str> = vec![];
    for p in path.split('/') {
        match p {
            "" | "." => {}
            ".." if parts.pop().is_none() => return (path.to_string(), None),
            ".." => {}
            p => parts.push(p),
        }
    }
    // a source reached through « . » or « .. » is that source, as `source_of(Path.join(crm, path))` finds it
    let joined = parts.join("/");
    if known(&joined).is_some() {
        return (joined, None);
    }
    let Some((base, dir)) = parts.split_last() else {
        return (path.to_string(), None);
    };
    let dir = dir.join("/");
    static TOP: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"\A\w[\w.-]*\.edn\z").expect("a top-level data file's shape")
    });
    let found = match (dir.as_str(), known(&dir)) {
        ("", _) if TOP.is_match(base) && *base != "mail-cursor.edn" => {
            Some((base.to_string(), None))
        }
        (_, Some(kind)) => entry_of(kind, base).map(|f| (dir.clone(), Some(f))),
        _ => None,
    };
    found.unwrap_or_else(|| (path.to_string(), None))
}

/// A collection and its rows, its shell's markers replaced as `Model.read_collections/2` does (a name, by the names alone).
fn read(db: &Connection, name: &str) -> Result<Option<(Collection, Vec<Row>)>, String> {
    let Some(mut c) = collection(db, name)? else {
        return Ok(None);
    };
    let rows = rows(db, &c)?;
    if crate::older::holds_marker(&encode(&c.shell, false)) {
        let names = Ctx {
            names: mention_names(db).map_err(sql)?,
            ..Ctx::default()
        };
        let at = Place {
            table: "",
            from: None,
            stage: None,
        };
        c.shell = unmark(c.shell, &names, &at)?;
    }
    Ok(Some((c, rows)))
}

/// `Store.text/2`: one source as its file's text, or one entry of a folder by its file name; "" for none.
pub fn text(db: &Connection, name: &str, key: Option<&str>) -> Result<String, String> {
    let Some((c, mut rows)) = read(db, name)? else {
        return Ok(String::new());
    };
    if let Some(k) = key {
        rows.retain(|r| key_text(&r.key) == k && !matches!(r.key, Json::Null));
    }
    Ok(render(&c, &rows)?.into_iter().map(|(_, t)| t).collect())
}

/// `Store.files/1`: a folder source as `{file name: text}`, None for a source that is no folder.
pub fn files(db: &Connection, name: &str) -> Result<Option<BTreeMap<String, String>>, String> {
    let Some((c, rows)) = read(db, name)? else {
        let folder = KNOWN
            .iter()
            .chain(&[("docs", "docs")])
            .any(|(n, kind)| *n == name && !matches!(*kind, "file" | "handoff" | "jsonl"));
        return Ok(folder.then(BTreeMap::new));
    };
    if matches!(c.kind.as_str(), "file" | "handoff" | "jsonl") {
        return Ok(None);
    }
    let files = render(&c, &rows)?.into_iter().map(|(rel, t)| {
        let base = rel.rsplit('/').next().unwrap_or_default().to_string();
        (base, t)
    });
    Ok(Some(files.collect()))
}

/// `dash task show <id>`, `dash store text` and `dash store files` as `Dashboard.CLI` answers them: `(exit, stdout,
/// stderr)`, None for a command the older app still answers (a reviewer's brief, a task it does not file). An error is the
/// reader's own (a schema it cannot read): the front door then asks the older app.
pub fn answer(older: &Path, args: &[String]) -> Result<Option<(i32, String, String)>, String> {
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args[..] {
        ["task", "show", id] => {
            Ok(crate::older::task_show(older, id)?.map(|shown| (0, shown + "\n", String::new())))
        }
        ["task", ..] => Ok(None),
        _ => store(older, &args).map(Some),
    }
}

/// One read transaction over the shell, names, links, timelines and rows: a write the older app commits between them
/// never mixes its old shell with its new rows (/code-review of PR 1604).
fn snapshot(db: &Connection) -> Result<rusqlite::Transaction<'_>, String> {
    db.unchecked_transaction().map_err(sql)
}

fn store(older: &Path, args: &[&str]) -> Result<(i32, String, String), String> {
    let refused = |why: String| (1, String::new(), why + "\n");
    match args[..] {
        ["store", "text", source, ref key @ ..] if key.len() < 2 => {
            let db = opened(older)?;
            let db = snapshot(&db)?;
            let (name, file) = match key.first() {
                None => locate(source),
                Some(k) => (source.to_string(), Some(k.to_string())),
            };
            Ok(match text(&db, &name, file.as_deref())? {
                t if t.is_empty() => refused(format!(
                    "store text: the model holds no {}",
                    args[2..].join(" ")
                )),
                t => (0, t, String::new()),
            })
        }
        ["store", "text", ..] => Ok((
            2,
            String::new(),
            "usage: dash store text <path> | <folder> <file>\n".into(),
        )),
        ["store", "files", folder] => {
            let db = opened(older)?;
            let db = snapshot(&db)?;
            Ok(match files(&db, folder)? {
                None => refused(format!("store files: {folder} is no folder of the store")),
                Some(files) => {
                    let obj =
                        Json::Obj(files.into_iter().map(|(k, t)| (k, Json::Str(t))).collect());
                    (0, encode(&obj, true) + "\n", String::new())
                }
            })
        }
        ["store", "files", ..] => Ok((
            2,
            String::new(),
            "usage: dash store files <folder>\n".into(),
        )),
        _ => Err("not a task show, store text or store files command".into()),
    }
}

#[cfg(test)]
mod tests;
