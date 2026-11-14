//! What an Elixir page shows that its Rust port lacks: each control, link, section and field, read from both pages' HTML
//! (task rust-port-parity; @nha, 2026-11-10: « they were NOT ported properly. the visuals are incomplete, far inferior, and
//! many features missing. »). Surface reads HTML as it draws it; `dash parity` and `dash land` fetch the two pages.
use blitz_dom::{BaseDocument, DocumentConfig, Node, local_name};
use blitz_html::HtmlDocument;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Control,
    Link,
    Section,
    Field,
}

impl Kind {
    pub const ALL: [Kind; 4] = [Kind::Control, Kind::Link, Kind::Section, Kind::Field];

    pub fn name(self) -> &'static str {
        match self {
            Kind::Control => "control",
            Kind::Link => "link",
            Kind::Section => "section",
            Kind::Field => "field",
        }
    }

    pub fn named(name: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.name() == name)
    }

    /// A control and a link both act: an Elixir button kept as a Rust link is no gap, nor the reverse.
    fn meets(self, other: Kind) -> bool {
        let acts = |k| matches!(k, Kind::Control | Kind::Link);
        self == other || acts(self) && acts(other)
    }
}

/// One thing a page shows: its kind, its words as shown, and the key two pages are matched on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub kind: Kind,
    pub label: String,
    /// A link's target, its fragment dropped: a row whose words are laid out anew still goes to the same page.
    pub href: Option<String>,
}

impl Item {
    /// Letters only, lower case, one space: counts and punctuation change with the data, the feature does not.
    pub fn key(&self) -> String {
        key(&self.label)
    }
}

pub fn key(label: &str) -> String {
    let letters: String = label
        .chars()
        .map(|c| if c.is_alphabetic() { c } else { ' ' })
        .collect();
    letters
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// A LiveView page, the older app's: a Rust front door that passes a path on to Elixir serves this, never a Rust page.
fn is_elixir(html: &str) -> bool {
    html.contains("data-phx-main")
}

/// The menu every page shares, each part by an attribute and its value: the older app's (layouts.ex: its menu, Home link,
/// search, theme and bell, never the header's whole, which also holds a page's own actions slot: /code-review), and Rust's
/// sidebar, the button opening it and the phone's tab bar (forms.rs). It is left out of each page's list and compared once on
/// its own, `dash parity menu` (task rust-parity-moved-routes: its nine items were listed on every page).
const MENU: [(&str, &str); 8] = [
    ("id", "menu"),
    ("id", "menu-search"),
    ("id", "notifications-shell"),
    ("aria-label", "Home"),
    ("aria-label", "Theme"),
    ("id", "dash-nav"),
    ("id", "dash-nav-open"),
    ("id", "dash-tabbar"),
];

/// What the Elixir page shows and the Rust page lacks: the page's own parts, and the shared menu's apart.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Gaps {
    pub page: Vec<Item>,
    pub menu: Vec<Item>,
}

/// Each control, link, section and field of a page, in page order, once each by kind and key: the page's own, then the
/// shared menu's.
pub fn parts(html: &str) -> (Vec<Item>, Vec<Item>) {
    // a base, or blitz panics on the page's first relative stylesheet; nothing is fetched, no provider is given
    let config = DocumentConfig {
        base_url: Some("http://localhost/".into()),
        ..Default::default()
    };
    let doc = HtmlDocument::from_html(html, config);
    let mut found = vec![];
    walk(&doc, doc.root_element(), false, &mut found);
    let (menu, page): (Vec<_>, Vec<_>) = found.into_iter().partition(|(menu, _)| *menu);
    let once = |found: Vec<(bool, Item)>| {
        let mut once: Vec<Item> = vec![];
        for (_, item) in found {
            let k = item.key();
            if !k.is_empty() && !once.iter().any(|o| o.kind == item.kind && o.key() == k) {
                once.push(item);
            }
        }
        once
    };
    (once(page), once(menu))
}

/// The controls the page's liveview draws into its root (`forms::LIVE_ROOT`) once it runs, which the page's HTML holds empty:
/// the compass's, its pick bar's Whole page and Cancel and its box's (task rust-doc-parity-rest; @nha's orchestrator, 2026-11-12,
/// in the question dialog: « teach dash parity to count the compass's liveview controls »). The Compass component is drawn in
/// each state, so a control it gains or loses is counted with no list to keep. None on a page with no liveview root.
fn live_parts(rust: &str) -> Vec<Item> {
    if !rust.contains(&format!(r#"id="{}""#, crate::forms::LIVE_ROOT)) {
        return vec![];
    }
    use dioxus::prelude::*;
    let drawn = dioxus_ssr::render_element(rsx! {
        crate::components::Compass { id: "parity-picking", picking: true }
        crate::components::Compass { id: "parity-box", opened: "" }
    });
    let (page, menu) = parts(&drawn);
    page.into_iter().chain(menu).collect()
}

/// What the Elixir page shows and the Rust page lacks. `opens` gives the page a link opens, read on both pages' links
/// (rust/parity.tsv's moves: a link to a route the port moved is there when Rust links its new path). A page part is met
/// anywhere on the Rust page, its menu included, and so is a menu item.
pub fn lacks(elixir: &str, rust: &str, opens: &dyn Fn(&str) -> String) -> Gaps {
    let ((want, want_menu), (has, has_menu)) = (parts(elixir), parts(rust));
    let has = has.into_iter().chain(live_parts(rust));
    // each Rust link's page read once, not once for each Elixir item (/code-review)
    let has: Vec<(Item, Option<String>)> = (has.into_iter().chain(has_menu))
        .map(|r| {
            let goes = r.href.as_deref().map(opens);
            (r, goes)
        })
        .collect();
    let missing = |want: Vec<Item>| -> Vec<Item> {
        want.into_iter()
            .filter(|e| meet(e, e.href.as_deref().map(opens).as_deref(), &has).is_none())
            .collect()
    };
    Gaps {
        page: missing(want),
        menu: missing(want_menu),
    }
}

/// The first Rust part that is Elixir's `want`: an acting kind meets an acting kind, by its words or, a link, by the page
/// it opens (`goes`, read through the moves). `has` holds each Rust part with the page it opens.
fn meet(want: &Item, goes: Option<&str>, has: &[(Item, Option<String>)]) -> Option<usize> {
    has.iter().position(|(r, rust)| {
        r.kind.meets(want.kind) && (r.key() == want.key() || goes_as(rust.as_deref(), goes))
    })
}

/// The computed styles compared, as Chrome resolves them: font, size, colour, spacing and border; `width` and `height`, the
/// element's box, are added from its bounding rectangle (task rust-parity-styles; @nha, 2026-11-10: « the visuals are
/// incomplete, far inferior »).
pub const STYLES: [&str; 12] = [
    "font-family",
    "font-size",
    "font-weight",
    "line-height",
    "color",
    "background-color",
    "padding",
    "margin",
    "border-width",
    "border-style",
    "border-color",
    "border-radius",
];

/// One element as Chrome shows it: its tag, attributes and seen words, whether it sits in the shared menu, and its
/// computed styles, each a name and value. `styles_js` reads them; `Seen::item` names it as `parts` names an HTML node.
#[derive(Clone, Debug, Default, serde::Deserialize)]
pub struct Seen {
    pub tag: String,
    pub attrs: Vec<(String, String)>,
    pub text: String,
    pub menu: bool,
    pub style: Vec<(String, String)>,
}

impl Seen {
    pub fn item(&self) -> Option<Item> {
        let attr = |name: &str| {
            (self.attrs.iter())
                .find(|(n, _)| n == name)
                .map(|(_, v)| v.trim().to_string())
                .filter(|v| !v.is_empty())
        };
        named(&self.tag, &attr, &self.text)
    }
}

/// The JavaScript expression `shot` evaluates on a page (`SHOT_EVAL`): one line of JSON, each element `parts` could name
/// that Chrome shows, as a `Seen`. The menu is the same attributes `MENU` lists.
pub fn styles_js() -> String {
    let tags = "h1,h2,h3,h4,h5,h6,summary,caption,legend,th,dt,label,a[href],input,select,textarea,button,\
                [role=button],[role=textbox],[phx-click]";
    let menu: Vec<String> = MENU.iter().map(|(a, v)| format!("[{a}=\"{v}\"]")).collect();
    format!(
        "JSON.stringify([...document.querySelectorAll({tags:?})].filter(e => e.checkVisibility()).map(e => {{ \
         const s = getComputedStyle(e), r = e.getBoundingClientRect(); \
         return {{ tag: e.localName, attrs: [...e.attributes].map(a => [a.name, a.value]), text: e.innerText ?? '', \
         menu: !!e.closest({menu:?}), \
         style: [...{props:?}.map(p => [p, s.getPropertyValue(p)]), ['width', Math.round(r.width) + 'px'], \
         ['height', Math.round(r.height) + 'px']] }}; }}))",
        menu = menu.join(","),
        props = STYLES,
    )
}

/// One element both pages hold: its Elixir item, and each style's name with its Elixir and Rust values.
#[derive(Debug, PartialEq, Eq)]
pub struct Styled {
    pub item: Item,
    pub styles: Vec<(String, String, String)>,
}

impl Styled {
    /// The styles whose values differ.
    pub fn differ(&self) -> usize {
        self.styles.iter().filter(|(_, e, r)| e != r).count()
    }

    /// Its lines: the element, then each style, Elixir's value and Rust's side by side, « differs » after each that does.
    pub fn lines(&self) -> Vec<String> {
        let head = format!(
            "style\t{}\t{}\t{} differ",
            self.item.kind.name(),
            self.item.label,
            self.differ()
        );
        let each = self.styles.iter().map(|(name, e, r)| match e == r {
            true => format!("\t{name}\t{e}\t{r}"),
            false => format!("\t{name}\t{e}\t{r}\tdiffers"),
        });
        std::iter::once(head).chain(each).collect()
    }
}

/// Each element both pages hold, its styles side by side: an Elixir element once by kind and words, and once by its look,
/// its tag and class (a list's rows are data: the first /mail run compared each of 45 threads), met by the first Rust element
/// `lacks` would meet it with, in the same part (`menu`: the shared menu's, else the page's own).
pub fn styles(
    elixir: &[Seen],
    rust: &[Seen],
    menu: bool,
    opens: &dyn Fn(&str) -> String,
) -> Vec<Styled> {
    fn read<'a>(
        seen: &'a [Seen],
        menu: bool,
        opens: &dyn Fn(&str) -> String,
    ) -> Vec<(Item, Option<String>, &'a Seen)> {
        (seen.iter())
            .filter(|s| s.menu == menu)
            .filter_map(|s| {
                let item = s.item()?;
                let goes = item.href.as_deref().map(opens);
                Some((item, goes, s))
            })
            .collect()
    }
    let (want, has) = (read(elixir, menu, opens), read(rust, menu, opens));
    let matchable: Vec<(Item, Option<String>)> =
        has.iter().map(|(i, g, _)| (i.clone(), g.clone())).collect();
    // a look is a tag and its class; an element with no class has none of its own, so it is never a row of another's
    let look = |s: &Seen| {
        let class = s
            .attrs
            .iter()
            .find(|(n, v)| n == "class" && !v.trim().is_empty());
        class.map(|(_, v)| (s.tag.clone(), v.clone()))
    };
    let (mut found, mut looks): (Vec<Styled>, Vec<(String, String)>) = (vec![], vec![]);
    for (item, goes, seen) in want {
        let again = (found.iter()).any(|s| s.item.kind == item.kind && s.item.key() == item.key());
        let look = look(seen);
        // ponytail: a Rust page whose rows of one Elixir look are drawn two ways shows only the first row's
        if again || look.as_ref().is_some_and(|l| looks.contains(l)) {
            continue;
        }
        let Some(at) = meet(&item, goes.as_deref(), &matchable) else {
            continue;
        };
        looks.extend(look);
        let theirs = &has[at].2.style;
        let styles = (seen.style.iter())
            .map(|(name, e)| {
                let r = (theirs.iter().find(|(n, _)| n == name))
                    .map(|(_, v)| v.clone())
                    .unwrap_or_default();
                (name.clone(), e.clone(), r)
            })
            .collect();
        found.push(Styled { item, styles });
    }
    found
}

/// Whether a Rust link goes where the Elixir one went: the same path, holding each of its query's pairs (Rust may add its
/// own, as `account=`), so `/mail?box=starred` is never met by a plain `/mail`.
fn goes_as(rust: Option<&str>, elixir: Option<&str>) -> bool {
    let (Some(rust), Some(elixir)) = (rust, elixir) else {
        return false;
    };
    let parts = |h: &str| {
        let (path, query) = h.split_once('?').unwrap_or((h, ""));
        (
            path.to_string(),
            query
                .split('&')
                .filter(|p| !p.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>(),
        )
    };
    let ((rp, rq), (ep, eq)) = (parts(rust), parts(elixir));
    rp == ep && eq.iter().all(|pair| rq.contains(pair))
}

/// `lacks`, once the Rust page is Rust's: a path Rust does not serve yet reaches the older app through the front door, and
/// compared with itself it would lack nothing.
pub fn gaps(elixir: &str, rust: &str, opens: &dyn Fn(&str) -> String) -> Result<Gaps, String> {
    if is_elixir(rust) {
        return Err(
            "Rust does not serve this page yet: its front door answers with the Elixir page".into(),
        );
    }
    Ok(lacks(elixir, rust, opens))
}

/// Hidden by its style: `display: none`, or Tailwind's `hidden` with no breakpoint showing it (the older app's closed
/// modals). `hidden @2xl:inline-flex` is a wide screen's part, and kept.
fn unseen(node: &Node) -> bool {
    let style = node
        .attr(local_name!("style"))
        .unwrap_or("")
        .replace(' ', "");
    let class = node.attr(local_name!("class")).unwrap_or("");
    // a state's variant (`group-data-copied/copy:inline`, the older copy row's « Copied ») shows only after a tap, so its
    // words name nothing on the page; any other (`@2xl:`, `md:`, `dark:`, `print:`) still shows (task rust-draft-state-edit)
    let state = |v: &str| {
        v.split(':').any(|p| {
            [
                "group-", "peer-", "data-", "aria-", "hover", "focus", "active", "open", "checked",
                "disabled",
            ]
            .iter()
            .any(|s| p.starts_with(s))
        })
    };
    let shown = |c: &str| {
        c.rsplit_once(':').is_some_and(|(v, d)| {
            !state(v)
                && matches!(
                    d,
                    "block"
                        | "inline"
                        | "inline-block"
                        | "flex"
                        | "inline-flex"
                        | "grid"
                        | "inline-grid"
                        | "table"
                        | "contents"
                )
        })
    };
    // `text-only` is the design system's words for agents alone (`display: none` in components.css): he never sees them, so
    // they name nothing on his page (task rust-issue-sidebar-menus: « todo: Todo » never met Elixir's « Todo »)
    style.contains("display:none")
        || class.split_whitespace().any(|c| c == "text-only")
        || class.split_whitespace().any(|c| c == "hidden") && !class.split_whitespace().any(shown)
}

/// The words a node shows: its text, what its unseen children hold left out, a blank between two elements side by side (an
/// avatar's « R » beside its name « Rust » reads « R Rust », as the page spaces them, never « RRust »); an element beside text
/// is the same word, as `x<sup>2</sup>` or `Mark<b>s</b>` (/code-review of PR 1399).
fn seen_text(doc: &BaseDocument, node: &Node, out: &mut String) {
    if let Some(text) = node.text_data() {
        out.push_str(&text.content);
    } else if node.element_data().is_none_or(|_| !unseen(node)) {
        let mut after_element = false;
        for child in node.children.iter().filter_map(|c| doc.get_node(*c)) {
            let element = child.element_data().is_some();
            if element && after_element {
                out.push(' ');
            }
            seen_text(doc, child, out);
            after_element = element;
        }
    }
}

/// `menu`: whether `node` sits in the shared menu.
fn walk(doc: &BaseDocument, node: &Node, menu: bool, found: &mut Vec<(bool, Item)>) {
    let Some(el) = node.element_data() else {
        return;
    };
    let tag: &str = &el.name.local;
    if matches!(tag, "script" | "style" | "template" | "head")
        || node.attr(local_name!("hidden")).is_some()
        || unseen(node)
    {
        return;
    }
    let menu = menu
        || node.attrs().is_some_and(|attrs| {
            attrs
                .iter()
                .any(|a| MENU.contains(&(&*a.name.local, a.value.as_str())))
        });
    if let Some(item) = item(doc, node, tag) {
        found.push((menu, item));
    }
    for child in &node.children {
        if let Some(child) = doc.get_node(*child) {
            walk(doc, child, menu, found);
        }
    }
}

fn item(doc: &BaseDocument, node: &Node, tag: &str) -> Option<Item> {
    let attr = |name: &str| {
        node.attrs()?
            .iter()
            .find(|a| &*a.name.local == name)
            .map(|a| a.value.trim().to_string())
            .filter(|v| !v.is_empty())
    };
    let mut shown = String::new();
    seen_text(doc, node, &mut shown);
    named(tag, &attr, &shown)
}

/// The item a `tag` names, by its attributes and its seen words: one rule for blitz's nodes and Chrome's elements (`Seen`).
fn named(tag: &str, attr: &dyn Fn(&str) -> Option<String>, shown: &str) -> Option<Item> {
    let text = shown.split_whitespace().collect::<Vec<_>>().join(" ");
    let named = || {
        Some(text.clone())
            .filter(|t| !key(t).is_empty())
            .or_else(|| attr("aria-label"))
            .or_else(|| attr("title"))
    };
    let (kind, label) = match tag {
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "summary" | "caption" | "legend" => {
            (Kind::Section, named()?)
        }
        "th" | "dt" | "label" => (Kind::Field, named()?),
        "a" if attr("href").is_some() => (Kind::Link, named().or_else(|| attr("href"))?),
        "input" if attr("type").as_deref() == Some("hidden") => return None,
        "input" => {
            let label = attr("aria-label")
                .or_else(|| attr("placeholder"))
                .or_else(|| {
                    attr("value")
                        .filter(|_| matches!(attr("type").as_deref(), Some("submit" | "button")))
                })
                .or_else(|| attr("name"))?;
            (Kind::Control, label)
        }
        "select" | "textarea" => (
            Kind::Control,
            attr("aria-label")
                .or_else(|| attr("placeholder"))
                .or_else(|| attr("name"))?,
        ),
        "button" => (Kind::Control, named().or_else(|| attr("name"))?),
        // the documents editor's field is a textarea's equal: named by its label or title, never its words (task
        // rust-comment-files-filter: Elixir's « Your comment » textarea); one with no name goes on to the arms below
        _ if attr("role").as_deref() == Some("textbox")
            && attr("aria-label").or_else(|| attr("title")).is_some() =>
        {
            (Kind::Control, attr("aria-label").or_else(|| attr("title"))?)
        }
        // a LiveView's own controls are any element with a click binding
        _ if attr("phx-click").is_some() || attr("role").as_deref() == Some("button") => {
            (Kind::Control, named()?)
        }
        _ => return None,
    };
    // a row's whole text is not a label: the first words name it
    let label: String = label.chars().take(80).collect();
    let href = attr("href")
        .filter(|_| kind == Kind::Link)
        // a place on the page itself (`#flow`) keeps its fragment: a contents link meets one to the same section, whatever
        // its words (the doc's own « 2. The flow » and Rust's heading « 2. The flow, walked on your case »)
        .map(|h| match h.starts_with('#') {
            true => h,
            false => h.split('#').next().unwrap_or_default().to_string(),
        })
        .filter(|h| !h.is_empty());
    Some(Item { kind, label, href })
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;

    /// An Elixir page holding one of each kind, the way the older app draws them.
    const ELIXIR: &str = r#"<!doctype html><html><head><title>Mail</title><link rel="stylesheet" href="/dashboard/assets/css/app.css"></head><body><main data-phx-main>
        <h2>Threads 12</h2>
        <button phx-click="archive">Archive</button>
        <a href="/drafts">Drafts (3)</a>
        <table><tr><th>Received</th></tr></table>
        <input type="hidden" name="_csrf">
        <script>var x = "<button>Ghost</button>";</script>
        </main></body></html>"#;

    /// No route moved: a link is there only at its own path.
    fn same(href: &str) -> String {
        href.to_string()
    }

    /// The page's own gaps, no route moved.
    fn page_lacks(elixir: &str, rust: &str) -> Vec<Item> {
        lacks(elixir, rust, &same).page
    }

    fn lacking(rust: &str) -> Vec<(Kind, String)> {
        page_lacks(ELIXIR, rust)
            .into_iter()
            .map(|i| (i.kind, i.label))
            .collect()
    }

    /// Rust drawing all four, counts changed and the button kept as a link: nothing lacks.
    const WHOLE: &str = r#"<h1>Threads 40</h1><a href="/mail/archive">Archive</a><a href="/drafts">Drafts 9</a>
        <table><thead><tr><th>Received</th></tr></thead></table>"#;

    #[dash_test]
    fn a_rust_page_with_every_part_lacks_nothing() {
        assert_eq!(lacking(WHOLE), vec![]);
        assert_eq!(gaps(ELIXIR, WHOLE, &same), Ok(Gaps::default()));
        // the front door's pass-through: the « Rust » page is Elixir's own
        assert!(
            gaps(ELIXIR, ELIXIR, &same)
                .expect_err("not Rust's")
                .contains("does not serve")
        );
    }

    /// Task rust-issue-sidebar-menus: a control's words for agents alone (`text-only`, never shown) do not name it, so a Rust
    /// menu choice « todo: Todo » meets Elixir's « Todo »; one whose shown words differ is still a gap.
    #[dash_test]
    fn words_for_agents_alone_do_not_name_a_control() {
        let elixir = ELIXIR.replace("Archive", "Todo");
        let rust = WHOLE.replace(
            r#"<a href="/mail/archive">Archive</a>"#,
            r#"<button><span class="text-only">todo:</span>Todo<span class="text-only">(set)</span></button>"#,
        );
        assert_eq!(page_lacks(&elixir, &rust), vec![]);
        // an avatar's letters beside the name, two spans: two words, as Elixir's « R rust »
        let elixir = ELIXIR.replace("Archive", "R rust");
        let avatar = WHOLE.replace(
            r#"<a href="/mail/archive">Archive</a>"#,
            r#"<button><span><span class="avatar">R</span></span><span>Rust</span></button>"#,
        );
        assert_eq!(page_lacks(&elixir, &avatar), vec![]);
        // an element beside text is the same word: « x<sup>2</sup> » is « x2 », never « x 2 »
        let elixir = ELIXIR.replace("Archive", "Mark2");
        let word = WHOLE.replace(
            r#"<a href="/mail/archive">Archive</a>"#,
            r#"<button>Mark<sup>2</sup></button>"#,
        );
        assert_eq!(page_lacks(&elixir, &word), vec![]);
        // a textarea named « Your comment » meets the documents editor's textbox of that title, never its typed words
        let elixir = ELIXIR.replace(
            r#"<button phx-click="archive">Archive</button>"#,
            r#"<textarea aria-label="Your comment"></textarea>"#,
        );
        let editor = r#"<div class="dv" contenteditable="plaintext-only" role="textbox" title="Your comment">Hi</div>"#;
        let editor = WHOLE.replace(r#"<a href="/mail/archive">Archive</a>"#, editor);
        assert_eq!(page_lacks(&elixir, &editor), vec![]);
        // a textbox with no name is still a LiveView control by its click, its words naming it (/code-review)
        let elixir = ELIXIR.replace(
            r#"<button phx-click="archive">"#,
            r#"<div role="textbox" phx-click="archive">"#,
        );
        let elixir = elixir.replace("Archive</button>", "Archive</div>");
        assert_eq!(page_lacks(&elixir, WHOLE), vec![]);
        assert_eq!(
            page_lacks(&elixir, &editor)
                .iter()
                .map(|i| i.label.as_str())
                .collect::<Vec<_>>(),
            ["Archive"]
        );
        let elixir = ELIXIR.replace("Archive", "Todo");
        let rust = rust.replace(">Todo<", ">Doing<");
        assert_eq!(
            page_lacks(&elixir, &rust)
                .iter()
                .map(|i| i.label.as_str())
                .collect::<Vec<_>>(),
            ["Todo"]
        );
    }

    #[dash_test]
    fn a_missing_control_is_listed() {
        let rust = WHOLE.replace(r#"<a href="/mail/archive">Archive</a>"#, "");
        assert_eq!(lacking(&rust), vec![(Kind::Control, "Archive".into())]);
    }

    #[dash_test]
    fn a_missing_link_is_listed() {
        let rust = WHOLE.replace(r#"<a href="/drafts">Drafts 9</a>"#, "");
        assert_eq!(lacking(&rust), vec![(Kind::Link, "Drafts (3)".into())]);
        // other words to the same page, its query aside: the link is there
        let rust = WHOLE.replace(
            r#"<a href="/drafts">Drafts 9</a>"#,
            r#"<a href="/drafts?account=me#top">Waiting</a>"#,
        );
        assert_eq!(lacking(&rust), vec![]);
        // a folder's link is not met by its page without the folder
        let elixir = ELIXIR.replace("/drafts", "/mail?box=drafts");
        let rust = WHOLE.replace(
            r#"<a href="/drafts">Drafts 9</a>"#,
            r#"<a href="/mail?account=me">Waiting</a>"#,
        );
        assert_eq!(page_lacks(&elixir, &rust).len(), 1);
    }

    #[dash_test]
    fn a_missing_section_is_listed() {
        // the same words in a paragraph are not a section
        let rust = WHOLE.replace("<h1>Threads 40</h1>", "<p>Threads</p>");
        assert_eq!(lacking(&rust), vec![(Kind::Section, "Threads 12".into())]);
    }

    #[dash_test]
    fn a_missing_field_is_listed() {
        let rust = WHOLE.replace("<th>Received</th>", "<th>Sent</th>");
        assert_eq!(lacking(&rust), vec![(Kind::Field, "Received".into())]);
    }

    /// Task rust-doc-parity-rest, done-when 3 and 4: the older /v/ page's pick bar (Whole page, Cancel) and its comment box's
    /// paper clip, as its HTML has them, are met by Rust's: the clip by the comment box's compact FileUpload, the pick bar by
    /// the Compass the page's liveview root draws, which the page's HTML holds empty. With no root, the pick bar is lacking.
    #[dash_test]
    fn the_compass_s_liveview_controls_and_the_paper_clip_meet_the_older_page_s() {
        let elixir = r#"<form id="thread-composer"><textarea aria-label="Your comment"></textarea>
            <label title="Attach images or PDFs" data-attach><span class="hero-paper-clip"></span><span class="sr-only">Attach images or PDFs</span>
            <input type="file" name="files" accept=".png,.pdf" multiple></label><button type="submit">Comment</button></form>
            <div data-pickbar role="status"><span>Tap the element to comment on</span><div aria-label="Comment on" role="group">
            <button type="button" data-whole-page>Whole page</button><button type="button">Cancel</button></div></div>"#;
        use dioxus::prelude::*;
        let rfc = crate::documents::Doc::Rfc(
            serde_json::from_value(serde_json::json!({
            "body": {"type": "doc", "content": []}, "page": "/v/private/docs/a.html"}))
            .expect("an rfc"),
        );
        let box_ = dioxus_ssr::render_element(rsx! { {crate::documents::draw("doc", &rfc)} });
        let rust = |root: &str| format!("<main>{box_}</main>{root}");
        let missing = |rust: &str| -> Vec<String> {
            lacks(elixir, rust, &|h| h.to_string())
                .page
                .into_iter()
                .map(|i| i.label)
                .collect()
        };
        assert_eq!(
            missing(&rust(r#"<div id="dash-live"></div>"#)),
            Vec::<String>::new()
        );
        let without = missing(&rust(""));
        assert!(
            ["Whole page", "Cancel"]
                .iter()
                .all(|c| without.contains(&c.to_string())),
            "{without:?}"
        );
        assert!(
            !without.iter().any(|c| c.starts_with("Attach")),
            "the clip is the page's own: {without:?}"
        );
    }

    #[dash_test]
    fn an_unnamed_control_takes_its_label_or_name_and_hidden_parts_are_skipped() {
        let page = r#"<input name="q" placeholder="Search mail"><select name="account"></select>
            <textarea aria-label="Reply"></textarea><input type="submit" value="Send"><div hidden><button>Gone</button></div>
            <div class="hidden fixed"><button>Delete forever</button></div><p style="display: none"><a href="/y">Y</a></p>
            <button class="hidden @2xl:inline-flex" title="Not starred"></button>
            <a href="/x"><svg></svg></a>"#;
        let got: Vec<_> = parts(page)
            .0
            .into_iter()
            .map(|i| (i.kind.name(), i.label))
            .collect();
        let want = [
            ("control", "Search mail"),
            ("control", "account"),
            ("control", "Reply"),
            ("control", "Send"),
            ("control", "Not starred"),
            ("link", "/x"),
        ];
        assert_eq!(got, want.map(|(k, l)| (k, l.to_string())));
    }

    /// Task rust-parity-moved-routes, done-when 1: a link to a route the port moved is there when Rust links its new path,
    /// whatever its words; without the move it is a gap.
    #[dash_test]
    fn a_link_to_a_moved_route_is_there_at_its_new_path() {
        let elixir = ELIXIR.replace("/drafts", "/d/acme");
        let rust = WHOLE.replace(
            r#"<a href="/drafts">Drafts 9</a>"#,
            r#"<a href="/drafts/beta">Open</a>"#,
        );
        // as dash's marks read both pages' links: a page of either route opens the new route, any row meeting any other
        let opens = |h: &str| match h.starts_with("/d/") || h.starts_with("/drafts/") {
            true => "/drafts/:id".to_string(),
            false => h.to_string(),
        };
        assert_eq!(lacks(&elixir, &rust, &opens), Gaps::default());
        assert_eq!(page_lacks(&elixir, &rust).len(), 1);
    }

    /// Task rust-parity-moved-routes, done-when 3: the shared menu's items leave each page's list and are compared on their
    /// own, Elixir's menu against Rust's; a page part Rust shows only in its menu is there.
    #[dash_test]
    fn the_shared_menu_is_compared_apart_from_the_page() {
        let menu = r#"<header id="site-header"><nav id="menu"><a href="/focus">Focus</a><a href="/mail">Mail</a></nav><button>Snooze</button></header>"#;
        let elixir = ELIXIR.replace(
            "<main data-phx-main>",
            &format!("{menu}<main data-phx-main>"),
        );
        let rust = format!(r#"<aside id="dash-nav"><nav id="dash-menu"><a href="/mail">Mail</a><a href="/drafts">Drafts</a></nav></aside>{WHOLE}"#)
            .replace(r#"<a href="/drafts">Drafts 9</a>"#, "");
        let gaps = lacks(&elixir, &rust, &same);
        let labels = |items: &[Item]| items.iter().map(|i| i.label.clone()).collect::<Vec<_>>();
        // the drafts link is in Rust's menu, the menu's items are not the page's; a page's own action in the header is
        assert_eq!(labels(&gaps.page), ["Snooze"]);
        assert_eq!(labels(&gaps.menu), ["Focus"]);
        // a menu item Rust draws in its page is there
        let body = rust.replace("<h1>", r#"<a href="/focus">Focus</a><h1>"#);
        assert_eq!(
            labels(&lacks(&elixir, &body, &same).menu),
            Vec::<String>::new()
        );
        let (page, menu) = parts(&elixir);
        assert!(
            menu.iter().all(|i| i.label != "Drafts (3)") && page.iter().all(|i| i.label != "Focus")
        );
    }

    /// One element as Chrome reads it, with these styles over a black, 14 px base.
    fn seen(
        tag: &str,
        attrs: &[(&str, &str)],
        text: &str,
        menu: bool,
        style: &[(&str, &str)],
    ) -> Seen {
        let mut base: Vec<(String, String)> = [("font-size", "14px"), ("color", "rgb(0, 0, 0)")]
            .iter()
            .map(|(n, v)| (n.to_string(), v.to_string()))
            .collect();
        for (n, v) in style {
            base.retain(|(b, _)| b != n);
            base.push((n.to_string(), v.to_string()));
        }
        let pairs = |p: &[(&str, &str)]| {
            p.iter()
                .map(|(n, v)| (n.to_string(), v.to_string()))
                .collect()
        };
        Seen {
            tag: tag.into(),
            attrs: pairs(attrs),
            text: text.into(),
            menu,
            style: base,
        }
    }

    /// Task rust-parity-styles, done-when 1: each element both pages hold gets both computed styles side by side, each
    /// difference flagged; an element Rust lacks, the other part's (menu or page) and a second row of the same look are not.
    #[dash_test]
    fn each_element_both_pages_hold_shows_both_styles_and_flags_each_difference() {
        let elixir = [
            seen("h2", &[], "Threads 12", false, &[]),
            seen(
                "button",
                &[("phx-click", "archive")],
                "Archive",
                false,
                &[("font-size", "14px")],
            ),
            seen(
                "a",
                &[("href", "/mail/t/a"), ("class", "row")],
                "First thread",
                false,
                &[],
            ),
            seen(
                "a",
                &[("href", "/mail/t/b"), ("class", "row")],
                "Second thread",
                false,
                &[("color", "red")],
            ),
            seen("th", &[], "Received", false, &[]),
            // a second heading with no class is no row of the first: compared too (/code-review)
            seen("h2", &[], "Labels", false, &[]),
            seen("a", &[("href", "/focus")], "Focus", true, &[]),
        ];
        let rust = [
            seen("h1", &[], "Threads 40", false, &[]),
            seen(
                "a",
                &[("href", "/mail/archive")],
                "Archive",
                false,
                &[("font-size", "13px")],
            ),
            seen("a", &[("href", "/mail/t/z")], "Other thread", false, &[]),
            seen("a", &[("href", "/mail/t/b")], "Second thread", false, &[]),
            seen(
                "a",
                &[("href", "/focus")],
                "Focus",
                true,
                &[("color", "rgb(1, 1, 1)")],
            ),
            seen("h3", &[], "Labels", false, &[]),
        ];
        let opens = |h: &str| match h.starts_with("/mail/t/") {
            true => "/mail/t/:id".to_string(),
            false => h.to_string(),
        };
        let page = styles(&elixir, &rust, false, &opens);
        let lines: Vec<String> = page.iter().flat_map(Styled::lines).collect();
        assert_eq!(
            lines,
            [
                "style\tsection\tThreads 12\t0 differ",
                "\tfont-size\t14px\t14px",
                "\tcolor\trgb(0, 0, 0)\trgb(0, 0, 0)",
                "style\tcontrol\tArchive\t1 differ",
                "\tcolor\trgb(0, 0, 0)\trgb(0, 0, 0)",
                "\tfont-size\t14px\t13px\tdiffers",
                "style\tlink\tFirst thread\t0 differ",
                "\tfont-size\t14px\t14px",
                "\tcolor\trgb(0, 0, 0)\trgb(0, 0, 0)",
                "style\tsection\tLabels\t0 differ",
                "\tfont-size\t14px\t14px",
                "\tcolor\trgb(0, 0, 0)\trgb(0, 0, 0)",
            ]
        );
        // the shared menu's elements are compared on their own
        let menu = styles(&elixir, &rust, true, &opens);
        assert_eq!(menu.len(), 1);
        assert_eq!(
            menu[0].lines()[2],
            "\tcolor\trgb(0, 0, 0)\trgb(1, 1, 1)\tdiffers"
        );
        // the script Chrome runs reads every style named, and the menu by MENU's attributes
        let js = styles_js();
        assert!(
            STYLES.iter().all(|s| js.contains(&format!("\"{s}\""))),
            "{js}"
        );
        assert!(js.contains(r#"[id=\"dash-nav\"]"#), "{js}");
    }

    /// Task rust-draft-state-edit: words a state shows only after a tap (the older copy row's « Copied ») name nothing, so its
    /// button is « Copy the text » as he sees it; a breakpoint's variant still shows (`md:inline`).
    #[dash_test]
    fn words_a_tap_shows_name_nothing_and_a_breakpoint_s_still_show() {
        let page = r#"<button>Copy the text <span class="hidden group-data-copied/copy:inline">Copied</span>
            <span class="hidden group-data-copy-failed/copy:inline">Copy failed, the text is selected</span></button>
            <button>Star <span class="hidden md:inline">this mail</span></button>
            <button>Theme <span class="hidden dark:inline">dark</span></button>"#;
        let got: Vec<_> = parts(page).0.into_iter().map(|i| i.label).collect();
        assert_eq!(got, ["Copy the text", "Star this mail", "Theme dark"]);
    }
}
