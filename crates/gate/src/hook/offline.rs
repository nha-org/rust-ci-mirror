//! The pre-commit's offline-check (once private/dashboard/test/scripts/offline-check; the Elixir app's templates and JS it
//! read are archived, RFC-0025): a page the CRM serves loads nothing from the internet (@nha, 2026-10-17: « I shouldn't depend on the internet to
//! have a local-first CRM work! »). A link a person clicks (<a href>) is fine; a stylesheet, script, image, font or @import
//! fetched on load is not: copy it into private/docs/vendor with private/dashboard/bin/dash vendor and point at the copy.
use regex::Regex;
use std::path::Path;

// each tag put together with concat!: no tag literal outside dash_surface (the_dash_crate_holds_no_html)
const LOAD: [&str; 7] = [
    concat!("<", r#"link\b[^>]*\bhref=["']?https?://"#),
    concat!("<", r#"script\b[^>]*\bsrc=["']?https?://"#),
    concat!("<", r#"img\b[^>]*\bsrc=["']?https?://"#),
    r#"<(?:source|video|audio|iframe|embed)\b[^>]*\bsrc=["']?https?://"#,
    r#"@import\s+(?:url\()?["']?https?://"#,
    r#"url\(\s*["']?https?://"#,
    // the same, written as EDN or Clojure hiccup (private/store/*.edn)
    r#":(?:href|src)\s+"https?://"#,
];

/// The files the CRM and the dashboard serve, relative to `root`: the docs' HTML, the store's EDN and the dashboard's CSS.
fn served(root: &Path) -> Vec<String> {
    let mut out = vec![];
    let mut walk = |dir: &str, deep: bool, exts: &[&str]| {
        let mut stack = vec![root.join(dir)];
        while let Some(d) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&d) else {
                continue;
            };
            for p in entries.flatten().map(|e| e.path()) {
                if p.is_dir() {
                    if deep {
                        stack.push(p);
                    }
                } else if p
                    .extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| exts.contains(&e))
                {
                    out.push(rel(root, &p));
                }
            }
        }
    };
    walk("private/docs", true, &["html"]);
    walk("private/store", false, &["edn"]);
    walk("private/dashboard/assets/css", false, &["css"]);
    out.sort();
    out
}

fn rel(root: &Path, p: &Path) -> String {
    p.strip_prefix(root)
        .unwrap_or(p)
        .to_string_lossy()
        .into_owned()
}

/// Each line that loads from the internet, as « path:n: line »; the number of files read.
pub fn found(root: &Path) -> (Vec<String>, usize) {
    let load: Vec<Regex> = LOAD
        .iter()
        .map(|p| Regex::new(&format!("(?i){p}")).expect("a pattern"))
        .collect();
    let files = served(root);
    let mut bad = vec![];
    for f in &files {
        let text =
            String::from_utf8_lossy(&std::fs::read(root.join(f)).unwrap_or_default()).into_owned();
        for (n, line) in text.lines().enumerate() {
            if load.iter().any(|r| r.is_match(line)) {
                let short: String = line.trim().chars().take(110).collect();
                bad.push(format!("  {f}:{}: {short}", n + 1));
            }
        }
    }
    (bad, files.len())
}

/// The step: its pass line, or its refusal naming each line.
pub fn check(root: &Path) -> Result<String, String> {
    let (bad, n) = found(root);
    if bad.is_empty() {
        return Ok(format!(
            "offline ok: nothing the CRM or the dashboard serves is fetched from the internet ({n} files)"
        ));
    }
    Err(format!(
        "offline-check: these load something from the internet; vendor it (private/dashboard/bin/dash vendor):\n{}",
        bad.join("\n")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::TestDir;
    use dash_meter::dash_test;

    /// HTML written with ‹ › for < >: the workspace rule the_dash_crate_holds_no_html keeps tag literals in dash_surface alone
    fn h(s: &str) -> &'static str {
        Box::leak(s.replace('‹', "<").replace('›', ">").into_boxed_str())
    }

    #[dash_test]
    fn a_page_loading_from_the_internet_is_named_a_link_is_not() {
        let dir = TestDir::new("offline-check");
        let write = |p: &str, t: &str| {
            let f = dir.0.join(p);
            std::fs::create_dir_all(f.parent().expect("a folder")).expect("mkdir");
            std::fs::write(f, t).expect("write");
        };
        write(
            "private/docs/a.html",
            h("‹a href=\"https://x.org\"›fine‹/a›\n‹SCRIPT src='https://cdn.x/y.js'›‹/script›\n"),
        );
        write(
            "private/store/s.edn",
            "[:link {:href \"https://f.x/a.css\"}]\n",
        );
        // the Elixir app's templates and JS are archived (RFC-0025): nothing serves them from here
        write(
            "private/dashboard/lib/x.ex",
            h("‹link href=\"https://f.x/c.css\"›\n"),
        );
        write(
            "private/dashboard/assets/css/app.css",
            "@import url(\"https://f.x/b.css\");\n",
        );
        let (bad, n) = found(&dir.0);
        assert_eq!(n, 3, "the three served files are read: {bad:?}");
        assert_eq!(
            bad,
            [
                "  private/dashboard/assets/css/app.css:1: @import url(\"https://f.x/b.css\");",
                h("  private/docs/a.html:2: ‹SCRIPT src='https://cdn.x/y.js'›‹/script›"),
                "  private/store/s.edn:1: [:link {:href \"https://f.x/a.css\"}]",
            ],
            "each load named, never the link"
        );
    }
}
