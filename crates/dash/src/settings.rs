//! /settings (task dash-settings-backup; @nha, 2026-11-09: « needs a settings tab, and a backup sql in there. sql -> zip,
//! and reimport with overwrite confirmation with count of entries per type. »): the backup of every store, its download and
//! what it holds, table by table.
use dash_store::backup;
use serde_json::json;
use std::io::Write;
use std::net::TcpStream;
use std::path::Path;

pub const BACKUP: &str = "/settings/backup.zip";

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
        "page": {"type": "stack", "children": ["intro", "download", "held"]},
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
