//! Adapter of section 3: the native window (section 9), Dioxus Native as rust/ui-spike picked it (@nha, 2026-11-08: « DIOXUS IS
//! VALIDATED »). It draws whatever component it is given, so the screen stays the surface crate's, the one the web page renders.
//! `open_pages` is the app (task rust-window-in-process): each page the very HTML dash serve answers, rendered in this process.
use blitz_dom::{Document, DocumentConfig};
use dioxus::prelude::*;
pub use dioxus_native_dom::DioxusDocument;
use std::sync::Arc;

/// The document a window would draw, built with no display: what a test reads of the window. It parses a page's HTML as the
/// window does.
pub fn document(dom: VirtualDom) -> DioxusDocument {
    let config = DocumentConfig {
        html_parser_provider: Some(Arc::new(blitz_html::HtmlProvider) as _),
        ..Default::default()
    };
    let mut doc = DioxusDocument::new(dom, config);
    doc.initial_build();
    doc
}

/// The words a window of this component shows, its stylesheet left out.
pub fn shown(dom: VirtualDom) -> String {
    let doc = document(dom);
    let doc = doc.inner();
    let mut words = String::new();
    let mut at = vec![doc.root_element().id];
    while let Some(id) = at.pop() {
        let node = doc.get_node(id).expect("a node of the tree");
        if node
            .data
            .is_element_with_tag_name(&blitz_dom::local_name!("style"))
        {
            continue;
        }
        if let Some(text) = node.text_data() {
            words += &text.content;
            words.push('\n');
        }
        at.extend(node.children.iter().rev());
    }
    words
}

/// The window, until it is closed.
#[cfg(feature = "window")]
pub fn open<P: Clone + 'static, M: 'static>(
    app: impl dioxus::dioxus_core::ComponentFunction<P, M>,
    props: P,
    title: &str,
) {
    use dioxus_native::{LogicalSize, WindowAttributes};
    let window = WindowAttributes::default()
        .with_title(title)
        .with_surface_size(LogicalSize::new(1200.0, 800.0));
    dioxus_native::launch_cfg_with_props(app, props, vec![], vec![Box::new(window)]);
}

/// A page's HTML, whole, as dash serve answers it.
#[derive(Props, Clone, PartialEq)]
pub struct PageProps {
    pub html: String,
}

/// Its root wears the page's `<html>` classes, its theme: parsed as a fragment, the page loses its `<html>` element.
/// ponytail: light whatever the Mac's mode, as the window always was; the web's DARK_JS script runs in a browser only.
#[allow(non_snake_case)]
pub fn Page(PageProps { html }: PageProps) -> Element {
    let theme = html_class(&html);
    rsx! { div { class: theme, dangerous_inner_html: "{html}" } }
}

/// The `class` of a page's `<html>` tag, as dash's pages write it (`class="app light"`); none when it has none.
fn html_class(html: &str) -> &str {
    let Some(at) = html.find("<html") else {
        return "";
    };
    let tag = &html[at..at + html[at..].find('>').unwrap_or(0)];
    tag.split_once(r#"class=""#)
        .and_then(|(_, rest)| rest.split_once('"'))
        .map_or("", |(class, _)| class)
}

/// The links of the app's pages resolve on this host, so a click names a page the window draws itself.
#[cfg(feature = "window")]
const HOST: &str = "window.dash";

/// A click or a GET form on a page of the app: its path and query go to the thread that renders; any other http(s) or mail
/// link opens in the browser.
#[cfg(feature = "window")]
struct InWindow(std::sync::mpsc::Sender<Ask>);

#[cfg(feature = "window")]
impl blitz_traits::navigation::NavigationProvider for InWindow {
    fn navigate_to(&self, asked: blitz_traits::navigation::NavigationOptions) {
        let url = asked.url;
        // no page of Rust's that the window draws has a POST form: the compass is a component beside the page (`Beside`)
        if asked.method != blitz_traits::net::Method::GET {
            eprintln!(
                "dash ui: {url} asks a {}: the window sends only GET",
                asked.method
            );
            return;
        }
        if url.host_str() == Some(HOST) {
            let target = match url.query() {
                Some(query) => format!("{}?{query}", url.path()),
                None => url.path().to_string(),
            };
            let _ = self.0.send(Ask::Page(target));
        } else if matches!(url.scheme(), "http" | "https" | "mailto") {
            let _ = std::process::Command::new("open").arg(url.as_str()).spawn();
        }
    }
}

/// How often `render` is asked a page that follows no view again, so a write to the store shows within it: such a page reads
/// the views, caught up with the op log on each read, so a write shows within a second with a render's time to spare
/// (rust-window-in-process done-when 2). `render` keeps what reads no store.
/// ponytail: a page that follows no view yet (the list, /inbox) is still rendered four times a second and swapped whole when it
/// changed; a page that follows one (`follows`) is drawn again only on its view's diff, as a patch (task rust-view-patches).
pub const REREAD: std::time::Duration = std::time::Duration::from_millis(250);

/// What the render thread is asked: a page a click names, or the ids a diff of the views changed.
pub enum Ask {
    Page(String),
    Changed(Vec<String>),
}

/// What the window is given: a page whole, or a page drawn again of which only the elements `ids` are put in (`patch`).
pub enum Drawn {
    Page(String),
    Patch(String, Vec<String>),
}

/// The window's render thread: `render` of `first`, then of each page `asked` names. A page that `follows` a view (given its
/// target and its HTML) is rendered again only when a diff changed some of its ids, and drawn as their patch; any other is
/// rendered again every `REREAD`, drawn whole when it changed. Until `draw` says the window is gone or `asked` closes. No
/// display: a test runs it.
pub fn render_loop(
    first: &str,
    mut render: impl FnMut(&str) -> String,
    mut follows: impl FnMut(&str, &str) -> bool,
    asked: std::sync::mpsc::Receiver<Ask>,
    mut draw: impl FnMut(Drawn) -> bool,
) {
    use std::sync::mpsc::RecvTimeoutError;
    let (mut target, mut last) = (first.to_string(), String::new());
    let mut changed: Option<Vec<String>> = None;
    loop {
        let page = render(&target);
        if page != last {
            let drawn = match changed.take() {
                Some(ids) => Drawn::Patch(page.clone(), ids),
                None => Drawn::Page(page.clone()),
            };
            if !draw(drawn) {
                return;
            }
            last = page;
        }
        let following = follows(&target, &last);
        let next = match following {
            true => asked.recv().map_err(|_| RecvTimeoutError::Disconnected),
            false => asked.recv_timeout(REREAD),
        };
        changed = None;
        match next {
            Ok(Ask::Page(next)) => target = next,
            Ok(Ask::Changed(ids)) => changed = following.then_some(ids),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

/// The element of `id` under `under`, by a walk of the tree: a subtree not in the document is in no id map.
fn holding(
    doc: &blitz_dom::BaseDocument,
    under: blitz_dom::NodeId,
    id: &str,
) -> Option<blitz_dom::NodeId> {
    let mut at = vec![under];
    while let Some(n) = at.pop() {
        let node = doc.get_node(n)?;
        if node.attr(blitz_dom::local_name!("id")) == Some(id) {
            return Some(n);
        }
        at.extend(node.children.iter().rev());
    }
    None
}

/// The elements `ids` of `page`, a page drawn again, put in place in the window's document under `root`, every other node
/// kept: a field's words, its focus and an open details with it (task rust-view-patches). As the web's LIVE_JS: an element
/// both drawings hold is replaced, one the newer no longer holds removed. ponytail: an element new to the page is left out,
/// where LIVE_JS puts in the nearest element around it; a board's patch names its columns, which every drawing holds.
pub fn patch(
    doc: &mut blitz_dom::BaseDocument,
    root: blitz_dom::NodeId,
    page: &str,
    ids: &[String],
) {
    let div = blitz_dom::QualName::new(None, blitz_dom::ns!(html), blitz_dom::local_name!("div"));
    // parsed, then moved in a mutator of its own: the parse's form controls are settled before any of them is dropped
    let newer = {
        let mut m = doc.mutate();
        let newer = m.create_element(div, vec![]);
        m.set_inner_html(newer, &unstyled(page));
        newer
    };
    let mut m = doc.mutate();
    for id in ids {
        let (old, new) = (holding(m.doc, root, id), holding(m.doc, newer, id));
        match (old, new) {
            (Some(old), Some(new)) => m.replace_node_with(old, &[new]),
            (Some(old), None) => drop(m.remove_and_drop_node(old)),
            _ => {}
        }
    }
    m.remove_and_drop_node(newer);
}

/// A page without its `<style>` blocks: the window holds them already, and blitz loads a stylesheet wherever one is parsed, a
/// detached copy too.
fn unstyled(page: &str) -> String {
    let (mut out, mut rest) = (String::new(), page);
    while let Some(at) = rest.find("<style") {
        out += &rest[..at];
        let end = rest[at..]
            .find("</style>")
            .map_or(rest.len(), |e| at + e + "</style>".len());
        rest = &rest[end..];
    }
    out + rest
}

/// The window's page as a component beside it reads it: the element its HTML is put under, once mounted.
#[derive(Clone, Copy)]
pub struct PageRoot(pub Signal<Option<dioxus_native_dom::NodeHandle>>);

/// A component the window draws beside the page, outside its HTML: the binary's compass, which this crate cannot name (ui
/// takes no surface, section 3). Its hooks are `Besides`', the same each render.
#[derive(Clone, Copy)]
pub struct Beside(pub fn() -> Element);

#[allow(non_snake_case)]
fn Besides() -> Element {
    (use_context::<Beside>().0)()
}

/// One element of the window's page under a point, as a component beside it reads it (the compass's quote).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Under {
    pub tag: String,
    pub id: String,
    pub role: String,
    /// Its words: its text, a field's value.
    pub text: String,
    /// What names it: aria-label, title, alt, placeholder.
    pub named: String,
}

/// The attribute the compass marks the element under the pointer with, outlined by the menu's look (forms.rs NAV_CSS).
const MARK: &str = "data-dash-pick";

/// What the window's page under `root` holds at `at`, innermost first up to the page itself, and the page's path (its rail's
/// `data-page`); `at` none, or a point on the page's background: no element, the whole page. The mark a look left is
/// cleared first; `mark` outlines the element found instead. A hit test of the page's own subtree, so what is drawn beside it
/// (the compass's layer over the screen) never answers. The document is laid out (`resolve`) before, as the shell does.
pub fn under(
    root: &dioxus_native_dom::NodeHandle,
    at: Option<(f32, f32)>,
    mark: bool,
) -> (String, Vec<Under>) {
    let mut doc = root.doc_mut();
    let named = |n: &blitz_dom::Node, a: &str| {
        n.attr(blitz_dom::LocalName::from(a))
            .unwrap_or_default()
            .to_string()
    };
    let (mut marked, mut at_) = (vec![], vec![root.node_id()]);
    while let Some(n) = at_.pop() {
        let Some(node) = doc.get_node(n) else {
            continue;
        };
        if node.attr(blitz_dom::LocalName::from(MARK)).is_some() {
            marked.push(n);
        }
        at_.extend(node.children.iter());
    }
    let qual =
        || blitz_dom::QualName::new(None, blitz_dom::ns!(), blitz_dom::LocalName::from(MARK));
    {
        let mut m = doc.mutate();
        for n in marked {
            m.clear_attribute(n, qual());
        }
    }
    let page = holding(&doc, root.node_id(), "dash-notes")
        .and_then(|n| doc.get_node(n))
        .map_or_else(|| "/".to_string(), |n| named(n, "data-page"));
    let Some((x, y)) = at else {
        return (page, vec![]);
    };
    let Some(page_node) = doc.get_node(root.node_id()) else {
        return (page, vec![]);
    };
    // the page's own subtree: its origin is where the root's containing block puts it
    let origin = page_node.absolute_position(0.0, 0.0);
    let loc = page_node.final_layout().location;
    let scale = doc.viewport().scale_f64();
    // the pointer's point is the viewport's; a hit test takes the document's, the viewport's scroll added (/code-review)
    let scroll = doc.viewport_scroll();
    let (x, y) = (x + scroll.x as f32, y + scroll.y as f32);
    let mut hit = page_node
        .hit(x - (origin.x - loc.x), y - (origin.y - loc.y), scale)
        .map(|h| h.node_id);
    while let Some(n) = hit
        .and_then(|n| doc.get_node(n))
        .filter(|n| !n.is_element())
    {
        hit = n.parent;
    }
    // up to the page's own element (`Page`'s, wearing its theme), which stands for the whole page
    let mut chain = vec![];
    while let Some(node) = hit.and_then(|n| doc.get_node(n)) {
        if node.parent.is_none_or(|p| p == root.node_id()) {
            break;
        }
        chain.push((
            node.id,
            Under {
                tag: node
                    .element_data()
                    .map_or_else(String::new, |e| e.name.local.to_string()),
                id: named(node, "id"),
                role: named(node, "role"),
                // a field's typed words, as the browser's (`UNDER_JS`), never a password's
                // ponytail: Blitz keeps a select's chosen option apart from none of its words, so a select quotes them all
                text: match node.element_data().and_then(|e| e.text_input_data()) {
                    Some(_) if named(node, "type") == "password" => String::new(),
                    Some(field) => field.editor.raw_text().to_string(),
                    None => node.text_content(),
                },
                named: ["aria-label", "title", "alt", "placeholder"]
                    .iter()
                    .map(|a| named(node, a))
                    .find(|v| !v.is_empty())
                    .unwrap_or_default(),
            },
        ));
        hit = node.parent;
    }
    if mark && let Some((first, _)) = chain.first() {
        doc.mutate().set_attribute(*first, qual(), "");
    }
    (page, chain.into_iter().map(|(_, u)| u).collect())
}

/// The renders the window's one component takes, once: the receiver goes to its future.
pub type Renders = Arc<std::sync::Mutex<Option<futures_channel::mpsc::UnboundedReceiver<Drawn>>>>;

/// The page's HTML in its signal, and a patch put in its document under the root mounted around it. A patch changes no
/// signal, and the shell repaints only when the vdom had work (blitz-shell's `poll`): the component asks a render after each,
/// else the patch showed on his next mouse move alone (/code-review of PR 1322). ponytail: a patch leaves the signal on the
/// page it patched; a later page drawn whole sets it, and one equal to that older page would not be drawn again.
#[allow(non_snake_case)]
pub fn Pages() -> Element {
    use futures_util::StreamExt;
    let mut html = use_signal(String::new);
    let mut root = use_signal(|| None::<dioxus_native_dom::NodeHandle>);
    let renders = use_hook(consume_context::<Renders>);
    let repaint = dioxus::dioxus_core::schedule_update();
    use_future(move || {
        let renders = renders.lock().expect("the renders").take();
        let repaint = repaint.clone();
        async move {
            if let Some(mut renders) = renders {
                while let Some(drawn) = renders.next().await {
                    match (drawn, root.peek().clone()) {
                        (Drawn::Patch(page, ids), Some(at)) => {
                            patch(&mut at.doc_mut(), at.node_id(), &page, &ids);
                            repaint();
                        }
                        (Drawn::Patch(page, _) | Drawn::Page(page), _) => html.set(page),
                    }
                }
            }
        }
    });
    use_context_provider(|| PageRoot(root));
    let beside = try_consume_context::<Beside>().is_some();
    rsx! {
        div { onmounted: move |e| root.set(e.data().downcast::<dioxus_native_dom::NodeHandle>().cloned()),
            Page { html: html() }
        }
        if beside {
            Besides {}
        }
    }
}

/// The app's window, until it is closed: `render` turns a page's path and query into its HTML, on a thread of its own, first
/// `first`, then each page a click asks; a page that `follows` a view is drawn again as a patch of the ids each of `changes`
/// names, any other rendered again every `REREAD` and drawn when it changed; `beside` drawn beside each page. No server under it.
#[cfg(feature = "window")]
pub fn open_pages(
    first: &str,
    title: &str,
    render: impl FnMut(&str) -> String + Send + 'static,
    follows: impl FnMut(&str, &str) -> bool + Send + 'static,
    changes: std::sync::mpsc::Receiver<Vec<String>>,
    beside: Beside,
) {
    use blitz_shell::{BlitzShellProxy, WindowConfig, create_default_event_loop};
    use dioxus_native::{
        DioxusNativeApplication, DioxusNativeWindowRenderer, LogicalSize, WindowAttributes,
    };
    let (asks, asked) = std::sync::mpsc::channel::<Ask>();
    let (drawn, renders) = futures_channel::mpsc::unbounded::<Drawn>();
    let first = first.to_string();
    let told = asks.clone();
    std::thread::spawn(move || {
        for ids in changes {
            if told.send(Ask::Changed(ids)).is_err() {
                return;
            }
        }
    });
    std::thread::spawn(move || {
        render_loop(&first, render, follows, asked, |page| {
            drawn.unbounded_send(page).is_ok()
        })
    });
    let event_loop = create_default_event_loop();
    let (proxy, events) = BlitzShellProxy::new(event_loop.create_proxy());
    let vdom = VirtualDom::new(Pages);
    vdom.provide_root_context(Arc::new(std::sync::Mutex::new(Some(renders))) as Renders);
    vdom.provide_root_context(beside);
    let doc = DioxusDocument::new(
        vdom,
        DocumentConfig {
            base_url: Some(format!("http://{HOST}/")),
            html_parser_provider: Some(Arc::new(blitz_html::HtmlProvider) as _),
            navigation_provider: Some(Arc::new(InWindow(asks)) as _),
            ..Default::default()
        },
    );
    let window = WindowAttributes::default()
        .with_title(title)
        .with_surface_size(LogicalSize::new(1280.0, 860.0));
    let config = WindowConfig::with_attributes(
        Box::new(doc) as _,
        DioxusNativeWindowRenderer::new(),
        window,
    );
    let app = DioxusNativeApplication::new(proxy, events, config);
    event_loop.run_app(app).expect("the window's event loop");
}

/// A page as its window draws it, painted on the CPU into a PNG of `width` by `height`: the window's own document, so a check
/// sees the window with no display taken (rust/CLAUDE.md rule 3).
#[cfg(feature = "window")]
pub fn shot(html: String, width: u32, height: u32, out: &std::path::Path) -> Result<(), String> {
    use anyrender::ImageRenderer;
    use blitz_traits::shell::{ColorScheme, Viewport};
    let config = DocumentConfig {
        base_url: Some(format!("http://{HOST}/")),
        html_parser_provider: Some(Arc::new(blitz_html::HtmlProvider) as _),
        viewport: Some(Viewport::new(width, height, 1.0, ColorScheme::Light)),
        ..Default::default()
    };
    let mut doc = DioxusDocument::new(VirtualDom::new_with_props(Page, PageProps { html }), config);
    doc.initial_build();
    for _ in 0..8 {
        if !doc.poll(None) {
            break;
        }
    }
    doc.inner_mut().resolve(0.0);
    let mut rgba = vec![0; (width * height * 4) as usize];
    let mut renderer = anyrender_vello_cpu::VelloCpuImageRenderer::new(width, height);
    renderer.render(
        |scene| blitz_paint::paint_scene(scene, &mut doc.inner_mut(), 1.0, width, height, 0, 0),
        &mut rgba,
    );
    let at = |e: &dyn std::fmt::Display| format!("{}: {e}", out.display());
    let file = std::fs::File::create(out).map_err(|e| at(&e))?;
    let mut png = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    png.set_color(png::ColorType::Rgba);
    png.set_depth(png::BitDepth::Eight);
    png.write_header()
        .and_then(|mut w| w.write_image_data(&rgba))
        .map_err(|e| at(&e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;

    #[dash_test]
    fn a_component_s_words_are_what_its_window_shows() {
        fn app() -> Element {
            rsx! { style { "p {{ color: red }}" } h1 { "Rust tasks" } p { "a-task" } }
        }
        assert_eq!(shown(VirtualDom::new(app)), "Rust tasks\na-task\n");
    }

    /// A page's HTML is parsed into the window's tree, its styles and scripts left out of the words.
    #[dash_test]
    fn the_window_wears_the_page_s_theme() {
        assert_eq!(
            html_class(r#"<!doctype html><html lang="en" class="app light"><head>"#),
            "app light"
        );
        assert_eq!(html_class(r#"<html lang="en"><body class="x">"#), "");
        assert_eq!(html_class("<p>no html</p>"), "");
    }

    /// Done-when 3 of rust-view-patches: a page that follows a view is rendered once, then only on its view's diff, drawn as a
    /// patch of the ids the diff names, with no REREAD timer; a page that follows none is still rendered every REREAD.
    #[dash_test]
    fn a_page_that_follows_a_view_is_drawn_again_only_on_its_diff() {
        use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
        use std::sync::{Arc, mpsc::channel};
        use std::time::Duration;
        let renders = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&renders);
        let (asks, asked) = channel();
        let (drew, drawn) = channel();
        std::thread::spawn(move || {
            let render = move |target: &str| {
                let n = counted.fetch_add(1, SeqCst);
                format!("{target} {n}")
            };
            let follows = |target: &str, _: &str| target == "/tasks";
            render_loop("/tasks", render, follows, asked, |d| drew.send(d).is_ok())
        });
        let wait = Duration::from_secs(5);
        assert!(matches!(drawn.recv_timeout(wait), Ok(Drawn::Page(p)) if p == "/tasks 0"));
        std::thread::sleep(REREAD * 4);
        assert_eq!(renders.load(SeqCst), 1, "no timer while it follows a view");
        asks.send(Ask::Changed(vec!["todo".into()])).unwrap();
        match drawn.recv_timeout(wait) {
            Ok(Drawn::Patch(page, ids)) => {
                assert_eq!((page.as_str(), ids), ("/tasks 1", vec!["todo".to_string()]))
            }
            _ => panic!("no patch drawn"),
        }
        asks.send(Ask::Page("/inbox".into())).unwrap();
        assert!(matches!(drawn.recv_timeout(wait), Ok(Drawn::Page(p)) if p == "/inbox 2"));
        assert!(
            matches!(drawn.recv_timeout(wait), Ok(Drawn::Page(p)) if p == "/inbox 3"),
            "every REREAD"
        );
    }

    /// A board page as dash serve draws it, a field and a fold above it: `todo` and `doing` the cards of those two columns.
    fn a_board(todo: &str, doing: &str) -> String {
        format!(
            r#"<!doctype html><html><body><style>a {{ color: red }}</style><input id="find" type="search"><details id="d" open><summary>s</summary>x</details><div id="board"><section id="backlog"><a id="task-b">B</a></section><section id="todo">{todo}</section><section id="in-progress">{doing}</section></div></body></html>"#
        )
    }

    const CARD: &str = r#"<a id="task-a">A</a>"#;

    /// Done-when 1 and 2 of rust-view-patches, the window's part: a board diff's patch puts in the columns it names, and a field
    /// with its focus, an open details and the column it left alone are the same nodes after it.
    #[dash_test]
    fn a_patch_puts_in_its_columns_and_keeps_every_other_node() {
        let (page, card) = (a_board, CARD);
        let mut doc = document(VirtualDom::new_with_props(
            Page,
            PageProps {
                html: page(card, ""),
            },
        ));
        let mut doc = doc.inner_mut();
        let root = doc.root_element().id;
        let at = |doc: &blitz_dom::BaseDocument, id| holding(doc, root, id).expect(id);
        let (find, details, backlog) = (at(&doc, "find"), at(&doc, "d"), at(&doc, "backlog"));
        doc.set_focus_to(find);
        patch(
            &mut doc,
            root,
            &page("", card),
            &["todo".into(), "in-progress".into()],
        );
        let parent =
            |doc: &blitz_dom::BaseDocument, id| doc.get_node(at(doc, id)).and_then(|n| n.parent);
        assert_eq!(parent(&doc, "task-a"), Some(at(&doc, "in-progress")));
        assert_eq!(
            doc.get_node(at(&doc, "todo")).map(|n| n.children.len()),
            Some(0)
        );
        assert_eq!(
            (at(&doc, "find"), at(&doc, "d"), at(&doc, "backlog")),
            (find, details, backlog)
        );
        assert_eq!(doc.get_focussed_node_id(), Some(find));
        assert!(
            doc.get_node(details)
                .is_some_and(|n| n.data.has_attr(blitz_dom::local_name!("open")))
        );
        // the newer drawing's scratch is gone: one board in the document
        let boards = doc.query_selector_all("#board").expect("a selector");
        assert_eq!(boards.len(), 1);
        // the newer drawing brings no stylesheet to load again
        assert_eq!(unstyled("a<style>x</style>b<style>y"), "ab");
    }

    /// The window's component, no display: a page drawn whole, then a patch of two columns, each a poll the shell would repaint
    /// on (`poll` true), the patch in the document under the mounted root. A patch alone changes no signal: without the render
    /// it asks, the shell would repaint on his next mouse move (/code-review of PR 1322).
    #[dash_test]
    fn a_patch_in_the_window_is_painted_at_once() {
        let (drawn, renders) = futures_channel::mpsc::unbounded::<Drawn>();
        let vdom = VirtualDom::new(Pages);
        vdom.provide_root_context(Arc::new(std::sync::Mutex::new(Some(renders))) as Renders);
        let mut doc = document(vdom);
        let settle = |doc: &mut DioxusDocument| {
            for _ in 0..8 {
                if !doc.poll(None) {
                    break;
                }
            }
        };
        settle(&mut doc);
        let root = doc.inner().root_element().id;
        let column_of = |doc: &DioxusDocument, card: &str| {
            let doc = doc.inner();
            let node = holding(&doc, root, card).expect(card);
            let parent = doc.get_node(node).and_then(|n| n.parent).expect("a column");
            doc.get_node(parent)
                .and_then(|n| n.attr(blitz_dom::local_name!("id")))
                .map(str::to_string)
        };
        drawn
            .unbounded_send(Drawn::Page(a_board(CARD, "")))
            .unwrap();
        assert!(
            doc.poll(None),
            "a page drawn whole is work the shell repaints on"
        );
        settle(&mut doc);
        assert_eq!(column_of(&doc, "task-a").as_deref(), Some("todo"));
        let columns = vec!["todo".to_string(), "in-progress".to_string()];
        drawn
            .unbounded_send(Drawn::Patch(a_board("", CARD), columns))
            .unwrap();
        assert!(doc.poll(None), "a patch is work the shell repaints on");
        assert_eq!(column_of(&doc, "task-a").as_deref(), Some("in-progress"));
        assert!(!doc.poll(None), "then nothing left to render");
    }

    #[dash_test]
    fn a_page_s_html_is_what_its_window_shows() {
        let html = "<!doctype html><html><head><style>p{color:red}</style></head><body><nav><a href=\"/tasks\">Tasks</a></nav><p>a-task</p></body></html>".to_string();
        let dom = VirtualDom::new_with_props(Page, PageProps { html });
        assert_eq!(shown(dom), "Tasks\na-task\n");
    }
}
