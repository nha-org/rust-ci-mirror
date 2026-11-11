use super::parts::{LINK, icon};
use crate::component::Component;
use dioxus::prelude::*;

inventory::submit! {
    Component {
        slug: "copy",
        classes: &["copies"],
        tags: &[],
        name: "CopyButton",
        used: "An issue page's round copy buttons, as Linear's: Copy link (after the page's origin), Copy ID, Copy branch name.",
        example: Some(|_| {
            rsx! { Copies { CopyButton { id: "c-copy", label: "Copy link", text: "/tasks/a-task", origin: true, icon: icon(LINK) } } }
        }),
        ..Component::BARE
    }
}

/// Copies `text` on a tap, Linear's round copy buttons over an issue (board_live.ex's `copy_button`: Copy link, Copy ID,
/// Copy branch name; task rust-board-buttons-new-issue). `origin`: `text` is a path, copied after the page's origin, so the
/// link works from any address the page was opened at. Its label is its title and its text form. It sits in a `Copies`,
/// whose script writes the clipboard.
#[component]
pub fn CopyButton(
    #[props(into)] id: String,
    label: &'static str,
    #[props(into)] text: String,
    #[props(default)] origin: bool,
    icon: Element,
) -> Element {
    rsx! {
        button { id, class: "button icon round", r#type: "button", title: label, "aria-label": label, "data-copy": "{text}",
            "data-origin": origin.then_some("true"),
            span { class: "text-only", "{label}: {text}" }
            {icon}
        }
    }
}

inventory::submit! {
    Component {
        slug: "copy-text",
        classes: &[],
        tags: &[],
        name: "CopyText",
        used: "A draft's « Copy the text »: the message copied whole, its words then say Copied, or Copy failed with the text selected.",
        example: Some(|_| rsx! { p { id: "c-copy-words", "Hello, a short message." } Copies { CopyText { id: "c-copy-text", text: "Hello, a short message.", select: "c-copy-words" } } }),
        ..Component::BARE
    }
}

/// Copies a message's whole text on a tap, for him to paste where it goes (the older draft page's « Copy the text »; task
/// rust-draft-state-edit): its words then say « Copied », or « Copy failed, the text is selected » with the element `select`
/// names selected, so he copies it by hand. It sits in a `Copies`, whose script writes the clipboard.
#[component]
pub fn CopyText(
    #[props(into)] id: String,
    #[props(into)] text: String,
    #[props(into)] select: String,
) -> Element {
    rsx! {
        button { id, class: "button", r#type: "button", "data-copy": "{text}", "data-label": "Copy the text",
            "data-select": "{select}", "Copy the text"
        }
    }
}

/// CopyButtons side by side, their script once: it writes the clipboard on a tap and says « Copied » in the title a moment.
#[component]
pub fn Copies(children: Element) -> Element {
    rsx! {
        div { class: "copies", {children} }
        script { dangerous_inner_html: COPY_JS }
    }
}

/// Copies' script, once a page however many groups draw it.
/// The clipboard API needs a secure page, and his phone opens plain http on the tailnet address: there a selected text's copy
/// command does it (/code-review of PR 1395). Either way the title says whether it copied; a CopyText says it in its words,
/// and on a failure selects its text.
const COPY_JS: &str = r#"(()=>{if(window.dashCopy)return;window.dashCopy=1;const put=t=>{if(navigator.clipboard&&isSecureContext)return navigator.clipboard.writeText(t);const a=document.createElement("textarea");a.value=t;document.body.append(a);a.select();const ok=document.execCommand("copy");a.remove();return ok?Promise.resolve():Promise.reject()};document.addEventListener("click",e=>{const b=e.target.closest&&e.target.closest("[data-copy]");if(!b)return;const was=b.dataset.was??(b.dataset.was=b.title),label=b.dataset.label;const say=w=>{b.title=w;if(label)b.textContent=w;clearTimeout(b.dashSaid);b.dashSaid=setTimeout(()=>{b.title=was;if(label)b.textContent=label},2500)};const pick=()=>{const t=b.dataset.select&&document.getElementById(b.dataset.select);if(!t)return;const r=document.createRange();r.selectNodeContents(t);const s=getSelection();s.removeAllRanges();s.addRange(r)};put((b.dataset.origin?location.origin:"")+b.dataset.copy).then(()=>say("Copied"),()=>{pick();say(label?"Copy failed, the text is selected":"Not copied")})})})()"#;
