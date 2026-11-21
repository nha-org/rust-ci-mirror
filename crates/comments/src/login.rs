//! `POST /extension/login`: a login he types in the extension's form, its site, user and password at once, written by
//! bin/secret into the secret the automations sign in with, never kept by the extension (task extension-login-form, item 7
//! of rfc-chrome-extension section 8; rfc-login-no-keychain: « His Chrome's stores are never read »). The form posts
//! straight here, never through the outbox, so a password never sits in Chrome's storage, even while the app is away.
//!
//! Only his tailnet login posts: Tailscale Serve forwards the post on loopback and names who asked in Tailscale-User-Login,
//! a header Serve drops from any client (« to avoid header spoofing », rfc-chrome-extension section 6, A). The login it must
//! name is the serving process's `DASH_HIS_LOGIN` (the Makefile's rust/serve), never this tree's, which the public mirror
//! publishes; unset, every login is refused. With Serve down, every login is refused too: a password never crosses an
//! unauthenticated route.
use dash_core::TEXT;
use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;

pub const PATH: &str = "/extension/login";
pub const LOGIN_ENV: &str = "DASH_HIS_LOGIN";
pub const REFUSED_LOGIN: &str =
    "refused: a login is posted through Tailscale Serve, from his tailnet login";
/// A login's form is three short fields: anything bigger is no login.
pub const BODY_MOST: usize = 16 * 1024;

/// Each site a login is kept for: its host, bin/secret's name and the keys its user and password go in, as their readers
/// read them (bin/linkedin-login-fill, freework_apply.py). A site no automation signs in to is refused: its secret would
/// be read by nothing.
const SITES: &[(&str, &str, &str, &str)] = &[
    (
        "linkedin.com",
        "linkedin-password",
        "LINKEDIN_EMAIL",
        "LINKEDIN_PASSWORD",
    ),
    (
        "free-work.com",
        "freework",
        "FREE_WORK_EMAIL",
        "FREE_WORK_PASSWORD",
    ),
];

/// `POST /extension/login`, whatever its query.
pub fn asked(line: &str) -> bool {
    crate::triage::posts_to(line, PATH)
}

static HIS: OnceLock<Option<String>> = OnceLock::new();

/// His tailnet login as the serving process holds it, or none: read once.
pub fn his() -> Option<String> {
    HIS.get_or_init(|| {
        std::env::var(LOGIN_ENV)
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
    })
    .clone()
}

/// Read once at the serve's start, then out of its environment: no child of it (a look, a task run, its test servers)
/// inherits his login (/code-review of #1676). Called before the serve starts a thread.
pub fn take() {
    his();
    // SAFETY: the serve's first step, before any thread of its own reads the environment
    unsafe { std::env::remove_var(LOGIN_ENV) };
}

/// Why a login post is refused, or `None` when it is his through Serve: from the extension or curl, on loopback (Serve's
/// forward; a post to the tailnet listener comes from a tailnet address, where the header is typed by hand), `login` his.
/// A process on this Mac can type the header on loopback too: it is no wider a door, since such a process runs as him and
/// can run `bin/secret set` itself (/code-review). The check keeps out the tailnet and the web, which only Serve can vouch for.
pub fn refused(
    peer: Option<SocketAddr>,
    origin: Option<&str>,
    login: Option<&str>,
    his: Option<&str>,
) -> Option<&'static str> {
    if !crate::triage::allowed(origin) {
        Some(crate::triage::REFUSED_ORIGIN)
    } else if !(peer.is_some_and(|p| p.ip().is_loopback())
        // an address in any case (/code-review)
        && login.zip(his).is_some_and(|(l, h)| l.trim().eq_ignore_ascii_case(h)))
    {
        Some(REFUSED_LOGIN)
    } else {
        None
    }
}

/// `POST /extension/login`'s status, type and text: refused unless his through Serve (`header` reads one of the request's by
/// name), then its form, read by `body` at most [`BODY_MOST`] bytes, written with `repo`'s bin/secret.
pub fn answer(
    peer: Option<SocketAddr>,
    header: impl Fn(&str) -> Option<String>,
    body: impl FnOnce(usize) -> Result<Vec<u8>, (&'static str, &'static str, String)>,
    repo: impl FnOnce() -> PathBuf,
) -> (&'static str, &'static str, String) {
    let (origin, who) = (header("origin"), header("tailscale-user-login"));
    if let Some(why) = refused(peer, origin.as_deref(), who.as_deref(), his().as_deref()) {
        // read and dropped: a close on an unread body resets the socket, and Serve would answer 502, not why (/code-review)
        let _ = body(BODY_MOST);
        return ("403 Forbidden", TEXT, why.into());
    }
    let body = match body(BODY_MOST) {
        Ok(body) => body,
        Err(refused) => return refused,
    };
    // the extension's FormData: a multipart form
    let (fields, _) = dash_core::form_parts(&body, &header("content-type").unwrap_or_default());
    let (status, said) = keep(&repo().join("bin/secret"), &fields, |_| {})
        .map_or_else(|e| e, |said| ("200 OK", said));
    (status, TEXT, said)
}

/// bin/secret's name and keys for `site`, a host or an address: the host or any of its subdomains.
pub fn secret_of(site: &str) -> Option<(&'static str, &'static str, &'static str)> {
    let rest = site
        .trim()
        .split_once("://")
        .map_or(site.trim(), |(_, r)| r);
    let host = rest.split(['/', '?', '#', ':']).next().unwrap_or_default();
    // a fully qualified name (linkedin.com.) is the same host (/code-review)
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    SITES
        .iter()
        .find(|(h, ..)| host == *h || host.ends_with(&format!(".{h}")))
        .map(|&(_, name, user, password)| (name, user, password))
}

/// The form's login written with `secret` (bin/secret): the user, then the password, each `bin/secret set` from stdin, never
/// an argument a process list shows. `with` adds to each command (a test's SECRETS_DIR). The answer names the site and the
/// secret, never a value.
pub fn keep(
    secret: &Path,
    fields: &BTreeMap<String, String>,
    with: impl Fn(&mut Command),
) -> Result<String, (&'static str, String)> {
    let field = |k: &str| fields.get(k).map(|v| v.trim()).filter(|v| !v.is_empty());
    let (Some(site), Some(user), Some(password)) =
        (field("site"), field("user"), field("password"))
    else {
        return Err((
            "400 Bad Request",
            "a login is a site, a user and a password".into(),
        ));
    };
    // bin/secret trims a value and its load() drops wrapping quotes: such a password would be read back as another one, a
    // failed sign-in toward the lockout, so it is refused, never changed (/code-review of #1676)
    let raw = fields.get("password").map_or("", String::as_str);
    if raw != password || password.starts_with(['"', '\'']) || password.ends_with(['"', '\'']) {
        return Err((
            "400 Bad Request",
            "a password bin/secret keeps starts and ends with no space or quote".into(),
        ));
    }
    if [user, password].iter().any(|v| v.contains(['\n', '\r'])) {
        return Err(("400 Bad Request", "a user or password is one line".into()));
    }
    let Some((name, user_key, password_key)) = secret_of(site) else {
        let known: Vec<_> = SITES.iter().map(|s| s.0).collect();
        return Err((
            "400 Bad Request",
            format!(
                "no automation signs in to {site}: logins are kept for {}",
                known.join(", ")
            ),
        ));
    };
    let run = |args: &[&str], value: &str| {
        let failed = |e: String| format!("bin/secret {}: {e}", args[..args.len().min(3)].join(" "));
        let mut c = Command::new(secret);
        c.args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        with(&mut c);
        let mut child = c.spawn().map_err(|e| failed(e.to_string()))?;
        if let Some(mut stdin) = child.stdin.take() {
            std::io::Write::write_all(&mut stdin, value.as_bytes())
                .map_err(|e| failed(e.to_string()))?;
        }
        let out = child
            .wait_with_output()
            .map_err(|e| failed(e.to_string()))?;
        match out.status.success() {
            true => Ok(String::from_utf8_lossy(&out.stdout).trim().to_string()),
            false => Err(failed(String::from_utf8_lossy(&out.stderr).trim().into())),
        }
    };
    let error = |e| ("500 Internal Server Error", e);
    // the user kept before, put back if the password fails: a new user beside the old password would sign in wrong, and a
    // failed sign-in counts toward the site's lockout (/code-review)
    let before = run(&["get", name, user_key], "")
        .ok()
        .filter(|v| !v.is_empty());
    run(&["set", name, user_key], user).map_err(error)?;
    if let Err(e) = run(&["set", name, password_key], password) {
        let back = match before {
            Some(old) => run(&["set", name, user_key], &old).map_or_else(
                |b| format!("; the user was not put back: {b}"),
                |_| String::new(),
            ),
            // bin/secret has no delete and refuses an empty value: the half-written login is named, to save again
            None => {
                format!("; {user_key} holds the new user beside no password: save the login again")
            }
        };
        return Err(error(format!("{e}{back}")));
    }
    Ok(format!("login for {site} kept in bin/secret's {name}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::{TestDir, dash_test};

    /// Done-when 3: no login, another login, his login typed to the tailnet listener, a web page, or no login set at start.
    #[dash_test]
    fn only_his_login_forwarded_by_serve_posts_a_login() {
        const HIS: &str = "him@example.com";
        let here: Option<SocketAddr> = Some(([127, 0, 0, 1], 50_000).into());
        let phone: Option<SocketAddr> = Some(([100, 80, 1, 2], 50_000).into());
        let ext = Some(crate::triage::EXTENSION_ORIGIN);
        assert_eq!(refused(here, ext, Some(HIS), Some(HIS)), None);
        assert_eq!(
            refused(here, ext, Some("Him@Example.com"), Some(HIS)),
            None,
            "any case"
        );
        assert_eq!(refused(here, None, Some(HIS), Some(HIS)), None, "curl");
        assert_eq!(refused(here, ext, None, Some(HIS)), Some(REFUSED_LOGIN));
        let other = Some("someone@example.com");
        assert_eq!(refused(here, ext, other, Some(HIS)), Some(REFUSED_LOGIN));
        assert_eq!(
            refused(phone, ext, Some(HIS), Some(HIS)),
            Some(REFUSED_LOGIN)
        );
        assert_eq!(
            refused(None, ext, Some(HIS), Some(HIS)),
            Some(REFUSED_LOGIN)
        );
        assert_eq!(
            refused(here, ext, None, None),
            Some(REFUSED_LOGIN),
            "unset at start"
        );
        let evil = Some("https://evil.example");
        assert_eq!(
            refused(here, evil, Some(HIS), Some(HIS)),
            Some(crate::triage::REFUSED_ORIGIN)
        );
        assert!(
            asked("POST /extension/login?x HTTP/1.1") && !asked("GET /extension/login HTTP/1.1")
        );
    }

    #[dash_test]
    fn a_site_maps_to_the_secret_its_automation_reads() {
        let linkedin = Some(("linkedin-password", "LINKEDIN_EMAIL", "LINKEDIN_PASSWORD"));
        assert_eq!(secret_of("www.LinkedIn.com"), linkedin);
        assert_eq!(secret_of("https://www.linkedin.com/login?x=1"), linkedin);
        assert_eq!(secret_of("free-work.com").map(|s| s.0), Some("freework"));
        assert_eq!(secret_of("www.linkedin.com."), linkedin, "fully qualified");
        assert_eq!(secret_of("evil-linkedin.com"), None);
        assert_eq!(secret_of("linkedin.com.evil.example"), None);
        assert_eq!(secret_of("klar.im"), None);
    }

    /// Done-when 1: the three fields at once, written with bin/secret (a stand-in here, recording its arguments and stdin;
    /// the real one in dash's login test), each value on stdin; a missing field or an unknown site writes nothing.
    #[dash_test]
    fn a_login_is_written_by_bin_secret_its_values_on_stdin() {
        let dir = TestDir::new("login-keep");
        let log = dir.0.join("log");
        // records each call; `get` answers the user kept before; a password set fails when FAIL is set
        let secret = dash_meter::stand_in(
            "#!/bin/sh\necho \"$@ $(cat)\" >> \"$LOG\"\ncase \"$*\" in get*) echo old@example.com;; *PASSWORD*) [ -z \"$FAIL\" ] || exit 1;; esac\n",
        );
        let form = |site: &str, user: &str, pw: &str| {
            BTreeMap::from(
                [("site", site), ("user", user), ("password", pw)]
                    .map(|(k, v)| (k.to_string(), v.to_string())),
            )
        };
        let env = |c: &mut Command| {
            c.env("LOG", &log);
        };
        let said = keep(
            &secret,
            &form("www.linkedin.com", "me@example.com", "pw 1"),
            env,
        )
        .expect("kept");
        assert!(
            !said.contains("pw 1") && said.contains("linkedin-password"),
            "{said}"
        );
        assert_eq!(
            std::fs::read_to_string(&log).expect("log"),
            "get linkedin-password LINKEDIN_EMAIL \nset linkedin-password LINKEDIN_EMAIL me@example.com\n\
             set linkedin-password LINKEDIN_PASSWORD pw 1\n"
        );
        std::fs::remove_file(&log).expect("reset");
        // the password refused: the user kept before is put back
        let failing = |c: &mut Command| {
            c.env("LOG", &log).env("FAIL", "1");
        };
        let (status, why) = keep(
            &secret,
            &form("linkedin.com", "new@example.com", "pw"),
            failing,
        )
        .unwrap_err();
        assert_eq!(status, "500 Internal Server Error", "{why}");
        assert!(
            std::fs::read_to_string(&log)
                .expect("log")
                .ends_with("set linkedin-password LINKEDIN_EMAIL old@example.com\n"),
            "the old user put back"
        );
        std::fs::remove_file(&log).expect("reset");
        assert_eq!(
            keep(&secret, &form("klar.im", "a", "b"), env)
                .unwrap_err()
                .0,
            "400 Bad Request"
        );
        assert_eq!(
            keep(&secret, &form("linkedin.com", "a", ""), env)
                .unwrap_err()
                .0,
            "400 Bad Request"
        );
        assert_eq!(
            keep(&secret, &form("linkedin.com", "a\nB=c", "b"), env)
                .unwrap_err()
                .0,
            "400 Bad Request"
        );
        for pw in [" pw", "pw ", "\"pw\"", "'pw"] {
            assert_eq!(
                keep(&secret, &form("linkedin.com", "a", pw), env)
                    .unwrap_err()
                    .0,
                "400 Bad Request",
                "{pw:?} would be read back as another password"
            );
        }
        assert!(!log.exists(), "nothing written");
    }
}
