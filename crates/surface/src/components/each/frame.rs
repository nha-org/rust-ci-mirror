use crate::component::Component;
use dioxus::prelude::*;

/// What a mail's HTML may do in its frame: nothing loads but its own data: images and inline styles, and no script runs
/// (the frame's sandbox holds no allow-scripts, and the policy says none). A `meta` of its own that sets a policy or a refresh
/// is made inert, so the mail cannot loosen this one or reload the frame from its server.
const FRAMED: &str = r#"<meta http-equiv="Content-Security-Policy" content="default-src 'none'; img-src data:; style-src 'unsafe-inline'; font-src data:"><base target="_blank">"#;

/// A mail that follows the theme lets its document take either scheme: the theme ground through its frame, its text light on dark.
const FOLLOWS_THEME: &str = r#"<meta name="color-scheme" content="light dark">"#;

/// Whether a mail follows the theme: its markup, its tags and style blocks but never its words, names no colour (a style's,
/// or the old `bgcolor`, `text=`, `link=` attributes) and holds no image, all of which were picked against white
/// (/code-review: « Your background in Rust » is words, not a colour; a transparent logo vanishes on dark).
pub fn follows_theme(html: &str) -> bool {
    let lower = html.to_ascii_lowercase();
    !lower.split('<').skip(1).any(|piece| {
        let tag = piece.split_once('>').map_or(piece, |(tag, _)| tag);
        let markup = if tag.starts_with("style") { piece } else { tag };
        ["color", "background", "text=", "link=", "img", "svg"]
            .iter()
            .any(|m| markup.contains(m))
    })
}

/// A mail's HTML as its frame's document: the policy first, then the mail, its own http-equiv metas made inert. A mail that
/// follows the theme (`follows_theme`) may take either scheme; any other is drawn as sent, light unless it names a
/// color-scheme of its own, which stays in force.
pub fn framed(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let mut out = String::from(FRAMED);
    if follows_theme(html) {
        out += FOLLOWS_THEME;
    }
    let mut at = 0;
    while let Some(found) = lower[at..].find("http-equiv") {
        out += &html[at..at + found];
        out += "data-off";
        at += found + "http-equiv".len();
    }
    out + &html[at..]
}

inventory::submit! {
    Component {
        slug: "frame",
        classes: &["frame"],
        tags: &[],
        name: "Frame",
        used: "A mail's HTML on /mail/t/<thread>: no script runs, nothing remote loads.",
        example: Some(|_| rsx! { Frame { title: "The mail as sent", html: "<p>Hello <b>Ada</b></p>" } }),
        ..Component::BARE
    }
}

/// Someone else's HTML, framed with scripts off and nothing remote loaded (`framed`): a mail's body.
#[component]
pub fn Frame(#[props(into)] html: String, #[props(into)] title: String) -> Element {
    // links open outside the frame, in a tab of their own, still with no script of the frame's
    rsx! {
        iframe { class: if follows_theme(&html) { "frame themed" } else { "frame" }, title, referrerpolicy: "no-referrer", "sandbox": "allow-popups allow-popups-to-escape-sandbox", srcdoc: framed(&html) }
    }
}
