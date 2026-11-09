//! The components every page draws with, each one of dioxuslabs.com/components ported to design.json's tokens (@nha,
//! 2026-11-09: « our design page should have https://dioxuslabs.com/components or equivalents either use or port what we
//! need », « We want all components to be dual-use »). Dual-use: each is a Dioxus element, so the window draws it and the
//! web's server render is its HTML, whose text form (crate::forms::text_of) is what an agent reads. /design?tab=components
//! (design/catalog.rs) shows each one's two forms side by side. `look` picks a component's look: its CSS class, never written by a page.
//! The gate (dash-gate's `page_local_markup`) refuses OWNED_TAGS and OWNED_CLASSES anywhere but here.
use dioxus::prelude::*;

/// The components' looks, which every page and /design load, and the menu scopes (crate::forms).
pub const CSS: &str = include_str!("components.css");

/// The elements a component here draws: a page writes `Button {}`, never `button {}`.
pub const OWNED_TAGS: [&str; 3] = ["button", "input", "textarea"];
/// The classes a component here draws, each a look a page passes as `look`.
pub const OWNED_CLASSES: [&str; 23] = [
    "fold",
    "frame",
    "textarea",
    "card",
    "li-card",
    "issue-row",
    "description",
    "item",
    "access",
    "badge",
    "chip",
    "count",
    "li-tag",
    "avatar",
    "li-who",
    "button",
    "input",
    "tabs",
    "tool",
    "skeleton",
    "logo",
    "live",
    "diagram",
];

/// A component's own class, then its look's, if any.
fn joined(base: &str, look: &str) -> String {
    match look {
        "" => base.to_string(),
        look => format!("{base} {look}"),
    }
}

/// A card: a link when it has `href`. `task`: the task it shows, `data-task`, the comment box's anchor (crate::forms).
/// `reading`: its loading state, the same card under the same id, greyed with a grey line where its title will be: the page's
/// LIVE_JS swaps in the filled card in place (task rust-today-doc; @nha, 2026-11-09: « This is a card loading state. same
/// element. »). `children` are what is known already, its description.
#[component]
pub fn Card(
    #[props(into)] id: String,
    #[props(default)] look: &'static str,
    #[props(default)] href: Option<String>,
    #[props(default, into)] task: Option<String>,
    #[props(default)] reading: bool,
    children: Element,
) -> Element {
    let class = joined("card", look);
    if reading {
        return rsx! { Skeleton { id, what: "", lines: 0, look: class, CardTitle {} {children} } };
    }
    match href {
        Some(href) => rsx! { a { id, class, "data-task": task, href, {children} } },
        None => rsx! { div { id, class, "data-task": task, {children} } },
    }
}

/// One row of a list: /tools' accesses, /tasks' rows (a link when it has `href`). `status` is `data-status`, which a look's
/// colours read (components.css); `task` is `data-task`, as Card's.
#[component]
pub fn Item(
    #[props(into)] id: String,
    #[props(default)] look: &'static str,
    #[props(default, into)] status: Option<String>,
    #[props(default)] href: Option<String>,
    #[props(default, into)] task: Option<String>,
    children: Element,
) -> Element {
    let class = joined("item", look);
    match href {
        Some(href) => {
            rsx! { a { id, class, "data-status": status, "data-task": task, href, {children} } }
        }
        None => rsx! { div { id, class, "data-status": status, "data-task": task, {children} } },
    }
}

#[component]
pub fn CardTitle(children: Element) -> Element {
    rsx! { b { class: "title", {children} } }
}

#[component]
pub fn CardDescription(children: Element) -> Element {
    rsx! { span { class: "description", {children} } }
}

/// A short label: a state, a count, a tag.
#[component]
pub fn Badge(#[props(default)] look: &'static str, children: Element) -> Element {
    rsx! { span { class: joined("badge", look), {children} } }
}

/// Someone's initials in a circle.
#[component]
pub fn Avatar(#[props(into)] initials: String, #[props(default)] look: &'static str) -> Element {
    rsx! { span { class: joined("avatar", look), "{initials}" } }
}

/// The tools' logos, Simple Icons' (CC0, simpleicons.org), by the word /tools names a tool by: its title and role stripped,
/// its colour the text's, so a dark theme draws it light.
const LOGOS: [(&str, &str); 26] = [
    ("vaultwarden", include_str!("logos/vaultwarden.svg")),
    ("anthropic", include_str!("logos/anthropic.svg")),
    ("aws", include_str!("logos/aws.svg")),
    ("buildkite", include_str!("logos/buildkite.svg")),
    ("circleci", include_str!("logos/circleci.svg")),
    ("cloudflare", include_str!("logos/cloudflare.svg")),
    ("datadog", include_str!("logos/datadog.svg")),
    ("docker", include_str!("logos/docker.svg")),
    ("fly", include_str!("logos/fly.svg")),
    ("github", include_str!("logos/github.svg")),
    ("gmail", include_str!("logos/gmail.svg")),
    ("hetzner", include_str!("logos/hetzner.svg")),
    ("huggingface", include_str!("logos/huggingface.svg")),
    ("kubectl", include_str!("logos/kubectl.svg")),
    ("linear", include_str!("logos/linear.svg")),
    ("linkedin", include_str!("logos/linkedin.svg")),
    ("malt", include_str!("logos/malt.svg")),
    ("npm", include_str!("logos/npm.svg")),
    ("openai", include_str!("logos/openai.svg")),
    ("paddle", include_str!("logos/paddle.svg")),
    ("scaleway", include_str!("logos/scaleway.svg")),
    ("stripe", include_str!("logos/stripe.svg")),
    ("tailnet", include_str!("logos/tailnet.svg")),
    ("terraform", include_str!("logos/terraform.svg")),
    ("warp", include_str!("logos/warp.svg")),
    ("whatsapp", include_str!("logos/whatsapp.svg")),
];

/// The tools Simple Icons lacks: each product's own favicon, 64 px PNG from its site (Google's favicon service, 2026-11-09),
/// a data URL built at compile time, so the page carries it. An image, never inlined markup: a fetched file runs no script on the page.
const PICTURES: [(&str, &str); 14] = [
    (
        "avrea",
        concat!(
            "data:image/png;base64,",
            include_str!("logos/avrea.png.b64")
        ),
    ),
    (
        "blacksmith",
        concat!(
            "data:image/png;base64,",
            include_str!("logos/blacksmith.png.b64")
        ),
    ),
    (
        "browserbase",
        concat!(
            "data:image/png;base64,",
            include_str!("logos/browserbase.png.b64")
        ),
    ),
    (
        "bunnynet",
        concat!(
            "data:image/png;base64,",
            include_str!("logos/bunnynet.png.b64")
        ),
    ),
    (
        "depot",
        concat!(
            "data:image/png;base64,",
            include_str!("logos/depot.png.b64")
        ),
    ),
    (
        "fal",
        concat!("data:image/png;base64,", include_str!("logos/fal.png.b64")),
    ),
    (
        "freework",
        concat!(
            "data:image/png;base64,",
            include_str!("logos/freework.png.b64")
        ),
    ),
    (
        "healthchecks",
        concat!(
            "data:image/png;base64,",
            include_str!("logos/healthchecks.png.b64")
        ),
    ),
    (
        "openrouter",
        concat!(
            "data:image/png;base64,",
            include_str!("logos/openrouter.png.b64")
        ),
    ),
    // his own nicolas-ha.com's
    (
        "site",
        concat!("data:image/png;base64,", include_str!("logos/site.png.b64")),
    ),
    (
        "skyvern",
        concat!(
            "data:image/png;base64,",
            include_str!("logos/skyvern.png.b64")
        ),
    ),
    (
        "tinyfish",
        concat!(
            "data:image/png;base64,",
            include_str!("logos/tinyfish.png.b64")
        ),
    ),
    (
        "typesafe",
        concat!(
            "data:image/png;base64,",
            include_str!("logos/typesafe.png.b64")
        ),
    ),
    (
        "ubicloud",
        concat!(
            "data:image/png;base64,",
            include_str!("logos/ubicloud.png.b64")
        ),
    ),
];

/// A tool's entry in one table of logos, the one lookup `has_logo` and `Logo` share. A name access.rs' provider() keeps
/// apart from a product's card wears that product's logo: DOCKERHUB_TOKEN's « dockerhub » is Docker's.
/// ponytail: the PNGs ride inline in every /tools answer, 27 KB a read and again each second the live part refetches; a
/// static URL of their own if that ever weighs.
fn found(table: &[(&str, &'static str)], name: &str) -> Option<&'static str> {
    let name = match name {
        "dockerhub" => "docker",
        word => word,
    };
    table.iter().find(|(word, _)| *word == name).map(|l| l.1)
}

/// Whether a tool has a logo here, Simple Icons' or its favicon.
pub fn has_logo(name: &str) -> bool {
    found(&LOGOS, name).or(found(&PICTURES, name)).is_some()
}

/// A tool's logo in an avatar's circle, or its first two letters where none is held: Avatar's image and its fallback.
#[component]
pub fn Logo(#[props(into)] name: String) -> Element {
    if let Some(svg) = found(&LOGOS, &name) {
        return rsx! { span { class: joined("avatar", "logo"), dangerous_inner_html: svg } };
    }
    if let Some(src) = found(&PICTURES, &name) {
        return rsx! { span { class: joined("avatar", "logo"), img { src, alt: "" } } };
    }
    let initials: String = name.chars().take(2).collect::<String>().to_uppercase();
    rsx! { Avatar { initials, look: "logo" } }
}

/// A tool's card on /tools: its logo and name, then its accesses. While its provider answers, it is a Skeleton of `reading`
/// grey lines under its head (@nha, 2026-11-09: « cards greyed out show a loading state »).
#[component]
pub fn Tool(
    #[props(into)] id: String,
    #[props(into)] name: String,
    #[props(default)] reading: usize,
    children: Element,
) -> Element {
    let head = rsx! { p { class: "head", Logo { name: name.clone() } CardTitle { "{name}" } } };
    if reading == 0 {
        return rsx! { div { id, class: "card tool", {head} {children} } };
    }
    rsx! { Skeleton { id, what: name, lines: reading, look: "card tool", {head} } }
}

/// What a page holds while a `Skeleton` is on it: a reader with no script (the window) renders the page again until it is
/// gone, where a browser runs LIVE_JS.
pub const BUSY: &str = r#"aria-busy="true""#;

/// What is still being read: grey lines, greyed and `aria-busy`, that fill in place when the data lands, no reload: the
/// page's LIVE_JS swaps in the server's newer drawing of the same id (task rust-skeleton-live; @nha, 2026-11-09: « Reading every access:
/// the next reload shows it. does not make sense. », « cards greyed out show a loading state »). `what` names what is read,
/// for a screen reader; `look` is the component it stands in for (`card tool`), `children` what is already known of it.
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

/// The page's one refill loop, drawn once a page that holds a busy element (screen::Screen): once a second while an element
/// is `aria-busy`, it fetches the page's own URL in HTML and swaps in, for each busy element, its live part (`.live`), else
/// the nearest element around it whose id the newer drawing holds, so a filled section need not keep the skeleton's shape.
/// A section the newer drawing no longer holds (only the page's root around it is shared) is removed, never the whole page
/// swapped. It stops once none is busy. A page that answers an error, or still busy after five minutes, is reloaded: never
/// grey lines with nothing saying so (/code-review of PR 1248 and of rust-skeleton-live; rust/CLAUDE.md 5).
pub(crate) const LIVE_JS: &str = r#"(()=>{if(window.dashLive)return;window.dashLive=1;let n=0;const tick=()=>{if(!document.querySelector("[aria-busy=true]"))return;if(++n>300)return location.reload();const u=new URL(location.href);u.searchParams.set("view","html");fetch(u).then(r=>{if(!r.ok){location.reload();throw r.status}return r.text()}).then(t=>{const d=new DOMParser().parseFromString(t,"text/html");for(const b of document.querySelectorAll("[aria-busy=true]")){if(!b.isConnected)continue;const own=b.closest(".live[id]")||b.closest("[id]");let e=own;while(e&&!d.getElementById(e.id))e=e.parentElement&&e.parentElement.closest("[id]");if(!e)continue;if(e!==own&&!(e.parentElement&&e.parentElement.closest("[id]"))){own.remove();continue}e.replaceWith(d.getElementById(e.id))}}).catch(()=>{}).finally(()=>setTimeout(tick,1000))};setTimeout(tick,1000)})()"#;

/// A part of a page refilled whole while a skeleton in it is busy: /tools' themes, whose cards appear as providers answer.
#[component]
pub fn Live(#[props(into)] id: String, children: Element) -> Element {
    rsx! { div { id, class: "live", {children} } }
}

/// A button: it submits the form around it unless given `r#type: "button"`. Any button attribute passes through.
#[component]
pub fn Button(
    #[props(default)] look: &'static str,
    #[props(extends = GlobalAttributes, extends = button)] attributes: Vec<Attribute>,
    children: Element,
) -> Element {
    rsx! { button { class: joined("button", look), ..attributes, {children} } }
}

/// A field: any input attribute passes through (`r#type`, `name`, `placeholder`, `hidden`).
#[component]
pub fn Input(
    #[props(extends = GlobalAttributes, extends = input)] attributes: Vec<Attribute>,
) -> Element {
    rsx! { input { class: "input", ..attributes } }
}

/// A field of several lines: the compass's comment box.
#[component]
pub fn Textarea(
    #[props(extends = GlobalAttributes, extends = textarea)] attributes: Vec<Attribute>,
) -> Element {
    rsx! { textarea { class: "textarea", ..attributes } }
}

/// One view of several, each a link: the server render runs no handler, so a tab changes the URL.
#[component]
pub fn Tabs(#[props(into)] id: String, label: &'static str, children: Element) -> Element {
    rsx! { nav { id, class: "tabs", "aria-label": label, {children} } }
}

#[component]
pub fn Tab(#[props(into)] href: String, current: bool, children: Element) -> Element {
    let on = current.then_some("page");
    rsx! { a { href, "aria-current": on, {children} } }
}

/// A part folded to its one line, opened by a tap: a conversation's older mails (task rust-mail-thread).
#[component]
pub fn Fold(#[props(into)] id: String, open: bool, summary: Element, children: Element) -> Element {
    // a present `open`, even « false », opens it: only an open fold carries one
    let open = open.then_some(true);
    rsx! { details { id, class: "fold", open, summary { {summary} } {children} } }
}

/// What a mail's HTML may do in its frame: nothing loads but its own data: images and inline styles, and no script runs
/// (the frame's sandbox holds no allow-scripts, and the policy says none). A `meta` of its own that sets a policy or a refresh
/// is made inert, so the mail cannot loosen this one or reload the frame from its server.
const FRAMED: &str = r#"<meta http-equiv="Content-Security-Policy" content="default-src 'none'; img-src data:; style-src 'unsafe-inline'; font-src data:"><base target="_blank">"#;

/// A mail's HTML as its frame's document: the policy first, then the mail, its own http-equiv metas made inert.
pub fn framed(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let mut out = String::from(FRAMED);
    let mut at = 0;
    while let Some(found) = lower[at..].find("http-equiv") {
        out += &html[at..at + found];
        out += "data-off";
        at += found + "http-equiv".len();
    }
    out + &html[at..]
}

/// Someone else's HTML, framed with scripts off and nothing remote loaded (`framed`): a mail's body.
#[component]
pub fn Frame(#[props(into)] html: String, #[props(into)] title: String) -> Element {
    // links open outside the frame, in a tab of their own, still with no script of the frame's
    rsx! {
        iframe { class: "frame", title, referrerpolicy: "no-referrer", "sandbox": "allow-popups allow-popups-to-escape-sandbox", srcdoc: framed(&html) }
    }
}

/// What draws a diagram, in its own frame: Mermaid, laid out by ELK, each subgraph a box (task rust-deps-view; @nha,
/// 2026-11-09: « there has to be off-the shelf techniques or even libs we can reuse here »), pinned from jsdelivr. The
/// frame's sandbox allows scripts but not this origin, so the CDN's code can never act as the dashboard (/code-review of
/// rust-deps-view). Its theme follows the system's, as the pages' DARK does. Once drawn, it tells the page its height.
const MERMAID_JS: &str = r#"import m from "https://cdn.jsdelivr.net/npm/mermaid@11.4.1/dist/mermaid.esm.min.mjs";import elk from "https://cdn.jsdelivr.net/npm/@mermaid-js/layout-elk@0.1.7/dist/mermaid-layout-elk.esm.min.mjs";m.registerLayoutLoaders(elk);m.initialize({startOnLoad:false,layout:"elk",securityLevel:"strict",theme:matchMedia("(prefers-color-scheme: dark)").matches?"dark":"neutral"});await m.run({querySelector:"pre"});parent.postMessage({diagram:document.documentElement.scrollHeight},"*");"#;

/// The page's side: each diagram frame sized to what it drew, from that frame's own message alone.
const FIT_JS: &str = r#"if(!window.dashDiagram){window.dashDiagram=1;addEventListener("message",e=>{for(const f of document.querySelectorAll("iframe.diagram"))if(f.contentWindow===e.source&&Number.isFinite(e.data&&e.data.diagram))f.style.height=e.data.diagram+"px"})}"#;

/// A diagram from its text form, Mermaid (rust/CLAUDE.md rule 4: « diagrams etc. need to have a text form. say mermaid for
/// example. »): the frame draws it, the fold under it holds the source, which an agent reads and a page offline still shows.
#[component]
pub fn Diagram(#[props(into)] id: String, #[props(into)] source: String) -> Element {
    let doc = format!(
        r#"<!doctype html><meta charset="utf-8"><style>body{{margin:0}}pre{{margin:0}}</style><pre>{}</pre><script type="module">{MERMAID_JS}</script>"#,
        // a pre's text: only these two open markup (crate::forms reads this file, so not its `escaped`)
        source.replace('&', "&amp;").replace('<', "&lt;")
    );
    rsx! {
        iframe { id: "{id}", class: "diagram", title: "Diagram", "sandbox": "allow-scripts", srcdoc: doc }
        Fold { id: "{id}-source", open: false, summary: rsx! { "Mermaid source" }, pre { "{source}" } }
        script { dangerous_inner_html: FIT_JS }
    }
}
