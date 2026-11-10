//! Adapter of section 3: the native window (section 9), Dioxus Native as rust/ui-spike picked it (@nha, 2026-11-08: « DIOXUS IS
//! VALIDATED »). It draws whatever component it is given, so the screen stays the surface crate's, the one the web page renders.
//! `open_pages` is the app (task rust-window-in-process): each page the very HTML dash serve answers, rendered in this process.
use blitz_dom::{Document, DocumentConfig};
use dioxus::prelude::*;
use dioxus_native_dom::DioxusDocument;
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
/// ponytail: light whatever the Mac's mode, as the window always was; the web's DARK script runs in a browser only.
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
struct InWindow(std::sync::mpsc::Sender<String>);

#[cfg(feature = "window")]
impl blitz_traits::navigation::NavigationProvider for InWindow {
    fn navigate_to(&self, asked: blitz_traits::navigation::NavigationOptions) {
        let url = asked.url;
        // no page of Rust's has a POST form, and the compass posts from its script, which a window does not run
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
            let _ = self.0.send(target);
        } else if matches!(url.scheme(), "http" | "https" | "mailto") {
            let _ = std::process::Command::new("open").arg(url.as_str()).spawn();
        }
    }
}

/// How often `render` is asked the page shown again, so a write to the store shows within it: a board page reads the views,
/// caught up with the op log on each read, so a write shows within a second with a render's time to spare (rust-window-in-process
/// done-when 2). `render` keeps what reads no store.
/// ponytail: a whole board page rendered four times a second, and swapped in whenever it changed, a field's words, focus and
/// an open details lost with it; the views' diffs as patches end both (task rust-view-patches).
pub const REREAD: std::time::Duration = std::time::Duration::from_millis(250);

/// The window's render thread: `render` of `first`, then of each page `asked` names, the page shown again every `REREAD`;
/// each page that changed goes to `draw`, until it says the window is gone or `asked` closes. No display: a test runs it.
pub fn render_loop(
    first: &str,
    mut render: impl FnMut(&str) -> String,
    asked: std::sync::mpsc::Receiver<String>,
    mut draw: impl FnMut(String) -> bool,
) {
    use std::sync::mpsc::RecvTimeoutError;
    let (mut target, mut last) = (first.to_string(), String::new());
    loop {
        let page = render(&target);
        if page != last {
            if !draw(page.clone()) {
                return;
            }
            last = page;
        }
        match asked.recv_timeout(REREAD) {
            Ok(next) => target = next,
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

/// The renders the window's one component takes, once: the receiver goes to its future.
#[cfg(feature = "window")]
type Renders = Arc<std::sync::Mutex<Option<futures_channel::mpsc::UnboundedReceiver<String>>>>;

#[cfg(feature = "window")]
#[allow(non_snake_case)]
fn Pages() -> Element {
    use futures_util::StreamExt;
    let mut html = use_signal(String::new);
    let renders = use_hook(consume_context::<Renders>);
    use_future(move || {
        let renders = renders.lock().expect("the renders").take();
        async move {
            if let Some(mut renders) = renders {
                while let Some(page) = renders.next().await {
                    html.set(page);
                }
            }
        }
    });
    rsx! { Page { html: html() } }
}

/// The app's window, until it is closed: `render` turns a page's path and query into its HTML, on a thread of its own, first
/// `first`, then each page a click asks, the page shown again every `REREAD` and drawn when it changed. No server under it.
#[cfg(feature = "window")]
pub fn open_pages(first: &str, title: &str, render: impl FnMut(&str) -> String + Send + 'static) {
    use blitz_shell::{BlitzShellProxy, WindowConfig, create_default_event_loop};
    use dioxus_native::{
        DioxusNativeApplication, DioxusNativeWindowRenderer, LogicalSize, WindowAttributes,
    };
    let (asks, asked) = std::sync::mpsc::channel::<String>();
    let (drawn, renders) = futures_channel::mpsc::unbounded::<String>();
    let first = first.to_string();
    std::thread::spawn(move || {
        render_loop(&first, render, asked, |page| {
            drawn.unbounded_send(page).is_ok()
        })
    });
    let event_loop = create_default_event_loop();
    let (proxy, events) = BlitzShellProxy::new(event_loop.create_proxy());
    let vdom = VirtualDom::new(Pages);
    vdom.provide_root_context(Arc::new(std::sync::Mutex::new(Some(renders))) as Renders);
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
    use dash_meter::memory::TestBudget;

    #[test]
    fn a_component_s_words_are_what_its_window_shows() {
        let _budget = TestBudget::new();
        fn app() -> Element {
            rsx! { style { "p {{ color: red }}" } h1 { "Rust tasks" } p { "a-task" } }
        }
        assert_eq!(shown(VirtualDom::new(app)), "Rust tasks\na-task\n");
    }

    /// A page's HTML is parsed into the window's tree, its styles and scripts left out of the words.
    #[test]
    fn the_window_wears_the_page_s_theme() {
        let _budget = TestBudget::new();
        assert_eq!(
            html_class(r#"<!doctype html><html lang="en" class="app light"><head>"#),
            "app light"
        );
        assert_eq!(html_class(r#"<html lang="en"><body class="x">"#), "");
        assert_eq!(html_class("<p>no html</p>"), "");
    }

    #[test]
    fn a_page_s_html_is_what_its_window_shows() {
        let _budget = TestBudget::new();
        let html = "<!doctype html><html><head><style>p{color:red}</style></head><body><nav><a href=\"/tasks\">Tasks</a></nav><p>a-task</p></body></html>".to_string();
        let dom = VirtualDom::new_with_props(Page, PageProps { html });
        assert_eq!(shown(dom), "Tasks\na-task\n");
    }
}
