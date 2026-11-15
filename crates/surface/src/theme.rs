//! The page's theme (task rust-shell-theme-bell): the browser's scheme by default, or the one picked on the menu's switch,
//! kept per browser in a cookie. Below the pages and the menu, which both read it: it reads nothing of theirs.

/// The theme is a class on the root, as the window sets it: a browser asks for dark in this one line.
/// Blocks its port: it runs before the first paint, before any handler could; the CSS route, theme.css keyed on the
/// color scheme, is one Blitz does not read (https://github.com/DioxusLabs/blitz/issues/1077).
pub(crate) const DARK_JS: &str = "if(matchMedia('(prefers-color-scheme: dark)').matches)document.documentElement.className='app dark'";

/// The theme picked on the menu's switch (task rust-shell-theme-bell; the older app's System, Light and Dark): kept per browser
/// in the cookie `THEME_COOKIE`, which `GET THEME_SET?set=<name>` sets, so no script of ours stores it. System, the default,
/// follows the browser's scheme (DARK_JS).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Theme {
    System,
    Light,
    Dark,
}

/// The cookie holding the theme picked.
pub const THEME_COOKIE: &str = "dash-theme";
/// Where the switch's options lead: `/theme?set=<name>`, answered by a redirect back that sets the cookie.
pub const THEME_SET: &str = "/theme";

impl Theme {
    pub const ALL: [Theme; 3] = [Theme::System, Theme::Light, Theme::Dark];

    pub fn name(self) -> &'static str {
        match self {
            Theme::System => "system",
            Theme::Light => "light",
            Theme::Dark => "dark",
        }
    }

    /// Its name as the switch shows it.
    pub fn label(self) -> &'static str {
        match self {
            Theme::System => "System",
            Theme::Light => "Light",
            Theme::Dark => "Dark",
        }
    }

    pub fn named(name: &str) -> Option<Theme> {
        Theme::ALL.into_iter().find(|t| t.name() == name)
    }

    /// The pick a Cookie header holds; System when it holds none.
    pub fn of_cookie(cookie: &str) -> Theme {
        (cookie.split(';'))
            .filter_map(|pair| pair.trim().split_once('='))
            .find(|(name, _)| *name == THEME_COOKIE)
            .and_then(|(_, value)| Theme::named(value))
            .unwrap_or(Theme::System)
    }
}

thread_local! {
    static PICKED: std::cell::Cell<Theme> = const { std::cell::Cell::new(Theme::System) };
}

/// The theme the page drawn on this thread is drawn in: the request's pick while `within` runs, else System (the window, a
/// test). The page's root (screen::page) and the menu's switch (forms) read it as they draw, so no page is patched after.
pub fn picked() -> Theme {
    PICKED.with(std::cell::Cell::get)
}

/// `draw` run with `theme` as the pick of every page it draws on this thread; the pick before it is back after.
pub fn within<R>(theme: Theme, draw: impl FnOnce() -> R) -> R {
    let before = PICKED.with(|p| p.replace(theme));
    let drawn = draw();
    PICKED.with(|p| p.set(before));
    drawn
}

/// The page root's classes and its head's script for `theme`: Light or Dark set on the root, the browser's scheme no longer
/// asked; System asks it (DARK_JS).
pub(crate) fn root(theme: Theme) -> (&'static str, String) {
    match theme {
        Theme::System => ("app light", format!("<script>{DARK_JS}</script>")),
        Theme::Light => ("app light", String::new()),
        Theme::Dark => ("app dark", String::new()),
    }
}

/// dash's theme, generated from design.json and kept fresh by the_theme_is_design_json_s: one file, so the window, /design and
/// the terminal match. Here, below the crate root, so `tone` reads it with no module loop (dash-gate's feature_loops_only_go_down).
pub(crate) const THEME: &str = include_str!("theme.css");

/// A theme colour by its token (`primary`, `info`, `base-content`), as the terminal draws it: theme.css's dark block, the
/// colours of /design's TuiScreen (`--color-neutral` under them), read from the file every page loads, never a second palette
/// (task rust-tui-app). Read once a process; `#fff` is `#ffffff`. A token theme.css lacks is the terminal's own colour.
pub fn tone(token: &str) -> ratatui::style::Color {
    use std::collections::HashMap;
    use std::sync::LazyLock;
    static DARK: LazyLock<HashMap<String, ratatui::style::Color>> = LazyLock::new(|| {
        let dark = THEME.split(".app.dark {").nth(1).unwrap_or_default();
        let dark = dark.split('}').next().unwrap_or_default();
        (dark.lines())
            .filter_map(|l| l.trim().strip_prefix("--color-")?.split_once(':'))
            .filter_map(|(name, v)| {
                let hex = v.trim().trim_end_matches(';').strip_prefix('#')?;
                let hex: String = match hex.len() {
                    3 => hex.chars().flat_map(|c| [c, c]).collect(),
                    _ => hex.to_string(),
                };
                Some((name.to_string(), format!("#{hex}").parse().ok()?))
            })
            .collect()
    });
    DARK.get(token)
        .copied()
        .unwrap_or(ratatui::style::Color::Reset)
}
