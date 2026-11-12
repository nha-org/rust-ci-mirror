use crate::component::Component;
use dioxus::prelude::*;

/// What a page holds while a `Skeleton` is on it: a reader with no script (the window) renders the page again until it is
/// gone, where a browser runs LIVE_JS.
pub const BUSY: &str = r#"aria-busy="true""#;

inventory::submit! {
    Component {
        slug: "skeleton",
        classes: &["skeleton"],
        tags: &[],
        name: "Skeleton",
        used: "Any section still being read, filled in place when it lands: /agents' usage, the tasks pages' lists, /tools' cards (Live), /today's lines (a Card reading).",
        example: Some(|_| rsx! { Skeleton { id: "c-skeleton", what: "Usage" } }),
        ..Component::BARE
    }
}

/// What is still being read: grey lines, greyed and `aria-busy`, that fill in place when the data lands, no reload: the
/// page's LIVE_JS swaps in the server's newer drawing of the same id (task rust-skeleton-live; @nha, 2026-11-09: « Reading every access:
/// the next reload shows it. does not make sense. », « cards greyed out show a loading state »). `what` names what is read,
/// for a screen reader; `look` is the component it stands in for (`tool`), `children` what is already known of it.
#[component]
pub fn Skeleton(
    #[props(into)] id: String,
    #[props(into)] what: String,
    #[props(default = 2)] lines: usize,
    #[props(default, into)] look: String,
    children: Element,
) -> Element {
    let class = match look.as_str() {
        "" => "skeleton".to_string(),
        look => format!("{look} skeleton"),
    };
    let label = match what.as_str() {
        "" => "Being read".to_string(),
        what => format!("{what}: being read"),
    };
    // all expressions, so the HTML keeps this order (a literal would be drawn first): id, class, then the busy state
    let (busy, role) = ("true", "status");
    rsx! {
        div { id, class, "aria-busy": busy, role, "aria-label": label,
            {children}
            for _ in 0..lines { span { class: "line" } }
        }
    }
}

/// The page's one patcher (`swap`): it fetches the page's own URL in HTML and puts in, for each id it is given, the newer
/// drawing of that element; an element the newer drawing no longer holds, the nearest element around it whose id it holds;
/// an element new to the page, the nearest element around it that the page holds. A section the newer drawing no longer holds
/// (only the page's root around it is shared) is removed, never the whole page swapped. Two callers:
/// - **a busy element:** once a second while one is `aria-busy`, its live part (`.live`), else its nearest id; it stops once
///   none is busy. A page that answers an error, or still busy after five minutes, is reloaded: never grey lines with nothing
///   saying so (/code-review of PR 1248 and of rust-skeleton-live; rust/CLAUDE.md 5).
/// - **a view's diff:** a page holding a `data-view` part listens to `?view=patches`, each event the ids a diff changes, and
///   once on (re)connecting the part whole, for a diff missed while away (task rust-view-patches; @nha, 2026-11-09: « both
///   should work together locally too, via the timely dataflow updates for views. »).
///
/// Blocks its port: no dioxus-liveview runs, so a web page is a dioxus-ssr picture with no handlers
/// (rfc-rust-unified#ui-lib's drift; liveview's own future is open upstream,
/// https://github.com/DioxusLabs/dioxus/issues/2699); liveview's own patches over its socket replace this whole script.
pub(crate) const LIVE_JS: &str = r#"(()=>{if(window.dashLive)return;window.dashLive=1;const up=e=>e.parentElement&&e.parentElement.closest("[id]");const put=(d,o,i)=>{if(o){if(!o.isConnected)return;let e=o;while(e&&!d.getElementById(e.id))e=up(e);if(!e)return;if(e!==o&&!up(e))return o.remove();return e.replaceWith(d.getElementById(e.id))}let n=d.getElementById(i);while(n&&!document.getElementById(n.id))n=up(n);if(n)document.getElementById(n.id).replaceWith(n)};const swap=ids=>{const u=new URL(location.href);u.searchParams.set("view","html");return fetch(u).then(r=>{if(!r.ok){location.reload();throw r.status}return r.text()}).then(t=>{const d=new DOMParser().parseFromString(t,"text/html"),os=ids.map(i=>document.getElementById(i));ids.forEach((i,k)=>put(d,os[k],i))})};let n=0;const tick=()=>{const busy=[...document.querySelectorAll("[aria-busy=true]")];if(!busy.length)return;if(++n>300)return location.reload();swap(busy.map(b=>b.closest(".live[id]")||b.closest("[id]")).filter(e=>e).map(e=>e.id)).catch(()=>{}).finally(()=>setTimeout(tick,1000))};setTimeout(tick,1000);const v=document.querySelector("[data-view][id]");if(v&&window.EventSource){const u=new URL(location.href);u.searchParams.set("view","patches");const s=new EventSource(u);s.onopen=()=>swap([v.id]).catch(()=>{});s.onmessage=e=>swap(JSON.parse(e.data)).catch(()=>{})}})()"#;
