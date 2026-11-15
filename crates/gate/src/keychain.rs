//! No process of ours asks his keychain (rfc-login-no-keychain; @nha, 2026-10-19: « without seeing precisely why this
//! keychain is asked I will NOT allow this »; 2026-11-14: « I get a bunch of popups chrome for testing wants confidential
//! storage information. Why ? »). bin/no-keychain-test's cases as `dash lint` rules (task chrome-mock-keychain-all): no
//! login file names a keychain read, Chrome's cookie key or a Chrome import; the LinkedIn MCP and its sign-in run the fork
//! with its browser import off; no keychain read elsewhere but the allowed ones; and no headless Chrome starts without
//! --use-mock-keychain. The needle is split so this source holds none.
use regex::Regex;
use std::path::Path;

/// A keychain read: `security`'s generic-password lookup.
pub const READ: &str = concat!("find-generic", "-password");

/// The LinkedIn and free-work login paths: each must exist (a successor is named here) and read neither keychain nor Chrome.
pub const LOGIN_FILES: [&str; 8] = [
    "bin/linkedin-login",
    "bin/linkedin-login-fill",
    "bin/freework-login",
    "bin/secret",
    "bin/logins",
    "bin/crm-phone",
    "private/dashboard/priv/commands/mcp-linkedin",
    ".claude/skills/nicolas-daily/freework_apply.py",
];

/// The keychain reads allowed in the tree: a path, the text its line holds, the item that line or the 3 after it name,
/// and whether it may read the secret (`-w`). klar-mail's own mail password (its script and dash mail's Rust port);
/// bin/comment-wake's and the agent's checks that an item exists, which never take `-w` on their line, so never the
/// secret and never a prompt. Frozen Elixir is not read: no-elixir refuses any change there, so it can hold no new read
/// (claude_accounts/keep.ex's login-keep reads, tasks no-keychain-test-keep and no-keychain-keep-ex).
pub const READS_ALLOWED: [(&str, &str, &str, bool); 4] = [
    (
        "private/dashboard/priv/commands/klar-mail",
        concat!(
            r#"r = subprocess.run(["security", "find-generic"#,
            r#"-password", "-s", "klar-mail""#
        ),
        "klar-mail",
        true,
    ),
    (
        "rust/crates/dash/src/mail.rs",
        concat!(r#""find-generic"#, r#"-password","#),
        r#""klar-mail""#,
        true,
    ),
    (
        "bin/comment-wake",
        concat!(
            r#"keychain:*) security find-generic"#,
            r#"-password -s "${n#keychain:}" > /dev/null 2>&1"#
        ),
        "${n#keychain:}",
        false,
    ),
    (
        "rust/crates/agent/src/login.rs",
        concat!(
            r#"let args = ["find-generic"#,
            r#"-password", "-s", item];"#
        ),
        "item",
        false,
    ),
];

/// Whether line `n` of `lines`, a file at `path`, is one of [`READS_ALLOWED`].
fn allowed(path: &str, lines: &[&str], n: usize) -> bool {
    let window = lines[n..lines.len().min(n + 4)].join("\n");
    READS_ALLOWED.iter().any(|(at, text, item, secret)| {
        *at == path
            && lines[n].contains(text)
            && window.contains(item)
            && (*secret || !lines[n].contains("-w"))
    })
}

/// A code line: a `#` comment may say why the old route is gone.
fn code(line: &str) -> bool {
    !line.trim_start().starts_with('#')
}

/// bin/no-keychain-test's cases on the tree at `root`, `files` the tracked files of ours (`dash lint` leaves out frozen
/// Elixir and vendored code, `ratchet::ours`): each finding, empty when it holds.
pub fn reads(root: &Path, files: &[(String, String)]) -> Vec<String> {
    let read = |p: &str| std::fs::read_to_string(root.join(p)).unwrap_or_default();
    let bad = Regex::new(&format!(
        "{READ}|security +(find|dump|export)|Safe Storage|browser_cookie3|import-from-browser|login-bridge|login-ctl"
    ))
    .expect("keychain regex");
    let mut out = vec![];
    // a tree holding none of the login paths (a lint fixture) has none to check; ponytail: all of them renamed at once
    // pass too, caught by keychain_rules.rs's every_login_file_is_on_the_tree
    if LOGIN_FILES.iter().any(|f| root.join(f).is_file()) {
        for f in LOGIN_FILES {
            if !root.join(f).is_file() {
                out.push(format!("{f}: a login file is missing; name its successor in dash_gate::keychain::LOGIN_FILES"));
                continue;
            }
            // the launch agent label bin/crm-phone removes is no read
            for (n, l) in read(f).lines().enumerate() {
                if code(l) && bad.is_match(l) && !l.contains("llabel=com.nicolasha.login-bridge") {
                    out.push(format!(
                        "{f}:{}: reads the keychain or Chrome: {}",
                        n + 1,
                        l.trim()
                    ));
                }
            }
        }
        let mcp = read("private/dashboard/priv/commands/mcp-linkedin");
        // a line's code only, before its comment: a commented exec above the real one never passes for it
        let exec = mcp
            .lines()
            .map(|l| l.split(" #").next().unwrap_or_default())
            .find(|l| code(l) && l.contains("exec uv run"))
            .unwrap_or_default();
        let off = Regex::new("AUTO_IMPORT_FROM_BROWSER=false.*LOGIN_COMMAND=").expect("exec regex");
        if !off.is_match(exec) {
            out.push(format!(
                "mcp-linkedin starts the fork without AUTO_IMPORT_FROM_BROWSER=false and LOGIN_COMMAND: {exec}"
            ));
        }
        let login = read("bin/linkedin-login");
        if !login.contains("AUTO_IMPORT_FROM_BROWSER=false") {
            out.push("bin/linkedin-login signs in with the fork's browser import on".into());
        }
        if !login
            .lines()
            .any(|l| l.ends_with(r#"bin/linkedin-login-fill""#))
        {
            out.push("bin/linkedin-login no longer runs bin/linkedin-login-fill".into());
        }
        if !read("bin/linkedin-login-fill").contains(r#"fork(["--login"])"#) {
            out.push("bin/linkedin-login-fill no longer runs the fork's --login (one window of the MCP's own profile)".into());
        }
    }
    // anywhere else, prose aside (docs, task data, notes): every code file of ours is read, wherever it lives
    for (_, p) in files {
        // a binary file, as git grep -I skipped them: the evidence PDFs, 26 MB each, held past a test's memory budget
        if prose(p) || crate::tool_flags::binary(&root.join(p)) {
            continue;
        }
        let text = read(p);
        let lines: Vec<&str> = text.lines().collect();
        for (n, l) in lines.iter().enumerate() {
            if l.contains(READ) && !allowed(p, &lines, n) {
                out.push(format!(
                    "{p}:{}: a keychain read outside the allowed ones: {}",
                    n + 1,
                    l.trim()
                ));
            }
        }
    }
    out
}

/// Prose, which names a read or a launch and runs none: docs, task data, notes.
fn prose(p: &str) -> bool {
    p.starts_with("private/docs/") || [".edn", ".md", ".jsonl"].iter().any(|e| p.ends_with(e))
}

/// What starts Chrome itself, of [`crate::BROWSER_LAUNCH`]: Playwright's launches (its first three) pass
/// --use-mock-keychain and --password-store=basic on their own (playwright-core's chromiumSwitches.js, verified
/// 2026-11-14) unless told to drop its defaults, so a raw launch is the rest or a Playwright one that drops them.
fn raw_launch(text: &str) -> bool {
    let drops = ["ignore_default_args", "ignoreDefaultArgs"]
        .iter()
        .any(|d| text.contains(d));
    crate::tool_flags::launches(text, &crate::tool_flags::BROWSER_LAUNCH[3..])
        || drops && crate::tool_flags::launches(text, &crate::tool_flags::BROWSER_LAUNCH[..3])
}

/// Whether `text`'s code passes the flag: a `//` or `#` comment naming it passes nothing, nor an import of chrome.mjs
/// whose flags are not spread.
fn passes_mock(text: &str) -> bool {
    let code = text.lines().any(|l| {
        let t = l.trim_start();
        !(t.starts_with("//") || t.starts_with('#')) && l.contains("--use-mock-keychain")
    });
    code || text.contains("chrome.mjs") && text.contains("...flags")
}

/// Each code file of `files` (the lint passes [`crate::lint::launch_files`]: ours and private/dashboard/test), prose,
/// Elixir and binary files aside, that starts headless Chrome with no --use-mock-keychain: Chrome then asks his keychain
/// for its cookie key (task chrome-mock-keychain-all). A script spreading chrome.mjs's flags takes it from there.
/// ponytail: one flag anywhere in a file passes every launch in it; per call needs each language's parser, worth it if a
/// file with two launches forgets one.
pub fn launches_without_mock(root: &Path, files: &[(String, String)]) -> Vec<String> {
    files
        .iter()
        .map(|(_, p)| p)
        // Elixir code is never a launcher: a script would be (lint::TEST_SCRIPTS, fixed by its deletion), an .ex file is the
        // frozen app's, which starts no browser of its own (no_elixir::PATHS: calling it here would close a module loop)
        .filter(|p| !prose(p) && ![".ex", ".exs", ".heex"].iter().any(|e| p.ends_with(e)))
        .filter(|p| !crate::tool_flags::binary(&root.join(p)))
        .filter(|p| {
            let text = std::fs::read_to_string(root.join(p)).unwrap_or_default();
            raw_launch(&text) && !passes_mock(&text)
        })
        .map(|p| format!("{p}: starts headless Chrome with no --use-mock-keychain"))
        .collect()
}
