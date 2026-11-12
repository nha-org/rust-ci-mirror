//! /v/<doc>: a page of private/docs as the one document model holds it (task editor-in-rfcs; rust/CLAUDE.md rule 7; @nha,
//! 2026-11-11: « we need to port documents to the new notion rust that is probably a DB reimport in the new system right ? »).
//! The document lives in Rust's store, `docs.sqlite`'s `docs` table on the op log, keyed by its repo path; a first read moves
//! the file in through `dash doc import` (rfc-rust-unified section 20i), every later read is the store's. It is drawn through
//! DocView's Notion view as the RFC type, his threads anchored on their quotes, and saved with `dash doc save`, which writes
//! the HTML file back as its export. Any other /v/ path stays the older app's (a 404 here is forwarded while it lasts).
use crate::pages::{Page, Reads};
use dash_core::{FieldWrite, Value as Field};
use dash_store::oplog::Replica;
use dash_surface as surface;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

const TABLE: &str = "docs";
const SCHEMA: &str =
    "CREATE TABLE IF NOT EXISTS docs (id TEXT PRIMARY KEY, record TEXT) WITHOUT ROWID;";
/// A posted edit at most: every passage of the largest document (rfc-rust-unified.html, about 500 KB) encoded, with room.
pub const BODY: usize = 8 * 1024 * 1024;
/// The folder whose pages Rust serves; the older app keeps every other /v/ path.
const FOLDER: &str = "private/docs/";

fn log(dir: &Path) -> Result<Replica, String> {
    let db = dash_store::connect(&dir.join("docs.sqlite")).map_err(|e| e.to_string())?;
    db.execute_batch(SCHEMA).map_err(|e| e.to_string())?;
    Replica::new(db, "dash").map_err(|e| e.to_string())
}

/// The repo path a /v/ request names, when it is a page of private/docs: decoded, with no way out of the folder.
pub fn doc_path(request: &str) -> Option<String> {
    let path = crate::pages::segment_decoded(request.strip_prefix("/v/")?);
    let name = path.strip_prefix(FOLDER)?;
    let plain =
        !name.is_empty() && !name.contains('/') && !name.contains('\\') && !name.starts_with('.');
    (plain && name.ends_with(".html")).then_some(path)
}

/// A document as the store keeps it, and its file's fingerprint when the store last read or wrote that file: the file is
/// still edited by hand and by agents (the rfc skill), so one that changed since is read in again, never shadowed.
pub struct Kept {
    pub doc: Value,
    pub file: String,
}

/// A file's bytes as a fingerprint: equal bytes, equal print. Not a secret, only a change seen.
fn print_of(bytes: &[u8]) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut h);
    format!("{:016x}", h.finish())
}

/// The file's fingerprint now; `None`: no file.
fn file_print(file: &Path) -> Option<String> {
    std::fs::read(file).ok().map(|b| print_of(&b))
}

/// The document kept for `path`, with its file's fingerprint then. ponytail: one indexed row read, never the whole op log
/// (dash_comments' read_table scans every op, fine for its few thousand rows, not for 71 documents of up to 500 KB).
pub fn stored(dir: &Path, path: &str) -> Result<Option<Kept>, String> {
    let log = log(dir)?;
    let records = log
        .read(|db| -> Result<Vec<String>, String> {
            let mut q = db
                .prepare("SELECT record FROM docs WHERE id = ?1")
                .map_err(|e| e.to_string())?;
            let rows = q
                .query_map([path], |r| r.get::<_, String>(0))
                .map_err(|e| e.to_string())?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())
        })
        .map_err(|e| e.to_string())??;
    let kept = |r: &String| -> Result<Kept, String> {
        let v: Value = serde_json::from_str(r).map_err(|e| format!("{path}: {e}"))?;
        let file = v["file"].as_str().unwrap_or_default().to_string();
        Ok(Kept {
            doc: v["doc"].clone(),
            file,
        })
    };
    records.first().map(kept).transpose()
}

fn keep(dir: &Path, path: &str, doc: &Value, file: &str) -> Result<(), String> {
    let write = FieldWrite {
        table: TABLE.into(),
        row: path.into(),
        field: "record".into(),
        value: Field::Text(json!({"doc": doc, "file": file}).to_string()),
    };
    log(dir)?
        .write(&[write])
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// The document for `path`: the store's while its file is as the store last saw it, else the file read in again by `dash doc
/// import` and kept (its first read, or an edit made to the file since). `None`: neither holds it.
pub fn opened(dir: &Path, repo: &Path, path: &str) -> Result<Option<Value>, String> {
    let file = repo.join(path);
    let now = file_print(&file);
    match (stored(dir, path)?, &now) {
        (Some(k), Some(now)) if &k.file == now => return Ok(Some(k.doc)),
        (Some(k), None) => return Ok(Some(k.doc)),
        (None, None) => return Ok(None),
        _ => {}
    }
    let doc = dash_ingest::doc::import(&file)?;
    surface::documents::loads(&doc)?;
    keep(dir, path, &doc, now.as_deref().unwrap_or_default())?;
    Ok(Some(doc))
}

/// `dash doc save`: `doc` kept as the document of `path`, then its HTML file in `repo` written again as its export. A
/// document the Notion view does not read is refused, and so is a save over a file changed since the store last read it (an
/// edit by hand would be lost): nothing is written.
pub fn save(dir: &Path, repo: &Path, path: &str, doc: &Value) -> Result<PathBuf, String> {
    surface::documents::loads(doc)?;
    let file = repo.join(path);
    let seen = stored(dir, path)?.map(|k| k.file);
    if let Some(now) = file_print(&file)
        && seen.as_deref() != Some(now.as_str())
    {
        return Err(format!(
            "{path} changed on disk since the store read it: open /v/{path} again, then save"
        ));
    }
    let export = surface::documents::export(doc.clone());
    std::fs::write(&file, &export).map_err(|e| format!("{}: {e}", file.display()))?;
    keep(dir, path, doc, &print_of(export.as_bytes()))?;
    Ok(file)
}

/// `dash doc save --data <dir> <path> <document.json>`: the document kept and its file written, its path said.
pub fn save_file(dir: &Path, path: &str, file: &Path, mut out: std::fs::File) {
    let saved = (std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display())))
        .and_then(|t| {
            serde_json::from_str::<Value>(&t).map_err(|e| format!("{}: {e}", file.display()))
        })
        .and_then(|doc| {
            let path =
                doc_path(&format!("/v/{path}")).ok_or(format!("{path}: not a page of {FOLDER}"))?;
            save(dir, &crate::pages::repo(), &path, &doc)
        });
    match saved {
        Ok(f) => std::io::Write::write_all(
            &mut out,
            format!("saved, its export written to {}\n", f.display()).as_bytes(),
        )
        .expect("stdout"),
        Err(e) => {
            eprintln!("dash doc save: {e}");
            std::process::exit(1);
        }
    }
}

/// `dash doc move --data <dir>`: every page of private/docs read into the store that does not hold it yet, so no reader of
/// /v/ waits on a first import (the largest, rfc-rust-unified.html, takes about a second). A line saying how many.
pub fn move_all(dir: &Path, mut out: std::fs::File) {
    let repo = crate::pages::repo();
    let moved = (|| {
        let mut names: Vec<String> = std::fs::read_dir(repo.join(FOLDER))
            .map_err(|e| format!("{FOLDER}: {e}"))?
            .filter_map(|e| e.ok()?.file_name().into_string().ok())
            .filter(|n| n.ends_with(".html"))
            .collect();
        names.sort();
        let (mut new, mut kept) = (0, 0);
        for name in &names {
            let path = format!("{FOLDER}{name}");
            // a page kept and unchanged since is left; a new one, or one edited on disk since, is read in
            let before = stored(dir, &path)?.map(|k| k.file);
            opened(dir, &repo, &path).map_err(|e| format!("{path}: {e}"))?;
            match before == stored(dir, &path)?.map(|k| k.file) {
                true => kept += 1,
                false => new += 1,
            }
        }
        Ok::<_, String>(format!(
            "{} pages of {FOLDER}: {new} read into the store, {kept} unchanged there",
            names.len()
        ))
    })();
    match moved {
        Ok(line) => {
            std::io::Write::write_all(&mut out, format!("{line}\n").as_bytes()).expect("stdout")
        }
        Err(e) => {
            eprintln!("dash doc move: {e}");
            std::process::exit(1);
        }
    }
}

/// The file's commits, newest first, 30 at most, as the older app's `DocEdit.versions/2` (none when git has none), and whether
/// the file differs from the last of them: a save is written, never committed (a commit is `dash ship`'s), so the last commit
/// is then no longer the current words. Read only when asked (`?versions`): `git log` on a path takes 0.4 s on rfc-inbox.html,
/// too long for every view (/code-review of 7c090408e).
fn versions(repo: &Path, path: &str) -> (Vec<surface::documents::Version>, bool) {
    let root = repo.to_string_lossy();
    let status = ["-C", root.as_ref(), "status", "--porcelain", "--", path];
    let edited = dash_process::run("git", &status, None, std::time::Duration::from_secs(30))
        .is_ok_and(|out| !out.trim().is_empty());
    let repo = root;
    let args = [
        "-C",
        repo.as_ref(),
        "log",
        "-n",
        "30",
        "--format=%H%x1f%an%x1f%aI%x1f%s",
        "--",
        path,
    ];
    let out = dash_process::run("git", &args, None, std::time::Duration::from_secs(30))
        .unwrap_or_default();
    let all = (out.lines())
        .filter_map(|l| {
            let mut f = l.splitn(4, '\x1f').map(str::to_string);
            Some(surface::documents::Version {
                sha: f.next()?,
                by: f.next()?,
                at: f.next()?,
                title: f.next()?,
            })
        })
        .collect();
    (all, edited)
}

/// A query's flag: `edit` or `edit=…`.
fn flag(query: &str, name: &str) -> bool {
    query
        .split('&')
        .any(|p| p == name || p.starts_with(&format!("{name}=")))
}

/// The page: the RFC type with its imported body, his threads (moved and new) and the comment box, its glossary the knowledge
/// base's; `?edit` each passage a field, `?present=<n>` its n-th screen.
pub fn page(
    dir: &Path,
    repo: &Path,
    request: &str,
    query: &str,
) -> Result<Option<surface::Drawing>, String> {
    let Some(path) = doc_path(request) else {
        return Ok(None);
    };
    let Some(mut body) = opened(dir, repo, &path)? else {
        return Ok(None);
    };
    let edit = flag(query, "edit");
    // the store's own words are edited, never the knowledge base's view of them, which a save would write into the file
    let print = print_of(body.to_string().as_bytes());
    // the knowledge base's view, or, when it cannot be read, the doc's own words: a glossary never takes the page down
    let mut viewed = body.clone();
    match edit {
        true => {}
        false => match crate::knowledge::glossary(dir, &mut viewed) {
            Ok(()) => body = viewed,
            Err(e) => {
                eprintln!("/v/{path}: the glossary as written, the knowledge base not read: {e}")
            }
        },
    }
    let present: usize = (crate::pages::param(query, "present"))
        .and_then(|n| n.parse().ok())
        .unwrap_or(0);
    let title = (body["content"].as_array().into_iter().flatten())
        .find(|b| b["type"] == "meta")
        .and_then(|m| m["attrs"]["title"].as_str())
        .unwrap_or(&path)
        .to_string();
    let page = dash_comments::threads::shown_on(&path);
    let threads = crate::comments::threads_on(&page);
    let (versions, edited) = match flag(query, "versions") && !edit && present == 0 {
        true => {
            let (all, edited) = versions(repo, &path);
            (Some(all), edited)
        }
        false => (None, false),
    };
    let doc = json!({"type": "document", "props": {"kind": "rfc", "body": body, "threads": threads, "page": page,
        "versions": versions, "edited": edited, "edit": edit, "print": print, "present": present}});
    let tree = json!({"root": "doc", "elements": {"doc": doc}}).to_string();
    Ok(Some(surface::Drawing::Tree { title, tree }))
}

/// `POST /v/private/docs/<page>.html`: Edit's Save or a version's Restore on that page, its document's path.
pub fn posted(line: &str) -> Option<String> {
    let mut words = line.split_whitespace();
    (words.next() == Some("POST")).then_some(())?;
    doc_path(words.next()?.split('?').next()?)
}

/// Edit's Save (`b<n>` each passage's words, `print` the document's when opened) or Restore (`restore`, a commit's sha) on
/// `path`, kept by `save` and its file written again; a 303 back to the page, or why not.
pub fn post(dir: &Path, repo: &Path, path: &str, form: &str) -> crate::pages::Response {
    let fields: std::collections::HashMap<String, String> = (form.split('&'))
        .filter_map(|p| p.split_once('='))
        .map(|(k, v)| (crate::pages::form_decoded(k), crate::pages::form_decoded(v)))
        .collect();
    let page = format!("/v/{path}");
    let done = match fields.get("restore") {
        Some(sha) => restore(dir, repo, path, sha),
        None => (|| {
            let kept = stored(dir, path)?
                .ok_or("not read yet: open the page, then edit it")?
                .doc;
            if fields.get("print").map(String::as_str)
                != Some(print_of(kept.to_string().as_bytes()).as_str())
            {
                return Err(format!(
                    "{path} changed since you opened it: open {page}?edit again"
                ));
            }
            match surface::documents::edited(&kept, &|n| fields.get(&format!("b{n}")).cloned()) {
                (_, 0) => Ok(()),
                (doc, _) => save(dir, repo, path, &doc).map(|_| ()),
            }
        })(),
    };
    match done {
        Ok(()) => ("303 See Other", crate::pages::TEXT, page),
        Err(why) => (
            "409 Conflict",
            crate::pages::TEXT,
            format!("Not saved: {why}\n\nBack to the page: {page}\n"),
        ),
    }
}

/// The file as commit `sha` held it, read in and saved as a new version: the older app's `doc_restore`. The version is read from
/// a file beside the page, so its pictures beside it are found, then removed; named `.tmp`, so a kill before its removal leaves
/// nothing `dash doc move` or /v/ reads as a page. The page is read first, so a save over it knows its file.
fn restore(dir: &Path, repo: &Path, path: &str, sha: &str) -> Result<(), String> {
    if !(7..=40).contains(&sha.len()) || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(format!("no version {sha}"));
    }
    opened(dir, repo, path)?;
    let root = repo.to_string_lossy();
    let shown = format!("{sha}:{path}");
    let text = dash_process::run(
        "git",
        &["-C", root.as_ref(), "show", &shown],
        None,
        std::time::Duration::from_secs(30),
    )
    .map_err(|_| format!("no version {sha} of {path}"))?;
    let file = repo.join(path);
    let beside = file.with_file_name(format!(".restore-{sha}.tmp"));
    std::fs::write(&beside, text).map_err(|e| format!("{}: {e}", beside.display()))?;
    let doc = dash_ingest::doc::import(&beside);
    let _ = std::fs::remove_file(&beside);
    save(dir, repo, path, &doc?).map(|_| ())
}

// /v/private/docs/<page>.html: one segment under the folder (pages::find's `/*`); any other /v/ path, a note's, stays the
// older app's
inventory::submit! {
    Page { path: "/v/private/docs/*", step: "route_doc", reads: Reads::Nothing, redraw: false, serve: |a| {
        let read = crate::pages::serve_from_dir().and_then(|dir| page(dir, &crate::pages::repo(), a.path, a.query));
        match read {
            Ok(Some(drawing)) => a.form(Some(Ok(drawing))),
            Ok(None) => ("404 Not Found", crate::pages::TEXT, "no such path".into()),
            Err(e) => a.form(Some(Err(e))),
        }
    } }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::TestDir;
    use dash_meter::dash_test;

    const PATH: &str = "private/docs/rfc-t.html";
    /// A small RFC, each part `dash doc import` keeps: sections, writer notes, a diagram, a fold, a decision with a link.
    const RFC: &str = include_str!("../tests/fixtures/rfc-t.html");

    /// A scratch repo holding the RFC, and the data folder the server reads.
    fn scratch(name: &str) -> (TestDir, PathBuf, PathBuf) {
        let dir = TestDir::new(name);
        let (repo, data) = (dir.0.join("repo"), dir.0.join("data"));
        std::fs::create_dir_all(repo.join("private/docs")).expect("repo");
        std::fs::create_dir_all(&data).expect("data");
        std::fs::write(repo.join(PATH), RFC).expect("the RFC");
        crate::pages::serve_from(&data);
        (dir, repo, data)
    }

    fn forms(data: &Path, repo: &Path) -> (String, String) {
        let drawing = || {
            page(data, repo, &format!("/v/{PATH}"), "")
                .expect("reads")
                .expect("a page")
        };
        let html = crate::pages::drawn(true, false, Some(Ok(drawing()))).2;
        let text = crate::pages::drawn(false, false, Some(Ok(drawing()))).2;
        (dash_surface::markup(&html), text)
    }

    /// Each block as the reader gives it back: its kind, words and ids, so two documents compare with no block ids.
    fn shape(doc: &Value) -> Vec<(String, String, String)> {
        fn walk(v: &Value, out: &mut Vec<(String, String, String)>) {
            for b in v.as_array().into_iter().flatten() {
                let words: String = (b["content"].as_array().into_iter().flatten())
                    .filter_map(|t| t["text"].as_str())
                    .collect();
                let a = &b["attrs"];
                let say = |k: &str| a[k].as_str().unwrap_or_default().to_string();
                let text = [
                    words,
                    say("summary"),
                    say("caption"),
                    say("status"),
                    say("text"),
                ]
                .concat();
                out.push((
                    b["type"].as_str().unwrap_or_default().into(),
                    text,
                    say("anchor"),
                ));
                walk(&b["content"], out);
            }
        }
        let mut out = vec![];
        walk(&doc["content"], &mut out);
        out.retain(|(kind, ..)| kind != "text");
        out
    }

    /// Task editor-in-rfcs, done-when 3: /v/<doc> is the document Rust's store holds, drawn through DocView's Notion view
    /// as the RFC type, its text form read here (dash-surface's test holds its HTML); the file is read once, and a save
    /// writes it again as the document's export, which reads back to the same blocks.
    #[dash_test]
    fn v_serves_a_doc_through_the_notion_view_and_a_save_rewrites_its_file() {
        let (_dir, repo, data) = scratch("docs-v");
        let (_, text) = forms(&data, &repo);
        let lines = [
            "Contents\t1. Intent\t2. The page",
            "RFC: test\n1. Intent\nWriter notes\nHis brief.\nThe check pass drops what is settled.\nWhy rows\nRows are data.",
            "2. The page\ndecided How many?\nOption\tCall\nA\tSee\tintent\t#intent",
            "Comments 0 open of 0",
        ];
        for part in lines {
            assert!(text.contains(part), "{part} in {text}");
        }
        let kept = (stored(&data, PATH).expect("reads"))
            .expect("its first read keeps it")
            .doc;
        assert!(
            forms(&data, &repo).1.contains("drops what is settled."),
            "the store's, read again"
        );

        // an edit to the file by hand (/code-review of fa5aa6443): the page shows it, and a save over an edit the store has
        // not read yet is refused, the file left as he wrote it
        let by_hand = RFC.replace("drops what is settled.", "drops what is settled, by hand.");
        std::fs::write(repo.join(PATH), &by_hand).expect("edit");
        let stale = save(&data, &repo, PATH, &kept).expect_err("a save over an unread edit");
        assert!(stale.contains("changed on disk"), "{stale}");
        assert_eq!(
            std::fs::read_to_string(repo.join(PATH)).expect("kept"),
            by_hand
        );
        assert!(
            forms(&data, &repo)
                .1
                .contains("drops what is settled, by hand."),
            "the edit is read in"
        );
        std::fs::write(repo.join(PATH), RFC).expect("back");
        assert!(
            forms(&data, &repo).1.contains("drops what is settled."),
            "read in again"
        );

        // a save: the paragraph edited, kept, and the file written again as its export
        let edited: Value = serde_json::from_str(&kept.to_string().replace(
            "drops what is settled.",
            "drops what is settled, holds what is gated.",
        ))
        .expect("json");
        let file = save(&data, &repo, PATH, &edited).expect("saves");
        let export = std::fs::read_to_string(&file).expect("the export");
        assert_eq!(
            export,
            surface::documents::export(edited.clone()),
            "the file is the document's export"
        );
        let again = dash_ingest::doc::import(&file).expect("the export reads back");
        assert_eq!(
            shape(&again),
            shape(&edited),
            "the export reads back to the saved blocks"
        );
        assert!(
            forms(&data, &repo).1.contains("holds what is gated."),
            "the page shows the save"
        );

        // a document the Notion view does not read is refused, and nothing is written
        let bad = serde_json::json!({"type": "doc", "content": [{"type": "nope"}]});
        assert!(save(&data, &repo, PATH, &bad).is_err());
        assert_eq!(std::fs::read_to_string(&file).expect("kept"), export);
        assert_eq!(
            doc_path("/v/private/docs/..%2Fnotes%2Fx.html"),
            None,
            "no way out of the folder"
        );
        assert_eq!(
            doc_path("/v/private/notes/a.md"),
            None,
            "another /v/ path stays the older app's"
        );
        assert!(crate::pages::find("/v/private/docs/rfc-t.html").is_some());
        assert!(
            crate::pages::find("/v/private/notes/a.md").is_none(),
            "a note is never this page's"
        );
    }

    /// Done-when 2: the older app's threads move into Rust's store whole, resolved ones too, and reach their page with their
    /// quote, the words DocView anchors them on (dash-surface's `an_imported_rfc_draws_through_the_notion_view` holds that a
    /// thread follows its paragraph when an edit moves it); after a save that moves the paragraph, the page still holds it.
    #[dash_test]
    fn moved_comments_reach_their_page_with_their_quote() {
        let (_dir, repo, data) = scratch("docs-moved");
        let export = serde_json::json!([
            {"id": "t1", "file": PATH, "quote": "drops what is settled", "text": "Why drop?", "date": "2026-11-01",
             "status": "~:resolved", "replies": [{"by": "~:claude", "text": "Settled means decided.", "date": "2026-11-01"}]},
            {"id": "t2", "file": "private/docs/other.html", "quote": "x", "text": "elsewhere", "date": "2026-11-01",
             "status": "~:open", "replies": []},
            {"id": "t3", "file": PATH, "text": "On the whole page.", "date": "2026-11-02", "status": "~:open", "replies": []},
            {"file": PATH, "text": "no id", "date": "2026-11-02", "status": "~:open", "replies": []}
        ]);
        let said = crate::comments::move_from(&data, &export.to_string()).expect("moves");
        assert_eq!(said, "moved 3 threads on 2 pages, 3 written");
        let again = crate::comments::move_from(&data, &export.to_string()).expect("again");
        assert_eq!(
            again, "moved 3 threads on 2 pages, 0 written",
            "a second move writes nothing"
        );

        let threads = |data: &Path| {
            let drawing = page(data, &repo, &format!("/v/{PATH}"), "")
                .expect("reads")
                .expect("a page");
            let surface::Drawing::Tree { tree, .. } = drawing else {
                panic!("a tree")
            };
            let tree: Value = serde_json::from_str(&tree).expect("json");
            tree["elements"]["doc"]["props"]["threads"].clone()
        };
        let on = threads(&data);
        assert_eq!(on[0]["id"], "c-t1");
        assert_eq!(
            on[0]["quote"], "drops what is settled",
            "anchored on its words, never a position"
        );
        assert_eq!(
            on[0]["said"][1]["words"], "Settled means decided.",
            "its reply moved with it"
        );
        assert_eq!(
            (on[0]["open"].clone(), on[1]["open"].clone()),
            (false.into(), true.into()),
            "resolved ones too"
        );
        assert_eq!(
            on.as_array().map(Vec::len),
            Some(2),
            "another page's thread stays there: {on}"
        );
        let (_, text) = forms(&data, &repo);
        for words in [
            "Comments 1 open of 2",
            "Why drop?",
            "Settled means decided.",
            "On the whole page.",
            "Whole page",
        ] {
            assert!(text.contains(words), "{words} in {text}");
        }

        // the paragraph moves under « 2. The page »: its thread still reaches it, by its words
        let mut doc = stored(&data, PATH).expect("reads").expect("kept").doc;
        let blocks = doc["content"].as_array_mut().expect("blocks");
        let from = blocks
            .iter()
            .position(|b| b["type"] == "paragraph")
            .expect("the paragraph");
        let paragraph = blocks.remove(from);
        let to = blocks
            .iter()
            .position(|b| b["attrs"]["anchor"] == "page")
            .expect("section 2")
            + 1;
        blocks.insert(to, paragraph);
        save(&data, &repo, PATH, &doc).expect("saves");
        let (_, text) = forms(&data, &repo);
        let at = |w: &str| text.find(w).unwrap_or_else(|| panic!("{w} in {text}"));
        assert!(
            at("2. The page") < at("The check pass drops what is settled."),
            "{text}"
        );
        assert_eq!(threads(&data)[0]["quote"], "drops what is settled");
    }
    /// A form as a browser posts it: each value percent-encoded, a space a plus.
    fn encoded(pairs: &[(String, String)]) -> String {
        let enc = |t: &str| -> String {
            t.bytes()
                .map(|b| match b {
                    b' ' => "+".to_string(),
                    b if b.is_ascii_alphanumeric() || b"-_.".contains(&b) => {
                        (b as char).to_string()
                    }
                    b => format!("%{b:02X}"),
                })
                .collect()
        };
        let all: Vec<String> = pairs
            .iter()
            .map(|(k, v)| format!("{}={}", enc(k), enc(v)))
            .collect();
        all.join("&")
    }

    /// The page's tree, as the design system is handed it.
    fn tree_of(data: &Path, repo: &Path, query: &str) -> Value {
        let drawing = page(data, repo, &format!("/v/{PATH}"), query)
            .expect("reads")
            .expect("a page");
        let surface::Drawing::Tree { tree, .. } = drawing else {
            panic!("a tree")
        };
        serde_json::from_str(&tree).expect("json")
    }

    fn text_of(data: &Path, repo: &Path, query: &str) -> String {
        let drawing = page(data, repo, &format!("/v/{PATH}"), query)
            .expect("reads")
            .expect("a page");
        crate::pages::drawn(false, false, Some(Ok(drawing))).2
    }

    /// Task rust-doc-parity-rest, done-when 1: a document's glossary is the knowledge base's view, as the older app's
    /// `Knowledge.view/1`: a row whose entry the store holds shows its term linked to /k/<id>, its meaning and source; a link
    /// to an entry the doc has no row for goes to /k/<id>, one to a row it has stays on the page; a row the store lacks keeps
    /// its words; the edit view and a save keep the doc's own words, never the view.
    #[dash_test]
    fn the_glossary_is_the_knowledge_base_s_view_linked_to_k() {
        let (_dir, repo, data) = scratch("docs-glossary");
        // an entry's meaning and source are HTML, as the older app's backup holds them
        let lines = include_str!("../tests/fixtures/knowledge.jsonl");
        assert_eq!(
            crate::knowledge::move_from(&data, lines).expect("moves"),
            "moved 2 entries, 2 written"
        );
        assert_eq!(
            crate::knowledge::move_from(&data, lines).expect("again"),
            "moved 2 entries, 0 written"
        );
        let text = text_of(&data, &repo, "");
        assert!(text.contains("Ask\t/k/ask\tOne item of\tpoint 6"), "{text}");
        // each link of the entry's row, in order, as the view hands it to the design system
        fn hrefs(v: &Value, out: &mut Vec<String>) {
            for m in v["marks"].as_array().into_iter().flatten() {
                out.extend(m["attrs"]["href"].as_str().map(str::to_string));
            }
            for c in v["content"].as_array().into_iter().flatten() {
                hrefs(c, out);
            }
        }
        let body = &tree_of(&data, &repo, "")["elements"]["doc"]["props"]["body"];
        let glossary = (body["content"].as_array().into_iter().flatten())
            .find(|b| b["type"] == "table" && b.to_string().contains("g-ask"))
            .expect("the glossary");
        let ask = (glossary["content"].as_array().into_iter().flatten())
            .find(|r| r["attrs"]["anchor"] == "g-ask")
            .expect("its row");
        let mut links = vec![];
        hrefs(ask, &mut links);
        assert_eq!(
            links,
            [
                "/k/ask",
                "/k/point6",
                "#g-own",
                "#bars",
                "/v/private/docs/cape.html"
            ],
            "its term to /k/, an entry with no row here to /k/, a row here kept, its home's file to /v/"
        );
        assert!(
            text.contains("Own\tOurs alone.\tThis doc"),
            "a row the store lacks keeps its words, never another entry's whose id it begins: {text}"
        );
        assert!(
            text.contains("Own it\t/k/own-it\tThe base says so.\tThe base"),
            "{text}"
        );
        assert!(!text.contains("What the doc says"), "{text}");
        let edit = text_of(&data, &repo, "edit");
        assert!(
            edit.contains("What the doc says an ask is.") && !edit.contains("/k/ask"),
            "the doc's own words: {edit}"
        );
    }

    /// Done-when 2, Edit: each passage a field; Save keeps the one changed, its file written again, the others' links kept;
    /// a save over a document changed since it was opened is refused and nothing is written.
    #[dash_test]
    fn edit_shows_each_passage_and_save_keeps_the_one_changed() {
        let (_dir, repo, data) = scratch("docs-edit");
        assert!(text_of(&data, &repo, "").contains("Edit\tPresent\tthe page alone"));
        // the fields themselves are the design system's: dash-surface's an_imported_rfc_draws_through_the_notion_view
        let edit = text_of(&data, &repo, "edit");
        assert!(
            edit.contains("Change any passage, then Save.\tSave\tCancel"),
            "{edit}"
        );
        let props = &tree_of(&data, &repo, "edit")["elements"]["doc"]["props"];
        let kept = stored(&data, PATH).expect("reads").expect("kept").doc;
        assert_eq!(
            (props["edit"].clone(), props["body"].clone()),
            (true.into(), kept.clone())
        );
        let passages = surface::documents::passages(&kept);
        let at = (passages.iter())
            .position(|(_, w)| w == "The check pass drops what is settled.")
            .expect("the paragraph");
        let print = print_of(kept.to_string().as_bytes());
        let mut form: Vec<(String, String)> = (passages.iter().enumerate())
            .map(|(n, (_, w))| (format!("b{n}"), w.replace('\n', "\r\n")))
            .collect();
        form[at].1 = "The check pass drops what is settled,\r\nand says why.".into();
        // a cell whose link words stay: the link is kept (/code-review of 7c090408e)
        let cell = (passages.iter())
            .position(|(_, w)| w == "See intent")
            .expect("the cell");
        form[cell].1 = "See intent, then decide".into();
        let stale = [vec![("print".to_string(), "0".to_string())], form.clone()].concat();
        let refused = post(&data, &repo, PATH, &encoded(&stale));
        assert_eq!(refused.0, "409 Conflict", "{}", refused.2);
        assert_eq!(
            std::fs::read_to_string(repo.join(PATH)).expect("kept"),
            RFC,
            "nothing written"
        );
        let saved = post(
            &data,
            &repo,
            PATH,
            &encoded(&[vec![("print".into(), print)], form].concat()),
        );
        assert_eq!(
            saved,
            ("303 See Other", crate::pages::TEXT, format!("/v/{PATH}"))
        );
        let file = std::fs::read_to_string(repo.join(PATH)).expect("written");
        assert!(file.contains("and says why."), "{file}");
        assert!(
            text_of(&data, &repo, "").contains("A\tSee\tintent\t, then decide #intent"),
            "a changed cell keeps the link whose words it still holds"
        );
        assert!(
            text_of(&data, &repo, "").contains("and says why."),
            "the page shows the save"
        );
        let kept = stored(&data, PATH).expect("reads").expect("kept").doc;
        assert!(
            kept.to_string()
                .contains(r"drops what is settled,\nand says why."),
            "a CRLF kept as LF: {kept}"
        );
        assert_eq!(
            posted("POST /v/private/docs/rfc-t.html HTTP/1.1").as_deref(),
            Some(PATH)
        );
        assert_eq!(posted("GET /v/private/docs/rfc-t.html HTTP/1.1"), None);
        assert_eq!(posted("POST /v/private/notes/a.md HTTP/1.1"), None);
    }

    /// Done-when 2, Present: one section a screen, Previous, Next and Exit; the first screen is what comes before the first
    /// section.
    #[dash_test]
    fn present_shows_one_section_a_screen() {
        let (_dir, repo, data) = scratch("docs-present");
        let shown = tree_of(&data, &repo, "present=2");
        assert_eq!(shown["elements"]["doc"]["props"]["present"], 2);
        let text = text_of(&data, &repo, "present=2");
        assert!(text.contains("Previous\t2 of 4\tNext\tExit"), "{text}");
        assert!(
            text.contains("1. Intent") && text.contains("The check pass"),
            "{text}"
        );
        assert!(
            !text.contains("2. The page") && !text.contains("Comments"),
            "{text}"
        );
        let last = text_of(&data, &repo, "present=9");
        assert!(
            last.contains("Previous\t4 of 4\tExit") && last.contains("3. Glossary"),
            "{last}"
        );
        assert!(text_of(&data, &repo, "present=1").contains("1 of 4\tNext\tExit"));
    }

    /// Done-when 2, Versions: the file's commits newest first, the current one said, an older one restored as a new version.
    #[dash_test]
    fn versions_list_the_file_s_commits_and_restore_one() {
        let (_dir, repo, data) = scratch("docs-versions");
        assert!(
            text_of(&data, &repo, "").contains("Versions\tShow the versions"),
            "read only when asked: git log is slow"
        );
        assert!(
            text_of(&data, &repo, "versions").contains("Versions 0"),
            "no git: none"
        );
        let root = repo.to_string_lossy().to_string();
        let git = |args: &[&str]| {
            let all = [
                &[
                    "-C",
                    root.as_str(),
                    "-c",
                    "user.name=T",
                    "-c",
                    "user.email=t@t",
                    "-c",
                    "commit.gpgsign=false",
                ][..],
                args,
            ]
            .concat();
            dash_process::run("git", &all, None, std::time::Duration::from_secs(30)).expect("git")
        };
        git(&["init", "-q"]);
        git(&["add", PATH]);
        git(&["commit", "-q", "-m", "First words"]);
        let first = git(&["rev-parse", "HEAD"]).trim().to_string();
        std::fs::write(
            repo.join(PATH),
            RFC.replace("drops what is settled.", "drops what is done."),
        )
        .expect("edit");
        git(&["commit", "-q", "-am", "Second words"]);
        let text = text_of(&data, &repo, "versions");
        assert!(text.contains("Versions 2"), "{text}");
        let at = |w: &str| text.find(w).unwrap_or_else(|| panic!("{w} in {text}"));
        assert!(
            at("Second words") < at("Current")
                && at("Current") < at("First words")
                && at("First words") < at("Restore")
        );
        let restored = post(
            &data,
            &repo,
            PATH,
            &encoded(&[("restore".into(), first.clone())]),
        );
        assert_eq!(restored.0, "303 See Other", "{}", restored.2);
        assert!(
            text_of(&data, &repo, "").contains("drops what is settled."),
            "the first version's words"
        );
        let file = std::fs::read_to_string(repo.join(PATH)).expect("written");
        assert!(file.contains("drops what is settled."), "{file}");
        let after = text_of(&data, &repo, "versions");
        assert!(
            after.contains("Edited since the last version") && !after.contains("Current"),
            "a save is not committed: no commit is current, each restorable: {after}"
        );
        let none = std::fs::read_dir(repo.join("private/docs"))
            .expect("dir")
            .count();
        assert_eq!(none, 1, "no version file left beside it");
        assert_eq!(post(&data, &repo, PATH, "restore=zz").0, "409 Conflict");
    }

    /// One model (rust/CLAUDE.md rule 7): every block `dash doc import` makes reads in the Notion view's schema unchanged, so
    /// the two definitions of those blocks (dash_ingest::doc, dash-surface's notion_doc) never drift apart.
    #[dash_test]
    fn an_imported_doc_loads_in_the_notion_view_unchanged() {
        let dir = TestDir::new("docs-schema");
        let fixture =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../ingest/tests/fixtures/rfc-inbox.html");
        std::fs::write(dir.0.join("rfc.html"), RFC).expect("the RFC");
        for file in [fixture, dir.0.join("rfc.html")] {
            let doc = dash_ingest::doc::import(&file).expect("imports");
            assert_eq!(
                surface::documents::loads(&doc).expect("loads"),
                doc,
                "{}",
                file.display()
            );
        }
    }
}
