//! /mail: the mail plugin's list, read through curl as the address asked, its password his login Keychain's (klar-mail's own
//! items, as the older app's Dashboard.Jmap reads them). Each call is bounded, and `serve` runs this page off the routing
//! thread, so a slow mail server never holds another page.
use dash_mail as mail;
use dash_surface as surface;
use std::collections::BTreeMap;
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

/// JMAP over curl, signed in as `user`: the password goes in curl's environment, never its command line. Only the session's
/// GET follows a redirect, on https alone, and curl sends no Authorization to another host; a POST is never replayed.
fn http(user: &str, password: &str) -> impl Fn(&str, Option<&str>) -> Result<String, String> {
    let key = format!("{user}:{password}");
    let user = user.to_string();
    move |url, body| {
        // the session names the account and the API, which do not change under a running server (as Dashboard.Jmap keeps it)
        let kept = SESSIONS.lock().ok().and_then(|s| s.get(&user).cloned());
        if let (None, Some(session)) = (body, kept) {
            return Ok(session);
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
        let (status, text, reason) =
            dash_process::answer("curl", &args, None, &[("KEY", &key)], LIMIT)
                .map_err(String::from)?;
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
                // a failed call may come of a session gone stale (a restart that moved its API): the next reads it again
                (Some(_), Err(_)) => {
                    sessions.remove(&user);
                }
                _ => {}
            }
        }
        got
    }
}

/// What `asked` lists, as `account` reads it (his own when it is not one of the switch's).
pub fn page(account: Option<&str>, asked: &mail::Asked) -> surface::Drawing {
    let accounts = mail::accounts(|a| password(a).is_some());
    let account = account
        .filter(|a| accounts.iter().any(|b| b == a))
        .unwrap_or(mail::OWN);
    let read = match password(account) {
        Some(p) => mail::list(&http(account, &p), asked),
        None => Err("no password in his Keychain (service klar-mail)".into()),
    };
    let today = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() / 86_400) as i64;
    surface::Drawing::Tree {
        title: "Mail".into(),
        tree: mail::tree(account, &accounts, asked, &read, today),
    }
}
