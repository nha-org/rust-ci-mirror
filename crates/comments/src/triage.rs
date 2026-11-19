//! `POST /comment/page`: what the Chrome extension sends, a comment on an element or a capture of a page, is a task in triage
//! labelled from-extension, never started by a run, its files in the live folder (task capture-extension, item 1 of
//! rfc-chrome-extension section 8; @nha, 2026-10-29: « just log a normal task, perhaps with an initial label to show the
//! provenance. status in triage could be a good fit, linear-style »). This module reads the post into its task; the binary
//! files it in Rust's task store (`SqliteTasks::triage`). It replaces the older app's route, which put them in the comment queue.
//!
//! A page on flare.nicolas-ha.com is a task of site_perso's flare-site sub-project, its URL and selector kept (task
//! flare-site-comments, item 3; @nha, 2026-11-01: « I'd like to leave comments in https://flare.nicolas-ha.com/ via the chrome
//! extension … in a sub-project in perso »).
//!
//! Sync: the store is this Mac's SQLite and the extension posts to it alone, so with the Mac off a comment waits in the
//! extension's outbox, retried each minute. No relay and no sync code here: the later path for the Mac off is a synced store,
//! task rfc-dash-crsqlite (cr-sqlite behind dash.nicolas-ha.com); rfc-chrome-extension section 6 kept nicolas-ha.com out.
use dash_core::Triaged;
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
/// Each host whose pages are a sub-project's, and that sub-project: `<project>/<sub-project>`.
const HOST_PROJECTS: &[(&str, &str)] = &[("flare.nicolas-ha.com", "site_perso/flare-site")];
/// Where a sending's files are kept in the data folder: `triage-files/<task id>/`.
const FILES: &str = "triage-files";
const TITLE_MOST: usize = 80;

/// `POST /comment/page`, whatever its query.
pub fn asked(line: &str) -> bool {
    posts_to(line, PATH)
}

/// A request line posting to `path`, whatever its query.
pub fn posts_to(line: &str, path: &str) -> bool {
    let mut words = line.split_whitespace();
    words.next() == Some("POST")
        && words
            .next()
            .is_some_and(|t| t.split('?').next() == Some(path))
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
            // a browser ends an http(s) host at `\` as at `/`: `evil.example\@flare...` is evil.example (/code-review of #1629)
            let (host, path) = rest
                .find(['/', '\\'])
                .map_or((rest, ""), |at| rest.split_at(at));
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

/// Whether a post carries a HAR: the panel's tag, or a file named as one.
fn har(fields: &BTreeMap<String, String>, files: &[(String, usize)]) -> bool {
    let named = |n: &str| {
        let n = n.to_ascii_lowercase();
        n.ends_with(".har") || n.ends_with(".har.gz")
    };
    fields.get("tag").is_some_and(|t| t == "har") || files.iter().any(|(n, _)| named(n))
}

/// Why a post's HAR is refused, its session cookies kept off the Mac: a page whose host is not `har_hosts`' or a subdomain of
/// one (task extension-har-panel; the panel checks the same before it sends, har.js). The app checks it again, since a HAR is
/// the one sending that holds his sessions.
pub fn har_refused(
    fields: &BTreeMap<String, String>,
    files: &[(String, usize)],
    har_hosts: &[String],
) -> Option<String> {
    if !har(fields, files) {
        return None;
    }
    let page = bare(fields.get("url").map_or("", String::as_str));
    let host = host_of(&page).unwrap_or_default();
    let held = |d: &String| host == *d || host.ends_with(&format!(".{d}"));
    (!har_hosts.iter().any(held))
        .then(|| format!("no HAR from {host}: the capture policy gives none there"))
}

/// The task its first line names, cut at a word.
fn title_of(text: &str, quote: &str, page: &str, tag: &str) -> String {
    // his words first, a capture's too (/code-review of PR 1615); a screenshot alone is a comment, not a capture (/code-review)
    let line = text.lines().map(str::trim).find(|l| !l.is_empty());
    let said = line.map_or_else(
        || {
            let what = if quote.trim().is_empty() {
                page
            } else {
                quote.trim()
            };
            let lead = match tag {
                "capture" => "Capture:",
                "har" => "HAR:",
                _ => "Comment on:",
            };
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

/// A bare page's host and port (`bare` dropped its login, query and fragment): what both the routings below read.
fn host_port(page: &str) -> Option<&str> {
    page.split_once("://")?
        .1
        .split(['/', '\\', '?', '#'])
        .next()
}

/// A bare page's host, lowercased, without its port or a trailing dot: the one parse both the sub-project routing and the HAR
/// check read (/code-review: two copies had drifted on an empty port).
fn host_of(page: &str) -> Option<String> {
    let host_port = host_port(page)?;
    let port = |(_, p): &(&str, &str)| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit());
    let host = (host_port.rsplit_once(':').filter(port)).map_or(host_port, |(h, _)| h);
    Some(host.trim_end_matches('.').to_ascii_lowercase())
}

/// The sub-project a page's host names, whatever its port or case.
fn project_of(page: &str) -> Option<String> {
    let host = host_of(page)?;
    let found = HOST_PROJECTS
        .iter()
        .find(|(h, _)| h.eq_ignore_ascii_case(&host));
    found.map(|(_, project)| project.to_string())
}

/// The port of a page this Mac serves (`localhost:<port>`, `127.0.0.1:<port>`), which a task's worktree may.
fn local_port(page: &str) -> Option<u16> {
    let (host, port) = host_port(page)?.rsplit_once(':')?;
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
        title: title_of(text, quote, page, field("tag")),
        text: text_of(text, quote, field("selector"), page),
        page: page.into(),
        files,
        task: local_port(page).and_then(task_serving),
        project: project_of(page),
        id,
    })
}

/// Why a sending is not filed: the status the outbox reads, and the app's reason.
pub type Refused = (&'static str, String);

/// `POST /comment/page`'s status, type and text: from the extension or curl only (`header` reads one of the request's by name),
/// its form read by `body` within a time and a size, then [`post`], the task kept by `file` (the store's triage).
pub fn posted(
    header: impl Fn(&str) -> Option<String>,
    body: impl FnOnce(
        std::time::Duration,
        usize,
    ) -> Result<Vec<u8>, (&'static str, &'static str, String)>,
    now_ms: i64,
    dir: Result<&std::path::Path, String>,
    task_serving: impl Fn(u16) -> Option<String>,
    file: impl FnOnce(&Triaged, &[&[u8]]) -> Result<bool, String>,
) -> (&'static str, &'static str, String) {
    if !allowed(header("origin").as_deref()) {
        return ("403 Forbidden", dash_core::TEXT, REFUSED_ORIGIN.into());
    }
    // a capture takes its time on a slow link; a body cut short is sent again, never final as a 400 is (/code-review)
    let body = match body(BODY_TIME, BODY_MOST) {
        Ok(body) => body,
        Err(("400 Bad Request", kind, why)) => return ("408 Request Timeout", kind, why),
        Err(refused) => return refused,
    };
    let (fields, files) = dash_core::form_parts(&body, &header("content-type").unwrap_or_default());
    let filed = post(&fields, &files, now_ms, dir, task_serving).and_then(|(t, bytes)| {
        let kept = file(&t, &bytes).map(|_| t.id);
        kept.map_err(|e| ("500 Internal Server Error", e))
    });
    match filed {
        Ok(id) => ("200 OK", dash_core::JSON, answer(&id)),
        Err((status, why)) => (status, dash_core::TEXT, why),
    }
}

/// `POST /comment/page`'s form read into its task and its files' bytes, or why not with its status: a HAR is checked against
/// the capture policy in `dir` first, read only for a HAR, since it holds his sessions (task extension-har-panel); a policy
/// that does not read is a 500, which the outbox sends again.
pub fn post<'a>(
    fields: &BTreeMap<String, String>,
    files: &[(String, &'a [u8])],
    now_ms: i64,
    dir: Result<&std::path::Path, String>,
    task_serving: impl Fn(u16) -> Option<String>,
) -> Result<(Triaged, Vec<&'a [u8]>), Refused> {
    let sizes: Vec<_> = files.iter().map(|(n, b)| (n.clone(), b.len())).collect();
    if har(fields, &sizes) {
        let hosts = dir.and_then(crate::capture::read);
        match hosts.map(|p| har_refused(fields, &sizes, &crate::capture::har_hosts(&p))) {
            Ok(None) => {}
            Ok(Some(why)) => return Err(("400 Bad Request", why)),
            Err(e) => return Err(("500 Internal Server Error", e)),
        }
    }
    let t = sending(fields, &sizes, now_ms, task_serving)?;
    Ok((t, files.iter().map(|(_, b)| *b).collect()))
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

    /// Task extension-har-panel, done-when 2 on the app's side: a HAR, by the panel's tag or a file's name, is taken only from
    /// a host the policy marks har or a subdomain of one; any other sending passes as before.
    #[dash_test]
    fn a_har_is_taken_only_from_a_host_the_capture_policy_marks_har() {
        let hosts = ["linear.app".to_string(), "slack.com".to_string()];
        let har = [("page.har.gz".to_string(), 9)];
        let post = |url: &str, tag: &str| form(&[("url", url), ("tag", tag)]);
        for url in [
            "https://linear.app/nha/issue/1?x=1",
            "https://app.slack.com/client",
            "https://LINEAR.app.:443/",
        ] {
            assert_eq!(har_refused(&post(url, "har"), &har, &hosts), None, "{url}");
        }
        for url in [
            "https://klar.im/",
            "https://notlinear.app/",
            "https://linear.app.evil.com/",
            "https://linear.app@evil.com/",
            "https://evil.com/?linear.app",
            "nonsense",
        ] {
            assert!(
                har_refused(&post(url, "har"), &har, &hosts).is_some(),
                "{url}"
            );
        }
        // a HAR file under another tag is a HAR still, a .har as much as a .har.gz; no policy read, no HAR anywhere
        let plain = post("https://klar.im/", "capture");
        assert!(har_refused(&plain, &[("x.HAR".into(), 1)], &hosts).is_some());
        assert!(har_refused(&post("https://linear.app/", "har"), &har, &[]).is_some());
        // a comment or a capture anywhere is no HAR
        assert_eq!(
            har_refused(&plain, &[("page.html".into(), 1)], &hosts),
            None
        );
        let t = sending(&post("https://linear.app/x", "har"), &har, 1, none).expect("a task");
        assert_eq!(t.title, "HAR: https://linear.app/x");
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

    /// Task flare-site-comments: the routing by host. A comment on flare.nicolas-ha.com, any port or case, is flare-site's,
    /// its URL (bare) and selector kept; nicolas-ha.com itself, a look-alike or another site names no sub-project.
    #[dash_test]
    fn a_comment_on_flare_s_site_is_a_task_of_the_flare_site_sub_project() {
        let on = |url: &str| {
            let fields = form(&[("url", url), ("text", "typo"), ("selector", "main > h1")]);
            sending(&fields, &[], 1, none).expect("a task")
        };
        let t = on("https://flare.nicolas-ha.com/docs?q=1#top");
        assert_eq!(t.project.as_deref(), Some("site_perso/flare-site"));
        assert_eq!(
            t.text,
            "typo\n\nAt: main > h1\n\nPage: https://flare.nicolas-ha.com/docs"
        );
        for url in [
            "https://FLARE.nicolas-ha.com:443/",
            "https://u:p@flare.nicolas-ha.com./a",
        ] {
            assert_eq!(
                on(url).project.as_deref(),
                Some("site_perso/flare-site"),
                "{url}"
            );
        }
        for url in [
            "https://nicolas-ha.com/",
            "https://flare.nicolas-ha.com.evil.example/",
            "https://notflare.nicolas-ha.com/",
            "http://localhost:4001/",
            "https://example.com/?u=flare.nicolas-ha.com",
            // a browser loads evil.example here: `\` ends the host (/code-review of #1629)
            "https://evil.example\\@flare.nicolas-ha.com/x",
        ] {
            assert_eq!(on(url).project, None, "{url}");
        }
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
