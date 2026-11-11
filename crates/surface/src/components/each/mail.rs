use super::attachment::{Attachment, AttachmentGroup};
use super::avatar::Avatar;
use super::badge::Badge;
use super::dropdown_menu::DropdownMenuItem;
use super::fold::Fold;
use super::frame::Frame;
use super::input::Input;
use super::parts::{
    INBOX, Link, MAIL, PEN, PERSON, SEARCH, SEND, STAR, card_href, icon, initials_of, is_picture,
    local_path,
};
use crate::component::{Component, Section};
use dioxus::prelude::*;

/// One conversation of /mail's list, as the older app's /mail drew it (Gmail's): the sender, the subject then its snippet,
/// the date; unread bold on the page's white, read on its grey. The whole row opens the conversation (task
/// rust-mail-design-parts; @nha, 2026-11-09: « they do not match the previous app /mail »). `href`: a path of this app or
/// an http(s) URL, else the row opens nothing. `count`: its mails, shown past one; `starred`: its star filled (task
/// rust-mail-parity).
#[component]
pub fn ThreadRow(
    #[props(into)] id: String,
    #[props(into)] from: String,
    #[props(into)] subject: String,
    #[props(into)] snippet: String,
    #[props(into)] at: String,
    unread: bool,
    #[props(default)] href: Option<String>,
    #[props(default)] count: u64,
    #[props(default)] starred: bool,
) -> Element {
    let class = if unread {
        "thread-row unread"
    } else {
        "thread-row"
    };
    let href = card_href(&href).map(str::to_string);
    let star = if starred { "star on" } else { "star" };
    let said = if starred { "Starred" } else { "Not starred" };
    // its own class names: /design's base.css styles a bare .subject and .snippet (/code-review)
    rsx! {
        a { id, class, href,
            span { class: star, role: "img", "aria-label": said, {icon(STAR)} }
            span { class: "from", "{from}"
                if count > 1 { span { class: "n", " {count}" } }
            }
            span { class: "words",
                span { class: "topic", "{subject}" }
                if !snippet.is_empty() {
                    span { class: "gist", " - {snippet}" }
                }
            }
            span { class: "at", "{at}" }
        }
    }
}

/// One mail of a conversation on /mail/t/<thread> (task rust-mail-thread): folded to its line unless `open`, then its words,
/// or its HTML framed with no script and nothing remote; `files`, each attachment's name, where it downloads and its MIME
/// type, drawn as `Attachment`s, a picture as its `Image` (task rust-ui-media). `address`: the sender's, beside the name, under
/// the initials' avatar, as the older app's head showed it (task rust-mail-parity).
#[component]
pub fn MailFold(
    #[props(into)] id: String,
    #[props(into)] from: String,
    #[props(into)] to: String,
    #[props(into)] at: String,
    #[props(into)] text: String,
    #[props(default)] html: Option<String>,
    open: bool,
    #[props(default)] files: Vec<(String, String, String)>,
    #[props(default, into)] address: String,
) -> Element {
    // a folded mail's line says how it starts, as Gmail's does
    let start: String = text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(90)
        .collect();
    let line = if open || start.is_empty() {
        String::new()
    } else {
        format!(" · {start}")
    };
    let address = (!address.is_empty() && address != from).then(|| format!(" <{address}>"));
    let head = rsx! {
        Avatar { initials: initials_of(&from) }
        b { "{from}" }
        if let Some(address) = address { span { class: "address", "{address}" } }
        " · {at}{line}"
    };
    let any = !files.is_empty();
    let files = files
        .into_iter()
        .enumerate()
        .map(|(i, (name, href, kind))| {
            let picture = is_picture(&kind);
            rsx! { Attachment { id: "{id}-file-{i}", name, kind, href, picture } }
        });
    rsx! {
        Fold { id: id.clone(), open, summary: rsx! { {head} },
            p { "To {to}" }
            if let Some(html) = html {
                Frame { title: "The mail from {from}", html }
            } else {
                pre { "{text}" }
            }
            if any {
                AttachmentGroup { {files} }
            }
        }
    }
}

const DRAFT: &str = "M7 3h7l4 4v14H7zM14 3v4h4";

const SPAM: &str = "M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18zM12 8v5M12 16h.01";

const TRASH: &str = "M4 7h16M10 11v6M14 11v6M6 7l1 13h10l1-13M9 7V4h6v3";

const LABEL: &str = "M3 12V4h8l10 10-8 8z M7.5 7.5h.01";

const REFRESH: &str = "M20 11a8 8 0 1 0-2.3 5.7M20 4v7h-7";

const CHEVRON: &str = "M6 9l6 6 6-6";

const BACK: &str = "M19 12H5M11 6l-6 6 6 6";

const PREVIOUS: &str = "M15 6l-6 6 6 6";

const NEXT: &str = "M9 6l6 6-6 6";

/// One folder or label of the side column: its name, where it lists, its count (none at 0), its kind, the one shown marked.
/// `kind`: a folder's JMAP role (inbox, sent, drafts, junk, trash), « starred », « all », or « label » for one of his own.
#[derive(Clone, PartialEq, serde::Deserialize, serde::Serialize, Debug)]
pub struct Folder {
    pub name: String,
    pub href: String,
    #[serde(default)]
    pub count: u64,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub current: bool,
}

fn folder_icon(kind: &str) -> &'static str {
    match kind {
        "inbox" => INBOX,
        "starred" => STAR,
        "sent" => SEND,
        "drafts" => DRAFT,
        "junk" => SPAM,
        "trash" => TRASH,
        "all" => MAIL,
        _ => LABEL,
    }
}

inventory::submit! {
    Component {
        slug: "mail-shell",
        classes: &["mail-shell", "mail-side", "mail-account", "mail-compose", "mail-folders", "mail-folder", "mail-main", "mail-search", "mail-bar", "mail-head", "thread-row"],
        tags: &[],
        sections: &[
            Section {
                family: "mail",
                id: "mail-shell",
                title: "Frame",
                after: "",
                rows: &[("Inbox, a label, the switch", |_| example("mail-shell"))],
            },
            Section {
                family: "mail",
                id: "mail-bar",
                title: "List bar",
                after: "mail-shell",
                rows: &[("Page 2 of 8", |_| example("mail-bar"))],
            },
            Section {
                family: "mail",
                id: "thread-row",
                title: "Thread row",
                after: "mail-bar",
                rows: &[
                    ("Unread", |_| thread_row("ds-thread-unread", true, "two clauses to check before Monday")),
                    ("Read", |_| thread_row("ds-thread-read", false, "two clauses to check before Monday")),
                    ("No snippet", |_| thread_row("ds-thread-bare", false, "")),
                ],
            },
            Section {
                family: "mail",
                id: "mail-head",
                title: "Conversation head",
                after: "thread-row",
                rows: &[("A conversation", |_| example("mail-head"))],
            },
        ],
        ..Component::BARE
    }
}

/// /mail's frame: the side column, then the search and `children`, the list or a conversation. `accounts`: the switch's
/// addresses, one marked shown; `compose`: where Compose opens; `hidden`: the search's fields kept (the account, the folder).
/// On a phone the side column folds above the list, its folders one scrolling row.
#[component]
pub fn MailShell(
    #[props(into)] id: String,
    #[props(into)] account: String,
    accounts: Vec<Link>,
    #[props(into)] compose: String,
    folders: Vec<Folder>,
    #[props(into)] search: String,
    #[props(default)] hidden: Vec<(String, String)>,
    children: Element,
) -> Element {
    let switch = accounts.into_iter().enumerate().map(|(i, a)| {
        let href = card_href(&Some(a.href)).map(str::to_string);
        let on = a.current.then_some("true");
        rsx! { DropdownMenuItem { id: "{id}-account-{i}", href, span { "aria-checked": on, "{a.name}" } } }
    });
    let folders = folders.into_iter().map(|f| {
        let href = card_href(&Some(f.href)).unwrap_or("#").to_string();
        let on = f.current.then_some("page");
        let class = if f.count > 0 {
            "mail-folder busy"
        } else {
            "mail-folder"
        };
        rsx! {
            a { class, href, "aria-current": on,
                {icon(folder_icon(&f.kind))}
                span { class: "name", "{f.name}" }
                if f.count > 0 { span { class: "count", "{f.count}" } }
            }
        }
    });
    let hidden = hidden.into_iter().map(|(name, value)| {
        rsx! { Input { r#type: "hidden", name: "{name}", value: "{value}" } }
    });
    let compose = card_href(&Some(compose))
        .unwrap_or("/mail/compose")
        .to_string();
    rsx! {
        div { id: "{id}", class: "mail-shell",
            aside { class: "mail-side",
                details { class: "mail-account dropdown-menu",
                    summary { {icon(PERSON)} span { class: "name", "{account}" } {icon(CHEVRON)} }
                    div { class: "dropdown-menu-content", role: "menu", {switch} }
                }
                a { class: "button mail-compose", href: compose, {icon(PEN)} "Compose" }
                nav { class: "mail-folders", "aria-label": "Folders", {folders} }
            }
            div { class: "mail-main",
                form { class: "form mail-search", role: "search", method: "get", action: "/mail",
                    {hidden}
                    label { {icon(SEARCH)} span { class: "sr-only", "Search mail" }
                        Input { r#type: "search", name: "q", placeholder: "Search mail", value: search }
                    }
                }
                {children}
            }
        }
    }
}

/// An arrow of a bar: a link while it leads somewhere, else greyed.
fn arrow(d: &'static str, label: &'static str, href: Option<String>) -> Element {
    match href
        .as_ref()
        .and_then(|h| card_href(&Some(h.clone())).map(str::to_string))
    {
        Some(href) => {
            rsx! { a { class: "button icon", href, "aria-label": label, title: label, {icon(d)} } }
        }
        None => {
            rsx! { span { class: "button icon", "aria-disabled": "true", "aria-label": label, {icon(d)} } }
        }
    }
}

/// The list's bar, Gmail's: Refresh at the left, the range (« 1–50 of 382 ») and the pager's arrows at the right.
#[component]
pub fn MailBar(
    #[props(into)] id: String,
    #[props(into)] range: String,
    #[props(into)] refresh: String,
    #[props(default)] newer: Option<String>,
    #[props(default)] older: Option<String>,
) -> Element {
    rsx! {
        div { id: "{id}", class: "mail-bar", role: "toolbar", "aria-label": "List",
            {arrow(REFRESH, "Refresh", Some(refresh))}
            span { class: "range", "{range}" }
            {arrow(PREVIOUS, "Newer", newer)}
            {arrow(NEXT, "Older", older)}
        }
    }
}

/// A conversation's head: back to its list, its subject, its folder as a chip, how many mails it holds.
#[component]
pub fn MailHead(
    #[props(into)] id: String,
    #[props(into)] back: String,
    #[props(into)] subject: String,
    #[props(into)] folder: String,
    #[props(into)] count: String,
) -> Element {
    let back = if local_path(&back) {
        back
    } else {
        "/mail".into()
    };
    rsx! {
        div { id: "{id}", class: "mail-head",
            div { class: "mail-bar", role: "toolbar", "aria-label": "Conversation",
                a { class: "button icon", href: back, "aria-label": "Back to the list", title: "Back to the list", {icon(BACK)} }
            }
            h1 { "{subject}" " "
                if !folder.is_empty() { Badge { "{folder}" } }
                span { class: "count", " {count}" }
            }
        }
    }
}

/// /mail's row, the component the page draws (components::ThreadRow).
fn thread_row(id: &'static str, unread: bool, snippet: &'static str) -> Element {
    rsx! {
        ThreadRow { id, from: "Ada Lovelace", subject: "Contract", snippet, at: "12:40 UTC", unread,
            href: "/mail/t/a?account=nicolas@klar.im".to_string() }
    }
}

/// /design?tab=mail's example of a part of /mail, by its section.
fn example(id: &str) -> Element {
    let folder = |name: &str, kind: &str, count: u64, current: bool| Folder {
        name: name.into(),
        href: format!("/mail?box={kind}"),
        count,
        kind: kind.into(),
        current,
    };
    let link = |name: &str, href: &str, current: bool| Link {
        name: name.into(),
        href: href.into(),
        current,
    };
    match id {
        "mail-shell" => rsx! {
            MailShell { id: "ds-mail-shell", account: "nicolas@klar.im",
                accounts: vec![link("nicolas@klar.im", "/mail", true), link("support@klar.im", "/mail?account=support@klar.im", false)],
                compose: "/mail/compose",
                folders: vec![folder("Inbox", "inbox", 150, true), folder("Starred", "starred", 0, false), folder("Drafts", "drafts", 11, false), folder("Marketing", "label", 297, false)],
                search: "",
                MailBar { id: "ds-mail-shell-bar", range: "1–50 of 382", refresh: "/mail", older: Some("/mail?page=2".to_string()) }
            }
        },
        "mail-bar" => rsx! {
            MailBar { id: "ds-mail-bar", range: "51–100 of 382", refresh: "/mail?page=2", newer: Some("/mail".to_string()), older: Some("/mail?page=3".to_string()) }
        },
        _ => rsx! {
            MailHead { id: "ds-mail-head", back: "/mail", subject: "Contract", folder: "Inbox", count: "3 messages" }
        },
    }
}
