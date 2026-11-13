use super::parts::joined;
use crate::component::Component;
use dioxus::prelude::*;

inventory::submit! {
    Component {
        slug: "avatar",
        classes: &["avatar", "li-who", "logo"],
        tags: &[],
        name: "Avatar",
        used: "A board card's assignee; a tool's logo (Simple Icons', else its favicon), its initials where none is held.",
        example: Some(|_| {
            rsx! {
                Avatar { initials: "C", look: "li-who" }
                Logo { name: "fly" }
                Logo { name: "tinyfish" }
                Logo { name: "vapid" }
            }
        }),
        menu: true,
        ..Component::BARE
    }
}

/// Someone's initials in a circle.
#[component]
pub fn Avatar(#[props(into)] initials: String, #[props(default)] look: &'static str) -> Element {
    rsx! { span { class: joined("avatar", look), "{initials}" } }
}

/// The tools' logos, Simple Icons' (CC0, simpleicons.org), by the word /tools names a tool by: its title and role stripped,
/// its colour the text's, so a dark theme draws it light.
const LOGOS: [(&str, &str); 26] = [
    ("vaultwarden", include_str!("../../logos/vaultwarden.svg")),
    ("anthropic", include_str!("../../logos/anthropic.svg")),
    ("aws", include_str!("../../logos/aws.svg")),
    ("buildkite", include_str!("../../logos/buildkite.svg")),
    ("circleci", include_str!("../../logos/circleci.svg")),
    ("cloudflare", include_str!("../../logos/cloudflare.svg")),
    ("datadog", include_str!("../../logos/datadog.svg")),
    ("docker", include_str!("../../logos/docker.svg")),
    ("fly", include_str!("../../logos/fly.svg")),
    ("github", include_str!("../../logos/github.svg")),
    ("gmail", include_str!("../../logos/gmail.svg")),
    ("hetzner", include_str!("../../logos/hetzner.svg")),
    ("huggingface", include_str!("../../logos/huggingface.svg")),
    ("kubectl", include_str!("../../logos/kubectl.svg")),
    ("linear", include_str!("../../logos/linear.svg")),
    ("linkedin", include_str!("../../logos/linkedin.svg")),
    ("malt", include_str!("../../logos/malt.svg")),
    ("npm", include_str!("../../logos/npm.svg")),
    ("openai", include_str!("../../logos/openai.svg")),
    ("paddle", include_str!("../../logos/paddle.svg")),
    ("scaleway", include_str!("../../logos/scaleway.svg")),
    ("stripe", include_str!("../../logos/stripe.svg")),
    ("tailnet", include_str!("../../logos/tailnet.svg")),
    ("terraform", include_str!("../../logos/terraform.svg")),
    ("warp", include_str!("../../logos/warp.svg")),
    ("whatsapp", include_str!("../../logos/whatsapp.svg")),
];

/// The tools Simple Icons lacks: each product's own favicon, 64 px PNG from its site (Google's favicon service, 2026-11-09),
/// a data URL built at compile time, so the page carries it. An image, never inlined markup: a fetched file runs no script on the page.
const PICTURES: [(&str, &str); 14] = [
    (
        "avrea",
        concat!(
            "data:image/png;base64,",
            include_str!("../../logos/avrea.png.b64")
        ),
    ),
    (
        "blacksmith",
        concat!(
            "data:image/png;base64,",
            include_str!("../../logos/blacksmith.png.b64")
        ),
    ),
    (
        "browserbase",
        concat!(
            "data:image/png;base64,",
            include_str!("../../logos/browserbase.png.b64")
        ),
    ),
    (
        "bunnynet",
        concat!(
            "data:image/png;base64,",
            include_str!("../../logos/bunnynet.png.b64")
        ),
    ),
    (
        "depot",
        concat!(
            "data:image/png;base64,",
            include_str!("../../logos/depot.png.b64")
        ),
    ),
    (
        "fal",
        concat!(
            "data:image/png;base64,",
            include_str!("../../logos/fal.png.b64")
        ),
    ),
    (
        "freework",
        concat!(
            "data:image/png;base64,",
            include_str!("../../logos/freework.png.b64")
        ),
    ),
    (
        "healthchecks",
        concat!(
            "data:image/png;base64,",
            include_str!("../../logos/healthchecks.png.b64")
        ),
    ),
    (
        "openrouter",
        concat!(
            "data:image/png;base64,",
            include_str!("../../logos/openrouter.png.b64")
        ),
    ),
    // his own nicolas-ha.com's
    (
        "site",
        concat!(
            "data:image/png;base64,",
            include_str!("../../logos/site.png.b64")
        ),
    ),
    (
        "skyvern",
        concat!(
            "data:image/png;base64,",
            include_str!("../../logos/skyvern.png.b64")
        ),
    ),
    (
        "tinyfish",
        concat!(
            "data:image/png;base64,",
            include_str!("../../logos/tinyfish.png.b64")
        ),
    ),
    (
        "typesafe",
        concat!(
            "data:image/png;base64,",
            include_str!("../../logos/typesafe.png.b64")
        ),
    ),
    (
        "ubicloud",
        concat!(
            "data:image/png;base64,",
            include_str!("../../logos/ubicloud.png.b64")
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
