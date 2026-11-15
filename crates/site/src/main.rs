//! nicolas-ha.com (rfc-dashboard R23; task rust-site-no-container: @nha, 2026-11-08: « why does the site build need a
//! container ? on mac ? I mean we deploy on linux even »). The Elixir site release's routes, one request a connection,
//! a thread each:
//! 1. **The files of site/** as PublicSite.Static answered them: a file as it is, a folder's index after a 301 to its slash,
//!    `CLAUDE.md` never, `www.` sent to the bare domain, ETag and 304, one byte range 206 (Safari's `<video>`), HSTS on
//!    every answer.
//! 2. **`/ops/chat` and `/ops/relay`** behind `CHAT_OPS_TOKEN` (PublicSite.OpsController): the chat is off
//!    (config/site.exs), so no record is kept and `/ops/chat` lists none; `/chat` and `/live` answer 404 as there.
//! 3. **`POST /hooks/github` and `GET /hooks/envelopes`** (PublicSite.HooksController): a signed delivery's ids and
//!    states kept in `store`, pulled behind `HOOKS_PULL_TOKEN`.
//!
//! Everything else answers 404. A secret unset makes its route answer 404 like an unknown path.
//!
//! `site start`: on [::]:8080 (fly.toml's internal_port), site/ at SITE_ROOT, the hooks' queue in SITE_DATA, the image's
//! Fly volume; bin/site-start runs it as nobody. `site serve <root> <data> <port>`: the same on loopback, its address
//! printed first (port 0 takes a free one). `site image <sha> <tag> <dir> <crane>`: builds and pushes the image
//! (image.rs), run by Dashboard.SiteDeploy from the checkout. One crate root, so dash-gate's module graph reads it whole:
//! tests/routes.rs drives this binary.
mod image;
mod server;
mod store;

use server::{Site, serve};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use store::Store;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let env = |k: &str, d: &str| std::env::var(k).unwrap_or_else(|_| d.to_string());
    match args.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        ["start"] => start(
            env("SITE_ROOT", "/app/site"),
            env("SITE_DATA", "/data"),
            &format!("[::]:{}", env("PORT", "8080")),
        ),
        ["serve", root, data, port] => {
            start(root.into(), data.into(), &format!("127.0.0.1:{port}"))
        }
        ["image", sha, tag, dir, crane] => {
            match image::build(Path::new("."), sha, tag, Path::new(dir), crane) {
                Ok(text) => print!("{text}"),
                Err(why) => {
                    eprintln!("site image: {why}");
                    std::process::exit(1);
                }
            }
        }
        _ => {
            eprintln!(
                "usage: site start | site serve <root> <data> <port> | site image <sha> <tag> <dir> <crane>"
            );
            std::process::exit(2);
        }
    }
}

fn start(root: String, data: String, at: &str) {
    let store = Store::open(Path::new(&data)).unwrap_or_else(|e| panic!("{data}: {e}"));
    let mut site = Site::new(PathBuf::from(root), store);
    let secret = |k: &str| std::env::var(k).unwrap_or_default();
    site.ops_token = secret("CHAT_OPS_TOKEN");
    site.pull_token = secret("HOOKS_PULL_TOKEN");
    site.hook_secret = secret("GITHUB_WEBHOOK_SECRET");
    if let Ok(proc) = std::env::var("SITE_PROC") {
        site.proc = proc.into();
    }
    let listener = TcpListener::bind(at).unwrap_or_else(|e| panic!("{at}: {e}"));
    let bound = listener
        .local_addr()
        .map_or(at.to_string(), |a| a.to_string());
    println!("site: {bound}");
    serve(listener, site);
}
