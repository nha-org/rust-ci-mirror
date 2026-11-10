//! /mail: the mail plugin's list, read through curl as the address asked, its password his login Keychain's (klar-mail's own
//! items, as the older app's Dashboard.Jmap reads them). Each call is bounded, and `serve` runs this page off the routing
//! thread, so a slow mail server never holds another page.
use dash_mail as mail;
use dash_store::oplog::Replica;
use dash_surface as surface;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// One call's bound, curl's --max-time 8 and its start: a page's three calls and four Keychain reads stay under the front
/// door's 60 s reply wait.
const LIMIT: Duration = Duration::from_secs(10);

/// Each address's last Keychain answer and when it was read: one `security` call takes a third of a second, and a page asks
/// for four addresses.
static KEPT: Mutex<BTreeMap<String, (Option<String>, Instant)>> = Mutex::new(BTreeMap::new());

/// klar-mail's item for `address` (`security add-generic-password -s klar-mail -a <address> -w`), read again after ten
/// minutes, or one when there was none: a Keychain locked over SSH hides them all (as Dashboard.Jmap keeps them).
fn password(address: &str) -> Option<String> {
    let kept = KEPT.lock().ok().and_then(|k| k.get(address).cloned());
    if let Some((found, at)) = kept {
        let keep = if found.is_some() { 600 } else { 60 };
        if at.elapsed() < Duration::from_secs(keep) {
            return found;
        }
    }
    let args = [
        "find-generic-password",
        "-s",
        "klar-mail",
        "-a",
        address,
        "-w",
    ];
    let found = dash_process::run("security", &args, None, Duration::from_secs(3)).ok();
    let found = found
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty());
    if let Ok(mut kept) = KEPT.lock() {
        kept.insert(address.to_string(), (found.clone(), Instant::now()));
    }
    found
}

/// Each address's session document, the one GET.
static SESSIONS: Mutex<BTreeMap<String, String>> = Mutex::new(BTreeMap::new());
/// Each address's answers to a folder's id (`Mailbox/query` alone), by the call: a folder's id does not change under a running
/// server, so /mail makes one round trip a view, not two (566 ms a view measured on 2026-11-10, two curls, each its TLS).
static FOLDERS: Mutex<BTreeMap<(String, String), String>> = Mutex::new(BTreeMap::new());

/// A call that asks only a folder's id, which `FOLDERS` keeps.
fn folder_only(body: &str) -> bool {
    body.contains("\"Mailbox/query\"") && !body.contains("\"Email/")
}

/// The JMAP method names a call body asks for, in order: the first element of each `methodCalls` entry.
pub fn method_names(body: Option<&str>) -> Vec<String> {
    let Some(body) = body else {
        return vec![];
    };
    let Ok(value): Result<serde_json::Value, _> = serde_json::from_str(body) else {
        return vec![];
    };
    value
        .get("methodCalls")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|call| call.as_array())
        .filter_map(|parts| parts.first())
        .filter_map(serde_json::Value::as_str)
        .map(String::from)
        .collect()
}

/// JMAP over curl, signed in as `user`: the password goes in curl's environment, never its command line. Only the session's
/// GET follows a redirect, on https alone, and curl sends no Authorization to another host; a POST is never replayed.
fn http(
    user: &str,
    password: &str,
) -> impl Fn(&str, Option<&str>) -> Result<String, String> + use<> {
    let key = format!("{user}:{password}");
    let user = user.to_string();
    move |url, body| {
        // the session names the account and the API, which do not change under a running server (as Dashboard.Jmap keeps it)
        let kept = SESSIONS.lock().ok().and_then(|s| s.get(&user).cloned());
        if let (None, Some(session)) = (body, kept) {
            return Ok(session);
        }
        let folder = body
            .filter(|b| folder_only(b))
            .map(|b| (user.clone(), b.to_string()));
        if let Some(known) = folder
            .as_ref()
            .and_then(|f| FOLDERS.lock().ok()?.get(f).cloned())
        {
            return Ok(known);
        }
        let mut args = vec![
            "-sS",
            "--max-time",
            "8",
            "--proto",
            "=https",
            "--variable",
            "%KEY",
        ];
        args.extend([
            "--expand-header",
            "authorization: Basic {{KEY:b64}}",
            "-w",
            "\n%{http_code}",
        ]);
        match body {
            None => args.extend(["-L", "--proto-redir", "=https"]),
            Some(body) => args.extend([
                "-H",
                "content-type: application/json",
                "--data-binary",
                body,
            ]),
        }
        args.push(url);
        let methods = method_names(body);
        let resource = if methods.is_empty() {
            "session".to_string()
        } else {
            methods.join(" ")
        };
        let span = dash_statsd::current().map(|parent| parent.child("jmap.call", &resource));
        let (status, text, reason) =
            dash_process::answer("curl", &args, None, &[("KEY", &key)], LIMIT)
                .map_err(String::from)?;
        if let Some(span) = span {
            span.finish();
        }
        let (answer, code) = text.rsplit_once('\n').unwrap_or(("", text.trim()));
        // curl's own failure first: a timeout mid-body still prints the 200 it had read
        let got = match code.trim() {
            _ if !status.success() => Err(format!("jmap: no answer: {}", reason.trim())),
            "200" | "201" => Ok(answer.to_string()),
            "401" => Err(format!(
                "jmap: the mail server refused the password of {user}"
            )),
            code => Err(format!(
                "jmap: the mail server answered {code}: {}",
                answer.chars().take(200).collect::<String>()
            )),
        };
        if let Ok(mut sessions) = SESSIONS.lock() {
            match (body, &got) {
                (None, Ok(session)) => {
                    sessions.insert(user.clone(), session.clone());
                }
                // a failed call may come of a session gone stale (a restart that moved its API): the next reads it again, and
                // the folders' ids with it
                (Some(_), Err(_)) => {
                    sessions.remove(&user);
                    if let Ok(mut folders) = FOLDERS.lock() {
                        folders.retain(|(u, _), _| u != &user);
                    }
                }
                _ => {}
            }
        }
        if let (Some(folder), Ok(answer)) = (folder, &got)
            && let Ok(mut folders) = FOLDERS.lock()
        {
            folders.insert(folder, answer.clone());
        }
        got
    }
}

/// The address asked when it is one of the switch's, else his own, and the switch's addresses.
fn shown(account: Option<&str>) -> (String, Vec<String>) {
    let accounts = mail::accounts(|a| password(a).is_some());
    let account = account
        .filter(|a| accounts.iter().any(|b| b == a))
        .unwrap_or(mail::OWN);
    (account.to_string(), accounts)
}

/// JMAP as `account`, or why there is none.
fn signed_in(
    account: &str,
) -> Result<impl Fn(&str, Option<&str>) -> Result<String, String> + use<>, String> {
    password(account)
        .map(|p| http(account, &p))
        .ok_or_else(|| "no password in his Keychain (service klar-mail)".into())
}

/// What `asked` lists, as `account` reads it (his own when it is not one of the switch's).
pub fn page(account: Option<&str>, asked: &mail::Asked) -> surface::Drawing {
    let (account, accounts) = shown(account);
    let account = account.as_str();
    let read = signed_in(account).and_then(|http| mail::list(&http, asked));
    let today = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() / 86_400) as i64;
    surface::Drawing::Tree {
        title: "Mail".into(),
        tree: mail::tree(account, &accounts, asked, &read, today),
    }
}

/// The thread's page as `account` reads it, after a send of `email`: where a reply goes back to, each value encoded, the
/// server's id included, since the Location header carries it.
pub fn thread_href(thread: &str, account: &str, email: &str) -> String {
    let enc = mail::encoded;
    format!(
        "/mail/t/{}?account={}&sent={}",
        enc(thread),
        enc(account),
        enc(email)
    )
}

/// /mail/t/<thread> as `account` reads it (his own when it is not one of the switch's). `sent`: the mail his last reply
/// made, which the page says went.
pub fn thread_page(account: Option<&str>, id: &str, sent: Option<&str>) -> surface::Drawing {
    let (account, _) = shown(account);
    let read = signed_in(&account).and_then(|http| mail::thread(&http, id));
    let said = sent.map(|_| "Sent.");
    surface::Drawing::Tree {
        title: "Mail".into(),
        tree: mail::thread_tree(&account, id, &read, said),
    }
}

/// One send at a time: two posts of one form never both pass the send log's check.
static SENDING: Mutex<()> = Mutex::new(());

/// The send log, in the server's data folder.
fn sent_log(dir: &Path) -> Result<Replica, String> {
    let db = dash_store::connect(&dir.join("mail.sqlite")).map_err(|e| e.to_string())?;
    db.execute_batch(mail::SCHEMA).map_err(|e| e.to_string())?;
    Replica::new(db, "dash").map_err(|e| e.to_string())
}

/// His reply to `thread`, from `account`, which must be one of the switch's: the mail it made, or why none went.
pub fn reply(dir: &Path, account: &str, thread: &str, text: &str) -> Result<String, String> {
    let (shown, _) = shown(Some(account));
    if shown != account {
        return Err(format!("{account} is not an address /mail reads"));
    }
    let http = signed_in(account)?;
    let now = crate::comments::iso(crate::tasks::now_ms());
    let sent = sent_by(dir, &http, account, thread, text, &now)?;
    // the older app's drafts and leads read what went in its outbound log (Webmail.send wrote it there)
    if let Err(e) = crate::tasks::older_app().outbound(&sent.record.to_string()) {
        eprintln!(
            "mail: sent {}, but the older app's outbound log refused it: {e}",
            sent.email
        );
    }
    Ok(sent.email)
}

/// `reply` through `http`, at `at`: the part a fake mail server tests. A send that went is `Ok` even when its row did not
/// write: the server's log says so, never his page, or he would send it again.
fn sent_by(
    dir: &Path,
    http: mail::Http,
    account: &str,
    thread: &str,
    text: &str,
    at: &str,
) -> Result<mail::Sent, String> {
    let read = mail::thread(http, thread)?;
    let _one = SENDING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let sent = mail::send_once(http, &mut sent_log(dir)?, account, &read, text, at)?;
    if let Err(e) = &sent.logged {
        eprintln!(
            "mail: sent {}, but mail.sqlite refused its row: {e}",
            sent.email
        );
    }
    Ok(sent)
}

/// His replies waiting, as the last read counted them (task rust-daily-today): read on its own thread past five minutes, so the
/// daily's page never waits on the mail server. `None` while the first read runs.
static REPLIES: Mutex<crate::fresh::Read<Result<u64, String>>> =
    Mutex::new(crate::fresh::Read::new());

fn replies_waiting() -> Result<Option<u64>, String> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH);
    let now = now.map_err(|e| e.to_string())?.as_secs();
    let read = crate::fresh::bounded(&REPLIES, now, "mail replies", || {
        signed_in(mail::OWN).and_then(|http| mail::replies_waiting(&http))
    })?;
    match read {
        None => Ok(None),
        Some((Ok(n), _)) => Ok(Some(n)),
        Some((Err(e), _)) => Err(e),
    }
}

/// The mail line's key: after the drafts to send.
pub const SLOT: &str = "today/20-mail";

/// The mail plugin's line in the daily's « today » (rfc-rust-plugins-ui section 3, the Slot port).
pub fn slot() -> dash_core::registry::Contribution {
    let line: Box<dyn dash_core::registry::Slot> = Box::new(mail::RepliesWaiting(replies_waiting));
    dash_core::registry::Contribution {
        slots: vec![(SLOT.into(), line)],
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::TestDir;
    use dash_meter::memory::TestBudget;
    use serde_json::{Value, json};
    use std::cell::RefCell;

    fn fixture() -> Value {
        serde_json::from_str(include_str!("../../mail/tests/thread.json")).expect("the fixture")
    }

    /// The mail plugin's JMAP fixture answering every read, and a send made and submitted, each send kept.
    fn server<'a>(
        sends: &'a RefCell<usize>,
    ) -> impl Fn(&str, Option<&str>) -> Result<String, String> + 'a {
        move |_, body| {
            let f = fixture();
            let Some(body) = body else {
                return Ok(f["session"].to_string());
            };
            let asked: Value = serde_json::from_str(body).expect("JSON");
            Ok(match asked["methodCalls"][0][0].as_str() {
                Some("Thread/get") => f["thread"].to_string(),
                Some("Identity/get") => json!({"methodResponses": [
                    ["Identity/get", {"list": [{"id": "i", "email": "nicolas@klar.im", "name": "N"}]}, "i"],
                    ["Mailbox/query", {"ids": ["d"]}, "d"], ["Mailbox/query", {"ids": ["s"]}, "s"]]})
                .to_string(),
                _ => {
                    *sends.borrow_mut() += 1;
                    json!({"methodResponses": [["Email/set", {"created": {"draft": {"id": "e1"}}}, "e"],
                        ["EmailSubmission/set", {"created": {"send": {"id": "x"}}}, "s"]]})
                    .to_string()
                }
            })
        }
    }

    /// Done-when 1 of rust-mail-thread through the page's one source: its HTML folds the older mails and opens the latest,
    /// its text gives every mail; the mail's script and remote image stay in the frame's document, never the page's.
    #[test]
    fn the_thread_page_folds_in_html_and_reads_whole_as_text() {
        let _budget = TestBudget::new();
        let sends = RefCell::new(0);
        let read = mail::thread(&server(&sends), "brq");
        let tree = mail::thread_tree(mail::OWN, "brq", &read, Some("Sent."));
        let html = surface::screen::page("Mail", &tree).expect("its HTML");
        // a tag built at run time: this crate's strings hold no HTML (dash-gate's the_dash_crate_holds_no_html)
        let tag = |words: &str| format!("{}{words}", '<');
        assert_eq!(html.matches(&tag("details")).count(), 3, "{html}");
        assert_eq!(html.matches(" open=true").count(), 1, "{html}");
        assert!(
            html.contains(&tag(r#"details class="fold" id="mail-2" open=true>"#)),
            "{html}"
        );
        assert!(
            !html.contains(&tag("script>alert"))
                && !html.contains(&tag(r#"img src="https://track"#)),
            "{html}"
        );
        let text = surface::text(&tree).expect("its text");
        for words in [
            "The grant",
            "Sent.",
            "Ada Example · 2026-11-06T09:00:00Z · to nicolas@klar.im",
            "Six questions on the grant, below.",
            "nicolas@klar.im · 2026-11-07T10:15:00Z · to ada@example.org, bob@example.org",
            "Monday works.",
            "Send to ada@example.org from nicolas@klar.im: POST /mail/t/brq/reply, his words as text",
        ] {
            assert!(text.contains(words), "{words:?} in {text}");
        }
        assert_eq!(
            thread_href("a b", "support@klar.im", "e1\r\nSet-Cookie: x"),
            "/mail/t/a%20b?account=support@klar.im&sent=e1%0D%0ASet-Cookie%3A%20x",
            "the server's id never breaks the Location header"
        );
    }

    /// Done-when 3 on the server's own store: a reply is one row of mail.sqlite, and the same form posted again sends nothing.
    #[test]
    fn a_reply_is_logged_once_in_the_data_folder() {
        let _budget = TestBudget::new();
        let dir = TestDir::new("mail-reply");
        let sends = RefCell::new(0);
        let http = server(&sends);
        let at = "2026-11-09T10:00:00Z";
        assert_eq!(
            sent_by(&dir.0, &http, mail::OWN, "brq", "Monday.", at).map(|s| (s.email, s.logged)),
            Ok(("e1".into(), Ok(())))
        );
        let again = sent_by(&dir.0, &http, mail::OWN, "brq", "Monday.", at).map(|s| s.email);
        assert_eq!(
            again,
            Err("this reply already went (the send log): nothing sent again".into())
        );
        assert_eq!(*sends.borrow(), 1);
        let log = sent_log(&dir.0).expect("the log");
        let rows: i64 = log
            .read(|db| db.query_row("SELECT count(*) FROM mail_sent", [], |r| r.get(0)))
            .expect("read")
            .expect("counted");
        assert_eq!(rows, 1);
    }

    /// Task mail-folders-kept: only a call that asks a folder's id alone is kept; a list's batch never is.
    #[test]
    fn only_a_folder_s_id_is_kept_never_a_list() {
        let _budget = TestBudget::new();
        let folder = r#"{"using":[],"methodCalls":[["Mailbox/query",{"accountId":"a","filter":{"role":"inbox"}},"b"]]}"#;
        let list = r#"{"using":[],"methodCalls":[["Email/query",{},"q"],["Email/get",{},"e"]]}"#;
        assert!(folder_only(folder));
        assert!(!folder_only(list));
    }

    #[test]
    fn method_names_are_read_from_a_call_body() {
        let _budget = TestBudget::new();
        let body =
            r#"{"using":[],"methodCalls":[["Mailbox/query",{},"c0"],["Email/query",{},"c1"]]}"#;
        assert_eq!(
            method_names(Some(body)),
            vec!["Mailbox/query", "Email/query"]
        );
        assert!(method_names(None).is_empty());
    }
}
