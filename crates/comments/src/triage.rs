//! `POST /comment/page`: what the Chrome extension sends, a comment on an element or a capture of a page, is a task in triage
//! labelled from-extension, never started by a run, its files in the live folder (task capture-extension, item 1 of
//! rfc-chrome-extension section 8; @nha, 2026-10-29: « just log a normal task, perhaps with an initial label to show the
//! provenance. status in triage could be a good fit, linear-style »). This module reads the post into its task; the binary
//! files it in Rust's task store (`SqliteTasks::triage`). It replaces the older app's route, which put them in the comment queue.
use dash_core::tasks::Triaged;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU32, Ordering};

/// The extension's origin, pinned by its manifest's `key`: the only page that posts here; a post with none is curl's.
pub const EXTENSION_ORIGIN: &str = "chrome-extension://dlalkjdbddiicogpifbmipffemoggjng";
pub const PATH: &str = "/comment/page";
pub const REFUSED_ORIGIN: &str = "refused: only the extension posts here";
/// A file at most, the older route's limit; a capture's page and its screenshot together, the body's.
pub const FILE_MOST: usize = 25 * 1024 * 1024;
pub const BODY_MOST: usize = 2 * FILE_MOST + 64 * 1024;
/// The time a body takes to arrive at most: the outbox's own limit for a capture that size, a minute plus one per 5 MB
/// (/code-review: the serve's 5 s cut a capture on a slow link, and its 400 was final for the outbox).
pub const BODY_TIME: std::time::Duration = std::time::Duration::from_secs(12 * 60);
/// Where a sending's files are kept in the data folder: `triage-files/<task id>/`.
const FILES: &str = "triage-files";
const TITLE_MOST: usize = 80;

/// `POST /comment/page`, whatever its query.
pub fn asked(line: &str) -> bool {
    let mut words = line.split_whitespace();
    words.next() == Some("POST")
        && words
            .next()
            .is_some_and(|t| t.split('?').next() == Some(PATH))
}

/// Whether `origin` may post: the extension, or no browser at all.
pub fn allowed(origin: Option<&str>) -> bool {
    origin.is_none_or(|o| o == EXTENSION_ORIGIN)
}

/// A web page's address without its query string, fragment and user:password@, the parts a token or a session id rides in: the
/// task's text is copied to Linear (rfc-chrome-extension section 7; /code-review: userinfo was kept).
pub fn bare(url: &str) -> String {
    let url = url.split(['?', '#']).next().unwrap_or(url);
    match url.split_once("://") {
        Some((scheme, rest)) => {
            let (host, path) = rest.find('/').map_or((rest, ""), |at| rest.split_at(at));
            let host = host.rsplit_once('@').map_or(host, |(_, h)| h);
            format!("{scheme}://{host}{path}")
        }
        None => url.to_string(),
    }
}

/// The answer the outbox reads: the task's id.
pub fn answer(id: &str) -> String {
    serde_json::json!({ "id": id }).to_string()
}

/// The sending's id: the extension's key (one per item, sent again with it after a lost answer), so a resend is the same task.
/// A key a task id cannot spell is hashed whole, never cut or folded, so two keys are never one task (/code-review).
fn id_of(key: Option<&str>, now_ms: i64) -> String {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let id_safe =
        |k: &str| k.len() <= 64 && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    match key.unwrap_or_default() {
        "" => format!("ext-{now_ms:x}-{}", NEXT.fetch_add(1, Ordering::Relaxed)),
        key if id_safe(key) => format!("ext-{key}"),
        // FNV-1a: the same key gives the same id in every build
        key => {
            let mut hash = dash_core::Fnv::new();
            hash.bytes(key.as_bytes());
            format!("ext-h{:016x}", hash.finish())
        }
    }
}

/// The task its first line names, cut at a word.
fn title_of(text: &str, quote: &str, page: &str, capture: bool) -> String {
    // his words first, a capture's too (/code-review of PR 1615); a screenshot alone is a comment, not a capture (/code-review)
    let line = text.lines().map(str::trim).find(|l| !l.is_empty());
    let said = line.map_or_else(
        || {
            let what = if quote.trim().is_empty() {
                page
            } else {
                quote.trim()
            };
            let lead = if capture { "Capture:" } else { "Comment on:" };
            format!("{lead} {what}")
        },
        str::to_string,
    );
    crate::one_line(&said, TITLE_MOST)
}

/// The sending's words: his comment, the element's quote and selector, and the page's address without its query string.
fn text_of(text: &str, quote: &str, selector: &str, page: &str) -> String {
    let mut out = vec![];
    for (lead, said) in [("", text), ("> ", quote), ("At: ", selector)] {
        if !said.trim().is_empty() {
            out.push(format!("{lead}{}", said.trim()));
        }
    }
    out.push(format!("Page: {page}"));
    out.join("\n\n")
}

/// The port of a page this Mac serves (`localhost:<port>`, `127.0.0.1:<port>`), which a task's worktree may.
fn local_port(url: &str) -> Option<u16> {
    let host_port = url.split_once("://")?.1.split(['/', '?', '#']).next()?;
    let (host, port) = host_port.rsplit_once(':')?;
    matches!(host, "localhost" | "127.0.0.1" | "[::1]").then(|| port.parse().ok())?
}

/// The post's fields and its files' names and sizes read into its task, or why not with the status the outbox reads (400 and
/// 413 are final there, any other is sent again). `task_serving` names the task whose worktree serves a local port.
pub fn sending(
    fields: &BTreeMap<String, String>,
    files: &[(String, usize)],
    now_ms: i64,
    task_serving: impl Fn(u16) -> Option<String>,
) -> Result<Triaged, (&'static str, String)> {
    let field = |name: &str| fields.get(name).map_or("", String::as_str);
    let url = field("url");
    if !(url.starts_with("http://") || url.starts_with("https://"))
        || url.contains(char::is_control)
    {
        return Err(("400 Bad Request", format!("not a web page's URL: {url:?}")));
    }
    if field("text").trim().is_empty() && files.is_empty() {
        return Err(("400 Bad Request", "an empty comment".into()));
    }
    if files.iter().any(|(_, size)| *size > FILE_MOST) {
        return Err(("413 Content Too Large", "A file over 25 MB.".into()));
    }
    let (page, text, quote) = (&bare(url), field("text"), field("quote"));
    let id = id_of(fields.get("key").map(String::as_str), now_ms);
    let files = (files.iter().enumerate())
        .map(|(i, (name, _))| format!("{FILES}/{id}/{}", crate::kept_name(i, name)))
        .collect();
    Ok(Triaged {
        title: title_of(text, quote, page, field("tag") == "capture"),
        text: text_of(text, quote, field("selector"), page),
        page: page.into(),
        files,
        task: local_port(page).and_then(task_serving),
        id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;

    fn form(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn none(_: u16) -> Option<String> {
        None
    }

    /// Done-when 3: the task's text keeps the comment and the page's address without its query string (nor its fragment); a
    /// capture is titled by its page; each file has its place in the live folder's triage-files; a key is the same task.
    #[dash_test]
    fn a_sending_keeps_the_comment_and_the_bare_address_and_files_its_files_under_its_id() {
        let comment = form(&[
            (
                "url",
                "https://example.com/pricing?session=s3cr3t&utm=x#plans",
            ),
            ("selector", "#plans > li:nth-of-type(2)"),
            ("quote", "Pro"),
            ("text", "cheaper here"),
            ("key", "1b9d6bcd-bbfd-4b2d-9b5d-ab8dfbbd4bed"),
        ]);
        let t = sending(&comment, &[("screenshot.png".into(), 9)], 1, none).expect("a task");
        assert_eq!(t.id, "ext-1b9d6bcd-bbfd-4b2d-9b5d-ab8dfbbd4bed");
        assert_eq!(
            (t.title.as_str(), t.page.as_str()),
            ("cheaper here", "https://example.com/pricing")
        );
        let text = "cheaper here\n\n> Pro\n\nAt: #plans > li:nth-of-type(2)\n\nPage: https://example.com/pricing";
        assert_eq!(t.text, text);
        assert_eq!(t.files, [format!("triage-files/{}/0-screenshot.png", t.id)]);
        assert_eq!(sending(&comment, &[], 2, none).expect("again").id, t.id);

        let capture = form(&[
            ("url", "https://example.com/post?ref=mail"),
            ("quote", "A post"),
            ("tag", "capture"),
        ]);
        let files = [
            ("screenshot.png".to_string(), 3),
            ("../page.html".to_string(), 5),
        ];
        let t = sending(&capture, &files, 3, none).expect("a task");
        assert_eq!(
            (t.title.as_str(), t.text.as_str()),
            (
                "Capture: A post",
                "> A post\n\nPage: https://example.com/post"
            )
        );
        let names: Vec<_> = t
            .files
            .iter()
            .map(|f| f.rsplit('/').next().unwrap())
            .collect();
        assert_eq!(names, ["0-screenshot.png", "1-page.html"]);
        let keyed = |key: &str| {
            sending(
                &form(&[("url", "https://e.x/"), ("text", "x"), ("key", key)]),
                &[],
                5,
                none,
            )
        };
        let ids: std::collections::BTreeSet<_> = [
            "k-1",
            "k1",
            "AB",
            "ab",
            "a b",
            "a_b",
            &"z".repeat(70),
            &format!("{}y", "z".repeat(69)),
        ]
        .iter()
        .map(|k| keyed(k).expect("a task").id)
        .collect();
        assert_eq!(ids.len(), 8, "two keys are never one task: {ids:?}");
        let shot_only = sending(
            &form(&[("url", "https://e.x/a"), ("quote", "Pro")]),
            &[("s.png".into(), 1)],
            6,
            none,
        );
        assert_eq!(shot_only.expect("a task").title, "Comment on: Pro");
        // his words title a capture too; a page's login never hides its port's task (/code-review of PR 1615)
        let said = form(&[
            ("url", "http://u:p@localhost:4001/a"),
            ("text", "pricing is wrong"),
            ("quote", "Pro"),
            ("tag", "capture"),
        ]);
        let served = |port: u16| (port == 4001).then(|| "rust-x".to_string());
        let t = sending(&said, &[], 7, served).expect("a task");
        assert_eq!(
            (t.title.as_str(), t.task.as_deref()),
            ("pricing is wrong", Some("rust-x"))
        );
        let secret = sending(
            &form(&[("url", "https://user:t0k@e.x:8/p?q#f"), ("text", "x")]),
            &[],
            7,
            none,
        );
        assert_eq!(secret.expect("a task").page, "https://e.x:8/p");
        let long = "a".repeat(200);
        let t = sending(
            &form(&[("url", "https://e.x/"), ("text", &long)]),
            &[],
            4,
            none,
        )
        .expect("a task");
        assert_eq!(t.title.chars().count(), TITLE_MOST + 1);
    }

    /// A page a task's worktree serves names that task, as the older route did; any other host names none.
    #[dash_test]
    fn a_local_page_names_the_task_serving_its_port() {
        let serving = |port: u16| (port == 4001).then(|| "port-demo".to_string());
        let on = |url: &str| {
            sending(&form(&[("url", url), ("text", "x")]), &[], 1, serving)
                .expect("a task")
                .task
        };
        assert_eq!(
            on("http://localhost:4001/app?x=1").as_deref(),
            Some("port-demo")
        );
        assert_eq!(on("http://127.0.0.1:4001/").as_deref(), Some("port-demo"));
        assert_eq!(on("https://example.com:4001/"), None);
        assert_eq!(on("http://localhost:4002/"), None);
    }

    #[dash_test]
    fn a_post_that_is_not_a_web_page_says_nothing_or_is_too_big_is_refused_and_only_the_extension_posts()
     {
        for pairs in [
            &[("url", "/board"), ("text", "x")][..],
            &[("url", "https://example.com/"), ("text", "  ")][..],
        ] {
            assert_eq!(
                sending(&form(pairs), &[], 1, none).expect_err("refused").0,
                "400 Bad Request"
            );
        }
        let big = [("a.png".to_string(), FILE_MOST + 1)];
        let refused =
            sending(&form(&[("url", "https://example.com/")]), &big, 1, none).expect_err("too big");
        assert_eq!(refused.0, "413 Content Too Large");
        assert!(
            allowed(None)
                && allowed(Some(EXTENSION_ORIGIN))
                && !allowed(Some("https://evil.example"))
        );
        assert!(asked("POST /comment/page?x HTTP/1.1") && !asked("GET /comment/page HTTP/1.1"));
        assert_eq!(answer("ext-a"), r#"{"id":"ext-a"}"#);
    }
}
