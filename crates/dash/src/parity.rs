//! `dash parity <rust-path> [<elixir-path>]`: what the frozen Elixir app on :7799 shows that the Rust page on :7788 lacks,
//! each control, link, section and field, and both screens; `dash land` refuses a PR deleting an Elixir route while that
//! list holds a gap he has not dropped (task rust-port-parity; rust/CLAUDE.md rule 9; @nha, 2026-11-10: « they were NOT
//! ported properly. the visuals are incomplete, far inferior, and many features missing. »). The pages are read by
//! dash_surface::parity, which owns HTML.
use dash_surface::parity::{self as page, Gaps, Item, Kind};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

const RUST: &str = "http://localhost:7788";
const ELIXIR: &str = "http://localhost:7799";
/// The older app's router: no-elixir.sh refuses any edit to it, so a route leaves Elixir with the file its module is in.
pub const ROUTER: &str = "private/dashboard/lib/dashboard_web/router.ex";
/// His marks: a sample page for a route with a parameter, each link pattern the port moved, and each gap he let go, in his
/// words.
pub const MARKS: &str = "rust/parity.tsv";
/// The route his drops of the shared menu's items name, the menu compared once apart from every page.
const MENU: &str = "menu";

/// A page route of the older app: its whole path and its whole module name.
#[derive(Debug, PartialEq, Eq)]
pub struct Route {
    pub path: String,
    pub module: String,
}

/// The router's `live` and `get` routes, each under its scopes' paths and module alias.
pub fn routes(router: &str) -> Vec<Route> {
    let quoted = |t: &str| t.split('"').nth(1).map(str::to_string);
    let (mut scopes, mut found) = (vec![], vec![]);
    for line in router.lines() {
        let t = line.trim();
        if t == "end" {
            scopes.pop();
        } else if t.ends_with(" do") {
            // a scope's path and alias; any other block (pipeline, live_session, defmodule) adds neither
            let (path, alias) = if t.starts_with("scope ") {
                // `scope "/x", DashboardWeb, as: :x do`: the alias is the first word after the path, when a module
                let alias = t.split_once("\",").and_then(|(_, rest)| {
                    let word = rest.trim_end_matches(" do").split(',').next()?.trim();
                    word.starts_with(char::is_uppercase)
                        .then(|| word.to_string())
                });
                (quoted(t).unwrap_or_default(), alias.unwrap_or_default())
            } else {
                Default::default()
            };
            scopes.push((path, alias));
        } else if let Some(path) =
            quoted(t).filter(|_| t.starts_with("live \"") || t.starts_with("get \""))
        {
            let module = t
                .splitn(3, '"')
                .nth(2)
                .and_then(|rest| rest.split(',').nth(1))
                .unwrap_or("")
                .trim();
            let whole: String = scopes
                .iter()
                .map(|(p, _)| p.as_str())
                .chain([path.as_str()])
                .collect();
            let mut path = String::new();
            for c in whole.chars() {
                if !(c == '/' && path.ends_with('/')) {
                    path.push(c);
                }
            }
            if path.len() > 1 {
                path = path.trim_end_matches('/').to_string();
            }
            // nested scopes' aliases join, as Phoenix's do
            let module = scopes
                .iter()
                .map(|(_, a)| a.as_str())
                .filter(|a| !a.is_empty())
                .chain([module])
                .collect::<Vec<_>>()
                .join(".");
            found.push(Route { path, module });
        }
    }
    found
}

/// The routes a PR takes out of Elixir: every one when it deletes the router, else those whose module a deleted file defined.
/// `deleted`: each deleted file's path and its content on main.
pub fn removed(deleted: &[(String, String)], router: &str) -> Vec<Route> {
    if deleted.iter().any(|(path, _)| path == ROUTER) {
        return routes(router);
    }
    let modules: Vec<&str> = deleted
        .iter()
        .flat_map(|(_, content)| content.lines())
        .filter_map(|l| l.trim().strip_prefix("defmodule ")?.strip_suffix(" do"))
        .collect();
    routes(router)
        .into_iter()
        .filter(|r| modules.contains(&r.module.as_str()))
        .collect()
}

/// Whether `path` is a page of route `pattern`: a `:name` segment takes any one, `*name` the rest.
pub fn of_route(pattern: &str, path: &str) -> bool {
    bind(pattern, path).is_some()
}

/// `path`'s values for `pattern`'s parameters, each by its name, when `path` is a page of it.
fn bind<'a>(pattern: &'a str, path: &str) -> Option<Vec<(&'a str, String)>> {
    let (mut want, mut have) = (pattern.split('/'), path.split('/'));
    let mut bound = vec![];
    loop {
        match (want.next(), have.next()) {
            (None, None) => return Some(bound),
            (Some(w), h) if w.starts_with('*') => {
                let rest: Vec<&str> = h.into_iter().chain(have).collect();
                bound.push((&w[1..], rest.join("/")));
                return Some(bound);
            }
            (Some(w), Some(h)) if w.starts_with(':') && !h.is_empty() => {
                bound.push((&w[1..], h.to_string()))
            }
            (Some(w), Some(h)) if w == h => {}
            _ => return None,
        }
    }
}

/// `to` with each `:name` or `*name` given its bound value; a name not bound stays as written.
fn fill(to: &str, bound: &[(&str, String)]) -> String {
    let (mut out, mut rest) = (String::new(), to);
    while let Some(at) = rest.find([':', '*']) {
        out.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        let len = after
            .find(|c: char| !(c.is_alphanumeric() || c == '_'))
            .unwrap_or(after.len());
        match bound.iter().find(|(name, _)| *name == &after[..len]) {
            Some((_, value)) => out.push_str(value),
            None => out.push_str(&rest[at..at + 1 + len]),
        }
        rest = &after[len..];
    }
    out + rest
}

/// His marks file read: samples by route, and the gaps he dropped by route, each with his words.
#[derive(Default)]
pub struct Marks {
    /// A route's Elixir page, and its Rust page when the port moved it.
    samples: Vec<(String, String, Option<String>)>,
    /// An Elixir link pattern and the Rust one its links moved to, as `/d/:id` to `/drafts/:id`.
    moves: Vec<(String, String)>,
    dropped: Vec<(String, Kind, String)>,
    /// A dropped line with no words of his: it drops nothing, and is said.
    pub unsigned: Vec<String>,
}

impl Marks {
    pub fn read(text: &str) -> Marks {
        let mut marks = Marks::default();
        for line in text
            .lines()
            .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        {
            match line.split('\t').collect::<Vec<_>>()[..] {
                [route, "sample", path] => marks.samples.push((route.into(), path.into(), None)),
                [route, "sample", path, rust] => {
                    marks
                        .samples
                        .push((route.into(), path.into(), Some(rust.into())))
                }
                [from, "moved", to] => marks.moves.push((from.into(), to.into())),
                [route, "dropped", kind, label, words]
                    if words.contains("(@nha, ")
                        && words.contains('«')
                        && Kind::named(kind).is_some() =>
                {
                    let kind = Kind::named(kind).expect("checked");
                    marks.dropped.push((route.into(), kind, page::key(label)));
                }
                _ => marks.unsigned.push(line.to_string()),
            }
        }
        marks
    }

    /// The Rust and Elixir pages to compare for `route`: its sample's, else the route itself when it has no parameter.
    fn pages(&self, route: &str) -> Option<(String, String)> {
        match self.samples.iter().find(|(r, _, _)| r == route) {
            Some((_, elixir, rust)) => {
                Some((rust.clone().unwrap_or(elixir.clone()), elixir.clone()))
            }
            None if route.contains([':', '*']) => None,
            None => Some((route.to_string(), route.to_string())),
        }
    }

    /// Where an Elixir link goes in Rust: its path through the first move whose pattern it fits, its own query kept; else
    /// itself.
    pub fn moved(&self, href: &str) -> String {
        let (path, query) = href.split_once('?').unwrap_or((href, ""));
        let Some(to) = self
            .moves
            .iter()
            .find_map(|(from, to)| Some(fill(to, &bind(from, path)?)))
        else {
            return href.to_string();
        };
        match (query.is_empty(), to.contains('?')) {
            (true, _) => to,
            (false, true) => format!("{to}&{query}"),
            (false, false) => format!("{to}?{query}"),
        }
    }

    /// The page a link opens, read on both pages' links: through its move, then, when that is a page of a moved route, the
    /// route itself, so one row's link meets every row's (Elixir's /dashboard/tasks linked all 2899 issues, Rust's board its
    /// first cards and « N more »: rows are data, the link is the feature). A query pair the route does not name stays.
    pub fn opens(&self, href: &str) -> String {
        let href = self.moved(href);
        let (path, query) = href.split_once('?').unwrap_or((&href, ""));
        let pairs: Vec<&str> = query.split('&').filter(|p| !p.is_empty()).collect();
        let key = |pair: &str| pair.split('=').next().unwrap_or("").to_string();
        for (_, to) in &self.moves {
            let (to_path, to_query) = to.split_once('?').unwrap_or((to, ""));
            let named: Vec<&str> = to_query.split('&').filter(|p| !p.is_empty()).collect();
            let fits = |want: &&str| match want.split_once('=') {
                Some((k, v)) if v.starts_with(':') => pairs.iter().any(|p| key(p) == k),
                _ => pairs.contains(want),
            };
            if bind(to_path, path).is_some() && named.iter().all(fits) {
                let rest: Vec<&str> = (pairs.iter().copied())
                    .filter(|p| !named.iter().any(|n| key(n) == key(p)))
                    .collect();
                return match (rest.is_empty(), to.contains('?')) {
                    (true, _) => to.clone(),
                    (false, true) => format!("{to}&{}", rest.join("&")),
                    (false, false) => format!("{to}?{}", rest.join("&")),
                };
            }
        }
        href
    }

    /// Whether he dropped this gap of `route`.
    pub fn drops(&self, route: &str, gap: &Item) -> bool {
        self.dropped
            .iter()
            .any(|(r, k, key)| r == route && *k == gap.kind && *key == gap.key())
    }
}

/// `dash land`'s check: Ok when no deleted route's Elixir page shows what its Rust page lacks, his drops aside, nor the
/// menu every page shares, compared once. `gaps(rust, elixir)` reads the two pages at concrete paths.
pub fn hold(
    deleted: &[(String, String)],
    router: &str,
    marks: &Marks,
    mut gaps: impl FnMut(&str, &str) -> Result<Gaps, String>,
) -> Result<(), String> {
    let (mut open, mut menu) = (vec![], None);
    for route in removed(deleted, router) {
        let Some((rust, path)) = marks.pages(&route.path) else {
            open.push(format!(
                "{}: no page to compare: add « {}\tsample\tPATH, one page of it[\tITS RUST PATH] » to {MARKS}",
                route.path, route.path
            ));
            continue;
        };
        match gaps(&rust, &path) {
            Ok(lacks) => {
                open.extend(
                    lacks
                        .page
                        .iter()
                        .filter(|gap| !marks.drops(&route.path, gap))
                        .map(|gap| format!("{}: {}\t{}", route.path, gap.kind.name(), gap.label)),
                );
                menu.get_or_insert(lacks.menu);
            }
            Err(e) => open.push(format!("{path}: {e}")),
        }
    }
    open.extend(
        menu.unwrap_or_default()
            .iter()
            .filter(|gap| !marks.drops(MENU, gap))
            .map(|gap| format!("{MENU}: {}\t{}", gap.kind.name(), gap.label)),
    );
    if open.is_empty() {
        return Ok(());
    }
    Err(format!(
        "this PR deletes Elixir routes whose Rust page lacks what the Elixir page shows (dash parity, rust/CLAUDE.md rule 9):\n{}\n\
         Port each first, or he drops it: « ROUTE\tdropped\tKIND\tLABEL\t(@nha, DATE: « his words ») » in {MARKS}",
        open.join("\n")
    ))
}

/// `path` on `base`, unless `path` is a whole URL: a worktree's own server, on its own port, is compared before its merge
/// (task rust-issue-sidebar-menus: :7788 serves main).
fn url_of(base: &str, path: &str) -> String {
    match path.starts_with("http://") || path.starts_with("https://") {
        true => path.to_string(),
        false => format!("{base}{path}"),
    }
}

/// A page's HTML, and whether Phoenix answered it: its Plug.RequestId sets `x-request-id`, which no Rust page sends.
fn fetch(base: &str, path: &str) -> Result<(String, bool), String> {
    let url = url_of(base, path).replace("localhost", "127.0.0.1");
    // :7799's first request builds and boots the frozen app: minutes, once
    let args = [
        "-sSfL",
        "--max-time",
        "600",
        "-H",
        "Accept: text/html",
        "-w",
        "\n%header{x-request-id}",
        url.as_str(),
    ];
    let answer = dash_process::run("curl", &args, None, Duration::from_secs(620))?;
    let (html, id) = answer.rsplit_once('\n').unwrap_or((&answer, ""));
    Ok((html.to_string(), !id.trim().is_empty()))
}

/// The gaps at concrete paths, each page read live. A Rust path the front door passes on to Elixir (a LiveView or a
/// controller's page) is refused: compared with itself it would lack nothing.
pub fn gaps(rust_path: &str, elixir_path: &str, marks: &Marks) -> Result<Gaps, String> {
    let (elixir, _) = fetch(ELIXIR, elixir_path)?;
    let (rust, phoenix) = fetch(RUST, rust_path)?;
    if phoenix {
        return Err(format!(
            "Rust does not serve {rust_path} yet: its front door answers with the Elixir app's page"
        ));
    }
    page::gaps(&elixir, &rust, &|href| marks.opens(href))
}

/// `dash land <pr>`'s parity check, before any hold or wait: the PR's deleted files (GitHub's list, paged, so none past
/// the first hundred is missed), each read as main holds it, with the router and his marks, never a checkout that may lag.
pub fn land_check(repo: &Path, n: u64) -> Result<(), String> {
    let git = |args: &[&str]| dash_process::run("git", args, Some(repo), Duration::from_secs(120));
    git(&["fetch", "-q", "origin", "main"])?;
    // a path main does not hold (added and deleted on the branch) held no route
    let on_main = |path: &str| match git(&["show", &format!("origin/main:{path}")]) {
        Ok(text) => Ok(text),
        Err(e) if e.contains("does not exist") || e.contains("exists on disk") => Ok(String::new()),
        Err(e) => Err(e),
    };
    let files = format!("repos/{{owner}}/{{repo}}/pulls/{n}/files");
    let query = r#".[] | select(.status == "removed") | .filename"#;
    let args = ["api", files.as_str(), "--paginate", "-q", query];
    let listed = dash_process::run("gh", &args, Some(repo), Duration::from_secs(120))?;
    let mut deleted = vec![];
    for path in listed.lines().filter(|l| !l.is_empty()) {
        deleted.push((path.to_string(), on_main(path)?));
    }
    let marks = Marks::read(&on_main(MARKS)?);
    hold(&deleted, &on_main(ROUTER)?, &marks, |rust, elixir| {
        gaps(rust, elixir, &marks)
    })
}

/// The route a page belongs to in the older app's router, else the path itself.
fn route_of(router: &str, path: &str) -> String {
    let path = path.split(['?', '#']).next().unwrap_or(path);
    routes(router)
        .into_iter()
        .map(|r| r.path)
        .find(|r| of_route(r, path))
        .unwrap_or_else(|| path.to_string())
}

/// The checkout this command runs in, its router, marks and screens: the first folder up from here holding `.git` (a
/// worktree's is a file), when it holds the marks too (another repo's checkout would read none: /code-review), else
/// `DASH_REPO`'s. A task's run writes its screens in its own worktree, never in the main checkout,
/// read-only to it (task rust-parity-moved-routes; the parity audit set `DASH_REPO=<worktree>` to get round it).
pub fn here() -> PathBuf {
    std::env::current_dir()
        .ok()
        .and_then(|dir| {
            dir.ancestors()
                .find(|d| d.join(".git").exists())
                .filter(|d| d.join(MARKS).is_file())
                .map(Path::to_path_buf)
        })
        .unwrap_or_else(crate::pages::repo)
}

/// Where the screen of `name`'s page (Rust or Elixir) is drawn, under `repo`.
fn screen(repo: &Path, elixir_path: &str, name: &str) -> PathBuf {
    let slug: String = elixir_path
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    repo.join("private/live/parity")
        .join(format!("parity{slug}-{}.png", name.to_lowercase()))
}

/// Both screens of `slug`'s page in one image beside them, Elixir's left and Rust's right: the line saying where, or why
/// not (task rust-parity-styles: two PM rounds were lost proving a visual match with two links).
fn both(repo: &Path, slug: &str) -> Result<String, String> {
    let side = screen(repo, slug, "side");
    let (left, right) = (screen(repo, slug, "Elixir"), screen(repo, slug, "Rust"));
    match side_by_side(&left, &right, &side) {
        Ok(()) => Ok(format!(
            "side by side, Elixir left, Rust right:\t{}",
            side.display()
        )),
        Err(e) => Err(format!("no side-by-side image: {e}")),
    }
}

/// A PNG's width, height and pixels, four bytes each (red, green, blue, alpha).
fn rgba(path: &Path) -> Result<(u32, u32, Vec<u8>), String> {
    let at = |e: &dyn std::fmt::Display| format!("{}: {e}", path.display());
    let file = std::fs::File::open(path).map_err(|e| at(&e))?;
    let mut decoder = png::Decoder::new(std::io::BufReader::new(file));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().map_err(|e| at(&e))?;
    let size = reader
        .output_buffer_size()
        .ok_or_else(|| at(&"too large"))?;
    let mut buf = vec![0; size];
    let info = reader.next_frame(&mut buf).map_err(|e| at(&e))?;
    let buf = &buf[..info.buffer_size()];
    let px = match info.color_type.samples() {
        1 => buf.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        2 => buf
            .chunks(2)
            .flat_map(|p| [p[0], p[0], p[0], p[1]])
            .collect(),
        3 => buf
            .chunks(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        _ => buf.to_vec(),
    };
    Ok((info.width, info.height, px))
}

/// `left` and `right` in one PNG at `out`, a grey bar between them, the shorter one's foot white.
fn side_by_side(left: &Path, right: &Path, out: &Path) -> Result<(), String> {
    const BAR: u32 = 16;
    let ((lw, lh, l), (rw, rh, r)) = (rgba(left)?, rgba(right)?);
    let (w, h) = (lw + BAR + rw, lh.max(rh));
    let mut px = vec![255u8; (w * h * 4) as usize];
    for y in 0..h {
        let row = (y * w * 4) as usize;
        let bar = row + (lw * 4) as usize;
        // opaque grey: its alpha is 255, never 160 (/code-review)
        for p in px[bar..bar + (BAR * 4) as usize].chunks_mut(4) {
            p.copy_from_slice(&[160, 160, 160, 255]);
        }
        for (x0, iw, ih, img) in [(0, lw, lh, &l), (lw + BAR, rw, rh, &r)] {
            if y < ih {
                let from = (y * iw * 4) as usize;
                let to = row + (x0 * 4) as usize;
                px[to..to + (iw * 4) as usize]
                    .copy_from_slice(&img[from..from + (iw * 4) as usize]);
            }
        }
    }
    let at = |e: &dyn std::fmt::Display| format!("{}: {e}", out.display());
    let file = std::fs::File::create(out).map_err(|e| at(&e))?;
    let mut png = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    png.set_color(png::ColorType::Rgba);
    png.set_depth(png::BitDepth::Eight);
    let mut writer = png.write_header().map_err(|e| at(&e))?;
    writer.write_image_data(&px).map_err(|e| at(&e))?;
    writer.finish().map_err(|e| at(&e))
}

inventory::submit! { crate::usage::Command::usage("dash parity menu") }
inventory::submit! { crate::usage::Command::usage("dash parity <rust-path|rust-url> [<elixir-path>]|menu") }
/// `dash parity <rust-path> [<elixir-path>]`: both screens and one image of them side by side, each element both pages hold
/// with its two computed styles, then each gap of the page, his drops apart; `menu`, the menu every page shares instead,
/// compared once on its own (`dash parity menu`). Exit 1 while a gap is open; a style that differs is shown, never refused.
pub fn run(repo: &Path, rust_path: &str, elixir_path: &str, menu: bool, mut out: std::fs::File) {
    let router = std::fs::read_to_string(repo.join(ROUTER)).unwrap_or_default();
    let marks = Marks::read(&std::fs::read_to_string(repo.join(MARKS)).unwrap_or_default());
    let route = match menu {
        true => MENU.to_string(),
        false => route_of(&router, elixir_path),
    };
    let shot = repo.join("private/dashboard/test/scripts/shot");
    let mut say = |line: String| writeln!(out, "{line}").expect("stdout");
    // the menu's screens are its own, never /dashboard's (/code-review)
    let slug = if menu { "/menu" } else { elixir_path };
    let js = page::styles_js();
    let mut seen = vec![];
    for (name, base, path) in [("Elixir", ELIXIR, elixir_path), ("Rust", RUST, rust_path)] {
        let png = screen(repo, slug, name);
        let url = url_of(base, path);
        let took = std::fs::create_dir_all(png.parent().unwrap_or(repo))
            .map_err(|e| e.to_string())
            .and_then(|()| {
                let args = [url.as_str(), png.to_str().unwrap_or(""), "1280"];
                let env = [("SHOT_EVAL", js.as_str())];
                let limit = Duration::from_secs(120);
                let shot = shot.to_string_lossy();
                match dash_process::answer(&shot, &args, None, &env, limit) {
                    Ok((status, text, _)) if status.success() => Ok(text),
                    Ok((status, text, why)) => {
                        Err(format!("{status}: {} {}", why.trim(), text.trim()))
                    }
                    Err(no) => Err(no.why),
                }
            });
        match took {
            Ok(text) => {
                say(format!("{name}:\t{url}\tscreen {}", png.display()));
                // the evaluated expression's one line of JSON, before shot's own line; a page whose styles are not read
                // says which and why (/code-review)
                let styles = (text.lines().find(|l| l.starts_with('[')))
                    .ok_or_else(|| "no line of JSON".to_string())
                    .and_then(|l| {
                        serde_json::from_str::<Vec<page::Seen>>(l).map_err(|e| e.to_string())
                    });
                if let Err(e) = &styles {
                    say(format!("{name}:\tno styles read: {e}"));
                }
                seen.push(styles.ok());
            }
            Err(e) => say(format!("{name}:\t{url}\tno screen: {}", e.trim())),
        }
    }
    // a screen not taken this run leaves an older one under its name: never composed
    match seen.len() == 2 {
        true => say(both(repo, slug).unwrap_or_else(|e| e)),
        false => say("no side-by-side image: a screen was not taken".into()),
    }
    if let [Some(elixir), Some(rust)] = &seen[..] {
        let styled = page::styles(elixir, rust, menu, &|href| marks.opens(href));
        for line in styled.iter().flat_map(page::Styled::lines) {
            say(line);
        }
        let differ = styled.iter().filter(|s| s.differ() > 0).count();
        say(format!(
            "{differ} of {} elements both pages hold differ in style",
            styled.len()
        ));
    } else {
        say("no styles compared: a page's screen or its styles were not read".into());
    }
    for line in &marks.unsigned {
        say(format!("ignored, not his drop: {line}"));
    }
    let lacks = match gaps(rust_path, elixir_path, &marks) {
        Ok(Gaps { menu: lacks, .. }) if menu => lacks,
        Ok(Gaps { page: lacks, .. }) => lacks,
        Err(e) => {
            eprintln!("dash parity: {e}");
            std::process::exit(1);
        }
    };
    said_gaps(&lacks, &marks, &route, menu, say);
}

/// `dash parity --html <path> <elixir.html> <rust.html>`: the same gaps, read from two pages saved as files, `path` the older
/// app's route they show: a page served on another port than :7788 (a branch's build on a data copy), or a page the frozen
/// app no longer holds (task editor-in-rfcs: :7799's checkout predates rfc-inbox.html).
pub fn files(repo: &Path, path: &str, elixir: &Path, rust: &Path, mut out: std::fs::File) {
    let router = std::fs::read_to_string(repo.join(ROUTER)).unwrap_or_default();
    let marks = Marks::read(&std::fs::read_to_string(repo.join(MARKS)).unwrap_or_default());
    let read = |f: &Path| std::fs::read_to_string(f).map_err(|e| format!("{}: {e}", f.display()));
    let gaps = read(elixir)
        .and_then(|e| read(rust).and_then(|r| page::gaps(&e, &r, &|href| marks.opens(href))));
    match gaps {
        Ok(Gaps { page: lacks, .. }) => {
            let say = |line: String| writeln!(out, "{line}").expect("stdout");
            said_gaps(&lacks, &marks, &route_of(&router, path), false, say)
        }
        Err(e) => {
            eprintln!("dash parity: {e}");
            std::process::exit(1);
        }
    }
}

/// Each gap still open, each he dropped, then their count; a gap open exits 1.
fn said_gaps(lacks: &[Item], marks: &Marks, route: &str, menu: bool, mut say: impl FnMut(String)) {
    let (dropped, open): (Vec<_>, Vec<_>) = lacks.iter().partition(|gap| marks.drops(route, gap));
    for gap in &open {
        say(format!("{}\t{}", gap.kind.name(), gap.label));
    }
    for gap in &dropped {
        say(format!(
            "dropped by him\t{}\t{}",
            gap.kind.name(),
            gap.label
        ));
    }
    say(format!(
        "{} missing on {route}, {} dropped by him{}",
        open.len(),
        dropped.len(),
        if menu {
            ""
        } else {
            "; the menu every page shares: dash parity menu"
        }
    ));
    if !open.is_empty() {
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::memory::TestBudget;

    const ROUTES: &str = r#"defmodule DashboardWeb.Router do
  scope "/dashboard", DashboardWeb do
    pipe_through :browser
    live "/", HomeLive
  end
  scope "/", DashboardWeb do
    live_session :pages do
      live "/mail", MailLive
      live "/mail/t/:thread", MailLive
      live "/drafts", DraftsLive
    end
    post "/mail/account", MailController, :account
    get "/tasks/:id", TaskController, :show
    scope "/admin", Admin, as: :admin do
      live "/keys", KeysLive
    end
  end
  scope "/api" do
    live "/x", Api.XLive
  end
end
"#;

    fn deleted(module: &str) -> Vec<(String, String)> {
        let content = format!("defmodule DashboardWeb.{module} do\n  use Phoenix.LiveView\nend\n");
        vec![(
            format!("private/dashboard/lib/dashboard_web/live/{module}.ex"),
            content,
        )]
    }

    fn gap(kind: Kind, label: &str) -> Item {
        Item {
            kind,
            label: label.into(),
            href: None,
        }
    }

    fn page_gaps(page: Vec<Item>) -> Gaps {
        Gaps { page, menu: vec![] }
    }

    /// The marks this repo holds.
    fn ours() -> Marks {
        Marks::read(include_str!("../../../parity.tsv"))
    }

    /// Task rust-parity-moved-routes, done-when 1: a `moved` line maps an Elixir link pattern to its Rust one, and a link
    /// to the moved path is there; one that fits no pattern is a gap still.
    #[test]
    fn a_link_to_a_moved_path_counts_as_there() {
        let _budget = TestBudget::new();
        let marks = Marks::read("/old/:id\tmoved\t/new/:id\n");
        assert!(marks.unsigned.is_empty());
        // both pages' links read through it: any page of the new route meets any of the old one (rows are data), and a
        // link that fits no pattern stays itself; dash_surface's a_link_to_a_moved_route_is_there_at_its_new_path counts it
        assert_eq!(marks.opens("/old/acme"), "/new/:id");
        assert_eq!(marks.opens("/new/beta"), "/new/:id");
        assert_eq!(marks.opens("/gone/acme"), "/gone/acme");
        // a name the pattern does not bind stays as written, and the link's own query rides along
        let marks = Marks::read("/a/:x\tmoved\t/b/:y?x=:x\n");
        assert_eq!(marks.moved("/a/1?z=2"), "/b/:y?x=1&z=2");
    }

    /// Done-when 2: the draft page /d/:id moved to /drafts/:id.
    #[test]
    fn our_marks_move_a_draft_link_to_drafts() {
        let _budget = TestBudget::new();
        assert_eq!(ours().moved("/d/acme-cto"), "/drafts/acme-cto");
        assert_eq!(ours().moved("/drafts"), "/drafts");
    }

    /// Done-when 2: an issue page moved to its board's query.
    #[test]
    fn our_marks_move_an_issue_link_to_its_board_s_query() {
        let _budget = TestBudget::new();
        let marks = ours();
        assert_eq!(
            marks.moved("/projects/site_perso/issue/rust-poc"),
            "/projects/site_perso?issue=rust-poc"
        );
        assert_eq!(
            marks.moved("/projects/graal/issue/g.1?tab=log"),
            "/projects/graal?issue=g.1&tab=log"
        );
        // both pages' issue links open the route; the board itself, with no issue, is not one
        let route = "/projects/:name?issue=:issue";
        assert_eq!(marks.opens("/projects/klar/issue/k-1"), route);
        assert_eq!(
            marks.opens("/projects/klar?issue=k-2&view=all"),
            format!("{route}&view=all")
        );
        assert_eq!(marks.opens("/projects/klar"), "/projects/klar");
    }

    /// Done-when 2: the older task list, /dashboard/tasks, is compared with Rust's /tasks, and its links go there.
    #[test]
    fn our_marks_sample_the_task_list_as_tasks() {
        let _budget = TestBudget::new();
        let marks = ours();
        assert!(marks.unsigned.is_empty(), "{:?}", marks.unsigned);
        assert_eq!(
            marks.pages("/dashboard/tasks"),
            Some(("/tasks".to_string(), "/dashboard/tasks".to_string()))
        );
        assert_eq!(
            marks.moved("/dashboard/tasks?example=AAA-1"),
            "/tasks?example=AAA-1"
        );
    }

    /// Done-when 3, `dash land`: the shared menu's gaps are listed once, under « menu », however many routes are compared, and
    /// his drop of one names that route.
    #[test]
    fn the_shared_menu_is_held_once() {
        let _budget = TestBudget::new();
        let lacks = || {
            Ok(Gaps {
                page: vec![],
                menu: vec![gap(Kind::Link, "Focus")],
            })
        };
        let held = hold(
            &deleted("MailLive"),
            ROUTES,
            &Marks::read("/mail/t/:thread\tsample\t/mail/t/a\n"),
            |_, _| lacks(),
        );
        let why = held.expect_err("refused");
        assert_eq!(why.matches("menu: link\tFocus").count(), 1, "{why}");
        assert!(!why.contains("/mail: link"), "{why}");
        let his = Marks::read(
            "/mail/t/:thread\tsample\t/mail/t/a\nmenu\tdropped\tlink\tFocus\t(@nha, 2026-11-11: « gone »)\n",
        );
        assert_eq!(
            hold(&deleted("MailLive"), ROUTES, &his, |_, _| lacks()),
            Ok(())
        );
    }

    /// Done-when 5: screens, router and marks are read in the checkout the command runs in, never `DASH_REPO`'s.
    #[test]
    fn screens_go_under_the_running_checkout() {
        let _budget = TestBudget::new();
        let main = dash_meter::TestDir::new("parity-main");
        let worktree = dash_meter::TestDir::new("parity-worktree");
        std::fs::write(worktree.0.join(".git"), "gitdir: elsewhere\n").expect(".git");
        let inside = worktree.0.join("rust/crates");
        std::fs::create_dir_all(&inside).expect("a folder");
        // nextest runs each test in its own process (rust/CLAUDE.md rule 1): nothing else reads cwd or DASH_REPO.
        // Another repo's checkout, with no marks: DASH_REPO's is read
        std::env::set_current_dir(&inside).expect("cd");
        unsafe { std::env::set_var("DASH_REPO", &main.0) };
        assert_eq!(here(), main.0);
        std::fs::write(worktree.0.join(MARKS), "").expect("marks");
        let worktree = worktree.0.canonicalize().expect("worktree");
        assert_eq!(here().canonicalize().expect("here"), worktree);
        let png = screen(&here(), "/dashboard/tasks", "Rust");
        assert!(
            png.starts_with(here())
                && png.ends_with("private/live/parity/parity-dashboard-tasks-rust.png"),
            "{}",
            png.display()
        );
    }

    /// A PNG of `w` by `h`, every pixel `rgb`.
    fn plain(path: &Path, w: u32, h: u32, rgb: [u8; 3]) {
        let file = std::fs::File::create(path).expect("a png");
        let mut png = png::Encoder::new(std::io::BufWriter::new(file), w, h);
        png.set_color(png::ColorType::Rgb);
        let mut writer = png.write_header().expect("a header");
        writer
            .write_image_data(&rgb.repeat((w * h) as usize))
            .expect("pixels");
    }

    /// Task rust-parity-styles, done-when 2: both screens become one image, Elixir's left and Rust's right, and the line
    /// printed names its path; a screen missing writes none and says why.
    #[test]
    fn both_screens_become_one_image_side_by_side_and_its_path_is_printed() {
        let _budget = TestBudget::new();
        let repo = dash_meter::TestDir::new("parity-side");
        let dir = repo.0.join("private/live/parity");
        std::fs::create_dir_all(&dir).expect("a folder");
        assert!(
            both(&repo.0, "/mail")
                .expect_err("no screens")
                .starts_with("no side-by-side image: ")
        );
        plain(&screen(&repo.0, "/mail", "Elixir"), 3, 2, [255, 0, 0]);
        plain(&screen(&repo.0, "/mail", "Rust"), 2, 4, [0, 0, 255]);
        let line = both(&repo.0, "/mail").expect("composed");
        let side = screen(&repo.0, "/mail", "side");
        assert_eq!(
            line,
            format!("side by side, Elixir left, Rust right:\t{}", side.display())
        );
        let (w, h, px) = rgba(&side).expect("a png");
        assert_eq!((w, h), (3 + 16 + 2, 4));
        let at = |x: u32, y: u32| px[((y * w + x) * 4) as usize..][..4].to_vec();
        assert_eq!(at(0, 0), [255, 0, 0, 255]);
        assert_eq!(at(3, 0), [160, 160, 160, 255]);
        assert_eq!(at(19, 3), [0, 0, 255, 255]);
        // below the shorter left screen, white
        assert_eq!(at(0, 3), [255, 255, 255, 255]);
    }

    /// Task rust-issue-sidebar-menus: a whole URL is a worktree's own server, compared before its merge; a path is :7788's.
    #[test]
    fn a_rust_page_is_a_path_on_the_live_server_or_a_whole_url() {
        let _budget = TestBudget::new();
        assert_eq!(url_of(RUST, "/tasks/a"), "http://localhost:7788/tasks/a");
        assert_eq!(
            url_of(RUST, "http://localhost:7801/tasks/a"),
            "http://localhost:7801/tasks/a"
        );
    }

    #[test]
    fn the_router_s_pages_take_their_scope_s_path_and_alias() {
        let _budget = TestBudget::new();
        let got: Vec<_> = routes(ROUTES)
            .into_iter()
            .map(|r| (r.path, r.module))
            .collect();
        let want = [
            ("/dashboard", "DashboardWeb.HomeLive"),
            ("/mail", "DashboardWeb.MailLive"),
            ("/mail/t/:thread", "DashboardWeb.MailLive"),
            ("/drafts", "DashboardWeb.DraftsLive"),
            ("/tasks/:id", "DashboardWeb.TaskController"),
            // nested aliases join, a scope's options are not its alias, and a scope with none adds nothing
            ("/admin/keys", "DashboardWeb.Admin.KeysLive"),
            ("/api/x", "Api.XLive"),
        ];
        assert_eq!(got, want.map(|(p, m)| (p.to_string(), m.to_string())));
        assert!(
            of_route("/mail/t/:thread", "/mail/t/abc") && !of_route("/mail/t/:thread", "/mail/t")
        );
        assert!(of_route("/v/*path", "/v/a/b") && !of_route("/mail", "/drafts"));
        assert_eq!(route_of(ROUTES, "/mail/t/x?y=1"), "/mail/t/:thread");
    }

    /// Done-when 2, a PR deleting no route: nothing is read, it lands.
    #[test]
    fn a_pr_deleting_no_elixir_route_lands_unread() {
        let _budget = TestBudget::new();
        let other = vec![("rust/x.rs".to_string(), "fn x() {}".to_string())];
        assert_eq!(
            hold(&other, ROUTES, &Marks::default(), |_, _| panic!("read")),
            Ok(())
        );
        assert_eq!(
            hold(
                &deleted("Unrouted"),
                ROUTES,
                &Marks::default(),
                |_, _| panic!("read")
            ),
            Ok(())
        );
    }

    /// Done-when 2, a gap open: the route deleted with its module is refused, each gap named.
    #[test]
    fn a_pr_deleting_a_route_whose_rust_page_lacks_a_part_is_refused() {
        let _budget = TestBudget::new();
        // the thread page moved in Rust: its sample names both pages
        let marks = Marks::read("/mail/t/:thread\tsample\t/mail/t/abc\t/m/abc\n");
        let mut read = vec![];
        let held = hold(&deleted("MailLive"), ROUTES, &marks, |rust, elixir| {
            read.push(format!("{rust} {elixir}"));
            Ok(page_gaps(vec![gap(Kind::Control, "Archive")]))
        });
        let why = held.expect_err("refused");
        assert!(
            why.contains("/mail: control\tArchive\n/mail/t/:thread: control\tArchive"),
            "{why}"
        );
        assert_eq!(read, ["/mail /mail", "/m/abc /mail/t/abc"]);
    }

    /// Done-when 2, every gap dropped by him: it lands. A drop with no words of his drops nothing.
    #[test]
    fn a_gap_he_dropped_lets_it_land_and_an_unsigned_drop_does_not() {
        let _budget = TestBudget::new();
        let his = "/drafts\tdropped\tlink\tOld CRM\t(@nha, 2026-11-10: « plus besoin »)\n";
        let lacks = || Ok(page_gaps(vec![gap(Kind::Link, "Old  CRM 2")]));
        assert_eq!(
            hold(&deleted("DraftsLive"), ROUTES, &Marks::read(his), |_, _| {
                lacks()
            }),
            Ok(())
        );
        let unsigned = Marks::read("/drafts\tdropped\tlink\tOld CRM\tno longer needed\n");
        assert_eq!(unsigned.unsigned.len(), 1);
        let held = hold(&deleted("DraftsLive"), ROUTES, &unsigned, |_, _| lacks());
        assert!(
            held.expect_err("refused")
                .contains("/drafts: link\tOld  CRM 2")
        );
        // dropped on another route, or as another kind, it drops nothing here
        let elsewhere = Marks::read(&his.replace("/drafts", "/mail"));
        assert!(hold(&deleted("DraftsLive"), ROUTES, &elsewhere, |_, _| lacks()).is_err());
    }

    /// Done-when 2, no page to compare: a route with a parameter and no sample, a Rust page that is still Elixir's, or a page
    /// that does not answer, is refused.
    #[test]
    fn a_route_that_cannot_be_compared_is_refused() {
        let _budget = TestBudget::new();
        let none = hold(
            &deleted("TaskController"),
            ROUTES,
            &Marks::default(),
            |_, _| panic!("read"),
        );
        assert!(
            none.expect_err("refused")
                .contains("/tasks/:id: no page to compare")
        );
        let down = hold(&deleted("DraftsLive"), ROUTES, &Marks::default(), |_, _| {
            Err("curl: refused".into())
        });
        assert!(
            down.expect_err("refused")
                .contains("/drafts: curl: refused")
        );
    }

    /// The router deleted whole takes every route with it.
    #[test]
    fn deleting_the_router_removes_every_route() {
        let _budget = TestBudget::new();
        let all = vec![(ROUTER.to_string(), ROUTES.to_string())];
        assert_eq!(removed(&all, ROUTES).len(), 7);
    }
}
