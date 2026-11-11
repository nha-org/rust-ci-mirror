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

/// Each control, link, section and field of a page, in page order, once each by kind and key.
pub fn items(html: &str) -> Vec<Item> {
    // a base, or blitz panics on the page's first relative stylesheet; nothing is fetched, no provider is given
    let config = DocumentConfig {
        base_url: Some("http://localhost/".into()),
        ..Default::default()
    };
    let doc = HtmlDocument::from_html(html, config);
    let mut found = vec![];
    walk(&doc, doc.root_element(), &mut found);
    let mut once: Vec<Item> = vec![];
    for item in found {
        let k = item.key();
        if !k.is_empty() && !once.iter().any(|o| o.kind == item.kind && o.key() == k) {
            once.push(item);
        }
    }
    once
}

/// What the Elixir page shows and the Rust page lacks.
pub fn lacks(elixir: &str, rust: &str) -> Vec<Item> {
    let has = items(rust);
    items(elixir)
        .into_iter()
        .filter(|e| {
            !has.iter().any(|r| {
                r.kind.meets(e.kind)
                    && (r.key() == e.key() || goes_as(r.href.as_deref(), e.href.as_deref()))
            })
        })
        .collect()
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
pub fn gaps(elixir: &str, rust: &str) -> Result<Vec<Item>, String> {
    if is_elixir(rust) {
        return Err(
            "Rust does not serve this page yet: its front door answers with the Elixir page".into(),
        );
    }
    Ok(lacks(elixir, rust))
}

/// Hidden by its style: `display: none`, or Tailwind's `hidden` with no breakpoint showing it (the older app's closed
/// modals). `hidden @2xl:inline-flex` is a wide screen's part, and kept.
fn unseen(node: &Node) -> bool {
    let style = node
        .attr(local_name!("style"))
        .unwrap_or("")
        .replace(' ', "");
    let class = node.attr(local_name!("class")).unwrap_or("");
    let shown = |c: &str| {
        c.rsplit_once(':').is_some_and(|(_, d)| {
            matches!(
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

fn walk(doc: &BaseDocument, node: &Node, found: &mut Vec<Item>) {
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
    if let Some(item) = item(doc, node, tag) {
        found.push(item);
    }
    for child in &node.children {
        if let Some(child) = doc.get_node(*child) {
            walk(doc, child, found);
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
        .and_then(|h| h.split('#').next().map(str::to_string))
        .filter(|h| !h.is_empty());
    Some(Item { kind, label, href })
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::memory::TestBudget;

    /// An Elixir page holding one of each kind, the way the older app draws them.
    const ELIXIR: &str = r#"<!doctype html><html><head><title>Mail</title><link rel="stylesheet" href="/dashboard/assets/css/app.css"></head><body><main data-phx-main>
        <h2>Threads 12</h2>
        <button phx-click="archive">Archive</button>
        <a href="/drafts">Drafts (3)</a>
        <table><tr><th>Received</th></tr></table>
        <input type="hidden" name="_csrf">
        <script>var x = "<button>Ghost</button>";</script>
        </main></body></html>"#;

    fn lacking(rust: &str) -> Vec<(Kind, String)> {
        lacks(ELIXIR, rust)
            .into_iter()
            .map(|i| (i.kind, i.label))
            .collect()
    }

    /// Rust drawing all four, counts changed and the button kept as a link: nothing lacks.
    const WHOLE: &str = r#"<h1>Threads 40</h1><a href="/mail/archive">Archive</a><a href="/drafts">Drafts 9</a>
        <table><thead><tr><th>Received</th></tr></thead></table>"#;

    #[test]
    fn a_rust_page_with_every_part_lacks_nothing() {
        let _budget = TestBudget::new();
        assert_eq!(lacking(WHOLE), vec![]);
        assert_eq!(gaps(ELIXIR, WHOLE), Ok(vec![]));
        // the front door's pass-through: the « Rust » page is Elixir's own
        assert!(
            gaps(ELIXIR, ELIXIR)
                .expect_err("not Rust's")
                .contains("does not serve")
        );
    }

    /// Task rust-issue-sidebar-menus: a control's words for agents alone (`text-only`, never shown) do not name it, so a Rust
    /// menu choice « todo: Todo » meets Elixir's « Todo »; one whose shown words differ is still a gap.
    #[test]
    fn words_for_agents_alone_do_not_name_a_control() {
        let _budget = TestBudget::new();
        let elixir = ELIXIR.replace("Archive", "Todo");
        let rust = WHOLE.replace(
            r#"<a href="/mail/archive">Archive</a>"#,
            r#"<button><span class="text-only">todo:</span>Todo<span class="text-only">(set)</span></button>"#,
        );
        assert_eq!(lacks(&elixir, &rust), vec![]);
        // an avatar's letters beside the name, two spans: two words, as Elixir's « R rust »
        let elixir = ELIXIR.replace("Archive", "R rust");
        let avatar = WHOLE.replace(
            r#"<a href="/mail/archive">Archive</a>"#,
            r#"<button><span><span class="avatar">R</span></span><span>Rust</span></button>"#,
        );
        assert_eq!(lacks(&elixir, &avatar), vec![]);
        // an element beside text is the same word: « x<sup>2</sup> » is « x2 », never « x 2 »
        let elixir = ELIXIR.replace("Archive", "Mark2");
        let word = WHOLE.replace(
            r#"<a href="/mail/archive">Archive</a>"#,
            r#"<button>Mark<sup>2</sup></button>"#,
        );
        assert_eq!(lacks(&elixir, &word), vec![]);
        let elixir = ELIXIR.replace("Archive", "Todo");
        let rust = rust.replace(">Todo<", ">Doing<");
        assert_eq!(
            lacks(&elixir, &rust)
                .iter()
                .map(|i| i.label.as_str())
                .collect::<Vec<_>>(),
            ["Todo"]
        );
    }

    #[test]
    fn a_missing_control_is_listed() {
        let _budget = TestBudget::new();
        let rust = WHOLE.replace(r#"<a href="/mail/archive">Archive</a>"#, "");
        assert_eq!(lacking(&rust), vec![(Kind::Control, "Archive".into())]);
    }

    #[test]
    fn a_missing_link_is_listed() {
        let _budget = TestBudget::new();
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
        assert_eq!(lacks(&elixir, &rust).len(), 1);
    }

    #[test]
    fn a_missing_section_is_listed() {
        let _budget = TestBudget::new();
        // the same words in a paragraph are not a section
        let rust = WHOLE.replace("<h1>Threads 40</h1>", "<p>Threads</p>");
        assert_eq!(lacking(&rust), vec![(Kind::Section, "Threads 12".into())]);
    }

    #[test]
    fn a_missing_field_is_listed() {
        let _budget = TestBudget::new();
        let rust = WHOLE.replace("<th>Received</th>", "<th>Sent</th>");
        assert_eq!(lacking(&rust), vec![(Kind::Field, "Received".into())]);
    }

    #[test]
    fn an_unnamed_control_takes_its_label_or_name_and_hidden_parts_are_skipped() {
        let _budget = TestBudget::new();
        let page = r#"<input name="q" placeholder="Search mail"><select name="account"></select>
            <textarea aria-label="Reply"></textarea><input type="submit" value="Send"><div hidden><button>Gone</button></div>
            <div class="hidden fixed"><button>Delete forever</button></div><p style="display: none"><a href="/y">Y</a></p>
            <button class="hidden @2xl:inline-flex" title="Not starred"></button>
            <a href="/x"><svg></svg></a>"#;
        let got: Vec<_> = items(page)
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
}
