//! What a plugin registers and reads by key (ARCHITECTURE.md section 3): a channel, an element, a post, a command, a context
//! provider and a slot's line, each a struct of keys and `fn` pointers gathered by `inventory`; the request a post is handed
//! and the answer it gives; the tree a page draws. The host's lookups with the off filter (`dash_core::page`) are its own.
use crate::host::Host;
use std::any::Any;
use std::io::Write;
use std::path::PathBuf;

/// What a page reads, which says where the server answers it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Reads {
    /// Nothing shared: on its connection's thread, no lock taken.
    Nothing,
    /// The board: on its connection's thread, the board's lock held.
    Board,
    /// The views alone, through `Host::view`: on its connection's thread, the board's lock taken only to hand them.
    View,
}

/// A page, registered from its own module with `inventory::submit! { Page { .. } }`: a new page edits no hub (rfc-rust-unified
/// section 11, flat dispatch).
#[derive(Clone, Copy)]
pub struct Page {
    /// Its path, or `<prefix>/*` for each path one segment under the prefix: `/tasks/*` serves /tasks/<id>.
    pub path: &'static str,
    /// Its metric step, so a client's paths never mint one.
    pub step: &'static str,
    pub reads: Reads,
    /// The window draws it again at each ask, as a page that reads the store; any other once a visit.
    pub redraw: bool,
    pub serve: fn(&Asked) -> Answer,
}

inventory::collect!(Page);

/// A plugin's page that follows views: each diff of `views` draws its element `live` again whole, at `stream` on the web
/// (`<page>?view=patches`) and in the window. The core's own pages hold their own (the board's, card by card).
pub struct Follows {
    pub page: &'static str,
    pub stream: &'static str,
    pub views: &'static [&'static str],
    pub live: &'static str,
}

inventory::collect!(Follows);

/// A UI tree of the design system's catalog elements (rfc-rust-unified section 10): each element by its id, its type, its
/// props and its children. The design system checks it and draws it in every form.
#[derive(Default)]
pub struct Tree {
    elements: serde_json::Map<String, serde_json::Value>,
}

impl Tree {
    /// The element `id` of type `kind` (« issue-row »), its props none when `props` is null. Its anchor (rfc-rust-plugins-ui
    /// section 3, the core's anchor prop) is its id until `anchor` names a lasting one.
    pub fn add(&mut self, id: &str, kind: &str, props: serde_json::Value, children: &[String]) {
        let mut element = serde_json::json!({ "type": kind, "children": children, "anchor": id });
        if !props.is_null() {
            element["props"] = props;
        }
        self.elements.insert(id.into(), element);
    }

    /// Element `id`'s anchor, `key`: one that outlives a redraw (« drafts/d1/text »), where comments attach. An id not added yet
    /// is left as it is.
    pub fn anchor(&mut self, id: &str, key: &str) {
        if let Some(element) = self.elements.get_mut(id) {
            element["anchor"] = key.into();
        }
    }

    /// Its elements by id, for a page that builds its own tree's map to take in.
    pub fn elements(self) -> serde_json::Map<String, serde_json::Value> {
        self.elements
    }

    /// The tree under `root`, titled, for the host to draw.
    pub fn drawing(self, title: &str, root: &str) -> Drawing {
        let tree = serde_json::json!({ "root": root, "elements": self.elements });
        Drawing::Tree {
            title: title.into(),
            tree: tree.to_string(),
        }
    }
}

/// A plugin's lines another plugin reads by key, through the host (principle 7: plugins compose by key, never by import),
/// registered from its own module with `inventory::submit! { ContextProvider { .. } }`.
pub struct ContextProvider {
    /// `<plugin>/<what>`, « launcher/parts ».
    pub key: &'static str,
    /// The lines about `about`, the host handed in.
    pub lines: fn(&dyn Host, &str) -> Result<Vec<String>, String>,
}

inventory::collect!(ContextProvider);

/// The lines the provider of `key` gives about `about`; no provider, or a key taken twice, is said.
pub fn context(host: &dyn Host, key: &str, about: &str) -> Result<Vec<String>, String> {
    let found = inventory::iter::<ContextProvider>().filter(|p| p.key == key);
    (one(key, found)?.lines)(host, about)
}

/// The one entry registered under `key`: none, or two, is said.
fn one<T>(key: &str, mut found: impl Iterator<Item = T>) -> Result<T, String> {
    match (found.next(), found.next()) {
        (Some(p), None) => Ok(p),
        (None, _) => Err(format!("no plugin provides {key}")),
        (Some(_), Some(_)) => Err(format!("{key} is provided twice")),
    }
}

/// A message a channel would send (rfc-rust-plugins-ui section 3, the Channel port): its key, which names it in the sender's
/// logs, from whom, to whom, its subject (empty for a reply: the thread's), its words, and the thread it answers.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Message {
    pub key: String,
    pub from: String,
    pub to: Vec<String>,
    pub subject: String,
    pub text: String,
    pub answers: Option<String>,
}

/// One check a channel runs on a message before he sees it: what it checks, and why it fails, `None` when it passes.
#[derive(Clone, Debug, PartialEq)]
pub struct Check {
    pub name: &'static str,
    pub fails: Option<String>,
    /// Its failure may pass on its own (a read or a gate that did not answer): shown, never held against the message.
    pub retry: bool,
}

/// A way a message leaves (« email »), registered by its plugin with `inventory::submit! { Channel { .. } }` and read by key,
/// never by import: drafts sends through it (principle 7).
pub struct Channel {
    pub key: &'static str,
    /// Every check of the message, passed or failed.
    pub checks: fn(&dyn Host, &Message) -> Vec<Check>,
    /// The message sent once: what the channel made of it (the mail's id), or why it did not go.
    pub send: fn(&dyn Host, &Message) -> Result<String, String>,
}

inventory::collect!(Channel);

/// The channel registered under `key`.
pub fn channel(key: &str) -> Result<&'static Channel, String> {
    one(key, inventory::iter::<Channel>().filter(|c| c.key == key))
}

/// A screen component another plugin places (rfc-rust-plugins-ui section 3, the Element port): mail's thread and compose box,
/// a draft's card. Its key is `<plugin>/<name>`; one placed in another plugin's page is keyed `<place>/<order>-<plugin>`, as a
/// slot's (« mail/thread/10-drafts »), so the page reads every key under its place in order, never a plugin by name.
pub struct Element {
    pub key: &'static str,
    /// Its parts added to the tree as `props` asks, each id under the id handed in: the ids of its top parts, none when it has
    /// nothing to show.
    pub draw: Draw,
}

/// An element's drawing: the host, its props, the id its parts go under, and the tree they are added to.
pub type Draw = fn(&dyn Host, &serde_json::Value, &str, &mut Tree) -> Result<Vec<String>, String>;

inventory::collect!(Element);

/// The element registered under `key`.
pub fn element(key: &str) -> Result<&'static Element, String> {
    one(key, inventory::iter::<Element>().filter(|e| e.key == key))
}

/// The elements placed under `place`, in key order.
pub fn placed(place: &str) -> Vec<&'static Element> {
    let under = format!("{place}/");
    let mut found: Vec<_> = inventory::iter::<Element>()
        .filter(|e| e.key.starts_with(&under))
        .collect();
    found.sort_by_key(|e| e.key);
    found
}

/// The place elements drawn on an anchor are put under (the comments plugin's threads): `on_anchor` draws each.
pub const ANCHORED: &str = "anchor";

/// Where a comment on an anchor is filed (its `anchor`, `from`, `to`, `quote`, `text` and `page` posted as a form, 200 and its
/// id answered): the plugin keeping comments registers its `Post` here, and an element's plugin posts to it, neither naming
/// the other.
pub const COMMENT_ON_ANCHOR: &str = "/anchor/comment";

/// What every plugin placed under `ANCHORED` draws on `anchor`, whose element shows `words`, each part under `id`: a plugin
/// draws on another's element by its anchor alone, never by its name (rfc-rust-plugins-ui section 3). A part that fails is
/// said as a line in its place, the rest still drawn.
pub fn on_anchor(
    host: &dyn Host,
    anchor: &str,
    words: &str,
    id: &str,
    tree: &mut Tree,
) -> Vec<String> {
    let props = serde_json::json!({ "anchor": anchor, "words": words });
    let each = placed(ANCHORED).into_iter().enumerate().map(|(i, part)| {
        let at = format!("{id}-{i}");
        (part.draw)(host, &props, &at, tree).unwrap_or_else(|e| {
            tree.add(
                &at,
                "text",
                serde_json::json!({ "text": format!("{}: {e}", part.key) }),
                &[],
            );
            vec![at]
        })
    });
    each.flatten().collect()
}

/// A form a plugin answers, as a page answers its path (a draft's Send): registered with `inventory::submit! { Post { .. } }`,
/// its path a page's, `<prefix>/*` for each path one segment under the prefix. `serve` is handed the request and the form.
pub struct Post {
    pub path: &'static str,
    /// Its metric step, as a page's.
    pub step: &'static str,
    pub serve: fn(&Asked, &str) -> Answer,
}

inventory::collect!(Post);

/// The post answering `path`: its exact path, else its prefix's `/*`.
pub fn post(path: &str) -> Option<&'static Post> {
    let wild = path.rfind('/').map(|cut| format!("{}*", &path[..=cut]));
    let mut posts = inventory::iter::<Post>();
    posts.find(|p| p.path == path || Some(p.path) == wild.as_deref())
}

/// A page's one source (the Rust tools' rule 4): the design system draws it as HTML, text or terminal, never the page.
pub enum Drawing {
    /// A UI tree of catalog elements, as JSON.
    Tree { title: String, tree: String },
    /// A page a component of its own draws whole, as /design: its text is its words.
    Page(String),
}

pub const TEXT: &str = "text/plain; charset=utf-8";
pub const HTML: &str = "text/html; charset=utf-8";
pub const JSON: &str = "application/json; charset=utf-8";

/// What a page answers: a drawing the host draws in the form asked, or a body as it is (a redirect, an error, a JSON reply).
pub enum Answer {
    Drawn(Drawing),
    /// Its status line, content type and text.
    Body(&'static str, &'static str, String),
}

impl From<(&'static str, &'static str, String)> for Answer {
    fn from((status, kind, body): (&'static str, &'static str, String)) -> Self {
        Answer::Body(status, kind, body)
    }
}

impl Page {
    /// The request it is served at `path?query`, in the form `html` asks: handed the board when it reads it, else the
    /// registry (/today's slot lines), one held, never both. The one rule the server, the window and `dash tui` hand a page by
    /// (task rust-tui-app's /arch-review: it was written in each).
    pub fn asked<'a>(
        &self,
        (path, query, html): (&'a str, &'a str, bool),
        (board, registry): (Option<&'a dyn Any>, Option<&'a dyn Any>),
        host: &'a dyn Host,
    ) -> Asked<'a> {
        let held = match self.reads {
            Reads::Board => board,
            Reads::View => None,
            Reads::Nothing => registry,
        };
        Asked {
            path,
            query,
            html,
            held,
            host,
        }
    }
}

/// A page's or a post's request: its path and query, the form asked, what its thread holds, and the host.
pub struct Asked<'a> {
    pub path: &'a str,
    pub query: &'a str,
    pub html: bool,
    /// What the host hands the page, when this process holds it: the board for a page whose `reads` names it, else the
    /// registry. A host-side type the sdk does not know, read back by dash-core's `held`, never by a plugin (ARCHITECTURE.md section 6 B8: a View's rows replace it).
    pub held: Option<&'a dyn Any>,
    pub host: &'a dyn Host,
}

impl Asked<'_> {
    pub fn param(&self, name: &str) -> Option<String> {
        param(self.query, name)
    }

    /// The path's last segment decoded, a `/*` page's id. A path keeps its + (only a form's + is a space; /code-review of
    /// #1197).
    pub fn id(&self) -> String {
        segment_decoded(&self.path[self.path.rfind('/').map_or(0, |i| i + 1)..])
    }

    /// The drawing, for the host to draw in the form asked; `None`: no board in this process.
    pub fn form(&self, read: Option<Result<Drawing, String>>) -> Answer {
        match read {
            Some(Ok(d)) => Answer::Drawn(d),
            Some(Err(e)) => self.failed(e),
            None => Answer::Body("404 Not Found", TEXT, "no port, no page".into()),
        }
    }

    pub fn failed(&self, e: String) -> Answer {
        Answer::Body("500 Internal Server Error", TEXT, e)
    }

    /// The logins' folder: no home is no login known, said on the page while the server stays up.
    pub fn home(&self) -> Result<PathBuf, Answer> {
        self.host.home().map_err(|e| self.failed(e))
    }

    /// The server's data folder.
    pub fn data(&self) -> PathBuf {
        self.host.data()
    }
}

/// A command, registered from its own module with `inventory::submit! { Command { .. } }`; `dash help` prints every usage
/// line sorted, and one with `run` needs no arm in the binary's match.
pub struct Command {
    /// One usage line, `dash <words> <arguments>`.
    pub usage: &'static str,
    /// The words naming it after `dash`, given with `run`.
    pub words: &'static [&'static str],
    /// What answers the arguments after its words, the host handed in; none while the binary's match answers it.
    // ponytail: a writer, never a file (the sdk reads none), so a command handing its output to a child process stays an
    // arm of the binary's match (`dash doctor`); a host method that runs a child is the upgrade when a plugin needs one.
    pub run: Option<Run>,
}

/// A command's answer to its arguments, the host handed in, its lines written to the writer.
pub type Run = fn(&dyn Host, &[&str], &mut dyn Write);

impl Command {
    pub const fn usage(usage: &'static str) -> Command {
        Command {
            usage,
            words: &[],
            run: None,
        }
    }
}

inventory::collect!(Command);

/// A slot's line: a part of another page, as a line in the daily's « today » (rfc-rust-plugins-ui section 3, the Slot port).
/// What it counts and where it opens are known before its count, so a line still reading is its card in its loading state,
/// never a line of text a count replaces (task rust-today-doc; @nha, 2026-11-09: « This is a card loading state. same element. »).
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    /// What it says, its count first: « 3 to send ». `Ok(None)`: its first read has not ended, so no count yet, never a zero;
    /// `Err`: why it has none.
    pub says: Result<Option<String>, String>,
    /// What it counts, in a few words.
    pub what: String,
    /// The list it opens.
    pub href: String,
}

/// A plugin's part of another page, registered under `<page>/<order>-<plugin>` (« today/10-drafts »): the page reads every
/// key under its name in key order, never a plugin by name. ARCHITECTURE.md section 6 B10 folds it into `Element`.
pub trait Slot: Send + Sync {
    fn line(&self) -> Line;
}

/// One segment of a path decoded: its + stays a plus, as only a form's + is a space (/code-review of #1197).
pub fn segment_decoded(segment: &str) -> String {
    form_decoded(&segment.replace('+', "%2B"))
}

/// The query's first `name=` value, decoded as a form sends it.
pub fn param(query: &str, name: &str) -> Option<String> {
    let prefix = format!("{name}=");
    query
        .split('&')
        .find_map(|pair| pair.strip_prefix(&prefix))
        .map(form_decoded)
}

/// `+` and `%XX` as a form sends them; a bad escape stays as it is.
pub fn form_decoded(text: &str) -> String {
    let bytes = text.as_bytes();
    let (mut out, mut i) = (Vec::with_capacity(bytes.len()), 0);
    while i < bytes.len() {
        let hex = bytes
            .get(i + 1..i + 3)
            .filter(|h| h.iter().all(u8::is_ascii_hexdigit)) // from_str_radix alone takes a leading +
            .and_then(|h| std::str::from_utf8(h).ok());
        match (bytes[i], hex.and_then(|h| u8::from_str_radix(h, 16).ok())) {
            (b'%', Some(byte)) => {
                out.push(byte);
                i += 3;
                continue;
            }
            (b'+', _) => out.push(b' '),
            (byte, _) => out.push(byte),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::StandIn;
    use dash_meter::dash_test;
    use serde_json::Value;

    inventory::submit! { ContextProvider { key: "test/echo", lines: |_, about| Ok(vec![about.into()]) } }
    inventory::submit! { ContextProvider { key: "test/twice", lines: |_, _| Ok(vec![]) } }
    inventory::submit! { ContextProvider { key: "test/twice", lines: |_, _| Ok(vec![]) } }

    inventory::submit! { Channel { key: "test/chan", checks: |_, m| vec![Check { name: "to", fails: m.to.is_empty().then(|| "nobody".into()), retry: false }], send: |_, m| Ok(m.key.clone()) } }
    inventory::submit! { Element { key: "test/page/20-b", draw: |_, _, id, _| Ok(vec![format!("{id}-b")]) } }
    inventory::submit! { Element { key: "test/page/10-a", draw: |_, _, id, _| Ok(vec![format!("{id}-a")]) } }
    inventory::submit! { Post { path: "/test/send/*", step: "route_test_send", serve: |a, form| Answer::Body("200 OK", TEXT, format!("{} {form}", a.id())) } }

    fn asked(host: &StandIn) -> Asked<'_> {
        Asked {
            path: "/tasks/a+b%20c",
            query: "show=x%2By&view=text",
            html: false,
            held: None,
            host,
        }
    }

    /// Done-when 1 of draft-from-is-address, the sdk's half: a channel and an element are read by key, a page's placed
    /// elements in key order, and a post by its path or its prefix's `/*`.
    #[dash_test]
    fn channels_elements_and_posts_are_read_by_key() {
        let host = StandIn::default();
        let chan = channel("test/chan").expect("registered");
        let m = Message {
            key: "k".into(),
            ..Message::default()
        };
        assert_eq!((chan.checks)(&host, &m)[0].fails.as_deref(), Some("nobody"));
        assert_eq!((chan.send)(&host, &m), Ok("k".into()));
        assert_eq!(
            channel("test/none").err(),
            Some("no plugin provides test/none".into())
        );
        let keys: Vec<&str> = placed("test/page").iter().map(|e| e.key).collect();
        assert_eq!(keys, ["test/page/10-a", "test/page/20-b"]);
        assert!(placed("test/pag").is_empty(), "a place is a whole segment");
        let drawn = (element("test/page/10-a").expect("one").draw)(
            &host,
            &Value::Null,
            "p",
            &mut Tree::default(),
        );
        assert_eq!(drawn, Ok(vec!["p-a".to_string()]));
        let a = Asked {
            path: "/test/send/d1",
            ..asked(&host)
        };
        let Some(Answer::Body(_, _, said)) = post("/test/send/d1").map(|p| (p.serve)(&a, "x=1"))
        else {
            panic!("the post answers")
        };
        assert_eq!(said, "d1 x=1");
        assert!(post("/test/send/d1/more").is_none() && post("/test/send").is_none());
        assert!(host.signed_in("a@b").is_err() && host.after(1, Box::new(|_| {})).is_err());
    }

    /// A key no plugin provides, or two provide, is said; one provider answers.
    #[dash_test]
    fn a_context_key_reads_its_one_provider() {
        let host = StandIn::default();
        assert_eq!(context(&host, "test/echo", "x"), Ok(vec!["x".to_string()]));
        assert_eq!(
            context(&host, "test/none", "x"),
            Err("no plugin provides test/none".into())
        );
        assert_eq!(
            context(&host, "test/twice", "x"),
            Err("test/twice is provided twice".into())
        );
    }

    /// A tree's element keeps its type, its props and its children; a null props is none.
    #[dash_test]
    fn a_tree_built_on_the_sdk_is_the_catalog_s_json() {
        let mut tree = Tree::default();
        tree.add("page", "stack", Value::Null, &["t".into()]);
        tree.add("t", "heading", serde_json::json!({"text": "Tasks"}), &[]);
        let Drawing::Tree { title, tree } = tree.drawing("Tasks", "page") else {
            panic!("a tree")
        };
        let tree: Value = serde_json::from_str(&tree).expect("json");
        assert_eq!(title, "Tasks");
        assert_eq!(tree["root"], "page");
        assert_eq!(
            tree["elements"]["page"],
            serde_json::json!({"type": "stack", "children": ["t"], "anchor": "page"})
        );
        assert_eq!(tree["elements"]["t"]["props"]["text"], "Tasks");
    }

    /// Done-when 3 of rust-move-core-ports: what a page reads of the world comes from the host it was handed, its id and
    /// params from its request.
    #[dash_test]
    fn a_page_reads_the_world_through_its_host() {
        let host = StandIn::default();
        let a = asked(&host);
        assert_eq!(a.data(), PathBuf::from("/data"));
        assert!(
            matches!(a.home(), Err(Answer::Body(s, _, e)) if s.starts_with("500") && e == "HOME: unset")
        );
        assert_eq!(a.id(), "a+b c", "a path keeps its +");
        assert_eq!(a.param("show").as_deref(), Some("x+y"));
        assert!(matches!(a.form(None), Answer::Body(s, ..) if s.starts_with("404")));
        let drawn = a.form(Some(Ok(Drawing::Page("p".into()))));
        assert!(matches!(drawn, Answer::Drawn(Drawing::Page(p)) if p == "p"));
        assert!(
            matches!(("308 Permanent Redirect", TEXT, "/x".to_string()).into(), Answer::Body(s, _, b) if s.starts_with("308") && b == "/x")
        );
    }
}
