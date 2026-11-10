//! /search, the menu's search: the leads, the tasks, his notes, the store's facts and Flare's hits, each kind its own part
//! (task rust-search-port, `DashboardWeb.FindLive`'s port; @nha, 2026-11-10: « remember we move to rust. no more elixir, so if
//! something needs fixing port and then fix »). Answered on its connection's thread: Flare takes 1 to 4 s, and no other
//! request waits on it (rust/CLAUDE.md rule 5). /inbox's field (`in=inbox`) stays the serial route's, its results on /inbox.
use crate::pages::{self, Asked, Page, Reads};
use dash_core::Store;
use dash_store::older;
use dash_surface::{self as surface, Drawing};
use serde_json::{Map, Value, json};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Its path, which the route's /inbox exception names too; its registration below spells it, so dash-gate reads it.
pub const PAGE: &str = "/search";
/// The store's facts a search shows, section 8's rerank window.
pub const LIMIT: u32 = 40;
/// The results a part shows: two letters in two words (« a e ») match nearly every row, as FindLive's did.
const PART: usize = 50;
/// Flare's hits a search asks for, as FindLive asked.
const FLARE_HITS: u32 = 8;
/// Past this, Flare's answer is no answer (FindLive's `@budget`); its part says so.
const FLARE_BUDGET: Duration = Duration::from_secs(3);

inventory::submit! {
    Page { path: "/search", step: "route_search", reads: Reads::Nothing, redraw: true, serve: |a| a.form(Some(Ok(answer(a)))) }
}

/// A result: its title, the line under it and where it opens.
#[derive(Debug, PartialEq)]
struct Found {
    title: String,
    line: String,
    href: Option<String>,
}

fn answer(asked: &Asked) -> Drawing {
    let q = asked.param("q").unwrap_or_default();
    let repo = pages::repo();
    let parts = words(&q).map(|words| {
        // Flare asked beside the other reads, so the page waits on the slower of the two, never their sum (/code-review).
        // ponytail: the page still waits on Flare, up to its budget; FindLive drew its own hits first. A part that refills
        // (the design system's reading card) once Flare's wait shows on the page's trace.
        std::thread::scope(|scope| {
            let asked = scope.spawn(|| flare(q.trim(), &repo));
            let (leads, tasks) = match leads_and_tasks(&crate::tasks::older_db(), &words) {
                Ok(both) => (Ok(both.0), Ok(both.1)),
                Err(e) => (Err(e.clone()), Err(e)),
            };
            let notes = notes(&repo.join("private/notes"), &repo, &words);
            let facts = facts(q.trim());
            let flare = asked
                .join()
                .unwrap_or_else(|_| Err("its read panicked".into()));
            vec![
                ("leads", "Leads", leads),
                ("tasks", "Tasks", tasks),
                ("notes", "Notes", notes),
                ("facts", "In the store", facts),
                ("flare", "Flare", flare),
            ]
        })
    });
    page(&q, parts.unwrap_or_default())
}

/// A part of the page: its key, its title and what it found, or why it was not read.
type Part<'a> = (&'a str, &'a str, Result<Vec<Found>, String>);

/// The page: its field holding the words, then each part that found something, Flare's always, saying why when it did not
/// answer (FindLive's « Flare did not answer »).
fn page(q: &str, parts: Vec<Part>) -> Drawing {
    let mut elements = Map::new();
    let mut children = vec!["title".to_string(), "search".to_string()];
    elements.insert(
        "title".into(),
        json!({"type": "heading", "props": {"text": "Search"}}),
    );
    elements.insert(
        "search".into(),
        json!({"type": "search-bar", "props": {"placeholder": "Leads, tasks, notes and Flare", "value": q}}),
    );
    let text = |t: String| json!({"type": "text", "props": {"text": t}});
    let searched = !parts.is_empty();
    // found in a part of ours, or a part of ours not read: either way « nothing found » would be untrue (/code-review)
    let (mut any, mut unread) = (false, false);
    for (key, title, found) in parts {
        let mut title = title.to_string();
        unread |= key != "flare" && found.is_err();
        let inside: Value = match found {
            Ok(found) if found.is_empty() && key != "flare" => continue,
            Ok(found) if found.is_empty() => text("Flare holds nothing for these words.".into()),
            Err(e) if key == "flare" => text(format!("Flare did not answer: {}.", one_line(&e))),
            Err(e) => text(format!("Not read: {}.", one_line(&e))),
            Ok(found) => {
                any |= key != "flare";
                let mut rows = vec![];
                if found.len() > PART {
                    title = format!("{title}, the first {PART} of {}", found.len());
                }
                for (i, f) in found.into_iter().take(PART).enumerate() {
                    let id = format!("{key}-{i}");
                    let card = json!({"type": "card", "props": {"title": one_line(&f.title), "line": one_line(&f.line), "href": f.href}});
                    elements.insert(id.clone(), card);
                    rows.push(id);
                }
                json!({"type": "list", "children": rows})
            }
        };
        let inner = format!("{key}-found");
        elements.insert(inner.clone(), inside);
        elements.insert(
            key.into(),
            json!({"type": "section", "props": {"title": title}, "children": [inner]}),
        );
        children.push(key.into());
    }
    if searched && !any && !unread {
        elements.insert(
            "none".into(),
            text(format!("Nothing of ours found for « {} ».", q.trim())),
        );
        children.insert(2, "none".into());
    }
    elements.insert(
        "page".into(),
        json!({"type": "stack", "children": children}),
    );
    Drawing::Tree {
        title: "Search".into(),
        tree: json!({"root": "page", "elements": elements}).to_string(),
    }
}

/// The words of `q` folded, each a result's text must hold; none under two letters in all, as FindLive.
fn words(q: &str) -> Option<Vec<String>> {
    let words: Vec<String> = fold(q).split_whitespace().map(String::from).collect();
    (words.iter().map(|w| w.chars().count()).sum::<usize>() >= 2).then_some(words)
}

/// Lowercase, accents dropped: « Équipe » is found by « equipe ».
/// ponytail: Latin accents only, no Unicode decomposition; a crate for it once a word in another script goes unfound.
fn fold(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .map(|c| match c {
            'à' | 'â' | 'ä' | 'á' | 'ã' | 'å' => 'a',
            'ç' => 'c',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'î' | 'ï' | 'í' | 'ì' => 'i',
            'ô' | 'ö' | 'ó' | 'ò' | 'õ' => 'o',
            'ù' | 'û' | 'ü' | 'ú' => 'u',
            'ÿ' => 'y',
            'ñ' => 'n',
            c => c,
        })
        .collect()
}

fn holds(text: &str, words: &[String]) -> bool {
    let text = fold(text);
    words.iter().all(|w| text.contains(w.as_str()))
}

/// The leads, then the tasks, of the older app's database whose title, id, line or words hold every word.
fn leads_and_tasks(db: &Path, words: &[String]) -> Result<(Vec<Found>, Vec<Found>), String> {
    let (mut leads, mut tasks) = (vec![], vec![]);
    for row in older::searchable(db)? {
        if !holds(
            &[&row.title, &row.id, &row.line, &row.words]
                .map(String::as_str)
                .join(" "),
            words,
        ) {
            continue;
        }
        let lead = row.kind == "lead";
        let found = Found {
            href: Some(match lead {
                // the older app's lead page, through the front door
                true => format!("/o/{}", surface::segment(&row.id)),
                false => surface::task_href(&row.id, None),
            }),
            title: mentions(&row.title),
            line: cut(&mentions(&row.line), 90),
        };
        if lead {
            leads.push(found)
        } else {
            tasks.push(found)
        }
    }
    Ok((leads, tasks))
}

/// His notes, private/notes' Markdown, by heading, path and text, each at its page, its line the first one holding every
/// word, else its path. ponytail: every note read at each search (57 files, 536 KB on 2026-11-10); held by mtime past a few MB.
fn notes(folder: &Path, repo: &Path, words: &[String]) -> Result<Vec<Found>, String> {
    let mut files = vec![];
    markdown(folder, &mut files);
    files.sort();
    let mut found = vec![];
    for file in files {
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        let rel = file
            .strip_prefix(repo)
            .unwrap_or(&file)
            .to_string_lossy()
            .into_owned();
        let heading = text
            .lines()
            .find_map(|l| l.strip_prefix("# "))
            .map(str::trim);
        let title = heading.map_or_else(
            || rel.rsplit('/').next().unwrap_or(&rel).to_string(),
            String::from,
        );
        if !holds(&format!("{title} {rel} {text}"), words) {
            continue;
        }
        // the first line holding every word, else one holding any
        let lines = || text.lines().map(str::trim).filter(|l| !l.starts_with('#'));
        let line = (lines().find(|l| holds(l, words)))
            .or_else(|| lines().find(|l| words.iter().any(|w| fold(l).contains(w.as_str()))));
        found.push(Found {
            title,
            line: line.map_or_else(|| rel.clone(), |l| l.chars().take(240).collect()),
            href: Some(viewed(&rel)),
        });
    }
    Ok(found)
}

/// A file of the checkout at its page, each segment of its path encoded: a « # » or « ? » in a name stays the name's.
fn viewed(rel: &str) -> String {
    let segments: Vec<String> = rel.split('/').map(surface::segment).collect();
    format!("/v/{}", segments.join("/"))
}

fn markdown(dir: &Path, into: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        match entry.file_type() {
            Ok(t) if t.is_dir() => markdown(&path, into),
            Ok(t) if t.is_file() && path.extension().is_some_and(|e| e == "md") => into.push(path),
            _ => {}
        }
    }
}

/// The store's facts holding the words, `dash ingest`'s, read on a connection of this thread's own.
/// ponytail: no rerank hook orders them here: the hooks live on the serial route's registry, and no extension installs one
/// live (private/live/rust/data holds no extensions.d, 2026-11-10); hand the registry over once one does. /inbox's field keeps them.
fn facts(q: &str) -> Result<Vec<Found>, String> {
    let db = dash_store::open_read(pages::serve_from_dir()?).map_err(|e| e.to_string())?;
    let hits = db.search(q, LIMIT)?;
    Ok(hits
        .into_iter()
        .map(|h| Found {
            line: if h.body.is_empty() {
                format!("fact {}", h.id)
            } else {
                // a fact is a whole file `dash ingest` stored, up to 4,000 characters in a hit
                cut(&one_line(&h.body), 240)
            },
            title: h.title,
            href: None,
        })
        .collect())
}

/// Flare's hits for `q`, asked as a program asks it (graal's F5): `POST /api/tools/flare.search` on the port the running app
/// wrote in its data folder (`FLARE_DATA_DIR`, else its macOS one), as `Dashboard.Flare.tool/2` did. A file of this checkout
/// links to its page.
fn flare(q: &str, repo: &Path) -> Result<Vec<Found>, String> {
    let dir = std::env::var("FLARE_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_default();
            Path::new(&home).join("Library/Application Support/com.flare.local")
        });
    let file = dir.join("mcp.port");
    let port: u16 = std::fs::read_to_string(&file)
        .map_err(|e| format!("Flare is not running ({}: {e})", file.display()))?
        .trim()
        .parse()
        .map_err(|e| format!("{}: {e}", file.display()))?;
    let body = json!({"query": q, "limit": FLARE_HITS}).to_string();
    let failed = |e: std::io::Error| format!("Flare on port {port}: {e}");
    // one deadline for the whole exchange, as FindLive's: each step waits only what is left of it (/code-review)
    let deadline = std::time::Instant::now() + FLARE_BUDGET;
    let left = || {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        (!left.is_zero()).then_some(left).ok_or(format!(
            "Flare gave no answer in {} s",
            FLARE_BUDGET.as_secs()
        ))
    };
    let mut stream =
        TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), left()?).map_err(failed)?;
    stream.set_write_timeout(Some(left()?)).map_err(failed)?;
    // HTTP/1.0: the answer comes whole and the connection closes, never chunked. One write: `write!` on a socket sends each
    // piece of its format as it goes
    let request = format!(
        "POST /api/tools/flare.search HTTP/1.0\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).map_err(failed)?;
    let mut answer = vec![];
    let mut piece = [0u8; 64 * 1024];
    loop {
        stream.set_read_timeout(Some(left()?)).map_err(failed)?;
        match stream.read(&mut piece).map_err(failed)? {
            0 => break,
            n if answer.len() + n > 4 << 20 => return Err("Flare's answer is past 4 MB".into()),
            n => answer.extend_from_slice(&piece[..n]),
        }
    }
    let answer = String::from_utf8_lossy(&answer);
    let (head, json) = answer
        .split_once("\r\n\r\n")
        .ok_or("Flare's answer has no body")?;
    if head.split_whitespace().nth(1) != Some("200") {
        return Err(format!(
            "Flare answered {}",
            head.lines().next().unwrap_or_default()
        ));
    }
    let json: Value =
        serde_json::from_str(json).map_err(|_| "an answer that is not Flare's".to_string())?;
    let results = json["result"]["results"]
        .as_array()
        .ok_or("an answer with no results")?;
    let text = |r: &Value, keys: &[&str]| {
        keys.iter()
            .find_map(|k| r[*k].as_str().filter(|s| !s.is_empty()))
            .map(String::from)
    };
    Ok(results
        .iter()
        .map(|r| Found {
            title: text(r, &["title", "name"]).unwrap_or_else(|| "Flare hit".into()),
            line: text(r, &["snippet", "excerpt", "summary"])
                .unwrap_or_default()
                .chars()
                .take(280)
                .collect(),
            href: r["path"]
                .as_str()
                .and_then(|p| Path::new(p).strip_prefix(repo).ok())
                .map(|rel| rel.to_string_lossy().into_owned())
                .filter(|rel| !rel.split('/').any(|s| s == ".."))
                .map(|rel| viewed(&rel)),
        })
        .collect())
}

/// A mention of the older app's text, `\u{e000}m:contacts:yann-prigent\u{e001}`, as its id: the markers are no letters to show.
/// ponytail: the id, not the name it stands for; a read of the contacts once a page shows names.
fn mentions(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(at) = rest.find('\u{e000}') {
        out.push_str(&rest[..at]);
        let inside = &rest[at + '\u{e000}'.len_utf8()..];
        let end = inside.find('\u{e001}').unwrap_or(inside.len());
        out.push_str(inside[..end].rsplit(':').next().unwrap_or_default());
        rest = inside
            .get(end + '\u{e001}'.len_utf8()..)
            .unwrap_or_default();
    }
    out + rest
}

/// One line under a title, or it pushes the next result off a phone's screen (FindLive's `one_line/1`).
fn cut(s: &str, n: usize) -> String {
    match s.char_indices().nth(n) {
        Some((at, _)) => format!("{}…", &s[..at]),
        None => s.to_string(),
    }
}

/// One line: a newline or a control character in a title or snippet would break a text form's record.
fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::TestDir;
    use dash_meter::memory::TestBudget;

    fn w(q: &str) -> Vec<String> {
        words(q).expect("words")
    }

    /// A note is found by its heading, path or text, accents aside, its line the first holding every word.
    #[test]
    fn a_note_is_found_by_its_heading_or_text_with_its_line() {
        let _budget = TestBudget::new();
        let dir = TestDir::new("search-notes");
        let notes = dir.0.join("private/notes/sub");
        std::fs::create_dir_all(&notes).expect("folder");
        std::fs::write(
            notes.join("call.md"),
            "# Call with Équipe\n\nWe spoke.\nThe budget is fine.\n",
        )
        .expect("note");
        std::fs::write(dir.0.join("private/notes/other.md"), "nothing here\n").expect("note");
        let folder = dir.0.join("private/notes");
        let found = notes_of(&folder, &dir.0, "equipe budget");
        assert_eq!(
            found,
            [Found {
                title: "Call with Équipe".into(),
                line: "The budget is fine.".into(),
                href: Some("/v/private/notes/sub/call.md".into()),
            }]
        );
        assert_eq!(notes_of(&folder, &dir.0, "other").len(), 1, "by its path");
        assert!(notes_of(&folder, &dir.0, "zzz").is_empty());
        assert_eq!(words("a"), None, "one letter searches nothing");
        assert_eq!(
            mentions("Originator selon \u{e000}m:contacts:yann-prigent\u{e001}, ok \u{e000}broken"),
            "Originator selon yann-prigent, ok broken"
        );
        assert_eq!(cut("abcdef", 3), "abc…");
        assert_eq!(
            viewed("private/notes/a b#1.md"),
            "/v/private/notes/a%20b%231.md"
        );
        assert_eq!(cut("abc", 3), "abc");
    }

    fn notes_of(folder: &Path, repo: &Path, q: &str) -> Vec<Found> {
        notes(folder, repo, &w(q)).expect("notes")
    }

    /// The page: the field holds the words, a part with nothing found is left out, Flare's says why it did not answer.
    #[test]
    fn the_page_shows_each_part_found_and_flare_s_failure() {
        let _budget = TestBudget::new();
        let found = |t: &str| Found {
            title: t.into(),
            line: "a line".into(),
            href: Some("/o/x".into()),
        };
        let parts = vec![
            ("leads", "Leads", Ok(vec![found("A lead")])),
            ("tasks", "Tasks", Ok(vec![])),
            ("flare", "Flare", Err("Flare is not running".into())),
        ];
        let Drawing::Tree { tree, .. } = page("lead", parts) else {
            panic!("a tree")
        };
        let text = surface::text(&tree).expect("its text");
        assert_eq!(
            text,
            "Search\nSearch: lead\n\nLeads\nA lead\ta line\t/o/x\n\nFlare\nFlare did not answer: Flare is not running.\n"
        );
    }
}
