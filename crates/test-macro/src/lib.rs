//! `#[dash_test]`: a test with its memory budget as its first statement, so no test can forget the line (task
//! rust-test-budget-attr; @nha, 2026-11-12: « always aim for the systemic improvement not the cheap one-off »). Taken
//! through `dash_meter::dash_test`, whose tests prove it.
use proc_macro::{Delimiter, Group, TokenStream, TokenTree};

/// `#[dash_test]` is `#[test]` with `let _budget = dash_meter::memory::TestBudget::new();` opening the body.
/// `#[dash_test(<expr>)]` takes that expression as the guard instead: `TestBudget::of(100.0)`, or the gate's `rule!()`.
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
