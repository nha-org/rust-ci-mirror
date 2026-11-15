//! What the lint's tools and its browser rule are told, a leaf both the crate root and `lint` read: `lint::lint_config` names
//! this file, so a change here runs every tool on every file, as a change to Dashboard.Lint's @deno and @browser_launch did
//! (/code-review of PR 1519).

/// What starts a browser, `Dashboard.Lint`'s @browser_launch: reaching one over CDP is none. Split so this source starts none:
/// the lint greps the whole checkout. The headless flag (index 4) counts only whole, never as a longer word's prefix.
pub const BROWSER_LAUNCH: [&str; 7] = [
    concat!("chromium", ".launch"),
    concat!("launch_persistent", "_context"),
    concat!("launchPersistent", "Context"),
    concat!("spawn", "(bin"),
    concat!("--head", "less"),
    concat!("--print-", "to-pdf"),
    concat!("--remote-debugging-", "port=0"),
];

/// Whether `text` names one of `patterns`, each of [`BROWSER_LAUNCH`]: its headless flag only whole, never a longer word's
/// prefix.
pub fn launches(text: &str, patterns: &[&str]) -> bool {
    patterns.iter().any(|p| {
        text.match_indices(p).any(|(i, _)| {
            let next = text[i + p.len()..].chars().next();
            *p != BROWSER_LAUNCH[4] || next.is_none_or(|c| !(c.is_alphanumeric() || c == '_'))
        })
    })
}

/// Whether git takes the file at `path` for binary, as `git grep -I` skips it: a NUL byte in its first 8000.
pub fn binary(path: &std::path::Path) -> bool {
    let mut head = Vec::with_capacity(8000);
    let file = std::fs::File::open(path).expect("open");
    std::io::Read::read_to_end(&mut std::io::Read::take(file, 8000), &mut head).expect("read");
    head.contains(&0)
}

/// Dashboard.Lint's `@deno` exclusions; workspace_rules.rs's the_deno_copies_are_mix_lints holds the two equal.
pub const DENO_RULES_EXCLUDE: &str = "--rules-exclude=no-window,no-window-prefix,require-await";
