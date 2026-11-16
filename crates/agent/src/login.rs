//! A Claude login's own pieces, phase 3 of rfc-rust-unified 20b (task rust-login-signin): its profile folder, linked to
//! `~/.claude` as `Dashboard.ClaudeAccounts.add` links it, its keychain item, the pane that signs it in, and its removal.
//! What claude-swap does, never how: nothing here reads a token, only whether its keychain item exists.
use dash_core::tasks::Terminal;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// What a profile never shares with `~/.claude`: its own account, and the profiles themselves (`@own`).
const OWN: [&str; 6] = [
    ".",
    "..",
    "accounts",
    ".claude.json",
    ".credentials.json",
    ".DS_Store",
];

/// The keys of `~/.claude.json` a new profile starts without: the account, what Claude Code caches for it, and an API key,
/// which would bill his own login (`@per_account`).
fn per_account(key: &str) -> bool {
    const STARTS: [&str; 9] = [
        "oauthAccount",
        "userID",
        "cached",
        "s1m",
        "passes",
        "overage",
        "subscription",
        "hasAvailable",
        "maxSubscription",
    ];
    STARTS.iter().any(|s| key.starts_with(s)) || key.ends_with("Cache") || key.contains("ApiKey")
}

/// The shape `@name` allows: what `logins` lists and `add` makes.
pub fn shaped(name: &str) -> bool {
    let ok = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit();
    name.len() <= 31 && name.starts_with(ok) && name.chars().all(|c| ok(c) || c == '-')
}

/// A login's name as `@name` allows, never `default` or `active`.
pub fn valid(name: &str) -> bool {
    shaped(name) && !matches!(name, "default" | "active")
}

/// Whether the login keychain holds an item. The Mac's `security` live, a set in a test.
pub trait Keychain {
    fn holds(&self, item: &str) -> Result<bool, String>;
}

/// The Mac's login keychain through `security`: attributes only, never the secret, so no prompt either.
pub struct Security;

impl Keychain for Security {
    fn holds(&self, item: &str) -> Result<bool, String> {
        let args = ["find-generic-password", "-s", item];
        match dash_process::run("security", &args, None, Duration::from_secs(10)) {
            Ok(_) => Ok(true),
            // security's exit 44 and its words when no such item.
            Err(e) if e.contains("could not be found") => Ok(false),
            Err(e) => Err(e),
        }
    }
}

/// The keychain item Claude Code keeps a folder's token in, as `Dashboard.ClaudeAccounts.keychain_item`: « Claude
/// Code-credentials » for `~/.claude`, with the first 8 hex of the folder string's sha256 for a profile.
pub fn item(folder: Option<&Path>) -> String {
    use sha2::Digest;
    match folder {
        None => "Claude Code-credentials".into(),
        Some(dir) => {
            let digest = sha2::Sha256::digest(dir.to_string_lossy().as_bytes());
            let hex: String = digest[..4].iter().map(|b| format!("{b:02x}")).collect();
            format!("Claude Code-credentials-{hex}")
        }
    }
}

/// `claude` is `~/.claude`; `None` for `default`, the variable unset, since the item is named after that exact string.
pub fn folder(claude: &Path, name: &str) -> Option<PathBuf> {
    (name != "default").then(|| claude.join("accounts").join(name))
}

/// « signed in » once its keychain item exists, else « not signed in ».
pub fn status(claude: &Path, name: &str, keychain: &dyn Keychain) -> Result<&'static str, String> {
    let held = keychain.holds(&item(folder(claude, name).as_deref()))?;
    Ok(if held { "signed in" } else { "not signed in" })
}

/// What `add` makes, or would make on a dry run.
#[derive(Debug, PartialEq)]
pub struct Plan {
    pub folder: PathBuf,
    /// The entries of `~/.claude` it links.
    pub links: Vec<String>,
    /// The line its sign-in pane runs.
    pub sign_in: String,
    /// The keychain item it reads « signed in » by.
    pub item: String,
    /// The item is there already: a removed login's token stayed, and Claude Code takes it up, so no sign-in.
    pub signed_in: bool,
}

impl std::fmt::Display for Plan {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        writeln!(f, "folder: {}", self.folder.display())?;
        writeln!(f, "links: {}", self.links.join(" "))?;
        if self.signed_in {
            return write!(f, "sign-in: none, the keychain holds {} already", self.item);
        }
        let name = self.folder.file_name().unwrap_or_default();
        let name = name.to_string_lossy();
        writeln!(
            f,
            "sign-in: a new pane « login {name} » runs {}",
            self.sign_in
        )?;
        write!(f, "signed in once the keychain holds: {}", self.item)
    }
}

/// The folder single-quoted: a space or a `$` in it never splits the line, which would sign another folder in.
fn sign_in_line(dir: &Path) -> String {
    let quoted = dir.to_string_lossy().replace('\'', "'\\''");
    format!("CLAUDE_CONFIG_DIR='{quoted}' claude auth login")
}

/// A new profile `name` under `claude` (`~/.claude`), its `.claude.json` seeded from `seed` (`~/.claude.json`) without its
/// account. A folder already there with no login is linked again: a sign-in that never finished. `make` false is the dry run.
pub fn add(
    claude: &Path,
    seed: &Path,
    name: &str,
    keychain: &dyn Keychain,
    make: bool,
) -> Result<Plan, String> {
    if !valid(name) {
        return Err(
            "a name is lower case letters, digits and dashes, not default or active".into(),
        );
    }
    let dir = claude.join("accounts").join(name);
    let item = item(Some(&dir));
    let signed_in = keychain.holds(&item)?;
    if signed_in && dir.exists() {
        return Err(format!("{name} exists, signed in"));
    }
    let links = to_link(claude, &dir);
    if make {
        let failed = |e: std::io::Error| format!("{}: {e}", dir.display());
        let fresh = !dir.exists();
        std::fs::create_dir_all(&dir).map_err(failed)?;
        if fresh {
            seed_json(seed, &dir)?;
        }
        for e in &links {
            // One made meanwhile (a second add, Claude Code itself) is no failure.
            match std::os::unix::fs::symlink(claude.join(e), dir.join(e)) {
                Err(e) if e.kind() != std::io::ErrorKind::AlreadyExists => return Err(failed(e)),
                _ => {}
            }
        }
        drop_dead_links(&dir);
    }
    Ok(Plan {
        sign_in: sign_in_line(&dir),
        folder: dir,
        links,
        item,
        signed_in,
    })
}

/// A temporary or lock file, as `link/1`'s `\.(tmp|lock)(\.|$)`.
fn temporary(e: &str) -> bool {
    [".tmp", ".lock"].iter().any(|s| {
        let mut rest = e.match_indices(s).map(|(i, _)| &e[i + s.len()..]);
        rest.any(|r| r.is_empty() || r.starts_with('.'))
    })
}

/// Every entry of `claude` the profile lacks: Claude Code adds folders over time. Never a temporary or lock file.
fn to_link(claude: &Path, dir: &Path) -> Vec<String> {
    let names = std::fs::read_dir(claude).into_iter().flatten().flatten();
    let names = names.filter_map(|e| e.file_name().into_string().ok());
    let mut links: Vec<String> = names
        .filter(|e| !OWN.contains(&e.as_str()) && !temporary(e))
        .filter(|e| dir.join(e).symlink_metadata().is_err())
        .collect();
    links.sort();
    links
}

/// A link whose target went is dropped.
fn drop_dead_links(dir: &Path) {
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.symlink_metadata().is_ok_and(|m| m.is_symlink()) && !p.exists() {
            let _ = std::fs::remove_file(p);
        }
    }
}

/// His own `.claude.json` without its account; none when he has none, as `seed/1`.
fn seed_json(seed: &Path, dir: &Path) -> Result<(), String> {
    let Ok(text) = std::fs::read_to_string(seed) else {
        return Ok(());
    };
    let Ok(Value::Object(mut map)) = serde_json::from_str(&text) else {
        return Ok(());
    };
    map.retain(|k, _| !per_account(k));
    let to = dir.join(".claude.json");
    std::fs::write(&to, Value::Object(map).to_string())
        .map_err(|e| format!("{}: {e}", to.display()))
}

/// A new pane in `claude` labelled « login <name> » that runs `claude auth login` on the profile's folder: never `/login` in a
/// running session, never on `default` or a profile that holds a login, whose token it would overwrite.
pub fn sign_in(
    claude: &Path,
    name: &str,
    keychain: &dyn Keychain,
    terminal: &dyn Terminal,
) -> Result<String, String> {
    let Some(dir) = folder(claude, name).filter(|_| valid(name)) else {
        return Err(format!(
            "{name}: only a free profile signs in here, never default"
        ));
    };
    if !dir.is_dir() {
        return Err(format!("no login {name}: add it first"));
    }
    if keychain.holds(&item(Some(&dir)))? {
        return Err(format!(
            "{name} holds a login of its own: a sign-in there would sign it out"
        ));
    }
    let pane = terminal.open(&claude.to_string_lossy(), &format!("login {name}"))?;
    terminal.run(&pane, &sign_in_line(&dir))?;
    Ok(pane)
}

/// The profile's folder removed, refused while a session of `running` (the Claude sessions open now, `None` one with no id
/// yet) may run on it. A session's login is its status line file in `claude/weekly-limit`, which only site_perso's status line
/// writes: a session with none, of another repo or before its first status line, refuses the removal too, since a folder
/// removed under a session is worse than one kept. The keychain item stays: a token is his to drop, and an add again takes it.
pub fn remove(claude: &Path, name: &str, running: &[Option<String>]) -> Result<PathBuf, String> {
    let Some(dir) = folder(claude, name).filter(|_| valid(name)) else {
        return Err(format!(
            "{name}: default is his own ~/.claude, never removed"
        ));
    };
    if !dir.is_dir() {
        return Err(format!("no login {name}"));
    }
    let config_of = |session: &str| {
        let file = claude.join("weekly-limit").join(format!("{session}.json"));
        let text = std::fs::read_to_string(file).ok()?;
        let v = serde_json::from_str::<Value>(&text).ok()?;
        v["config_dir"].as_str().map(PathBuf::from)
    };
    for session in running {
        let Some(session) = session else {
            return Err(
                "a Claude session with no id yet may run on it: try again in a minute".into(),
            );
        };
        match config_of(session) {
            Some(c) if c == dir => {
                return Err(format!(
                    "session {session} runs on {name}: removed once it ends"
                ));
            }
            Some(_) => {}
            None => {
                return Err(format!(
                    "session {session} names no login (no status line file): removed once it ends"
                ));
            }
        }
    }
    // Its links go, never what they point to: remove_dir_all follows no link.
    std::fs::remove_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_core::tasks::Seen;
    use dash_meter::TestDir;
    use dash_meter::dash_test;
    use std::cell::RefCell;
    use std::collections::HashSet;

    /// A stand-in keychain: the items it holds.
    struct Held(RefCell<HashSet<String>>);

    impl Keychain for Held {
        fn holds(&self, item: &str) -> Result<bool, String> {
            Ok(self.0.borrow().contains(item))
        }
    }

    fn empty() -> Held {
        Held(RefCell::new(HashSet::new()))
    }

    /// A temp home: `.claude` with what Claude Code keeps there, and his `.claude.json`.
    fn home(name: &str) -> (TestDir, PathBuf, PathBuf) {
        let dir = TestDir::new(name);
        let claude = dir.0.join(".claude");
        for d in ["projects", "weekly-limit", "accounts", "skills"] {
            std::fs::create_dir_all(claude.join(d)).expect("dirs");
        }
        for f in [
            "settings.json",
            "CLAUDE.md",
            "history.jsonl.tmp",
            ".credentials.json",
            "x.lock.1",
            "a.lockfile",
            "b.tmpl",
        ] {
            std::fs::write(claude.join(f), "{}").expect("file");
        }
        let seed = dir.0.join(".claude.json");
        let json = r#"{"oauthAccount":{"emailAddress":"him@x"},"userID":"u","primaryApiKey":"k",
                       "mcpServers":{"flare":{}},"theme":"dark","cachedGrowthBookFeatures":{}}"#;
        std::fs::write(&seed, json).expect("seed");
        (dir, claude, seed)
    }

    #[dash_test]
    fn adding_a_login_makes_its_folder_linked_to_claude_as_the_elixir_app_does() {
        let (_dir, claude, seed) = home("login-add");
        let dry = add(&claude, &seed, "fourth", &empty(), false).expect("a plan");
        assert!(!dry.folder.exists(), "a dry run makes nothing");
        let plan = add(&claude, &seed, "fourth", &empty(), true).expect("made");
        assert_eq!(plan, dry, "the dry run says what the add makes");
        let dir = claude.join("accounts/fourth");
        assert_eq!(plan.folder, dir);
        assert_eq!(
            plan.links,
            [
                "CLAUDE.md",
                "a.lockfile",
                "b.tmpl",
                "projects",
                "settings.json",
                "skills",
                "weekly-limit"
            ],
            "only a temporary or lock file is left out, as link/1's pattern"
        );
        for e in &plan.links {
            assert_eq!(
                std::fs::read_link(dir.join(e)).expect("a link"),
                claude.join(e)
            );
        }
        for own in [
            "accounts",
            ".credentials.json",
            "history.jsonl.tmp",
            "x.lock.1",
        ] {
            assert!(
                dir.join(own).symlink_metadata().is_err(),
                "{own} is never linked"
            );
        }
        let json: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join(".claude.json")).expect("json"))
                .expect("json");
        assert_eq!(
            json,
            serde_json::json!({"mcpServers": {"flare": {}}, "theme": "dark"})
        );
        assert_eq!(
            plan.sign_in,
            format!("CLAUDE_CONFIG_DIR='{}' claude auth login", dir.display())
        );
        let odd = sign_in_line(Path::new("/h/a b/it's $x"));
        assert_eq!(
            odd,
            r#"CLAUDE_CONFIG_DIR='/h/a b/it'\''s $x' claude auth login"#
        );
        // A folder already there links what it lacks again and drops a dead link; its own .claude.json stays.
        std::fs::remove_file(dir.join("skills")).expect("unlink");
        std::os::unix::fs::symlink(claude.join("gone"), dir.join("gone")).expect("dead link");
        std::fs::write(dir.join(".claude.json"), "{}").expect("its own");
        let again = add(&claude, &seed, "fourth", &empty(), true).expect("again");
        assert_eq!(again.links, ["skills"]);
        assert!(
            dir.join("gone").symlink_metadata().is_err(),
            "a dead link goes"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join(".claude.json")).expect("json"),
            "{}"
        );
        for bad in ["default", "active", "Upper", "-x", ""] {
            assert!(add(&claude, &seed, bad, &empty(), false).is_err(), "{bad}");
        }
    }

    #[dash_test]
    fn a_login_reads_not_signed_in_until_its_keychain_item_exists() {
        let (_dir, claude, seed) = home("login-status");
        let keychain = empty();
        let plan = add(&claude, &seed, "fourth", &keychain, true).expect("made");
        assert_eq!(status(&claude, "fourth", &keychain), Ok("not signed in"));
        // A profile's item is named after its folder string, as Dashboard.ClaudeAccounts.keychain_item names it.
        let digits = plan
            .item
            .strip_prefix("Claude Code-credentials-")
            .expect("its prefix");
        assert_eq!(digits, &item(Some(&plan.folder))[24..]);
        assert_eq!(digits.len(), 8);
        assert_eq!(status(&claude, "default", &keychain), Ok("not signed in"));
    }

    #[dash_test]
    fn a_login_reads_signed_in_once_its_keychain_item_exists_and_adds_no_more() {
        let (_dir, claude, seed) = home("login-signed");
        let keychain = empty();
        let plan = add(&claude, &seed, "fourth", &keychain, true).expect("made");
        keychain.0.borrow_mut().insert(plan.item.clone());
        assert_eq!(status(&claude, "fourth", &keychain), Ok("signed in"));
        keychain
            .0
            .borrow_mut()
            .insert("Claude Code-credentials".into());
        assert_eq!(status(&claude, "default", &keychain), Ok("signed in"));
        let again = add(&claude, &seed, "fourth", &keychain, false).expect_err("signed in");
        assert!(again.contains("exists"), "{again}");
    }

    #[dash_test]
    fn a_profile_s_item_is_named_as_dash_claude_accounts_names_it() {
        // The item `dash claude-accounts` names for second, a path string only.
        let second = Path::new("/Users/nicolasha/.claude/accounts/second");
        assert_eq!(item(Some(second)), "Claude Code-credentials-b1ddf444");
        assert_eq!(item(None), "Claude Code-credentials");
    }

    /// A stand-in terminal: what it was asked to open and run.
    #[derive(Default)]
    struct Panes(RefCell<Vec<String>>);

    impl Terminal for Panes {
        fn open(&self, folder: &str, label: &str) -> Result<String, String> {
            self.0
                .borrow_mut()
                .push(format!("open {folder} « {label} »"));
            Ok("p1".into())
        }
        fn run(&self, pane: &str, line: &str) -> Result<(), String> {
            self.0.borrow_mut().push(format!("run {pane} {line}"));
            Ok(())
        }
        fn agent(&self, _: &str) -> Result<Option<Seen>, String> {
            Ok(None)
        }
        fn sessions(&self) -> Result<usize, String> {
            Ok(0)
        }
        fn read(&self, _: &str) -> Result<String, String> {
            Ok(String::new())
        }
        fn send(&self, pane: &str, text: &str) -> Result<(), String> {
            self.0.borrow_mut().push(format!("send {pane} {text}"));
            Ok(())
        }
        fn key(&self, pane: &str, key: &str) -> Result<(), String> {
            self.0.borrow_mut().push(format!("key {pane} {key}"));
            Ok(())
        }
        fn close(&self, _: &str) -> Result<(), String> {
            Ok(())
        }
    }

    #[dash_test]
    fn its_sign_in_opens_a_new_pane_on_the_free_profile_never_login_in_a_session() {
        let (_dir, claude, seed) = home("login-sign-in");
        let keychain = empty();
        let plan = add(&claude, &seed, "fourth", &keychain, true).expect("made");
        let panes = Panes::default();
        assert_eq!(
            sign_in(&claude, "fourth", &keychain, &panes),
            Ok("p1".into())
        );
        assert_eq!(
            *panes.0.borrow(),
            [
                format!("open {} « login fourth »", claude.display()),
                format!("run p1 {}", plan.sign_in)
            ],
            "a pane of its own, its shell running claude auth login: nothing typed into a session"
        );
        let refused = Panes::default();
        assert!(sign_in(&claude, "default", &keychain, &refused).is_err());
        assert!(sign_in(&claude, "nobody", &keychain, &refused).is_err());
        keychain.0.borrow_mut().insert(plan.item);
        let held = sign_in(&claude, "fourth", &keychain, &refused).expect_err("holds a login");
        assert!(held.contains("sign it out"), "{held}");
        assert!(refused.0.borrow().is_empty(), "a refusal opens no pane");
    }

    #[dash_test]
    fn removing_a_login_is_refused_while_a_session_runs_on_it() {
        let (_dir, claude, seed) = home("login-remove");
        let plan = add(&claude, &seed, "fourth", &empty(), true).expect("made");
        let line = |session: &str, config: &Path| {
            let text = serde_json::json!({"config_dir": config, "session_id": session}).to_string();
            std::fs::write(
                claude.join("weekly-limit").join(format!("{session}.json")),
                text,
            )
            .expect("status line");
        };
        line("s-on-it", &plan.folder);
        line("s-elsewhere", &claude.join("accounts/second"));
        let id = |s: &str| Some(s.to_string());
        let running = [id("s-elsewhere"), id("s-on-it")];
        let refused = remove(&claude, "fourth", &running).expect_err("a session runs on it");
        assert!(refused.contains("s-on-it"), "{refused}");
        // A session whose login is unknown, of another repo or with no id yet, refuses it too: it may run there.
        let unknown = remove(&claude, "fourth", &[id("s-klar")]).expect_err("unknown");
        assert!(unknown.contains("names no login"), "{unknown}");
        assert!(remove(&claude, "fourth", &[None]).is_err(), "no id yet");
        assert!(plan.folder.is_dir(), "refused, the folder stays");
        assert!(remove(&claude, "default", &[]).is_err(), "never default");
        // Once that session ended, the folder goes and what its links point to stays.
        assert_eq!(
            remove(&claude, "fourth", &running[..1]),
            Ok(plan.folder.clone())
        );
        assert!(!plan.folder.exists());
        assert!(claude.join("projects").is_dir() && claude.join("settings.json").is_file());
    }

    #[dash_test]
    fn a_login_removed_signed_in_adds_again_with_its_token_and_no_sign_in() {
        let (_dir, claude, seed) = home("login-readd");
        let keychain = empty();
        let plan = add(&claude, &seed, "fourth", &keychain, true).expect("made");
        keychain.0.borrow_mut().insert(plan.item.clone());
        remove(&claude, "fourth", &[]).expect("removed");
        let again = add(&claude, &seed, "fourth", &keychain, true).expect("added again");
        assert!(again.signed_in && again.folder.is_dir(), "{again:?}");
        assert!(again.to_string().contains("sign-in: none"), "{again}");
    }
}
