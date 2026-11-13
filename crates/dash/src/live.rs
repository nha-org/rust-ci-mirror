//! The live compass (task rust-window-compass; rfc-rust-unified#ui-lib's default, never built until then): the design system's
//! `Compass`, its behaviour Rust handlers, run on the web through dioxus-liveview, which keeps its state here and patches the
//! page over a WebSocket, and natively in the window (`window_compass`). Its Send writes the comment queue in this process.
use dash_surface::components::{Clip, CompassHost, Copied, Look, Part, PartHost, Pick, live_root};
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
        // the page's parts (components::placed): their props sent first, then each part the liveview draws moved into its
        // place (task rust-js-port-small). Their events bubble to the document, where liveview's client listens in place of
        // its root, and only an element of its own (`data-dioxus-id` on it or above) is sent: the page's own are not.
        let parts = format!(
            r#"ws.onopen = () => {{ws.send(JSON.stringify([...document.querySelectorAll(".{place}")].map(p=>p.dataset.part)));"#,
            place = dash_surface::components::PLACE
        );
        let graft = format!(
            r#"window.interpreter.run_from_bytes(messageData);const fresh=document.querySelectorAll(".part-home>[{graft}]");if(fresh.length){{const places=document.querySelectorAll(".{place}");fresh.forEach(g=>{{const p=places[g.getAttribute("{graft}")];p&&p.replaceChildren(g)}})}}"#,
            place = dash_surface::components::PLACE,
            graft = dash_surface::components::GRAFT
        );
        let glue = glue[start..end]
            .replace(r#"getElementById("main")"#, &root)
            .replace("new WebSocket(WS_ADDR)", socket)
            .replace("ws.onopen = () => {", &parts)
            .replace("window.interpreter.run_from_bytes(messageData);", &graft)
            .replace(
                "this.root.addEventListener(event_name,this.handler)",
                "document.addEventListener(event_name,this.handler)",
            )
            .replace(
                "this.root.removeEventListener(",
                "document.removeEventListener(",
            )
            .replace(
                "let handler=(event)=>this.handleEvent(event,event.type,event.bubbles);",
                "let handler=(event)=>{if(getTargetId(event.target)!==null)this.handleEvent(event,event.type,event.bubbles)};",
            );
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

/// What stays a script for a copy on the web: the browser's clipboard, which a handler on the server cannot reach. The
/// clipboard API needs a secure page, and his phone opens plain http on the tailnet address: there a selected text's copy
/// command does it (/code-review of PR 1395). On a failure the element the copy names is selected, so he copies it by hand.
/// Every decision on what is copied and what the button then says is the component's, in Rust. It answers what it did.
/// ponytail: it runs a round trip after the tap (the server's handler, then liveview's eval): Chrome and Firefox keep the
/// tap's activation 5 s, which the copy command needs, so it copies; Safari keeps it only inside the tap's own handler, so
/// there the text is selected for him to copy by hand. A tap-time client copy, if Safari matters (/code-review).
/// Blocks its port: a handler on the server reaches no client clipboard, and Dioxus has no clipboard API but the desktop's
/// (https://github.com/DioxusLabs/dioxus/issues/3855).
const CLIPBOARD_JS: &str = r#"const c=CLIP,t=(c.origin?location.origin:"")+c.text;
try{if(navigator.clipboard&&isSecureContext){await navigator.clipboard.writeText(t);return "yes"}
const a=document.createElement("textarea");a.value=t;document.body.append(a);a.select();const ok=document.execCommand("copy");a.remove();if(ok)return "yes"}catch(e){}
const s=c.select&&document.getElementById(c.select);if(!s)return "no";getSelection().selectAllChildren(s);return "selected""#;

/// A form of the page sent as its button sends it (a menu's digit picks its choice): the browser's post, so its answer is the
/// page it loads, as a tap on the choice.
/// Blocks its port: liveview's handlers run on the server, which has no route to post the page's own form
/// (https://docs.rs/dioxus-document/latest/dioxus_document/fn.eval.html is the one way to the page).
const SUBMIT_JS: &str = "document.getElementById(FORM)?.requestSubmit()";

/// The web page's parts host: a copy asked of the browser's clipboard, a form sent by the browser, both through liveview's
/// eval; its client moves the parts into their places itself.
fn web_parts() -> PartHost {
    PartHost {
        copy: Rc::new(|clip: Clip| {
            let clip = serde_json::to_string(&clip).unwrap_or_else(|_| "{}".into());
            let asked = dioxus_document::eval(&CLIPBOARD_JS.replace("CLIP", &clip));
            Box::pin(async move { asked.join::<Copied>().await.unwrap_or(Copied::No) })
        }),
        submit: Rc::new(|form: &str| {
            let form = serde_json::to_string(form).unwrap_or_default();
            dioxus_document::eval(&SUBMIT_JS.replace("FORM", &form));
        }),
        graft: None,
    }
}

/// The Mac's clipboard written with `text`: pbcopy, as `dash ui` runs on the Mac. A test reads what it was given instead.
#[cfg(all(feature = "window", not(test)))]
fn clipboard(text: &str) -> bool {
    use std::io::Write;
    let spawned = std::process::Command::new("pbcopy")
        .stdin(std::process::Stdio::piped())
        .spawn();
    let Ok(mut child) = spawned else {
        return false;
    };
    let wrote = child
        .stdin
        .take()
        .is_some_and(|mut i| i.write_all(text.as_bytes()).is_ok());
    wrote && child.wait().is_ok_and(|s| s.success())
}

#[cfg(test)]
thread_local! {
    static CLIPBOARD: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
}

#[cfg(test)]
fn clipboard(text: &str) -> bool {
    CLIPBOARD.with(|c| *c.borrow_mut() = text.to_string());
    true
}

/// Where a link the window copies opens: the window's own pages have no address, so dash serve's, on the tailnet, as his
/// phone opens it (CLAUDE.md, links never paths).
#[cfg(any(feature = "window", test))]
const SERVED: &str = "http://100.72.158.115:7788";

/// The window's parts host: a copy written to the Mac's clipboard, a link after dash serve's address; a form's send refused,
/// as the window sends only GET; each part, once mounted, moved into its place in the page.
#[cfg(any(feature = "window", test))]
fn window_parts(root: Signal<Option<dash_ui::NodeHandle>>) -> PartHost {
    PartHost {
        copy: Rc::new(|clip: Clip| {
            let text = match clip.origin {
                true => format!("{SERVED}{}", clip.text),
                false => clip.text,
            };
            // the window selects nothing on a failure: it says only that it did not copy
            let copied = if clipboard(&text) {
                Copied::Yes
            } else {
                Copied::No
            };
            Box::pin(std::future::ready(copied))
        }),
        submit: Rc::new(|form: &str| {
            eprintln!("dash ui: {form} asks a POST: the window sends only GET")
        }),
        graft: Some(Rc::new(move |n, e: MountedEvent| {
            let Some(part) = e.data().downcast::<dash_ui::NodeHandle>().cloned() else {
                return;
            };
            // after the event: the document is the handler's while it runs
            spawn(async move {
                if let Some(root) = root.peek().as_ref() {
                    dash_ui::graft(root, &part, dash_surface::components::PLACE, n);
                }
            });
        })),
    }
}

/// The window's compass, drawn beside its page (dash_ui's `Beside`): its looks hit-test the window's own document, its Send
/// written here. The window then shows the comment in the rail on its next render of the page (`dash_ui::REREAD`).
/// ponytail: a page that follows a view (the board) is drawn again only on its view's diff, so its rail shows the comment
/// then; a comment written to the rail's view would patch it at once.
#[cfg(any(feature = "window", test))]
pub fn window_compass() -> Element {
    let root = use_context::<dash_ui::PageRoot>().0;
    let shown = use_context::<dash_ui::Shown>();
    // read once a page given, never on each of the compass's own renders (/code-review)
    let parts = use_memo(move || {
        let _given = (shown.drawn)();
        dash_surface::components::parts_in(&shown.html.peek())
    });
    // the compass, and the parts the page placed, grafted into their places, anew for each page given
    let compass = CompassHost {
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
    };
    dash_surface::components::window_beside(compass, window_parts(root), parts(), (shown.drawn)())
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
                // the client's first message: the parts its page placed (`client`)
                // bounded as a silent socket is: a page that never sends it holds no slot (rust/CLAUDE.md 5)
                let Ok((first, ws)) =
                    tokio::time::timeout(SILENT, futures_util::StreamExt::into_future(ws)).await
                else {
                    return;
                };
                let first = first
                    .and_then(Result::ok)
                    .and_then(|m| m.into_text().ok())
                    .map(|t| t.to_string());
                let parts = dash_surface::components::parts_sent(first.as_deref().unwrap_or("[]"));
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
                    _ = run(ws, island, parts) => {}
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
pub async fn run<S>(
    ws: S,
    island: String,
    parts: Vec<Option<Part>>,
) -> Result<(), dioxus_liveview::LiveViewError>
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
        VirtualDom::new_with_props(
            live_root,
            (web_host(), web_parts(), island, documents, parts),
        ),
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
        let mut twin = dash_ui::document(VirtualDom::new_with_props(
            live_root,
            (twin_host(), twin_parts(), String::new(), documents, vec![]),
        ));
        let (page, from_page) = futures_channel::mpsc::unbounded();
        let (to_page, frames) = std::sync::mpsc::channel();
        let socket = Duplex { from_page, to_page };
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .build()
                .expect("a runtime");
            rt.block_on(run(socket, String::new(), vec![]))
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

    /// The parts' host of a page's twin: a copy always written, a send recorded in `SENT`, nothing grafted.
    fn twin_parts() -> PartHost {
        PartHost {
            copy: Rc::new(|_| Box::pin(std::future::ready(Copied::Yes))),
            submit: Rc::new(|form: &str| SENT.with(|s| s.borrow_mut().push(form.to_string()))),
            graft: None,
        }
    }

    thread_local! {
        static SENT: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(vec![]) };
    }

    /// A page's liveview running `parts` on its own thread, and its twin: the events sent as liveview's client posts them,
    /// applied to the twin as the liveview applies them, its frames read back.
    struct LivePage {
        twin: dash_ui::DioxusDocument,
        page: futures_channel::mpsc::UnboundedSender<Result<Message, Error>>,
        frames: std::sync::mpsc::Receiver<Message>,
    }

    impl LivePage {
        fn new(parts: Vec<Option<Part>>) -> Self {
            let twin = dash_ui::document(VirtualDom::new_with_props(
                live_root,
                (
                    twin_host(),
                    twin_parts(),
                    String::new(),
                    documents,
                    parts.clone(),
                ),
            ));
            let (page, from_page) = futures_channel::mpsc::unbounded();
            let (to_page, frames) = std::sync::mpsc::channel();
            let socket = Duplex { from_page, to_page };
            std::thread::spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .build()
                    .expect("a runtime");
                rt.block_on(run(socket, String::new(), parts))
            });
            assert_eq!(
                next(&frames).first(),
                Some(&1),
                "its first patches draw the parts"
            );
            LivePage { twin, page, frames }
        }

        /// The event `name` on the element at `at`, `data` its contents.
        fn send(&mut self, name: &str, at: &str, data: serde_json::Value) {
            let element: usize = {
                let doc = self.twin.inner();
                let node =
                    (doc.query_selector(at).expect("a selector")).unwrap_or_else(|| panic!("{at}"));
                let node = doc.get_node(node).expect("a node");
                (node.attr(blitz_dom::LocalName::from("data-dioxus-id")))
                    .expect("a listener's id")
                    .parse()
                    .expect("a number")
            };
            let event = serde_json::json!({"name": name, "element": element, "bubbles": true, "data": data});
            let message = serde_json::json!({"method": "user_event", "params": event});
            self.page
                .unbounded_send(Ok(Message::Text(message.to_string().into())))
                .expect("the liveview");
            let parsed: dioxus::html::HtmlEvent = serde_json::from_value(event).expect("an event");
            let any = dioxus::prelude::Event::new(parsed.data.into_any(), parsed.bubbles);
            self.twin
                .vdom
                .runtime()
                .handle_event(&parsed.name, any, parsed.element);
            while self.twin.poll(None) {}
        }

        /// The value of `attr` on the twin's element at `at`.
        fn attr(&self, at: &str, attr: &str) -> Option<String> {
            let doc = self.twin.inner();
            let node =
                (doc.query_selector(at).expect("a selector")).unwrap_or_else(|| panic!("{at}"));
            let node = doc.get_node(node).expect("a node");
            node.attr(blitz_dom::LocalName::from(attr))
                .map(str::to_string)
        }
    }

    fn click() -> serde_json::Value {
        serde_json::json!({"alt_key": false, "button": 0, "buttons": 0, "client_x": 0.0, "client_y": 0.0, "ctrl_key": false,
            "meta_key": false, "offset_x": 0.0, "offset_y": 0.0, "page_x": 0.0, "page_y": 0.0, "screen_x": 0.0,
            "screen_y": 0.0, "shift_key": false})
    }

    fn key(k: &str) -> serde_json::Value {
        serde_json::json!({"char_code": 0, "is_composing": false, "key": k, "key_code": 48, "code": "Digit0", "alt_key": false,
            "ctrl_key": false, "meta_key": false, "shift_key": false, "location": 0, "repeat": false, "which": 48})
    }

    fn copy_part() -> Part {
        Part::Copy(dash_surface::components::CopyPart {
            id: "c".into(),
            label: "Copy link".into(),
            text: "/tasks/a-task".into(),
            origin: true,
            icon: Some(String::new()),
            select: None,
        })
    }

    fn menu_part() -> Part {
        let choices = [
            ("1", "Urgent", false),
            ("2", "High", true),
            ("3", "Medium", false),
        ];
        Part::Menu(dash_surface::components::MenuPart {
            id: "m".into(),
            action: "/tasks/a-task/set".into(),
            field: "priority".into(),
            label: "Set priority".into(),
            current: "High".into(),
            choices: (choices.iter())
                .map(|(v, n, on)| (v.to_string(), n.to_string(), *on))
                .collect(),
            numbered: true,
            ..Default::default()
        })
    }

    /// The window as `dash ui` opens it, its page `html`, laid out at its size: Pages, the compass and the parts beside it.
    fn window(html: String) -> dash_ui::DioxusDocument {
        use blitz_traits::shell::{ColorScheme, Viewport};
        let (drawn, renders) = futures_channel::mpsc::unbounded::<dash_ui::Drawn>();
        let vdom = VirtualDom::new(dash_ui::Pages);
        vdom.provide_root_context(
            std::sync::Arc::new(std::sync::Mutex::new(Some(renders))) as dash_ui::Renders
        );
        vdom.provide_root_context(dash_ui::Beside(window_compass));
        let mut doc = dash_ui::document(vdom);
        doc.inner_mut()
            .set_viewport(Viewport::new(1280, 860, 1.0, ColorScheme::Light));
        drawn
            .unbounded_send(dash_ui::Drawn::Page(html))
            .expect("the window's component");
        settle(&mut doc);
        doc
    }

    fn settle(doc: &mut dash_ui::DioxusDocument) {
        for _ in 0..16 {
            if !doc.poll(None) {
                break;
            }
        }
        doc.inner_mut().resolve(0.0);
    }

    /// The window's element at `at`, the one only, tapped as the shell taps it.
    fn tap(doc: &mut dash_ui::DioxusDocument, at: &str) {
        use blitz_traits::events::{
            BlitzPointerEvent, BlitzPointerId, MouseEventButton, MouseEventButtons, PointerCoords,
            UiEvent,
        };
        let found = doc.inner().query_selector_all(at).expect("a selector");
        assert_eq!(found.len(), 1, "{at} once: {found:?}");
        let (x, y) = {
            let inner = doc.inner();
            let node = inner.get_node(found[0]).expect("a node");
            let (p, size) = (node.absolute_position(0.0, 0.0), node.final_layout().size);
            (p.x + size.width / 2.0, p.y + size.height / 2.0)
        };
        let pointer = |buttons| BlitzPointerEvent {
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
        doc.handle_ui_event(UiEvent::PointerMove(pointer(MouseEventButtons::None)));
        doc.handle_ui_event(UiEvent::PointerDown(pointer(MouseEventButtons::Primary)));
        doc.handle_ui_event(UiEvent::PointerUp(pointer(MouseEventButtons::None)));
        settle(doc);
    }

    /// A key typed in the window, as Blitz's shell sends it.
    fn type_key(doc: &mut dash_ui::DioxusDocument, c: &str) {
        use blitz_traits::events::{BlitzKeyEvent, KeyState, UiEvent};
        doc.handle_ui_event(UiEvent::KeyDown(BlitzKeyEvent {
            key: Key::Character(c.into()),
            code: Default::default(),
            modifiers: Default::default(),
            location: Default::default(),
            is_auto_repeating: false,
            is_composing: false,
            state: KeyState::Pressed,
            text: Some(c.into()),
        }));
        settle(doc);
    }

    /// The window's element at `at`'s attribute `attr`.
    fn attr_of(doc: &dash_ui::DioxusDocument, at: &str, attr: &str) -> Option<String> {
        let inner = doc.inner();
        let node =
            (inner.query_selector(at).expect("a selector")).unwrap_or_else(|| panic!("{at}"));
        let node = inner.get_node(node).expect("a node");
        node.attr(blitz_dom::LocalName::from(attr))
            .map(str::to_string)
    }

    /// A page as dash serves it, holding `parts` where it draws them: the design system's placeholders.
    fn page_of(parts: &[Part]) -> String {
        let placed: Vec<Element> = parts.iter().cloned().map(Part::placed).collect();
        dioxus_ssr::render_element(rsx! { h1 { "A task" } {placed.into_iter()} })
    }

    /// The window's ungraft finds the parts by the design system's attribute: ui names it apart, taking no surface.
    #[dash_test]
    fn the_window_s_graft_names_are_the_design_system_s() {
        assert_eq!(dash_ui::GRAFT, dash_surface::components::GRAFT);
    }

    /// Done-when 1 and 2 of rust-js-port-small, Copy: CopyButton's tap is CopyLive's Rust handler. Through liveview, as the
    /// page's client posts it, it asks the browser's clipboard for the link after the page's origin, and once the browser
    /// says it copied, the title says « Copied ». In the window, the part drawn beside the page is grafted into its place,
    /// its picture gone, and the same tap writes the Mac's clipboard, the link after dash serve's address.
    #[dash_test]
    fn copy_writes_the_clipboard_through_liveview_and_in_the_window() {
        let mut live = LivePage::new(vec![Some(copy_part())]);
        assert_eq!(live.attr("#c", "title").as_deref(), Some("Copy link"));
        live.send("click", "#c", click());
        let (query, script) = asked(&live.frames, "navigator.clipboard");
        assert!(
            script.contains(r#""text":"/tasks/a-task""#) && script.contains(r#""origin":true"#),
            "the browser is asked to copy the link after its origin: {script}"
        );
        let answer = serde_json::json!({"method": "query", "params": {"id": query, "data": "yes", "returned_value": true}});
        live.page
            .unbounded_send(Ok(Message::Text(answer.to_string().into())))
            .expect("the liveview");
        assert_eq!(next(&live.frames).first(), Some(&1), "the title is patched");
        assert_eq!(live.attr("#c", "title").as_deref(), Some("Copied"));

        let mut window = window(page_of(&[copy_part()]));
        assert!(
            window
                .inner()
                .query_selector(".live-part > [data-graft] #c")
                .expect("a selector")
                .is_some(),
            "the live copy is grafted into its place"
        );
        tap(&mut window, "#c");
        assert_eq!(
            CLIPBOARD.with(|c| c.borrow().clone()),
            "http://100.72.158.115:7788/tasks/a-task"
        );
        assert_eq!(attr_of(&window, "#c", "title").as_deref(), Some("Copied"));
    }

    /// Done-when 1 and 2 of rust-js-port-small, the menu filter: PropertyMenu's filter and digits are MenuLive's Rust handlers.
    /// Through liveview, words typed in its filter hide the choices without them, and a digit pressed on the open menu sends
    /// the numbered choice the filter shows, by the browser's own form. In the window, the part grafted into its place filters
    /// as he types in it.
    #[dash_test]
    fn the_menu_filters_through_liveview_and_in_the_window() {
        let mut live = LivePage::new(vec![Some(menu_part())]);
        let typed = serde_json::json!({"value": "urg", "values": [], "valid": true});
        live.send("input", "#m .prop-menu-filter", typed);
        while next(&live.frames).first() != Some(&1) {}
        assert_eq!(live.attr("#m-0", "hidden"), None, "Urgent stays");
        assert!(live.attr("#m-1", "hidden").is_some(), "High goes");
        assert!(live.attr("#m-2", "hidden").is_some(), "Medium goes");
        live.send("keydown", "#m", key("1"));
        live.send("keydown", "#m", key("0"));
        let (_, sent) = asked(&live.frames, "requestSubmit");
        assert!(
            sent.contains(r#"getElementById("m-0")"#),
            "0 sends Urgent, 1 was hidden: {sent}"
        );
        assert_eq!(SENT.with(|s| s.borrow().clone()), vec!["m-0".to_string()]);

        let mut window = window(page_of(&[menu_part()]));
        tap(&mut window, "#m > summary");
        tap(&mut window, "#m .prop-menu-filter");
        for c in ["h", "i"] {
            type_key(&mut window, c);
        }
        assert!(attr_of(&window, "#m-0", "hidden").is_some(), "Urgent goes");
        assert_eq!(attr_of(&window, "#m-1", "hidden"), None, "High stays");
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
        let mut twin = dash_ui::document(VirtualDom::new_with_props(
            dash_surface::components::compass_hosted,
            host,
        ));
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
        // the parts (task rust-js-port-small): each patch found its place in liveview's client, or the parts would not run
        for patched in [
            r#"ws.onopen = () => {ws.send(JSON.stringify([...document.querySelectorAll(".live-part")]"#,
            r#"window.interpreter.run_from_bytes(messageData);const fresh=document.querySelectorAll(".part-home>[data-graft]");"#,
            "document.addEventListener(event_name,this.handler)",
            "document.removeEventListener(",
            "let handler=(event)=>{if(getTargetId(event.target)!==null)",
        ] {
            assert!(client.contains(patched), "{patched}");
        }
        assert!(
            !client.contains("this.root.addEventListener(")
                && !client.contains("this.root.removeEventListener("),
            "liveview's client listens on the document, where the grafted parts' events bubble"
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
            (twin_host(), twin_parts(), island.clone(), documents, vec![]),
        ));
        let (page, from_page) = futures_channel::mpsc::unbounded();
        let (to_page, frames) = std::sync::mpsc::channel();
        let socket = Duplex { from_page, to_page };
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .build()
                .expect("a runtime");
            rt.block_on(run(socket, island, vec![]))
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
