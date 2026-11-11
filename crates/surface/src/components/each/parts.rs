//! What several components share: their class joiner, the links they open, the strokes they draw, a day's number. A
//! component's own helper stays in its module; this one names no component (task rust-components-register).
use dioxus::prelude::*;

/// A component's own class, then its look's, if any.
pub(crate) fn joined(base: &str, look: &str) -> String {
    match look {
        "" => base.to_string(),
        look => format!("{base} {look}"),
    }
}

/// A card's link when it is one a page may open: a path of this app or an http(s) URL, never a script.
pub(crate) fn card_href(href: &Option<String>) -> Option<&str> {
    let href = href.as_deref()?;
    // a browser reads « /\\host » as « //host »: no backslash in a link of ours (/code-review of rust-nav-polish)
    if href.contains('\\') {
        return None;
    }
    (local_path(href) || href.starts_with("https://") || href.starts_with("http://"))
        .then_some(href)
}

/// A path of this app, never another host's (« //host », « /\\host »): a link or a form's target.
pub(crate) fn local_path(path: &str) -> bool {
    path.starts_with('/') && !path.starts_with("//") && !path.contains('\\')
}

/// Today's day number since 1970-01-01 (UTC), the clock's: what a page's `When` counts from, and /drafts' day.
/// ponytail: past 23:00 in Paris it is still yesterday, so an age is a day short for an hour or two; his zone here when it
/// matters (/code-review of rust-timeline-component).
pub fn today() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| (d.as_secs() / 86_400) as i64)
}

/// A plus in Lucide's strokes, New issue's mark: here, and re-exported by design::families, so no module loop.
pub(crate) const PLUS: &str = "M12 5v14M5 12h14";

/// A name's avatar letters, as the older app's: each word's first, two at most (« Nicolas Ha » NH, « Rust » R).
pub fn initials_of(name: &str) -> String {
    name.split_whitespace()
        .filter_map(|w| w.chars().next())
        .take(2)
        .flat_map(char::to_uppercase)
        .collect()
}

/// One link of a `links` element.
#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
pub struct Link {
    pub name: String,
    pub href: String,
    #[serde(default)]
    pub current: bool,
}

/// A document, as a Lucide-style stroke (crate::design::families holds the others).
pub(crate) const LINK: &str =
    "M10 14a4 4 0 006 0l3-3a4 4 0 00-6-6l-1 1M14 10a4 4 0 00-6 0l-3 3a4 4 0 006 6l1-1";

pub(crate) const DOTS: &str = "M5 12h.01M12 12h.01M19 12h.01";

/// The menu's list view (task rust-menu-icons).
pub(crate) const LIST: &str = "M4 6h16M4 12h10M4 18h6";

/// Three columns of a board, the tasks' board view.
pub(crate) const BOARD: &str = "M4 5h4v14H4zM10 5h4v10h-4zM16 5h4v7h-4z";

pub(crate) const CALENDAR: &str = "M4 6h16v14H4zM4 10h16M8 3v4M16 3v4";

/// A one-pixel green PNG: /design's picture, drawn with nothing fetched.
pub(crate) const PIXEL: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";

pub(crate) const DOC: &str = "M7 3h7l4 4v14H7zM14 3v4h4";

/// A stroke `d` in `currentColor`, the one way an SVG follows the theme (a CSS stroke does not reach it in Blitz): every
/// icon of the design system.
pub(crate) fn icon(d: &'static str) -> Element {
    rsx! {
        svg { class: "ic", width: "16", height: "16", view_box: "0 0 24 24",
            path { d, fill: "none", stroke: "currentColor", stroke_width: "2", stroke_linecap: "round", stroke_linejoin: "round" }
        }
    }
}

/// Whether a file of MIME `kind` draws as a picture: a raster image every browser decodes, read case-blind and without its
/// parameters. An SVG, a TIFF or a HEIC stays a file (/code-review: the mail server's bytes come as octet-stream, which no
/// browser draws as SVG, and few decode the others).
pub fn is_picture(kind: &str) -> bool {
    let kind = kind
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    [
        "image/png",
        "image/jpeg",
        "image/gif",
        "image/webp",
        "image/avif",
    ]
    .contains(&kind.as_str())
}

// /mail's frame, as the older app's /mail on :7799 drew it (Gmail's; task rust-mail-parity; @nha, 2026-11-10: « the mail app
// needs to be _mostly_ the same and we're maybe 20% there »): the side column (the account switcher, Compose, the folders and
// labels with their counts), the search over the list, the list's bar (refresh, the range, the pager), a conversation's head.
// Each shown on /design?tab=mail, so the list, a thread and Compose draw with them alone (rust/CLAUDE.md rule 6).
pub(crate) const INBOX: &str = "M4 13l3-8h10l3 8v6H4zM4 13h5l1 2h4l1-2h5";

pub(crate) const MAIL: &str = "M4 6h16v12H4zM4 7l8 6 8-6";

pub(crate) const PEN: &str = "M4 20l4-1 11-11-3-3L5 16z";

pub(crate) const PERSON: &str = "M12 4a4 4 0 100 8 4 4 0 000-8zM5 20a7 7 0 0114 0";

pub(crate) const SEARCH: &str = "M11 4a7 7 0 100 14 7 7 0 000-14zM16 16l5 5";

pub(crate) const SEND: &str = "M12 19V6M6 12l6-6 6 6";

pub(crate) const STAR: &str =
    "M12 3l2.8 5.7 6.2.9-4.5 4.4 1 6.2-5.5-2.9-5.5 2.9 1-6.2L3 9.6l6.2-.9z";
