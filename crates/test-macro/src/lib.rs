//! `#[dash_test]`: a test with its memory budget as its first statement, so no test can forget the line (task
//! rust-test-budget-attr; @nha, 2026-11-12: « always aim for the systemic improvement not the cheap one-off »). Taken
//! through `dash_meter::dash_test`, whose tests prove it.
use proc_macro::{Delimiter, Group, TokenStream, TokenTree};

/// `#[dash_test]` is `#[test]` with `let _budget = dash_meter::memory::TestBudget::new();` opening the body.
/// `#[dash_test(<expr>)]` takes that expression as the guard instead: `TestBudget::of(100.0)`, or the gate's `rule!()`.
/// Then git's global and system config are cut off for the test's process (`GIT_CONFIG_GLOBAL=/dev/null`,
/// `GIT_CONFIG_NOSYSTEM=1`) and it guesses no identity (`user.useConfigOnly`), so a fixture names its own, as on CI; every
/// other `GIT_*` variable it inherited is removed first.
#[proc_macro_attribute]
pub fn dash_test(guard: TokenStream, item: TokenStream) -> TokenStream {
    let guard = match guard.is_empty() {
        true => "::dash_meter::memory::TestBudget::new()"
            .parse()
            .expect("a path"),
        false => guard,
    };
    let mut tokens: Vec<TokenTree> = item.into_iter().collect();
    // A fn item ends on its body, the one brace group at its end.
    let body = match tokens.pop() {
        Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Brace => g,
        _ => {
            return "::core::compile_error!(\"#[dash_test] goes on a fn with a body\");"
                .parse()
                .expect("an item");
        }
    };
    // A body's inner attributes (`#![allow(..)]`) must stay first, so the guard goes after them (/code-review).
    // An outer `#[..]` there belongs to the first statement and stays with it.
    let body_tokens: Vec<TokenTree> = body.stream().into_iter().collect();
    let punct = |i: usize, c: char| matches!(body_tokens.get(i), Some(TokenTree::Punct(p)) if p.as_char() == c);
    let mut lead = 0;
    while punct(lead, '#') && punct(lead + 1, '!') {
        lead += 3;
    }
    let lead = lead.min(body_tokens.len());
    let mut inner: TokenStream = body_tokens[..lead].iter().cloned().collect();
    inner.extend("let _budget =".parse::<TokenStream>().expect("a let"));
    inner.extend(guard);
    inner.extend(";".parse::<TokenStream>().expect("a semicolon"));
    // Git reads no global or system config, as on CI, so a fixture names its own identity (task rust-tests-git-hermetic:
    // PR 1591's merge fixture passed on the Mac's global identity and failed on CI with « Committer identity unknown »).
    // `user.useConfigOnly` too: without it the Mac's git guesses `<user>@<host>.local` and the fixture still passes here,
    // where a CI runner's dotless hostname gives no guess. Every inherited GIT_* goes first: a hook running the test (the
    // pre-commit's rust-gate) exports GIT_DIR and GIT_INDEX_FILE, which aim a fixture's git at the real repository, and
    // GIT_AUTHOR_*, an identity the fixture did not name (/code-review). After the guard: every guard is a TestBudget,
    // whose nextest check means one test per process, so no other thread reads the environment while it is set.
    let git_env = [
        ("GIT_CONFIG_GLOBAL", "/dev/null"),
        ("GIT_CONFIG_NOSYSTEM", "1"),
        ("GIT_CONFIG_COUNT", "1"),
        ("GIT_CONFIG_KEY_0", "user.useConfigOnly"),
        ("GIT_CONFIG_VALUE_0", "true"),
    ]
    .map(|(k, v)| format!("::std::env::set_var({k:?}, {v:?});"))
    .concat();
    inner.extend(
        format!(
            "unsafe {{ for (k, _) in ::std::env::vars_os() {{ if k.to_string_lossy().starts_with(\"GIT_\") {{ ::std::env::remove_var(k); }} }} {git_env} }}"
        )
            .parse::<TokenStream>()
            .expect("the git environment"),
    );
    inner.extend(body_tokens[lead..].iter().cloned());
    let mut wrapped = Group::new(Delimiter::Brace, inner);
    wrapped.set_span(body.span());
    tokens.push(TokenTree::Group(wrapped));
    let mut out: TokenStream = "#[::core::prelude::v1::test]"
        .parse()
        .expect("the attribute");
    out.extend(tokens);
    out
}

/// `#[wall_budget("<the bound and why a shared runner meets it>")]`: a test asserting an elapsed time under 1 s names its
/// budget here, or dash-gate's `a_sub_second_wall_bound_names_its_budget` fails it (task rust-store-rebuild-write-flake: such
/// bounds reddened PRs 1287, 1476, 1496, 1498, 1510 and 1525). The item is left as it is. Its argument is one string literal
/// holding a number, the bound: `#[wall_budget(1)]` or `#[wall_budget("fast")]` does not build (/code-review).
#[proc_macro_attribute]
pub fn wall_budget(why: TokenStream, item: TokenStream) -> TokenStream {
    let tokens: Vec<TokenTree> = why.into_iter().collect();
    let named = match tokens.as_slice() {
        [TokenTree::Literal(l)] => {
            let text = l.to_string();
            text.trim_start_matches(['r', '#']).starts_with('"')
                && text.chars().any(|c| c.is_ascii_digit())
        }
        _ => false,
    };
    match named {
        true => item,
        false => "::core::compile_error!(\"#[wall_budget(\\\"<the bound, in ms or s, and why a shared runner meets it>\\\")]\");"
            .parse()
            .expect("an item"),
    }
}
