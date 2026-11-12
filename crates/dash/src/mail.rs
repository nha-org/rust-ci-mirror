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
    // a span (task rust-dd-spans): /mail's first view read every account's password, one `security` each, before any call
    let span = dash_statsd::current().map(|parent| parent.child("keychain.read", address));
    let found = dash_process::run("security", &args, None, Duration::from_secs(3)).ok();
    if let Some(span) = span {
        span.finish();
    }
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
    // each address's password read at once, not one after another: a cold view read four in series, ~170 ms each (Datadog,
    // 2026-11-10); `password` keeps each, so the switch below reads them from memory. The request's span goes with each thread.
    let parent = dash_statsd::current();
    std::thread::scope(|s| {
        for address in std::iter::once(mail::OWN).chain(mail::OTHERS) {
            let parent = parent.clone();
            s.spawn(move || {
                dash_statsd::set_current(parent);
                password(address);
            });
        }
    });
    let accounts = mail::accounts(|a| password(a).is_some());
    let account = account
        .filter(|a| accounts.iter().any(|b| b == a))
        .unwrap_or(mail::OWN);
    (account.to_string(), accounts)
}

/// His own inbox's conversations holding `q`, newest first: `dash apply greenhouse` reads its emailed security code here.
pub(crate) fn search(q: &str) -> Result<Vec<mail::Row>, String> {
    let http = signed_in(mail::OWN)?;
    mail::list(&http, &mail::Asked::of(None, Some(q), None)).map(|l| l.rows)
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
    let (account, accounts) = shown(account);
    let read = signed_in(&account).and_then(|http| mail::thread(&http, id));
    let said = sent.map(|_| "Sent.");
    surface::Drawing::Tree {
        title: "Mail".into(),
        tree: mail::thread_tree(&account, &accounts, id, &read, said),
    }
}

/// /mail/compose as `account` writes (his own when it is not one of the switch's): a new mail, its side column's counts read
/// alongside. `sent`: the mail his last send made, which the page says went.
pub fn compose_page(account: Option<&str>, sent: Option<&str>) -> surface::Drawing {
    let (account, accounts) = shown(account);
    let boxes = signed_in(&account).and_then(|http| mail::folders(&http));
    let said = match (&boxes, sent) {
        (_, Some(_)) => Some("Message sent.".to_string()),
        (Err(e), None) => Some(format!("The folders of {account} could not be read: {e}")),
        _ => None,
    };
    surface::Drawing::Tree {
        title: "Mail".into(),
        tree: mail::compose_tree(
            &account,
            &accounts,
            boxes.as_deref().unwrap_or_default(),
            said.as_deref(),
            // this page's token: the same words from a new page go, the same page posted twice sends once
            &crate::tasks::now_ms().to_string(),
        ),
    }
}

/// Where a new mail's send goes back to: Compose, as `account` writes, saying it went.
pub fn compose_href(account: &str, email: &str) -> String {
    format!(
        "{COMPOSE}?account={}&sent={}",
        mail::encoded(account),
        mail::encoded(email)
    )
}

/// His new mail from `account`, which must be one of the switch's: the mail it made, or why none went.
pub fn compose(dir: &Path, account: &str, new: &mail::New) -> Result<String, String> {
    sent_from(account, |http, at| new_sent_by(dir, http, account, new, at))
}

/// `compose` through `http`, at `at`: the part a fake mail server tests.
fn new_sent_by(
    dir: &Path,
    http: mail::Http,
    account: &str,
    new: &mail::New,
    at: &str,
) -> Result<mail::Sent, String> {
    let html = surface::documents::mail_html(&new.text);
    logged(dir, |log| {
        mail::send_new_once(http, log, account, new, Some(&html), at)
    })
}

/// A new mail's page, GET to write it and POST to send it.
pub const COMPOSE: &str = "/mail/compose";

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
    sent_from(account, |http, at| {
        sent_by(dir, http, account, thread, text, at)
    })
}

/// A send from `account`, which must be one of the switch's, signed in and stamped now: the mail it made. What went is also
/// written to the older app's outbound log, which its drafts and leads read (Webmail.send wrote it there); a reply and a new
/// mail both go through here.
fn sent_from(
    account: &str,
    send: impl FnOnce(mail::Http, &str) -> Result<mail::Sent, String>,
) -> Result<String, String> {
    let (shown, _) = shown(Some(account));
    if shown != account {
        return Err(format!("{account} is not an address /mail reads"));
    }
    let http = signed_in(account)?;
    let now = crate::comments::iso(crate::tasks::now_ms());
    let sent = send(&http, &now)?;
    // bin/dash's own 60 s and restart waits, as before; a line it surely could not log is kept for the app's next check
    let line = sent.record.to_string();
    let older = crate::tasks::older_app();
    if let Err(e) = older.outbound(&line, &[], Duration::from_secs(60), &crate::pages::repo()) {
        eprintln!(
            "mail: sent {}, but the older app's outbound log did not confirm it: {e}",
            sent.email
        );
    }
    Ok(sent.email)
}

/// `send` against the send log, one at a time: two posts of one form never both pass its check. A send that went is `Ok` even
/// when its row did not write: the server's log says so, never his page, or he would send it again.
fn logged(
    dir: &Path,
    send: impl FnOnce(&mut Replica) -> Result<mail::Sent, String>,
) -> Result<mail::Sent, String> {
    let _one = SENDING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let sent = send(&mut sent_log(dir)?)?;
    if let Err(e) = &sent.logged {
        eprintln!(
            "mail: sent {}, but mail.sqlite refused its row: {e}",
            sent.email
        );
    }
    Ok(sent)
}

/// `reply` through `http`, at `at`: the part a fake mail server tests.
fn sent_by(
    dir: &Path,
    http: mail::Http,
    account: &str,
    thread: &str,
    text: &str,
    at: &str,
) -> Result<mail::Sent, String> {
    let read = mail::thread(http, thread)?;
    // the mail's HTML part, the design system's drawing of his formatting bar's marks
    let html = surface::documents::mail_html(text.trim());
    logged(dir, |log| {
        mail::send_once(http, log, account, &read, text, Some(&html), at)
    })
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

/// A thread's page is under this prefix, its reply posted to `<prefix><thread>/reply`.
pub const THREAD: &str = "/mail/t/";

// the mail server's, answered on its connection's thread, never the serial one
inventory::submit! {
    crate::pages::Page { path: "/mail", step: "route_mail", reads: crate::pages::Reads::Nothing, redraw: false, serve: |a| {
        let at = a.param("page").and_then(|p| p.trim().parse().ok());
        let (folder, q) = (a.param("box"), a.param("q"));
        let asked = mail::Asked::of(folder.as_deref(), q.as_deref(), at);
        let account = a.param("account").filter(|a| !a.is_empty());
        a.form(Some(Ok(page(account.as_deref(), &asked))))
    } }
}
inventory::submit! {
    crate::pages::Page { path: COMPOSE, step: "route_mail_compose", reads: crate::pages::Reads::Nothing, redraw: false, serve: |a| {
        let account = a.param("account").filter(|a| !a.is_empty());
        let sent = a.param("sent").filter(|s| !s.is_empty());
        a.form(Some(Ok(compose_page(account.as_deref(), sent.as_deref()))))
    } }
}
inventory::submit! {
    crate::pages::Page { path: "/mail/t/*", step: "route_mail_thread", reads: crate::pages::Reads::Nothing, redraw: false, serve: |a| {
        let account = a.param("account").filter(|a| !a.is_empty());
        let sent = a.param("sent").filter(|s| !s.is_empty());
        a.form(Some(Ok(thread_page(account.as_deref(), &a.id(), sent.as_deref()))))
    } }
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
        let tree = mail::thread_tree(mail::OWN, &[mail::OWN.into()], "brq", &read, Some("Sent."));
        let html = surface::markup(&surface::screen::page("Mail", &tree).expect("its HTML"));
        // a tag built at run time: this crate's strings hold no HTML (dash-gate's the_dash_crate_holds_no_html)
        let tag = |words: &str| format!("{}{words}", '<');
        assert_eq!(
            html.matches(&tag(r#"details class="fold""#)).count(),
            3,
            "{html}"
        );
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
        // Done-when 2 of rust-ui-media: the latest mail's files are Attachments, the picture an Image of itself
        for file in [
            r#"a class="attachment" id="mail-2-file-0" data-orientation="horizontal" href="/mail/blob/b1?name=slots.pdf&#38;type=application%2Fpdf&#38;account=nicolas@klar.im" title="slots.pdf">"#,
            r#"a class="attachment" id="mail-2-file-1" data-orientation="vertical" href="/mail/blob/b2?name=room.png&#38;type=image%2Fpng&#38;account=nicolas@klar.im" title="room.png">"#,
            r#"img class="image" loading="lazy" decoding="async" src="/mail/blob/b2?name=room.png&#38;type=image%2Fpng&#38;account=nicolas@klar.im" alt=""/>"#,
        ] {
            assert!(html.contains(&tag(file)), "{file} in {html}");
        }
        assert_eq!(
            html.matches(&tag(r#"img class="image""#)).count(),
            1,
            "the PDF is no picture"
        );
        // task compose-uses-doc-editor: his reply box is the documents editor (documents::Composer), no textarea, its Send in
        // the field, after its Notion-style words
        let form = &html[html
            .find(&tag(
                r#"form class="form" id="reply" method="post" action="/mail/t/brq/reply">"#,
            ))
            .expect("the reply form")..];
        let form = &form[..form.find(&tag("/form>")).expect("its end")];
        assert!(!form.contains(&tag("textarea")), "{form}");
        let field = form
            .find(r#"class="editor-field" data-empty=true data-placeholder="Your reply">"#)
            .expect(form);
        let words = form
            .find(r#"class="dv notion" contenteditable="plaintext-only" role="textbox" aria-multiline="true" title="Your reply">"#)
            .expect(form);
        let send = form
            .find(r#"class="button editor-send" title="Send to ada@example.org from nicolas@klar.im (⌘Enter)">"#)
            .expect(form);
        assert!(field < words && words < send, "{form}");
        let text = surface::text(&tree).expect("its text");
        for words in [
            "slots.pdf\tapplication/pdf\t/mail/blob/b1?name=slots.pdf&type=application%2Fpdf&account=nicolas@klar.im",
            "room.png\timage/png\t/mail/blob/b2",
            "The grant",
            "Sent.",
            "Ada Example · Fri 6 Nov 2026, 09:00 UTC · to nicolas@klar.im",
            "Six questions on the grant, below.",
            "nicolas@klar.im · Sat 7 Nov 2026, 10:15 UTC · to ada@example.org, bob@example.org",
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

    /// Done-when 2 of rust-mail-parity, Compose: /mail/compose's text names From, To, Cc, Bcc and Subject and where it posts,
    /// its HTML draws the formatting bar; a new mail goes once, its HTML part made from the bar's marks, logged in the data folder.
    #[test]
    fn a_new_mail_is_composed_with_its_bar_sent_once_and_logged() {
        let _budget = TestBudget::new();
        let tree = mail::compose_tree(mail::OWN, &[mail::OWN.into()], &[], None, "p1");
        let text = surface::text(&tree).expect("its text");
        for line in [
            "New message",
            "From: nicolas@klar.im",
            "To: field to",
            "Cc: field cc",
            "Bcc: field bcc",
            "Subject: field subject",
            "Send: POST /mail/compose, his words as text",
        ] {
            assert!(text.contains(line), "{line:?} in {text}");
        }
        let html = surface::markup(&surface::screen::page("Mail", &tree).expect("its HTML"));
        for mark in ["bold", "italic", "link", "ul", "ol", "quote"] {
            assert!(html.contains(&format!(r#"data-format="{mark}""#)), "{mark}");
        }

        let dir = TestDir::new("mail-compose");
        let made = RefCell::new(vec![]);
        let http = |_: &str, body: Option<&str>| -> Result<String, String> {
            let f = fixture();
            let Some(body) = body else {
                return Ok(f["session"].to_string());
            };
            let asked: Value = serde_json::from_str(body).expect("JSON");
            Ok(match asked["methodCalls"][0][0].as_str() {
                Some("Identity/get") => json!({"methodResponses": [
                    ["Identity/get", {"list": [{"id": "i", "email": "nicolas@klar.im", "name": "N"}]}, "i"],
                    ["Mailbox/query", {"ids": ["d"]}, "d"], ["Mailbox/query", {"ids": ["s"]}, "s"]]})
                .to_string(),
                _ => {
                    made.borrow_mut().push(asked["methodCalls"][0][1]["create"]["draft"].clone());
                    json!({"methodResponses": [["Email/set", {"created": {"draft": {"id": "e9"}}}, "e"],
                        ["EmailSubmission/set", {"created": {"send": {"id": "x"}}}, "s"]]})
                    .to_string()
                }
            })
        };
        let new = mail::New::of(
            "ada@example.org, <bob@example.org>",
            "",
            "eve@example.org",
            " Plan ",
            "**Monday**",
            "p1",
        )
        .expect("a mail");
        let at = "2026-11-11T10:00:00Z";
        assert_eq!(
            new_sent_by(&dir.0, &http, mail::OWN, &new, at).map(|s| s.email),
            Ok("e9".into())
        );
        assert_eq!(
            new_sent_by(&dir.0, &http, mail::OWN, &new, at).map(|s| s.email),
            Err("this mail already went (the send log): nothing sent again".into())
        );
        assert_eq!(made.borrow().len(), 1, "sent once");
        let made = made.borrow().clone();
        assert_eq!(
            made[0]["to"],
            json!([{"email": "ada@example.org"}, {"email": "bob@example.org"}])
        );
        assert_eq!(made[0]["bcc"], json!([{"email": "eve@example.org"}]));
        assert_eq!(made[0]["subject"], "Plan");
        assert_eq!(made[0]["bodyValues"]["body"]["value"], "**Monday**");
        assert!(
            made[0]["bodyValues"]["html"]["value"]
                .as_str()
                .is_some_and(|h| h.contains("Monday")),
            "{}",
            made[0]
        );
        assert_eq!(
            mail::New::of("", "", "", "", "x", ""),
            Err("a new mail names who it goes to".into())
        );
        assert_eq!(
            mail::New::of("ada", "", "", "", "x", ""),
            Err("« ada » is no address".into())
        );
        assert_eq!(
            mail::New::of("a@b.c", "", "", "", " ", ""),
            Err("an empty mail is not sent".into())
        );
        // /code-review: « Name <address> » is one address; a malformed one is refused before the server sees it
        assert_eq!(
            mail::New::of(
                "Ada Lovelace <ada@example.org>; b@x.org",
                "",
                "",
                "",
                "x",
                ""
            )
            .map(|n| n.to),
            Ok(vec!["ada@example.org".to_string(), "b@x.org".into()])
        );
        for bad in ["a@b@c.d", "x@.", "x@y", "a b@c.d"] {
            assert!(mail::New::of(bad, "", "", "", "x", "").is_err(), "{bad}");
        }
        // the same words from a new compose page (a new token) go again; the same page posted twice did not (above)
        let again = mail::New {
            draft: "p2".into(),
            ..new.clone()
        };
        assert_eq!(
            new_sent_by(&dir.0, &http, mail::OWN, &again, at).map(|s| s.email),
            Ok("e9".into())
        );
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
