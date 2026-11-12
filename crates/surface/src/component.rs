//! What each design-system component registers from its own module, as each page does (inventory, PR 1336): its /design
//! entry, the classes and tags it owns, its look. No list names the components, so two tasks that each add one edit no file
//! in common (task rust-components-register; @nha, 2026-11-10: « this then finally unblocks adding parallelism and unblocks
//! velocity development »). A new component is a file in components/each/ and its two lines in components/each.rs, which
//! .gitattributes merges as the union of both sides.
//! dash-gate reads `slug`, `classes` and `tags` from each `inventory::submit!` block's text: write them as literals, first.
//! A leaf: it names no module of this crate, so a component's own module can use it with no cycle.
use dioxus::prelude::*;
use std::sync::LazyLock;

pub struct Component {
    /// Its id: /design's `s-<slug>` section and dioxus-components' page of it.
    pub slug: &'static str,
    /// The classes it draws: its own (`timeline`, `when`), or a look a page passes as `look`. No page writes one (dash-gate's
    /// `page_local_markup`).
    pub classes: &'static [&'static str],
    /// The elements it draws: a page writes `Button {}`, never `button {}`.
    pub tags: &'static [&'static str],
    /// /design's name of it, where it is used, and its example given the page's query; with no example it is not on /design yet.
    pub name: &'static str,
    pub used: &'static str,
    pub example: Option<fn(&str) -> Element>,
    /// The library page it was ported from, when not dioxus-components' page of `slug` (rust-ui's for those it lacks).
    pub library: Option<&'static str>,
    /// Its look, served after components.css on every page and in the window: a rule in components.css that changes it needs
    /// a higher specificity (`.ask .kbd-group`), as it comes first.
    pub css: &'static str,
    /// /design draws its example as its HTML beside its text, not light beside dark: the tables and graphs, how agents see
    /// them (task design-agent-forms; @nha, 2026-11-09: « tables, graphs, boards. »).
    pub formed: bool,
    /// The menu or its comment box draws it: its looks are carried scoped to them too, for an older page that loads no
    /// components.css (crate::forms' MENU_LOOKS).
    pub menu: bool,
    /// Its sections on /design's family tabs, drawn where `after` says.
    pub sections: &'static [Section],
    /// Its row in the Layout tab's « Against ours » table: HeroUI's component, what we did to it, our name for it (Badge:
    /// `("chip", "restyled", "badge")`; task rust-components-register).
    pub heroui: Option<(&'static str, &'static str, &'static str)>,
}

/// A section a component draws on a /design family tab (design::FAMILIES names the families): each row its label and its
/// drawing, given the page's query, drawn light and dark (or HTML and text) as the family's own sections are.
pub struct Section {
    pub family: &'static str,
    /// Its anchor, `#s-<id>`, and its `?tab=<id>`.
    pub id: &'static str,
    pub title: &'static str,
    /// The section it follows in its family, a family's own or another component's; "" first.
    pub after: &'static str,
    pub rows: &'static [Row],
}

/// A section's row: its label, and its drawing given the page's query.
pub type Row = (&'static str, fn(&str) -> Element);

impl Component {
    /// What a registration leaves out: `..Component::BARE`.
    pub const BARE: Self = Self {
        slug: "",
        classes: &[],
        tags: &[],
        name: "",
        used: "",
        example: None,
        library: None,
        css: "",
        formed: false,
        menu: false,
        sections: &[],
        heroui: None,
    };

    /// The library's page of it.
    pub fn source(&self) -> String {
        self.library.map_or_else(
            || {
                format!(
                    "https://dioxuslabs.github.io/dioxus-components/component/?name={}",
                    self.slug
                )
            },
            String::from,
        )
    }
}

inventory::collect!(Component);

/// The sections components draw on the family tab `family`.
pub fn sections(family: &str) -> impl Iterator<Item = &'static Section> + '_ {
    registered()
        .iter()
        .flat_map(|c| c.sections.iter())
        .filter(move |s| s.family == family)
}

/// Every component registered, by slug: the order /design shows them in. Gathered once a process.
pub fn registered() -> &'static [&'static Component] {
    static ALL: LazyLock<Vec<&'static Component>> = LazyLock::new(|| {
        let mut all: Vec<&'static Component> = inventory::iter::<Component>.into_iter().collect();
        all.sort_by_key(|c| c.slug);
        all
    });
    &ALL
}

/// The components' looks, which every page, /design and the window load: components.css, then each one's own, by slug.
pub fn css() -> &'static str {
    static CSS: LazyLock<String> = LazyLock::new(|| {
        let own = registered().iter().map(|c| c.css.trim_end());
        let all: Vec<&str> = std::iter::once(include_str!("components.css").trim_end())
            .chain(own)
            .filter(|css| !css.is_empty())
            .collect();
        all.join("\n") + "\n"
    });
    &CSS
}
