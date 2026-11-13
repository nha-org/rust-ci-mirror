use super::copy::{CopyLive, CopyPart};
use super::place::{PLACE, PartHost};
use super::select::{MenuLive, MenuPart};
use crate::component::Component;
use dioxus::prelude::*;

inventory::submit! {
    Component {
        slug: "live-part",
        classes: &["live-part", "part-home"],
        tags: &[],
        name: "LivePart",
        used: "A component of a page whose handlers are Rust, run live where it stands (task rust-js-port-small): a Copy, a \
               property menu's filter. The page draws it in place, its props beside it; the page's liveview (web) and the \
               window draw it live and move it into its place. Its look is its component's.",
        // a part not in its place yet is hidden wherever its renderer holds it (/code-review: the window's swap puts them home)
        css: "[data-graft]{display:none}.live-part,.live-part>[data-graft]{display:contents}",
        ..Component::BARE
    }
}

/// A component of a page whose handlers run live, as its props: the page's HTML carries them (`place::placed`, its variant's
/// name the `part` tag each component writes), its renderer draws them again.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "part")]
pub enum Part {
    Copy(CopyPart),
    Menu(MenuPart),
}

impl Part {
    /// The part where a page draws it, as its component places it.
    pub fn placed(self) -> Element {
        match self {
            Part::Copy(part) => {
                super::place::placed("Copy", &part, rsx! { CopyLive { part: part.clone() } })
            }
            Part::Menu(part) => {
                super::place::placed("Menu", &part, rsx! { MenuLive { part: part.clone() } })
            }
        }
    }

    /// The part with its handlers: they act once a `PartHost` is given, else it is a picture.
    fn draw(self) -> Element {
        match self {
            Part::Copy(part) => rsx! { CopyLive { part } },
            Part::Menu(part) => rsx! { MenuLive { part } },
        }
    }
}

/// The parts a page's HTML places, in its order, as `placed` wrote them (dioxus-ssr's escapes undone): the window reads them
/// off the page it was given. A part this build cannot read is none, its place keeping its picture.
pub fn parts_in(html: &str) -> Vec<Option<Part>> {
    let open = format!(r#"<span class="{PLACE}" data-part=""#);
    html.split(&open)
        .skip(1)
        .filter_map(|rest| rest.split_once('"'))
        .map(|(props, _)| serde_json::from_str(&unescaped(props)).ok())
        .collect()
}

/// An attribute's value with its character references undone: numeric ones, as dioxus-ssr's escaper writes them (`&#34;`),
/// and the five named ones. One it cannot read stays as written.
fn unescaped(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let entity = rest.find(';').map(|end| (&rest[1..end], end));
        let char = entity.and_then(|(name, _)| match name {
            "quot" => Some('"'),
            "apos" => Some('\''),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "amp" => Some('&'),
            n => match n.strip_prefix("#x").or_else(|| n.strip_prefix("#X")) {
                Some(hex) => u32::from_str_radix(hex, 16).ok().and_then(char::from_u32),
                None => n
                    .strip_prefix('#')
                    .and_then(|d| d.parse().ok())
                    .and_then(char::from_u32),
            },
        });
        match (char, entity) {
            (Some(c), Some((_, end))) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            _ => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out + rest
}

/// The parts a liveview client sent, each the `data-part` of a place, in the page's order (dash's live.rs reads its first
/// message): none for a part this build cannot read.
pub fn parts_sent(message: &str) -> Vec<Option<Part>> {
    let places: Vec<String> = serde_json::from_str(message).unwrap_or_default();
    places
        .iter()
        .map(|p| serde_json::from_str(p).ok())
        .collect()
}

/// A page's parts drawn live, the nth in a `GRAFT` its renderer moves into the page's nth place, `host` theirs. `drawn` keys
/// them: a new page's parts are drawn anew, so each is mounted, and moved, again.
pub fn live_parts(host: PartHost, parts: Vec<Option<Part>>, drawn: u64) -> Element {
    use_context_provider(|| host.clone());
    let graft = host.graft;
    rsx! {
        div { class: "part-home",
            for (n, part) in parts.into_iter().enumerate().filter_map(|(n, p)| Some((n, p?))) {
                // only a host that grafts hears the mount: liveview's client posts a mount as a message of its own
                if let Some(graft) = graft.clone() {
                    span { key: "{drawn}-{n}", "data-graft": "{n}", onmounted: move |e: MountedEvent| graft(n, e),
                        {part.draw()}
                    }
                } else {
                    span { key: "{drawn}-{n}", "data-graft": "{n}", {part.draw()} }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The window reads back the parts a page placed, each component's tag naming its variant, quotes and ampersands in their
    /// props included.
    #[dash_meter::dash_test]
    fn a_page_s_parts_read_back_off_its_html() {
        use crate::components::{CopyButton, CopyText, PropertyMenu};
        let text = r#"a "b" & <c> 'd'"#;
        let page = dioxus_ssr::render_element(rsx! {
            p { "x" }
            CopyText { id: "c", text, select: "w" }
            CopyButton { id: "b", label: "Copy ID", text: "t", icon: rsx! { "i" } }
            PropertyMenu { id: "m", action: "/a", field: "f", label: "L", current: rsx! { "now" },
                choices: vec![("1".into(), "One & two".into(), true)] }
        });
        let copy = Part::Copy(CopyPart {
            id: "c".into(),
            label: "Copy the text".into(),
            text: text.into(),
            select: Some("w".into()),
            ..CopyPart::default()
        });
        let menu = Part::Menu(MenuPart {
            id: "m".into(),
            action: "/a".into(),
            field: "f".into(),
            label: "L".into(),
            current: "now".into(),
            choices: vec![("1".into(), "One & two".into(), true)],
            ..MenuPart::default()
        });
        let button = Part::Copy(CopyPart {
            id: "b".into(),
            label: "Copy ID".into(),
            text: "t".into(),
            icon: Some("i".into()),
            ..CopyPart::default()
        });
        // each component's kind, written as a literal beside its props, is a variant `Part` reads (/code-review)
        assert_eq!(
            parts_in(&page),
            vec![Some(copy.clone()), Some(button), Some(menu)],
            "{page}"
        );
        let sent =
            serde_json::to_string(&[serde_json::to_string(&copy).expect("json"), "{}".into()])
                .expect("json");
        assert_eq!(parts_sent(&sent), vec![Some(copy), None]);
    }
}
