//! /settings (task dash-settings-backup; @nha, 2026-11-09: « needs a settings tab, and a backup sql in there. sql -> zip,
//! and reimport with overwrite confirmation with count of entries per type. »): the backup of every store, its download and
//! what it holds, table by table.
use dash_store::backup;
use serde_json::json;
use std::io::Write;
use std::net::TcpStream;
use std::path::Path;

pub const BACKUP: &str = "/settings/backup.zip";
pub const RESTORE: &str = "/settings/restore";
pub const CONFIRM: &str = "/settings/restore/confirm";
/// The largest backup it reads back: today's stores zip to under a megabyte.
pub const UPLOAD_MOST: usize = 64 * 1024 * 1024;

/// Where an upload waits for his confirmation.
fn pending(data: &Path) -> std::path::PathBuf {
    data.join("backup-to-restore.zip")
}

/// The page: what a backup holds now, each store's tables and their rows, and the link that downloads it.
pub fn page(data: &Path) -> dash_surface::Drawing {
    let read = backup::zip(data).and_then(|zip| backup::counts(&zip));
    let (intro, rows) = match read {
        Ok(counts) => {
            let rows: Vec<_> = counts
                .iter()
                .map(|(store, table, n)| json!({"cells": [store, table, n.to_string()]}))
                .collect();
            let intro = "The backup: every store of this data folder as SQL, one file a store, in one zip. What it holds now:";
            (intro.to_string(), rows)
        }
        Err(e) => (format!("No backup can be made now: {e}"), vec![]),
    };
    let link = json!({"name": "Download the backup (zip)", "href": BACKUP});
    let tree = json!({"root": "page", "elements": {
        "page": {"type": "stack", "children": ["intro", "download", "held", "restore"]},
        "restore": {"type": "upload", "props": {"action": RESTORE, "label": "Read a backup", "accept": ".zip"}},
        "intro": {"type": "text", "props": {"text": intro}},
        "download": {"type": "links", "props": {"links": [link]}},
        "held": {"type": "table", "props": {"columns": ["Store", "Table", "Rows"], "rows": rows}},
    }});
    dash_surface::Drawing::Tree {
        title: "Settings".into(),
        tree: tree.to_string(),
    }
}

/// `GET /settings/backup.zip`: the zip itself, as a file to save, named by the day it was made.
pub fn download(stream: &TcpStream, data: &Path, day: &str) -> bool {
    let (status, kind, extra, body) = match backup::zip(data) {
        Ok(zip) => (
            "200 OK",
            "application/zip",
            format!("Content-Disposition: attachment; filename=\"dash-backup-{day}.zip\"\r\n"),
            zip,
        ),
        Err(e) => (
            "500 Internal Server Error",
            "text/plain; charset=utf-8",
            String::new(),
            e.into_bytes(),
        ),
    };
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    (&mut &*stream)
        .write_all(head.as_bytes())
        .and_then(|()| (&mut &*stream).write_all(&body))
        .is_ok()
}

/// The file of a one-field multipart form: what lies between its part's head and the next boundary.
fn file_of<'a>(body: &'a [u8], content_type: &str) -> Option<&'a [u8]> {
    let boundary = content_type.split("boundary=").nth(1)?.trim_matches('"');
    let find = |hay: &[u8], needle: &[u8]| hay.windows(needle.len()).position(|w| w == needle);
    let start = find(body, b"\r\n\r\n")? + 4;
    let rest = &body[start..];
    Some(&rest[..find(rest, format!("\r\n--{boundary}").as_bytes())?])
}

fn drawn(
    title: &str,
    intro: String,
    rows: Vec<serde_json::Value>,
    columns: &[&str],
    last: serde_json::Value,
) -> dash_surface::Drawing {
    let tree = json!({"root": "page", "elements": {
        "page": {"type": "stack", "children": ["intro", "held", "next"]},
        "intro": {"type": "text", "props": {"text": intro}},
        "held": {"type": "table", "props": {"columns": columns, "rows": rows}},
        "next": last,
    }});
    dash_surface::Drawing::Tree {
        title: title.into(),
        tree: tree.to_string(),
    }
}

/// An uploaded backup: kept aside, and what it would put back, table by table, beside what each holds now. Nothing is
/// overwritten until he confirms (@nha, 2026-11-09: « reimport with overwrite confirmation with count of entries per type »).
pub fn read_upload(data: &Path, body: &[u8], content_type: &str) -> dash_surface::Drawing {
    let back = json!({"type": "links", "props": {"links": [{"name": "Back to Settings", "href": "/settings"}]}});
    let Some(zip) = file_of(body, content_type) else {
        return drawn(
            "Settings",
            "No file came with the form.".into(),
            vec![],
            &[],
            back,
        );
    };
    let read = backup::counts(zip);
    let now = backup::zip(data)
        .and_then(|z| backup::counts(&z))
        .unwrap_or_default();
    let (Ok(counts), Ok(())) = (
        &read,
        std::fs::write(pending(data), zip).map_err(|e| e.to_string()),
    ) else {
        let why = read
            .err()
            .unwrap_or_else(|| "it cannot be kept aside".into());
        return drawn(
            "Settings",
            format!("This file is no backup this app reads: {why}"),
            vec![],
            &[],
            back,
        );
    };
    let rows = counts.iter().map(|(store, table, n)| {
        let held = now
            .iter()
            .find(|(s, t, _)| s == store && t == table)
            .map_or("no such table".into(), |(_, _, m)| m.to_string());
        json!({"cells": [store, table, n.to_string(), held]})
    });
    let intro = "This backup would replace, store by store, every row of these tables. Nothing is written until you confirm.";
    let confirm = json!({"type": "action", "props": {"action": CONFIRM, "label": "Replace the stores with this backup"}});
    drawn(
        "Settings",
        intro.into(),
        rows.collect(),
        &["Store", "Table", "In the backup", "Now"],
        confirm,
    )
}

/// His confirmation: the backup kept aside is put back, then dropped. What became of each store, a row each.
pub fn confirm(data: &Path) -> dash_surface::Drawing {
    let back = json!({"type": "links", "props": {"links": [{"name": "Back to Settings", "href": "/settings"}]}});
    let done =
        std::fs::read(pending(data)).map_err(|_| "no backup waits for a confirmation".to_string());
    let said = done.and_then(|zip| backup::restore(data, &zip));
    let _ = std::fs::remove_file(pending(data));
    match said {
        Ok(lines) => {
            let rows = lines.iter().map(|l| json!({"cells": [l]})).collect();
            drawn(
                "Settings",
                "The backup is put back.".into(),
                rows,
                &["Store"],
                back,
            )
        }
        Err(e) => drawn(
            "Settings",
            format!("Nothing was put back: {e}"),
            vec![],
            &[],
            back,
        ),
    }
}
