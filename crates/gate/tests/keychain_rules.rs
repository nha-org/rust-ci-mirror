//! bin/no-keychain-test's cases, each now a dash_gate::keychain rule held here (task chrome-mock-keychain-all), and the
//! headless launch rule on stand-in launchers. lint_rules.rs's the_tree_passes_every_rule holds the tree to both.
use dash_gate::BROWSER_LAUNCH;
use dash_gate::keychain::{LOGIN_FILES, READ, launches_without_mock, reads};
use dash_meter::TestDir;
use dash_meter::dash_test;
use std::fs;
use std::path::Path;

/// A tree that passes: each login file, the MCP's and the sign-in's lines as they are, klar-mail's allowed read.
fn passing() -> Vec<(String, String)> {
    let mut files: Vec<(String, String)> = LOGIN_FILES
        .iter()
        .map(|f| (f.to_string(), "#!/bin/sh\necho ok\n".to_string()))
        .collect();
    let set = |files: &mut Vec<(String, String)>, path: &str, text: String| {
        files.retain(|(p, _)| p != path);
        files.push((path.to_string(), text));
    };
    set(
        &mut files,
        "private/dashboard/priv/commands/mcp-linkedin",
        "#!/bin/sh\n# exec uv run the old way\nAUTO_IMPORT_FROM_BROWSER=false LOGIN_COMMAND=x exec uv run fork\n".into(),
    );
    set(
        &mut files,
        "bin/linkedin-login",
        "#!/bin/sh\nAUTO_IMPORT_FROM_BROWSER=false \"$here/bin/linkedin-login-fill\"\n".into(),
    );
    set(
        &mut files,
        "bin/linkedin-login-fill",
        "fork([\"--login\"])\n".into(),
    );
    files.push((
        "private/dashboard/priv/commands/klar-mail".into(),
        format!("    r = subprocess.run([\"security\", \"{READ}\", \"-s\", \"klar-mail\", \"-a\", USER, \"-w\"])\n"),
    ));
    files
}

/// `files` written under a fresh folder, then both rules' findings on it.
fn found(name: &str, files: &[(String, String)]) -> (Vec<String>, Vec<String>) {
    let dir = TestDir::new(name);
    write(&dir.0, files);
    let listed: Vec<(String, String)> = files
        .iter()
        .map(|(p, _)| ("100644".into(), p.clone()))
        .collect();
    // the lint's own scope for both rules (/code-review of #1636)
    let scope = dash_gate::lint::launch_files(&listed);
    (reads(&dir.0, &scope), launches_without_mock(&dir.0, &scope))
}

fn write(root: &Path, files: &[(String, String)]) {
    for (path, text) in files {
        let at = root.join(path);
        fs::create_dir_all(at.parent().expect("parent")).expect("mkdir");
        fs::write(at, text).expect("write");
    }
}

/// `passing()` with `path` holding `text` (added, or in place of its text).
fn with(path: &str, text: &str) -> Vec<(String, String)> {
    let mut files = passing();
    files.retain(|(p, _)| p != path);
    files.push((path.into(), text.into()));
    files
}

#[dash_test]
fn the_passing_tree_passes_and_a_missing_login_file_fails() {
    assert_eq!(found("keychain-pass", &passing()), (vec![], vec![]));
    let mut files = passing();
    files.retain(|(p, _)| p != "bin/freework-login");
    let (hits, _) = found("keychain-missing", &files);
    assert_eq!(
        hits,
        [
            "bin/freework-login: a login file is missing; name its successor in dash_gate::keychain::LOGIN_FILES"
        ]
    );
}

#[dash_test]
fn a_login_file_s_code_reading_the_keychain_or_chrome_fails_its_comments_and_the_agent_label_pass()
{
    for bad in [
        format!("security {READ} -s x -w"),
        "security dump-keychain".into(),
        "security  export -k login".into(),
        "key = 'Chrome Safe Storage'".into(),
        "import browser_cookie3".into(),
        "fork --import-from-browser".into(),
        "launchctl kickstart login-bridge".into(),
        "bin/login-ctl start".into(),
    ] {
        let (hits, _) = found(
            "keychain-login",
            &with("bin/secret", &format!("#!/bin/sh\n{bad}\n")),
        );
        // a keychain read is also one outside the allowed ones, as the script said both
        assert_eq!(
            hits[0],
            format!("bin/secret:2: reads the keychain or Chrome: {bad}"),
            "{bad}"
        );
        assert_eq!(hits.len(), 1 + usize::from(bad.contains(READ)), "{hits:?}");
    }
    let fine = "#!/bin/sh\n  # the old route read Chrome Safe Storage\nlaunchctl remove -llabel=com.nicolasha.login-bridge\n";
    assert_eq!(
        found("keychain-comment", &with("bin/crm-phone", fine)).0,
        Vec::<String>::new(),
        "a comment and the agent label crm-phone removes are no read: let them pass"
    );
}

#[dash_test]
fn the_mcp_and_the_sign_in_run_the_fork_with_its_browser_import_off() {
    let mcp = "private/dashboard/priv/commands/mcp-linkedin";
    for exec in [
        "exec uv run fork",
        "LOGIN_COMMAND=x AUTO_IMPORT_FROM_BROWSER=false exec uv run fork",
        "AUTO_IMPORT_FROM_BROWSER=true LOGIN_COMMAND=x exec uv run fork",
    ] {
        let (hits, _) = found("keychain-mcp", &with(mcp, &format!("#!/bin/sh\n{exec}\n")));
        assert_eq!(
            hits,
            [format!(
                "mcp-linkedin starts the fork without AUTO_IMPORT_FROM_BROWSER=false and LOGIN_COMMAND: {exec}"
            )]
        );
    }
    // a comment after code never passes for the real exec below it
    let two = "#!/bin/sh\nfoo # exec uv run AUTO_IMPORT_FROM_BROWSER=false LOGIN_COMMAND=x\nexec uv run fork\n";
    let (hits, _) = found("keychain-mcp-comment", &with(mcp, two));
    assert_eq!(hits.len(), 1, "{hits:?}");
    let (hits, _) = found(
        "keychain-login-on",
        &with("bin/linkedin-login", "\"$here/bin/linkedin-login-fill\"\n"),
    );
    assert_eq!(
        hits,
        ["bin/linkedin-login signs in with the fork's browser import on"]
    );
    let (hits, _) = found(
        "keychain-no-fill",
        &with("bin/linkedin-login", "AUTO_IMPORT_FROM_BROWSER=false\n"),
    );
    assert_eq!(
        hits,
        ["bin/linkedin-login no longer runs bin/linkedin-login-fill"]
    );
    let (hits, _) = found(
        "keychain-no-fork",
        &with("bin/linkedin-login-fill", "fork([])\n"),
    );
    assert_eq!(
        hits,
        [
            "bin/linkedin-login-fill no longer runs the fork's --login (one window of the MCP's own profile)"
        ]
    );
}

#[dash_test]
fn a_keychain_read_outside_the_allowed_ones_fails_and_prose_passes() {
    let read = format!("let args = [\"{READ}\", \"-s\", \"other\", \"-w\"];");
    let (hits, _) = found("keychain-other", &with("rust/crates/x/src/lib.rs", &read));
    assert_eq!(
        hits,
        [format!(
            "rust/crates/x/src/lib.rs:1: a keychain read outside the allowed ones: {read}"
        )]
    );
    // klar-mail's line read for another item is no longer the allowed one
    let klar = format!("    r = subprocess.run([\"security\", \"{READ}\", \"-s\", \"other\"])");
    let (hits, _) = found(
        "keychain-klar-other",
        &with("private/dashboard/priv/commands/klar-mail", &klar),
    );
    assert_eq!(hits.len(), 1, "{hits:?}");
    // dash mail's read pinned to its klar-mail item, the agent's existence check to no -w
    let mail = |item: &str| {
        format!("    let args = [\n        \"{READ}\",\n        \"-s\",\n        {item},\n")
    };
    let at = "rust/crates/dash/src/mail.rs";
    let klar = mail(r#""klar-mail""#);
    assert_eq!(
        found("keychain-mail", &with(at, &klar)).0,
        Vec::<String>::new(),
        "the klar-mail item passes: keep it in READS_ALLOWED"
    );
    assert_eq!(
        found(
            "keychain-mail-other",
            &with(at, &mail("\"Chrome Safe Storage\""))
        )
        .0
        .len(),
        1
    );
    let at = "rust/crates/agent/src/login.rs";
    let check = format!("        let args = [\"{READ}\", \"-s\", item];");
    assert_eq!(
        found("keychain-agent", &with(at, &check)).0,
        Vec::<String>::new(),
        "an existence check passes"
    );
    let secret = check.replace("item];", "item, \"-w\"];");
    assert_eq!(
        found("keychain-agent-w", &with(at, &secret)).0.len(),
        1,
        "its secret fails"
    );
    for prose in [
        "private/docs/rfc-x.html",
        "notes.md",
        "tasks.edn",
        "log.jsonl",
    ] {
        assert_eq!(
            found("keychain-prose", &with(prose, &read)).0,
            Vec::<String>::new(),
            "prose is never read: skip {prose}"
        );
    }
}

#[dash_test]
fn a_stand_in_launcher_starting_headless_chrome_without_the_mock_keychain_fails() {
    let headless = BROWSER_LAUNCH[4];
    let launcher =
        format!("Command::new(chrome).args([\"{headless}\", \"--remote-debugging-pipe\"])");
    for at in [
        "rust/crates/x/src/browser.rs",
        "bin/start-chrome",
        "private/dashboard/test/scripts/start-chrome",
        ".claude/skills/x/render.py",
    ] {
        let (_, hits) = found("keychain-launch", &with(at, &launcher));
        assert_eq!(
            hits,
            [format!(
                "{at}: starts headless Chrome with no --use-mock-keychain"
            )],
            "{at}"
        );
    }
    let spawn = format!(
        "const ch = {}, [...flags, '--hide-scrollbars']);",
        BROWSER_LAUNCH[3]
    );
    // the same refusal in each of the done-when's roots, rust/, bin/ and test/scripts, both launch shapes, through the lint's
    // own scope (launch_files): its review found ours() dropping the frozen private/dashboard/test
    for at in [
        "rust/crates/x/tests/shot.rs",
        "bin/shot",
        "private/dashboard/test/scripts/shot-raw",
        "private/malt/shot",
    ] {
        let (_, hits) = found("keychain-spawn", &with(at, &spawn));
        assert_eq!(
            hits,
            [format!(
                "{at}: starts headless Chrome with no --use-mock-keychain"
            )],
            "{at}"
        );
    }
    // a flag in a comment passes nothing, nor chrome.mjs imported with its flags left out, nor Playwright told to drop its own
    for (at, text) in [
        ("bin/a", format!("// add --use-mock-keychain\n{launcher}")),
        (
            "bin/b",
            format!(
                "import {{ bin }} from './chrome.mjs';\nconst ch = {}, ['--x']);",
                BROWSER_LAUNCH[3]
            ),
        ),
        (
            "bin/c",
            format!("ctx = p.{}(d, ignore_default_args=True)", BROWSER_LAUNCH[1]),
        ),
        // anywhere of ours, not only rust/ and bin/: a skill's or a project's own launcher asks the keychain too
        ("private/other/browser.rs", launcher.clone()),
    ] {
        assert_eq!(
            found("keychain-launch-bad", &with(at, &text)).1.len(),
            1,
            "{at}"
        );
    }
    for (at, text) in [
        (
            "rust/crates/x/src/browser.rs",
            format!("{launcher}.args([\"--use-mock-keychain\"])"),
        ),
        (
            "private/dashboard/test/scripts/shot",
            format!("import {{ bin, flags }} from './chrome.mjs';\n{spawn}"),
        ),
        // Playwright's own launch passes the flag itself
        ("bin/check", format!("browser = p.{}()", BROWSER_LAUNCH[0])),
        ("rust/README.md", launcher.clone()),
        // out of the lint's scope: vendored code, the frozen app and the test folder's fixtures (/code-review of #1636)
        ("bin/vendor/x.sh", launcher.clone()),
        ("private/dashboard/lib/x.sh", launcher.clone()),
        ("private/dashboard/test/fixtures/x.sh", launcher.clone()),
        (
            "rust/crates/x/src/shell.rs",
            format!("let a = \"{headless}_shell\";"),
        ),
    ] {
        assert_eq!(
            found("keychain-launch-ok", &with(at, &text)).1,
            Vec::<String>::new(),
            "a launch with the flag, chrome.mjs's or Playwright's, or in a prose file passes: {at}"
        );
    }
}

/// The rule checks the login paths only in a tree holding one: every one of them is on site_perso's, so a rename of all at
/// once still fails here.
#[dash_test]
fn every_login_file_is_on_the_tree() {
    let Some(top) = dash_meter::monorepo() else {
        return;
    };
    let gone: Vec<&str> = LOGIN_FILES
        .into_iter()
        .filter(|f| !top.join(f).is_file())
        .collect();
    assert_eq!(
        gone,
        Vec::<&str>::new(),
        "name each one's successor in LOGIN_FILES"
    );
}
