//! The live compass (task rust-window-compass; rfc-rust-unified#ui-lib's default, never built until then): the design system's
//! `Compass`, its behaviour Rust handlers, run on the web through dioxus-liveview, which keeps its state here and patches the
//! page over a WebSocket, and natively in the window (`window_compass`). Its Send writes the comment queue in this process.
// the window's own compass and its test twin draw compass_hosted; the web's liveview runs the design system's live_root
#[cfg(any(feature = "window", test))]
use dash_surface::components::compass_hosted;
use dash_surface::components::{CompassHost, Look, Pick, live_root};
use dioxus::prelude::*;
use std::rc::Rc;

/// dioxus-liveview's own client, the interpreter that applies its patches, opening `/live` on the page's host: Dioxus's, never
/// a script of ours. Served at /live.js so a page carries one tag, cached by the browser.
pub fn client() -> &'static str {
    static CLIENT: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
        let glue = dioxus_liveview::interpreter_glue(dash_surface::forms::LIVE);
        // the script element's contents: past its opening tag's end, before the closing tag's start
        let start = glue.find('>').map_or(0, |at| at + 1);
        let end = glue.rfind('<').unwrap_or(glue.len());
        // it mounts in the menu's own root, never liveview's `main`, an id a page of either app may hold (/code-review); on a
        // page with an island (components::Island), in its first one instead, its name sent on the socket so `run` draws it.
        // Started once the page is read: this script is in the menu, before the page's own parts.
        let root = format!(
            "querySelector(\"[data-live]\")||document.getElementById(\"{}\")",
            dash_surface::forms::LIVE_ROOT
        );
        let socket =
            r#"new WebSocket(WS_ADDR+"?island="+encodeURIComponent(root.dataset.live||""))"#;
        let glue = glue[start..end]
            .replace(r#"getElementById("main")"#, &root)
            .replace("new WebSocket(WS_ADDR)", socket);
        let started = glue
            .rfind("main();")
            .expect("liveview's glue starts with main()");
        format!(
            "{}document.readyState===\"loading\"?addEventListener(\"DOMContentLoaded\",main):main();{}",
            &glue[..started],
            &glue[started + "main();".len()..]
        )
    });
    &CLIENT
}

/// A comment of the compass, one row of the queue (`comments::post`), as `POST /compass` writes it.
fn post(page: &str, quote: &str, text: &str) -> Result<String, String> {
    let dir = crate::pages::serve_from_dir()?;
    crate::comments::post(dir, page, quote, text, &[], crate::tasks::now_ms())
}

/// What stays a script, as rfc-rust-unified#ui-lib allows: the browser's own layout, which has no Rust route from the server.
/// `elementFromPoint` finds what is under the tap (the pick layer let through for that one call), `innerText` its words as
/// shown; the mark is an attribute the menu's look outlines. Every decision on what it returns (the quote) is Rust's.
/// Blocks its port: a Dioxus element offers its rect, scroll and focus, no hit test at a point and no shown text
/// (https://docs.rs/dioxus-html/latest/dioxus_html/events/struct.MountedData.html).
const UNDER_JS: &str = r#"const layer=document.getElementById('dash-pick-layer');
document.querySelectorAll('[data-dash-pick]').forEach(n=>n.removeAttribute('data-dash-pick'));
const page=location.pathname,at=AT;if(!at||!layer)return{page,chain:[]};
layer.style.pointerEvents='none';let e=document.elementFromPoint(at[0],at[1]);layer.style.pointerEvents='';
if(MARK){if(e&&e!==document.body&&e!==document.documentElement)e.setAttribute('data-dash-pick','');return{page,chain:[]}}
const chain=[];for(;e&&e!==document.body&&e!==document.documentElement;e=e.parentElement)chain.push({tag:e.localName,id:e.id,
role:e.getAttribute('role')||'',text:(e.localName==='select'?e.selectedOptions[0]?.text:e.localName==='textarea'?e.value:e.innerText)||'',
named:e.getAttribute('aria-label')||e.getAttribute('title')||e.getAttribute('alt')||e.getAttribute('placeholder')
||(/^(text|search|email|url|tel|number)$/.test(e.type)&&e.value)||''});return{page,chain}"#;

/// The script asking the browser for `look`.
fn under_js(look: Look) -> String {
    let (at, mark) = match look {
        Look::Mark(x, y) => (format!("[{x},{y}]"), true),
        Look::Take(x, y) => (format!("[{x},{y}]"), false),
        Look::Whole | Look::Unmark => ("null".to_string(), false),
    };
    UNDER_JS
        .replace("AT", &at)
        .replace("MARK", &mark.to_string())
}

/// The web page's compass host: its looks asked of the browser through liveview's eval, its Send written here, then the page
/// reloaded so its rail lists the comment, as the script did.
fn web_host() -> CompassHost {
    CompassHost {
        look: Rc::new(|look| {
            let asked = dioxus_document::eval(&under_js(look));
            Box::pin(async move { asked.join::<Pick>().await.unwrap_or_default() })
        }),
        send: Rc::new(|page, quote, text| {
            let id = post(page, quote, text)?;
            dioxus_document::eval("location.reload()");
            Ok(id)
        }),
    }
}

/// The window's compass, drawn beside its page (dash_ui's `Beside`): its looks hit-test the window's own document, its Send
/// written here. The window then shows the comment in the rail on its next render of the page (`dash_ui::REREAD`).
/// ponytail: a page that follows a view (the board) is drawn again only on its view's diff, so its rail shows the comment
/// then; a comment written to the rail's view would patch it at once.
#[cfg(any(feature = "window", test))]
pub fn window_compass() -> Element {
    let root = use_context::<dash_ui::PageRoot>().0;
    compass_hosted(CompassHost {
        look: Rc::new(move |look| {
            let (at, mark) = match look {
                Look::Mark(x, y) => (Some((x as f32, y as f32)), true),
                Look::Take(x, y) => (Some((x as f32, y as f32)), false),
                Look::Whole | Look::Unmark => (None, false),
            };
            let found = root.peek().as_ref().map(|r| dash_ui::under(r, at, mark));
            let pick = found.map_or_else(Pick::default, |(page, chain)| Pick {
                page,
                chain: (chain.into_iter())
                    .map(|u| dash_surface::components::Node {
                        tag: u.tag,
                        id: u.id,
                        role: u.role,
                        text: u.text,
                        named: u.named,
                    })
                    .collect(),
            });
            Box::pin(std::future::ready(pick))
        }),
        send: Rc::new(post),
    })
}

/// Live sockets open at once, at most: each holds a connection slot of serve's 64 while its page lives, so half of them stay
/// for every other request (/code-review; rust/CLAUDE.md 5, one slow request never stalls the others).
const LIVE_MOST: usize = 32;
static LIVE_OPEN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
/// A page's client pings every 30 s: a socket silent this long is a page gone without closing (a phone asleep on the tailnet).
const SILENT: std::time::Duration = std::time::Duration::from_secs(90);

/// A socket's handshake as asked: from a page of this app (its Origin is the Host it came to, as `POST /compass` requires, so no
/// other site a browser shows can write his comment queue), in WebSocket's version 13, with its key, under `LIVE_MOST` sockets.
/// Its key, or the status and why it is refused.
pub fn asked(
    host: &str,
    origin: Option<&str>,
    version: Option<&str>,
    key: Option<&str>,
) -> Result<String, (&'static str, String)> {
    if origin != Some(&format!("http://{host}")) {
        return Err((
            "403 Forbidden",
            "the live compass is a page of this app's".into(),
        ));
    }
    if version != Some("13") {
        return Err(("426 Upgrade Required", "WebSocket version 13".into()));
    }
    let Some(key) = key.filter(|k| !k.trim().is_empty()) else {
        return Err(("400 Bad Request", "no Sec-WebSocket-Key".into()));
    };
    if LIVE_OPEN.load(std::sync::atomic::Ordering::Relaxed) >= LIVE_MOST {
        return Err((
            "503 Service Unavailable",
            format!("{LIVE_MOST} live compasses open already"),
        ));
    }
    Ok(key.trim().to_string())
}

/// `GET /live` upgraded (`asked` passed): the compass of the page that opened it, on this connection's thread, until the page
/// closes or goes silent `SILENT`, and the `island` its page named. `rest` is what the head's reader took past the head: a frame
/// sent right behind it.
pub fn serve(stream: &std::net::TcpStream, key: &str, rest: &[u8], island: String) {
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};
    struct Open;
    impl Drop for Open {
        fn drop(&mut self) {
            LIVE_OPEN.fetch_sub(1, Ordering::Relaxed);
        }
    }
    LIVE_OPEN.fetch_add(1, Ordering::Relaxed);
    let _open = Open;
    let accept = tokio_tungstenite::tungstenite::handshake::derive_accept_key(key.as_bytes());
    let answered = write!(
        &*stream,
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n"
    );
    let owned = stream.try_clone();
    let (Ok(()), Ok(owned)) = (answered, owned) else {
        return;
    };
    let rest = rest.to_vec();
    let ran = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
        .map(|rt| {
            rt.block_on(async move {
                let socket = owned
                    .set_nonblocking(true)
                    .and_then(|()| tokio::net::TcpStream::from_std(owned));
                let Ok(socket) = socket else { return };
                let ws = tokio_tungstenite::WebSocketStream::from_partially_read(
                    socket,
                    rest,
                    tokio_tungstenite::tungstenite::protocol::Role::Server,
                    None,
                )
                .await;
                let began = std::time::Instant::now();
                let seen = std::sync::Arc::new(AtomicU64::new(0));
                let heard = std::sync::Arc::clone(&seen);
                let ws = futures_util::StreamExt::inspect(ws, move |_| {
                    heard.store(began.elapsed().as_secs(), Ordering::Relaxed)
                });
                let silent = async {
                    loop {
                        tokio::time::sleep(SILENT / 3).await;
                        let quiet = began.elapsed().as_secs() - seen.load(Ordering::Relaxed);
                        if quiet >= SILENT.as_secs() {
                            return;
                        }
                    }
                };
                tokio::select! {
                    _ = run(ws, island) => {}
                    () = silent => {}
                }
            })
        });
    if let Err(e) = ran {
        eprintln!("dash serve: the live compass has no runtime: {e}");
    }
}

/// The island a socket was opened for, named in its request line (`GET /live?island=<name> HTTP/1.1`): "" for none.
pub fn island_of(line: &str) -> String {
    let target = line.split_whitespace().nth(1).unwrap_or_default();
    let query = target.split_once('?').map_or("", |q| q.1);
    let named = query.split('&').find_map(|kv| kv.strip_prefix("island="));
    // as the client sends it, through encodeURIComponent (/code-review)
    crate::pages::segment_decoded(named.unwrap_or_default())
}

/// /documents' list for its island, which the design system's live root asks for: the docs folder is this crate's to read.
fn documents() -> Vec<dash_surface::components::Listed> {
    crate::doc_index::listed(&crate::pages::repo())
}

/// The compass's liveview over `ws`, with the page's `island`, until it closes: a test drives it with a socket of its own.
pub async fn run<S>(ws: S, island: String) -> Result<(), dioxus_liveview::LiveViewError>
where
    S: futures_util::Stream<
            Item = Result<
                tokio_tungstenite::tungstenite::Message,
                tokio_tungstenite::tungstenite::Error,
            >,
        > + futures_util::Sink<tokio_tungstenite::tungstenite::Message>
        + Send
        + 'static,
{
    use dioxus_liveview::LiveViewError;
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;
    // liveview's events arrive serialized: its converter reads them (LiveViewPool::new sets it, a pool we do not run)
    static CONVERTER: std::sync::Once = std::sync::Once::new();
    CONVERTER.call_once(|| {
        dioxus::html::set_event_converter(Box::new(dioxus::html::SerializedHtmlEventConverter))
    });
    let socket = ws
        .sink_map_err(|_| LiveViewError::SendingFailed)
        .map(|m| match m {
            Ok(Message::Text(t)) => Ok(t.as_bytes().to_vec()),
            Ok(Message::Binary(b)) => Ok(b.to_vec()),
            // a ping is answered by tungstenite itself; a close ends the stream next
            Ok(_) => Ok(vec![]),
            Err(_) => Err(LiveViewError::SendingFailed),
        })
        .with(
            |bytes: Vec<u8>| async move { Ok::<_, LiveViewError>(Message::Binary(bytes.into())) },
        );
    dioxus_liveview::run(
        VirtualDom::new_with_props(live_root, (web_host(), island, documents)),
        socket,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use blitz_dom::Document;
    use dash_meter::dash_test;
    use dash_surface::components::Node;
    use futures_util::StreamExt;
    use std::pin::Pin;
    use std::task::{Context, Poll};
    use tokio_tungstenite::tungstenite::{Error, Message};

    /// A socket in memory: what the test sends as the page, what the liveview sends back.
    struct Duplex {
        from_page: futures_channel::mpsc::UnboundedReceiver<Result<Message, Error>>,
        to_page: std::sync::mpsc::Sender<Message>,
    }

    impl futures_util::Stream for Duplex {
        type Item = Result<Message, Error>;
        fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
            self.from_page.poll_next_unpin(cx)
        }
    }

    impl futures_util::Sink<Message> for Duplex {
        type Error = Error;
        fn poll_ready(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Error>> {
            Poll::Ready(Ok(()))
        }
        fn start_send(self: Pin<&mut Self>, item: Message) -> Result<(), Error> {
            self.to_page.send(item).map_err(|_| Error::ConnectionClosed)
        }
        fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Error>> {
            Poll::Ready(Ok(()))
        }
        fn poll_close(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Error>> {
            Poll::Ready(Ok(()))
        }
    }

    /// What the browser answers for a tap on the page's title.
    fn tapped() -> Pick {
        let h1 = Node {
            tag: "h1".into(),
            id: "t".into(),
            text: "Rust tasks".into(),
            ..Node::default()
        };
        Pick {
            page: "/tasks".into(),
            chain: vec![h1],
        }
    }

    /// The web page's compass with a host answering at once and sending nothing: a document of it names the element ids the
    /// liveview gives the same elements, its listeners' (`data-dioxus-id`), each event applied to both.
    fn twin_host() -> CompassHost {
        CompassHost {
            look: Rc::new(|look| {
                let pick = matches!(look, Look::Take(..))
                    .then(tapped)
                    .unwrap_or_default();
                Box::pin(std::future::ready(pick))
            }),
            send: Rc::new(|_, _, _| Ok("twin".into())),
        }
    }

    /// The liveview's next frame: its patches (binary, first byte 1) or a script it asks the page to run (`query`).
    fn next(frames: &std::sync::mpsc::Receiver<Message>) -> Vec<u8> {
        match frames.recv_timeout(std::time::Duration::from_secs(10)) {
            Ok(Message::Binary(b)) => b.to_vec(),
            other => panic!("no frame from the liveview: {other:?}"),
        }
    }

    /// The next script the liveview asks the page to run holding `about`, and its id; its patches and other scripts (liveview's
    /// own history, asked on start) skipped.
    fn asked(frames: &std::sync::mpsc::Receiver<Message>, about: &str) -> (usize, String) {
        loop {
            let frame = next(frames);
            if frame.first() == Some(&0) {
                let query: serde_json::Value =
                    serde_json::from_slice(&frame[1..]).expect("a query");
                let text = query["data"].as_str().expect("its script").to_string();
                let at = text.find("let _request_id = ").expect("a query's id")
                    + "let _request_id = ".len();
                let id = text[at..]
                    .split(|c: char| !c.is_ascii_digit())
                    .next()
                    .expect("digits");
                if text.contains(about) {
                    return (id.parse().expect("an id"), text);
                }
            }
        }
    }

    /// Done-when 2 of rust-window-compass: the web page's compass is the design system's Compass run by dioxus-liveview, in
    /// this process: the page's events go in as the liveview's client sends them (a switch, a tap on the pick layer, the words,
    /// Send), its look is the script it asks the page to run, answered as the browser would, and Send writes the comment queue
    /// here, then asks the page to reload so its rail lists it.
    #[dash_test]
    fn compass_liveview_sends_a_comment() {
        let dir = dash_meter::TestDir::new("live-compass");
        crate::pages::serve_from(&dir.0);
        let mut twin = dash_ui::document(VirtualDom::new_with_props(compass_hosted, twin_host()));
        let (page, from_page) = futures_channel::mpsc::unbounded();
        let (to_page, frames) = std::sync::mpsc::channel();
        let socket = Duplex { from_page, to_page };
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .build()
                .expect("a runtime");
            rt.block_on(run(socket, String::new()))
        });
        assert_eq!(
            next(&frames).first(),
            Some(&1),
            "its first patches draw the compass"
        );
        let id = |twin: &dash_ui::DioxusDocument, at: &str| -> usize {
            let doc = twin.inner();
            let node = doc
                .query_selector(at)
                .expect("a selector")
                .unwrap_or_else(|| panic!("{at}"));
            let node = doc.get_node(node).expect("a node");
            node.attr(blitz_dom::LocalName::from("data-dioxus-id"))
                .expect("a listener's id")
                .parse()
                .expect("a number")
        };
        // an event as liveview's client posts it, applied to the twin as the liveview applies it
        let send = |twin: &mut dash_ui::DioxusDocument,
                    name: &str,
                    at: &str,
                    data: serde_json::Value| {
            let event = serde_json::json!({"name": name, "element": id(twin, at), "bubbles": true, "data": data});
            let message = serde_json::json!({"method": "user_event", "params": event});
            page.unbounded_send(Ok(Message::Text(message.to_string().into())))
                .expect("the liveview");
            let parsed: dioxus::html::HtmlEvent = serde_json::from_value(event).expect("an event");
            let any = dioxus::prelude::Event::new(parsed.data.into_any(), parsed.bubbles);
            twin.vdom
                .runtime()
                .handle_event(&parsed.name, any, parsed.element);
            while twin.poll(None) {}
        };
        let mouse = |x: f64, y: f64| {
            serde_json::json!({"alt_key": false, "button": 0, "buttons": 0, "client_x": x, "client_y": y, "ctrl_key": false,
                "meta_key": false, "offset_x": x, "offset_y": y, "page_x": x, "page_y": y, "screen_x": x, "screen_y": y,
                "shift_key": false})
        };
        // the menu's compass button is the switch's label: the browser sends the switch's input
        send(
            &mut twin,
            "input",
            "#dash-compass-on",
            serde_json::json!({"value": "true", "values": [], "valid": true}),
        );
        send(&mut twin, "click", "#dash-pick-layer", mouse(300.0, 40.0));
        let (query, script) = asked(&frames, "elementFromPoint");
        assert!(
            script.contains("document.elementFromPoint(at[0],at[1])")
                && script.contains("at=[300,40]"),
            "its look asks the browser what is under the tap: {script}"
        );
        let answer = serde_json::json!({"method": "query",
            "params": {"id": query, "data": {"page": "/tasks", "chain": [{"tag": "h1", "id": "t", "text": "Rust tasks"}]},
            "returned_value": true}});
        page.unbounded_send(Ok(Message::Text(answer.to_string().into())))
            .expect("the liveview");
        assert_eq!(
            next(&frames).first(),
            Some(&1),
            "the box opens, quoting the tap"
        );
        let words = serde_json::json!({"value": "Too dense", "values": [], "valid": true});
        send(&mut twin, "input", "#dash-compose textarea", words);
        send(&mut twin, "click", "#dash-compose button", mouse(0.0, 0.0));
        let (_, reload) = asked(&frames, "location.reload()");
        assert!(
            reload.contains("location.reload()"),
            "then the page reloads: {reload}"
        );
        let listed = crate::comments::compass(&dir.0, "/tasks").expect("the queue");
        let notes: serde_json::Value = serde_json::from_str(&listed).expect("json");
        assert_eq!(notes["notes"][0]["quote"], "Rust tasks", "{listed}");
        assert_eq!(notes["notes"][0]["text"], "Too dense", "{listed}");
        assert_eq!(
            notes["notes"].as_array().map(Vec::len),
            Some(1),
            "one row: {listed}"
        );
    }

    /// Task compass-selection-anchor (@nha, 2026-11-12: « something broke the comment selection mechanism (compass). so this
    /// means all my feedback is apparently on "the whole page" because the selection is broken »): liveview sends a render's
    /// patches before the script a spawned look asks, so a tap that dropped the pick layer at once had the browser look with
    /// no layer, and UNDER_JS answered the whole page. The layer stays until the browser has answered, then the box quotes it.
    #[dash_test]
    fn the_pick_layer_stays_until_the_browser_says_what_is_under_the_tap() {
        let (answer, answered) = futures_channel::oneshot::channel::<Pick>();
        let answered = std::cell::RefCell::new(Some(answered));
        let host = CompassHost {
            look: Rc::new(move |look| {
                let pending = matches!(look, Look::Take(..))
                    .then(|| answered.borrow_mut().take())
                    .flatten();
                Box::pin(async move {
                    match pending {
                        Some(p) => p.await.unwrap_or_default(),
                        None => Pick::default(),
                    }
                })
            }),
            send: Rc::new(|_, _, _| Ok("twin".into())),
        };
        let mut twin = dash_ui::document(VirtualDom::new_with_props(compass_hosted, host));
        // its events come as liveview's client sends them, read as the liveview reads them (the document set native's)
        dioxus::html::set_event_converter(Box::new(dioxus::html::SerializedHtmlEventConverter));
        let fire = |twin: &mut dash_ui::DioxusDocument,
                    name: &str,
                    at: &str,
                    data: serde_json::Value| {
            let doc = twin.inner();
            let node = doc
                .query_selector(at)
                .expect("a selector")
                .unwrap_or_else(|| panic!("{at}"));
            let element: usize = (doc.get_node(node).expect("a node"))
                .attr(blitz_dom::LocalName::from("data-dioxus-id"))
                .expect("a listener's id")
                .parse()
                .expect("a number");
            drop(doc);
            let event = serde_json::json!({"name": name, "element": element, "bubbles": true, "data": data});
            let parsed: dioxus::html::HtmlEvent = serde_json::from_value(event).expect("an event");
            let any = dioxus::prelude::Event::new(parsed.data.into_any(), parsed.bubbles);
            twin.vdom
                .runtime()
                .handle_event(&parsed.name, any, parsed.element);
            while twin.poll(None) {}
        };
        let has = |twin: &dash_ui::DioxusDocument, at: &str| {
            twin.inner()
                .query_selector(at)
                .expect("a selector")
                .is_some()
        };
        fire(
            &mut twin,
            "input",
            "#dash-compass-on",
            serde_json::json!({"value": "true", "values": [], "valid": true}),
        );
        let tap = serde_json::json!({"alt_key": false, "button": 0, "buttons": 0, "client_x": 300.0, "client_y": 40.0,
            "ctrl_key": false, "meta_key": false, "offset_x": 300.0, "offset_y": 40.0, "page_x": 300.0, "page_y": 40.0,
            "screen_x": 300.0, "screen_y": 40.0, "shift_key": false});
        fire(&mut twin, "click", "#dash-pick-layer", tap);
        assert!(
            has(&twin, "#dash-pick-layer"),
            "the layer is on the page while the browser is asked what is under the tap"
        );
        answer.send(tapped()).expect("the look waits");
        while twin.poll(None) {}
        assert!(
            !has(&twin, "#dash-pick-layer"),
            "the pick ends once answered"
        );
        let doc = twin.inner();
        let quote = (doc
            .query_selector("#dash-compose blockquote")
            .expect("a selector"))
        .and_then(|q| doc.get_node(q))
        .map(|q| q.text_content());
        assert_eq!(
            quote.as_deref(),
            Some("Rust tasks"),
            "the box quotes the tap, never the whole page"
        );
    }

    /// The blocks of a workspace's document in the order shown, each its id and depth (`--depth:<n>`).
    fn shown_blocks(doc: &dash_ui::DioxusDocument, list: &str) -> Vec<(String, String)> {
        let doc = doc.inner();
        let blocks = (doc.query_selector_all(&format!("#{list} > .block"))).expect("a selector");
        (blocks.iter())
            .filter_map(|&b| doc.get_node(b))
            .map(|n| {
                let attr = |a: &str| n.attr(blitz_dom::LocalName::from(a)).unwrap_or_default();
                let id = attr("id")
                    .rsplit('-')
                    .next()
                    .unwrap_or_default()
                    .to_string();
                (id, attr("style").replace("--depth:", ""))
            })
            .collect()
    }

    fn style_of(doc: &dash_ui::DioxusDocument, at: &str) -> String {
        let doc = doc.inner();
        let node = doc
            .query_selector(at)
            .expect("a selector")
            .unwrap_or_else(|| panic!("{at}"));
        let node = doc.get_node(node).expect("a node");
        node.attr(blitz_dom::LocalName::from("style"))
            .unwrap_or_default()
            .to_string()
    }

    fn shape(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect()
    }

    /// The client a page loads mounts in its first island when it has one, names it on the socket, and starts once the page
    /// is read (it loads in the menu, before the page's parts); `island_of` reads the name back off the socket's request.
    #[dash_test]
    fn the_client_opens_the_page_s_island() {
        let client = client();
        for patched in [
            r#"document.querySelector("[data-live]")||document.getElementById("dash-live")"#,
            r#"new WebSocket(WS_ADDR+"?island="+encodeURIComponent(root.dataset.live||""))"#,
            r#"document.readyState==="loading"?addEventListener("DOMContentLoaded",main):main();"#,
        ] {
            assert!(client.contains(patched), "{patched}");
        }
        assert!(
            !client.contains(r#"getElementById("main")"#)
                && !client.contains("new WebSocket(WS_ADDR)")
        );
        assert_eq!(
            island_of("GET /live?island=doc-workspace HTTP/1.1"),
            "doc-workspace"
        );
        assert_eq!(island_of("GET /live?island= HTTP/1.1"), "");
        assert_eq!(island_of("GET /live HTTP/1.1"), "");
        assert_eq!(
            island_of("GET /live?island=doc%20tree HTTP/1.1"),
            "doc tree"
        );
    }

    /// Done-when 3 and 4 of design-doc-tree-menu, on the web: /design's doc workspace is an island the page's liveview runs, so
    /// Chrome's mouse events reach its Rust handlers. Driven in this process as liveview's client posts them (a press on a
    /// block's handle, the pointer entering another block's left half, the release; a press on a splitter, a move, the
    /// release), each answered by patches, the twin showing the order and the width they draw.
    #[dash_test]
    fn workspace_island_drags_blocks_and_sizes_columns_through_liveview() {
        let island = dash_surface::components::WORKSPACE_ISLAND.to_string();
        let mut twin = dash_ui::document(VirtualDom::new_with_props(
            live_root,
            (twin_host(), island.clone(), documents),
        ));
        let (page, from_page) = futures_channel::mpsc::unbounded();
        let (to_page, frames) = std::sync::mpsc::channel();
        let socket = Duplex { from_page, to_page };
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .build()
                .expect("a runtime");
            rt.block_on(run(socket, island))
        });
        assert_eq!(
            next(&frames).first(),
            Some(&1),
            "its first patches draw the island"
        );
        let mut send = |name: &str, at: &str, x: f64| {
            let doc = twin.inner();
            let node = doc
                .query_selector(at)
                .expect("a selector")
                .unwrap_or_else(|| panic!("{at}"));
            let node = doc.get_node(node).expect("a node");
            let element: usize = (node.attr(blitz_dom::LocalName::from("data-dioxus-id")))
                .expect("a listener's id")
                .parse()
                .expect("a number");
            drop(doc);
            let data = serde_json::json!({"alt_key": false, "button": 0, "buttons": 1, "client_x": x, "client_y": 10.0,
                "ctrl_key": false, "meta_key": false, "offset_x": x, "offset_y": 10.0, "page_x": x, "page_y": 10.0,
                "screen_x": x, "screen_y": 10.0, "shift_key": false});
            let event = serde_json::json!({"name": name, "element": element, "bubbles": name != "mouseenter", "data": data});
            let message = serde_json::json!({"method": "user_event", "params": event});
            page.unbounded_send(Ok(Message::Text(message.to_string().into())))
                .expect("the liveview");
            let parsed: dioxus::html::HtmlEvent = serde_json::from_value(event).expect("an event");
            let any = dioxus::prelude::Event::new(parsed.data.into_any(), parsed.bubbles);
            twin.vdom
                .runtime()
                .handle_event(&parsed.name, any, parsed.element);
            while twin.poll(None) {}
            // its patches, past any script the liveview asks the page to run (its history, on start)
            while next(&frames).first() != Some(&1) {}
            shown_blocks(&twin, "ds-live-workspace-blocks")
        };
        let before = shape(&[("1", "0"), ("2", "0"), ("3", "1"), ("4", "0")]);
        assert_eq!(
            send(
                "mousedown",
                "#ds-live-workspace-blocks-4 .block-handle",
                0.0
            ),
            before
        );
        send(
            "mouseenter",
            r#"#ds-live-workspace-blocks-1 [data-drop="before"]"#,
            0.0,
        );
        let moved = send(
            "mouseup",
            r#"#ds-live-workspace-blocks-1 [data-drop="before"]"#,
            0.0,
        );
        assert_eq!(
            moved,
            shape(&[("4", "0"), ("1", "0"), ("2", "0"), ("3", "1")]),
            "reordered"
        );
        send(
            "mousedown",
            "#ds-live-workspace-blocks-2 .block-handle",
            0.0,
        );
        send(
            "mouseenter",
            r#"#ds-live-workspace-blocks-1 [data-drop="into"]"#,
            0.0,
        );
        let nested = send(
            "mouseup",
            r#"#ds-live-workspace-blocks-1 [data-drop="into"]"#,
            0.0,
        );
        assert_eq!(
            nested,
            shape(&[("4", "0"), ("1", "0"), ("2", "1"), ("3", "2")]),
            "nested, its own nested one carried"
        );
        send("mousedown", "#ds-live-workspace-split > .splitter", 100.0);
        send("mousemove", "#ds-live-workspace-split > .split-drag", 160.0);
        send("mouseup", "#ds-live-workspace-split > .split-drag", 160.0);
        let style = style_of(&twin, "#ds-live-workspace-split");
        assert!(
            style.contains("--split:230px 6px 220px 6px "),
            "the menu 60px wider: {style}"
        );
    }

    /// Done-when 3 and 4 of design-doc-tree-menu, in the window: the same workspace drawn as the window draws it, its looks
    /// loaded and laid out, driven by pointer events as the shell sends them (Blitz has no HTML5 drag): a block's handle
    /// pressed, the pointer moved over another block's left half and released there reorders; over its right half, nests;
    /// a splitter pressed and dragged 60px widens its column 60px.
    #[dash_test]
    fn window_workspace_drags_blocks_and_sizes_columns() {
        use blitz_dom::Document;
        use blitz_traits::events::{
            BlitzPointerEvent, BlitzPointerId, MouseEventButton, MouseEventButtons, PointerCoords,
            UiEvent,
        };
        use blitz_traits::shell::{ColorScheme, Viewport};
        fn app() -> Element {
            let css = dash_surface::component::css();
            rsx! {
                style { "{css}" }
                dash_surface::components::DocWorkspace { id: "w", layout: dash_surface::components::DocLayout::Beside }
            }
        }
        let mut doc = dash_ui::document(VirtualDom::new(app));
        doc.inner_mut()
            .set_viewport(Viewport::new(1280, 860, 1.0, ColorScheme::Light));
        let settle = |doc: &mut dash_ui::DioxusDocument| {
            for _ in 0..16 {
                if !doc.poll(None) {
                    break;
                }
            }
            doc.inner_mut().resolve(0.0);
        };
        settle(&mut doc);
        // where `at` is, at `x` of its width (0 its left edge, 1 its right), halfway down
        let point = |doc: &dash_ui::DioxusDocument, at: &str, x: f32| {
            let inner = doc.inner();
            let node = inner
                .query_selector(at)
                .expect("a selector")
                .unwrap_or_else(|| panic!("{at}"));
            let node = inner.get_node(node).expect("a node");
            let (p, size) = (node.absolute_position(0.0, 0.0), node.final_layout().size);
            (p.x + size.width * x, p.y + size.height / 2.0)
        };
        let pointer = |(x, y): (f32, f32), buttons| BlitzPointerEvent {
            id: BlitzPointerId::Mouse,
            is_primary: true,
            coords: PointerCoords {
                page_x: x,
                page_y: y,
                screen_x: x,
                screen_y: y,
                client_x: x,
                client_y: y,
            },
            button: MouseEventButton::Main,
            buttons,
            mods: Default::default(),
            details: Default::default(),
            element: Default::default(),
            active_pointers: Default::default(),
        };
        let drag = |doc: &mut dash_ui::DioxusDocument, from: (f32, f32), to: (&str, f32, f32)| {
            doc.handle_ui_event(UiEvent::PointerMove(pointer(from, MouseEventButtons::None)));
            doc.handle_ui_event(UiEvent::PointerDown(pointer(
                from,
                MouseEventButtons::Primary,
            )));
            settle(doc);
            let (at, x, dx) = to;
            let (mut x, y) = point(doc, at, x);
            x += dx;
            for step in [0.5, 1.0] {
                let (fx, fy) = from;
                let mid = (fx + (x - fx) * step, fy + (y - fy) * step);
                doc.handle_ui_event(UiEvent::PointerMove(pointer(
                    mid,
                    MouseEventButtons::Primary,
                )));
                settle(doc);
            }
            doc.handle_ui_event(UiEvent::PointerUp(pointer((x, y), MouseEventButtons::None)));
            settle(doc);
        };
        let handle = |doc: &dash_ui::DioxusDocument, n: u8| {
            point(doc, &format!("#w-blocks-{n} .block-handle"), 0.5)
        };
        let from = handle(&doc, 4);
        drag(&mut doc, from, ("#w-blocks-1", 0.25, 0.0));
        assert_eq!(
            shown_blocks(&doc, "w-blocks"),
            shape(&[("4", "0"), ("1", "0"), ("2", "0"), ("3", "1")]),
            "the last block dropped on the first's left half: before it"
        );
        let from = handle(&doc, 2);
        drag(&mut doc, from, ("#w-blocks-1", 0.75, 0.0));
        assert_eq!(
            shown_blocks(&doc, "w-blocks"),
            shape(&[("4", "0"), ("1", "0"), ("2", "1"), ("3", "2")]),
            "dropped on its right half: nested under it, its own nested one carried"
        );
        let bar = point(&doc, "#w-split > .splitter", 0.5);
        drag(&mut doc, bar, ("#w-split > .splitter", 0.5, 60.0));
        let style = style_of(&doc, "#w-split");
        assert!(
            style.contains("--split:230px 6px 220px 6px "),
            "the menu 60px wider: {style}"
        );
    }
}
