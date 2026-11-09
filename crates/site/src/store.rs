//! The boards' webhook queue on the site's Fly volume (rfc-project-boards section 5), PublicSite.DB's `hook_envelopes`
//! as JSON Lines: `hooks.jsonl`, one row a line, an id per row that only grows, and `epoch`, the file's random name.
//! A boot row marks each start (a deploy's gap may have lost a delivery); rows older than 3 days, GitHub's redelivery
//! window, are pruned, the newest always kept so the ids never start again under the same epoch. A line cut by a crash
//! is skipped on read. No SQLite: its C would need a Linux C toolchain here, and the chat that used its other table is
//! off (config/site.exs).
use serde_json::{Map, Value, json};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const KEEP: Duration = Duration::from_secs(3 * 86_400);

pub struct Store {
    file: PathBuf,
    rows: Vec<Map<String, Value>>,
    epoch: String,
    next: u64,
}

impl Store {
    pub fn open(dir: &Path) -> io::Result<Store> {
        fs::create_dir_all(dir)?;
        let file = dir.join("hooks.jsonl");
        let rows: Vec<Map<String, Value>> = fs::read_to_string(&file)
            .unwrap_or_default()
            .lines()
            .filter_map(|l| serde_json::from_str::<Map<String, Value>>(l).ok())
            .filter(|r| r.get("id").and_then(Value::as_u64).is_some())
            .collect();
        // the newest row is never pruned, so no row means the file was lost: its ids start again at 1, under a new
        // epoch, as SQLite's one file kept both (/code-review)
        let epoch_file = dir.join("epoch");
        let epoch = match fs::read_to_string(&epoch_file) {
            Ok(e) if !e.trim().is_empty() && !rows.is_empty() => e.trim().to_string(),
            _ => {
                let mut bytes = [0u8; 6];
                File::open("/dev/urandom")?.read_exact(&mut bytes)?;
                let e: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
                fs::write(&epoch_file, &e)?;
                e
            }
        };
        let next = rows
            .iter()
            .filter_map(|r| r["id"].as_u64())
            .max()
            .unwrap_or(0)
            + 1;
        let mut store = Store {
            file,
            rows,
            epoch,
            next,
        };
        store.prune()?;
        store.append(Map::from_iter([("kind".into(), json!("boot"))]))?;
        Ok(store)
    }

    /// One delivery's envelope: its non-null fields, stamped with an id and the time.
    pub fn envelope(&mut self, fields: Map<String, Value>) -> io::Result<()> {
        self.prune()?;
        let mut row = fields;
        row.retain(|_, v| !v.is_null());
        row.insert("kind".into(), json!("envelope"));
        self.append(row)
    }

    /// The rows after id `after`, oldest first, at most `limit`, the oldest id kept and the epoch.
    pub fn envelopes(&self, after: u64, limit: usize) -> Value {
        let rows: Vec<&Map<String, Value>> = self
            .rows
            .iter()
            .filter(|r| r["id"].as_u64().is_some_and(|id| id > after))
            .take(limit)
            .collect();
        let oldest = self.rows.first().map(|r| r["id"].clone());
        json!({"envelopes": rows, "oldest": oldest, "epoch": self.epoch})
    }

    fn append(&mut self, mut row: Map<String, Value>) -> io::Result<()> {
        row.insert("id".into(), json!(self.next));
        row.insert("at".into(), json!(iso(SystemTime::now())));
        let mut f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.file)?;
        let len = f.metadata()?.len();
        let written = f
            .write_all(format!("{}\n", Value::Object(row.clone())).as_bytes())
            .and_then(|()| f.sync_data());
        // a line cut by a full volume is taken back, or the next row would be glued onto it and both lost
        if let Err(e) = written {
            let _ = f.set_len(len);
            return Err(e);
        }
        self.next += 1;
        self.rows.push(row);
        Ok(())
    }

    /// ISO times of one width compare as strings.
    fn prune(&mut self) -> io::Result<()> {
        let cut = iso(SystemTime::now() - KEEP);
        let old = |r: &Map<String, Value>| r["at"].as_str().is_none_or(|at| at < cut.as_str());
        let drop = self
            .rows
            .iter()
            .take(self.rows.len().saturating_sub(1))
            .take_while(|r| old(r))
            .count();
        if drop == 0 {
            return Ok(());
        }
        self.rows.drain(..drop);
        let tmp = self.file.with_extension("jsonl.tmp");
        let text: String = self
            .rows
            .iter()
            .map(|r| format!("{}\n", Value::Object(r.clone())))
            .collect();
        fs::write(&tmp, text)?;
        fs::rename(&tmp, &self.file)
    }
}

/// `2026-11-08T12:34:56.789Z`, as Elixir's DateTime.to_iso8601 wrote PublicSite.DB's times.
pub fn iso(t: SystemTime) -> String {
    let d = t.duration_since(UNIX_EPOCH).unwrap_or_default();
    let (secs, ms) = (d.as_secs() as i64, d.subsec_millis());
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Howard Hinnant's civil_from_days
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{ms:03}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}
