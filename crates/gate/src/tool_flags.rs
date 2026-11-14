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

/// Dashboard.Lint's `@deno` exclusions; workspace_rules.rs's the_deno_copies_are_mix_lints holds the two equal.
pub const DENO_RULES_EXCLUDE: &str = "--rules-exclude=no-window,no-window-prefix,require-await";
