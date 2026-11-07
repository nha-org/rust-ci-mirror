//! theme.css here is dash's, generated from design.json, with Tailwind's wrappers turned into plain CSS: the window has
//! no Tailwind, and rust/ alone rides to the CI mirror, so the file is copied, and a test fails when the copy is stale.

/// dash's assets/css/theme.css as plain rules on the app's root.
pub fn from_dash(source: &str) -> String {
    let mut out = String::from("/* Generated from private/dashboard/assets/css/theme.css by theme::from_dash: UPDATE_THEME=1 cargo test. */\n");
    for line in source.lines().skip(1) {
        let t = line.trim();
        if t.starts_with("@plugin") {
            continue;
        }
        if let Some(n) = t.strip_prefix("name: \"") {
            out += &format!(".app.{} {{\n", n.trim_end_matches("\";"));
        } else if t.starts_with("@theme") {
            out += ".app {\n";
        } else if !["default:", "prefersdark:", "color-scheme:"].iter().any(|p| t.starts_with(p)) {
            out += line;
            out.push('\n');
        }
    }
    out
}
