//! Section 10's try: the window's own components in a browser, nothing rewritten.
//!   page-web --ssr     prints the page as HTML once, dioxus-ssr (no handlers: it is a picture until something patches it)
//!   page-web [port]    serves it live, dioxus-liveview: the vdom on the server, DOM patches and events over a websocket
use axum::Router;
use axum::http::header;
use axum::routing::get;
use dioxus::prelude::*;
use dioxus_liveview::LiveviewRouter;
use dioxus_page::{App, Screen};
use ui_spike_common::INTER;
use ui_spike_common::meter::memory;

/// The window loads Inter from its font context; a browser needs it served.
#[allow(non_snake_case)]
fn WebPage() -> Element {
    rsx! {
        style { "@font-face {{ font-family: Inter; src: url(/inter.ttf); font-weight: 100 900; }}" }
        App { start: Screen::Board }
    }
}

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let first = args.next();
    if first.as_deref() == Some("--ssr") {
        let mut vdom = dioxus_page::vdom(Screen::Board);
        vdom.rebuild_in_place();
        println!("<!doctype html><meta charset=utf-8>{}", dioxus_ssr::render(&vdom));
        return;
    }
    let port: u16 = first.and_then(|p| p.parse().ok()).unwrap_or(7798);
    let app = Router::new()
        .route("/inter.ttf", get(|| async { ([(header::CONTENT_TYPE, "font/ttf")], INTER) }))
        // The server's footprint, for web-check.mjs: each open tab holds a vdom here.
        .route("/memory", get(|| async {
            let m = memory();
            format!("{{\"current_mb\":{:.1},\"peak_mb\":{:.1}}}", m.current_mb, m.peak_mb)
        }))
        .with_app("/", WebPage);
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await.expect("port");
    println!("http://127.0.0.1:{port}/page");
    axum::serve(listener, app.into_make_service()).await.expect("serve");
}
