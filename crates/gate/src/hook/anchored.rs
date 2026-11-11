//! The pre-commit's comments-anchored (once private/dashboard/priv/commands/comments-anchored, which the frozen app's
//! `dash comments anchored` still runs): a page edit must not strand an open comment of his (@nha, 2026-10-17: « how my
//! comments are lost while unhandled »). A comment is anchored by the passage he quoted; rewriting that passage while the
//! comment is open drifts it away from what it was about. Each open comment whose quote a staged HTML page no longer holds
//! refuses the commit, unless Claude's last reply says where the passage went: its task (« Tâche <id> », « task <id> », its
//! ?issue=<id> link), its fix (« Fixed (<sha>) »), its pull request, or a section of that same page (/v/<file>#<id>). A
//! bare « looking » still blocks. The page is read as staged, never from the working tree.
use super::Ctx;
use regex::Regex;
use serde_json::Value;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

// Compiled once: the comments dump holds thousands, and a compile per comment took the step 13 s (its first commit)
static TAGS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<[^>]+>").expect("tags"));
// a task id has a hyphen: « the task now » named none (/code-review of #355)
static NAMED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(?:t[aâ]che|task) [a-z0-9]+(?:-[a-z0-9]+)+|[?&]issue=[a-z0-9]+(?:-[a-z0-9]+)+|\bFixed \([0-9a-f]{7,}\)|https://github\.com/[\w.-]+/[\w.-]+/pull/\d+\b",
    )
    .expect("pattern")
});

/// A comment that still waits on him: id, page, quote without whitespace, its text
pub struct Open {
    pub id: String,
    pub file: String,
    pub quote: String,
    pub text: String,
}

/// A string of the model's JSON as written: a keyword's "~:" or a symbol's "~$" goes, a leading ~ escaped as "~~" is one again.
pub fn untag(v: &Value) -> Option<String> {
    let s = v.as_str()?;
    Some(if s.starts_with("~:") || s.starts_with("~$") {
        s[2..].to_string()
    } else if let Some(rest) = s.strip_prefix('~') {
        rest.to_string()
    } else {
        s.to_string()
    })
}

/// A page's text as the viewer shows it, whitespace gone: tags dropped from HTML, every entity read, as Python's
/// html.unescape did (six named ones missed « Décision » on a page that writes D&eacute;cision, 9,926 times in ours).
pub fn text(path: &str, t: &str) -> String {
    let t = if path.ends_with(".html") {
        html_escape::decode_html_entities(&TAGS.replace_all(t, "")).into_owned()
    } else {
        t.to_string()
    };
    t.chars().filter(|c| !c.is_whitespace()).collect()
}

/// Percent-decoding of a link, as urllib's unquote
fn unquote(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && b[i + 1].is_ascii_hexdigit()
            && b[i + 2].is_ascii_hexdigit()
            && let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16)
        {
            out.push(v);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Claude's last reply says where the passage went; `page` gives a page's raw HTML, to tell a section that exists.
pub fn answered(c: &Value, file: &str, page: &dyn Fn(&str) -> String) -> bool {
    let replies: Vec<&Value> = c["replies"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|r| r["by"].as_str().is_some_and(|b| !b.is_empty()) && r["text"].is_string())
        .collect();
    let Some(last) = replies.last() else {
        return false;
    };
    if last["by"].as_str() != Some("~:claude") {
        return false;
    }
    let said = unquote(&untag(&last["text"]).unwrap_or_default());
    if NAMED.is_match(&said) {
        return true;
    }
    if file.is_empty() {
        return false;
    }
    // a section link counts only if that id is on the page (/code-review of #847): /v/<file>#<id>, or <basename>#<id> with
    // no path or word before it, so a link to another page's section is a reference, not where the passage went
    let base = Path::new(file)
        .file_name()
        .map_or(file.to_string(), |b| b.to_string_lossy().into_owned());
    let ids = section_ids(&said, &format!("/v/{file}#"), None)
        .into_iter()
        .chain(section_ids(
            &said,
            &format!("{base}#"),
            Some(|c: char| c.is_alphanumeric() || "_/.-".contains(c)),
        ));
    // the page only when a section is named: a git show per open comment took the hook's step 10 s
    let ids: Vec<String> = ids.collect();
    if ids.is_empty() {
        return false;
    }
    let html = page(file);
    ids.iter().any(|id| html.contains(&format!("id=\"{id}\"")))
}

/// The ids after each `needle`, one not preceded by a character `before` matches
fn section_ids(said: &str, needle: &str, before: Option<fn(char) -> bool>) -> Vec<String> {
    said.match_indices(needle)
        .filter(|(at, _)| before.is_none_or(|f| !said[..*at].chars().next_back().is_some_and(f)))
        .map(|(at, _)| {
            said[at + needle.len()..]
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
                .collect::<String>()
        })
        .filter(|id| !id.is_empty())
        .collect()
}

/// The checkout the dashboard serves holds his comments: a worktree's own copy is stale (review of 2026-10-17). None when
/// the app does not answer in 2 s.
fn served_root() -> Option<String> {
    let url = std::env::var("DASHBOARD_URL").unwrap_or_else(|_| "http://127.0.0.1:7788".into());
    let rest = url.strip_prefix("http://")?;
    let (host, _) = rest.split_once('/').unwrap_or((rest, ""));
    let addr = if host.contains(':') {
        host.to_string()
    } else {
        format!("{host}:80")
    };
    let addr = std::net::ToSocketAddrs::to_socket_addrs(&addr)
        .ok()?
        .next()?;
    let two = std::time::Duration::from_secs(2);
    let mut s = std::net::TcpStream::connect_timeout(&addr, two).ok()?;
    s.set_read_timeout(Some(two)).ok()?;
    write!(
        s,
        "GET /live HTTP/1.0\r\nHost: {host}\r\nAccept: application/json\r\n\r\n"
    )
    .ok()?;
    let mut body = String::new();
    s.read_to_string(&mut body).ok()?;
    // HTTP/1.0: never chunked, the body is the object (a 1.1 chunk's size line broke the JSON: /code-review)
    let json: Value = serde_json::from_str(body.split_once("\r\n\r\n")?.1).ok()?;
    json["root"].as_str().map(String::from)
}

/// The open comments, from the served checkout's private/live/comments.edn first, then this one's: the app's JSON dump, read
/// here since the pre-commit runs while the app may be down; none in a fresh clone. A note when it is not JSON yet.
pub fn open_comments(
    top: &Path,
    pages: &[&str],
    page: &dyn Fn(&str) -> String,
) -> (Vec<Open>, Option<String>) {
    let roots: Vec<PathBuf> = served_root()
        .into_iter()
        .map(PathBuf::from)
        .chain([top.to_path_buf()])
        .collect();
    let Some(path) = roots
        .iter()
        .map(|r| r.join("private/live/comments.edn"))
        .find(|p| p.is_file())
    else {
        return (vec![], None);
    };
    let Ok(all) =
        serde_json::from_str::<Value>(&std::fs::read_to_string(&path).unwrap_or_default())
    else {
        return (
            vec![],
            Some(format!(
                "comments-anchored: {} is not JSON yet (an app of before store-edn-gone), not checked",
                path.display()
            )),
        );
    };
    let mut out = vec![];
    for c in all.as_array().into_iter().flatten() {
        let (Some(file), Some(quote)) = (untag(&c["file"]), untag(&c["quote"])) else {
            continue;
        };
        // only a staged page's comments are judged: answered() on all of them took the step seconds (/code-review)
        if quote.is_empty()
            || c["status"].as_str() != Some("~:open")
            || !pages.contains(&file.as_str())
            || answered(c, &file, page)
        {
            continue;
        }
        out.push(Open {
            id: untag(&c["id"]).unwrap_or_default(),
            file,
            quote: quote.chars().filter(|c| !c.is_whitespace()).collect(),
            text: untag(&c["text"]).unwrap_or_default(),
        });
    }
    (out, None)
}

/// The step, on what is staged: each open comment whose quote a staged page drops, that the page held at HEAD and that a
/// merged-in side did not remove itself (a merge takes the other side's edits as they are: comment-draft-reload,
/// 2026-10-22; a page only their side holds is all theirs, PR 372's lane, 2026-10-25).
pub fn check(ctx: &Ctx) -> Result<String, String> {
    let mut cache: HashMap<String, Option<String>> = HashMap::new();
    let mut show = |rev: &str, f: &str| -> Option<String> {
        cache
            .entry(format!("{rev}:{f}"))
            .or_insert_with(|| ctx.git(&["show", &format!("{rev}:{f}")]).ok())
            .clone()
    };
    let pages_cache = std::cell::RefCell::new(HashMap::<String, String>::new());
    let staged_now = |f: &str| {
        pages_cache
            .borrow_mut()
            .entry(f.to_string())
            .or_insert_with(|| ctx.git(&["show", &format!(":{f}")]).unwrap_or_default())
            .clone()
    };
    let staged = ctx.git(&["diff", "--cached", "--name-only", "--diff-filter=d"])?;
    let pages: Vec<&str> = staged.lines().filter(|f| f.ends_with(".html")).collect();
    // no page staged, nothing to strand: no /live round trip, no read of the dump (/code-review)
    if pages.is_empty() {
        return Ok("comments-anchored: ok, no page staged".into());
    }
    let (comments, note) = open_comments(&ctx.top, &pages, &staged_now);
    if let Some(n) = note {
        ctx.say(&n);
    }
    let base = ctx
        .rev("MERGE_HEAD")
        .and_then(|_| ctx.git(&["merge-base", "HEAD", "MERGE_HEAD"]).ok())
        .map(|b| b.trim().to_string());
    let (mut bad, mut checked) = (vec![], 0);
    for c in comments.iter().filter(|c| pages.contains(&c.file.as_str())) {
        checked += 1;
        let q = c.quote.to_lowercase();
        let now = text(&c.file, &staged_now(&c.file)).to_lowercase();
        let before = show("HEAD", &c.file).map(|t| text(&c.file, &t).to_lowercase());
        if now.contains(&q) || before.as_ref().is_some_and(|b| !b.contains(&q)) {
            continue;
        }
        if let Some(base) = &base {
            let had = show(base, &c.file).map(|t| text(&c.file, &t).to_lowercase());
            let theirs = show("MERGE_HEAD", &c.file).map(|t| text(&c.file, &t).to_lowercase());
            let theirs_removed = match (&had, &before) {
                (None, None) => theirs.as_ref().is_some_and(|t| !t.contains(&q)),
                _ => had.is_some_and(|h| h.contains(&q)) && theirs.is_some_and(|t| !t.contains(&q)),
            };
            if theirs_removed {
                continue;
            }
        }
        let body: String = c.text.chars().take(80).collect();
        bad.push(format!("  {}  {}: « {body} »", c.id, c.file));
    }
    if !bad.is_empty() {
        return Err(format!(
            "comments-anchored: open comments whose quoted text this edit removes. Answer each first (private/dashboard/bin/dash queue done <id> \"...\"):\n{}",
            bad.join("\n")
        ));
    }
    Ok(format!(
        "comments-anchored: ok, {checked} open comment(s) on {} page(s), none stranded",
        pages.len()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::memory::TestBudget;
    use serde_json::json;

    /// HTML written with ‹ › for < >: the workspace rule the_dash_crate_holds_no_html keeps tag literals in dash_surface alone
    fn h(s: &str) -> &'static str {
        Box::leak(s.replace('‹', "<").replace('›', ">").into_boxed_str())
    }

    fn says(t: &str) -> Value {
        json!({"replies": [{"by": "~:claude", "date": "x", "text": t}]})
    }

    #[test]
    fn an_answer_says_where_the_passage_went_or_it_still_blocks() {
        let _budget = TestBudget::new();
        let none = |_: &str| String::new();
        let page = |_: &str| h("‹section id=\"decision\"›x‹/section›").to_string();
        for (t, ok) in [
            ("Tâche rfc-decision-clarity", true),
            ("ok, looking at the task now", false),
            (
                "Tâche : http://h/projects/site_perso?issue=comment-39ce1982",
                true,
            ),
            ("see http://h/projects/site_perso?issue=x", false),
            (
                "Fait (PR https://github.com/nha-org/site_perso/pull/545).",
                true,
            ),
            ("see github.com/nha-org/site_perso/pulls", false),
            ("as in https://notgithub.com/a/b/pull/1", false),
            ("Fixed (abc1234)", true),
        ] {
            assert_eq!(answered(&says(t), "", &none), ok, "« {t} » answered: {ok}");
        }
        let f = "rfc-bb app.html";
        for (t, ok) in [
            (
                "Section 5 now: http://h:7788/v/rfc-bb%20app.html#decision",
                true,
            ),
            ("Section 5 now: rfc-bb app.html#decision", true),
            (
                "looking, see http://h:7788/v/rfc-bb app.html#g-thread",
                false,
            ),
            ("see http://h:7788/v/rfc-bb app.html", false),
            (
                "see http://h:7788/v/private/docs/rfc-other.html#decision",
                false,
            ),
        ] {
            assert_eq!(answered(&says(t), f, &page), ok, "« {t} » on {f}: {ok}");
        }
        let later = json!({"replies": [{"by": "~:claude", "text": "Tâche a-b"}, {"by": "~:nha", "date": "x", "text": "still?"}]});
        assert!(
            !answered(&later, "", &none),
            "his reply after Claude's reopens it"
        );
        assert!(!answered(&json!({}), "", &none), "no reply answers nothing");
        assert_eq!(untag(&json!("~~x")).as_deref(), Some("~x"));
        assert_eq!(text("a.html", h("‹p›A&amp;B&#233;  c‹/p›")), "A&Béc");
        // every named entity, as html.unescape: our pages write &eacute; thousands of times (/code-review)
        assert_eq!(
            text(
                "a.html",
                h("‹p›D&eacute;cision&rsquo;s &mdash; &hellip;‹/p›")
            ),
            "Décision’s—…"
        );
    }
}
