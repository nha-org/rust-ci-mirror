//! Adapter of section 3: what the app shows outside. For now the web renderer of an extension's UI tree and the MCP server
//! of the registry's tools. Hand-written JSON-RPC: rmcp would bring an async runtime for four methods. The later Mac window
//! draws the same HTML and SVG, so a screen is a tree here and never markup in the crate that fills it.
use dash_core::registry::Registry;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, Read, Write};

mod board;
pub mod components;
pub mod design;
pub mod documents;
use components::card_href;
use documents::IssueStatus;
pub mod forms;
pub mod pdf;
pub mod screen;
pub mod words;

use forms::escaped;

/// Section 10's tree: a flat map of elements by id. Parsing it is the check: a bad type or prop is refused here.
#[derive(Deserialize)]
struct Tree {
    root: String,
    elements: HashMap<String, Node>,
}

#[derive(Deserialize)]
struct Node {
    #[serde(flatten)]
    element: Element,
    #[serde(default)]
    children: Vec<String>,
}

/// Section 12's catalog, a closed set: a new element type is a rebuild, a new tree of these is not. Props are meaning,
/// not markup: a plugin gives text, always escaped, and chooses no URL.
#[derive(Deserialize)]
#[serde(tag = "type", content = "props", rename_all = "kebab-case")]
enum Element {
    Stack,
    /// The search field (task rust-inbox-search-place): it GETs /search. `value`: the words searched, shown back in it;
    /// `hidden`: fields sent with them, as /inbox's `in=inbox`, which names the page that draws the results.
    SearchBar {
        placeholder: String,
        #[serde(default)]
        value: String,
        #[serde(default)]
        hidden: Vec<(String, String)>,
    },
    Heading {
        text: String,
    },
    Text {
        text: String,
    },
    TaskGraph {
        #[serde(deserialize_with = "drawn")]
        tasks: Vec<GraphTask>,
    },
    /// One status's tasks under its name and their count, its children their rows.
    IssueGroup {
        status: IssueStatus,
        /// Its own name where the status's is not: /comments' columns (task comments-by-state).
        #[serde(default)]
        title: Option<String>,
    },
    /// A task's row: the whole of it opens the task. `says`: what it waits on or how it stands. `comments`: his, that it handles.
    IssueRow {
        id: String,
        title: String,
        status: IssueStatus,
        says: String,
        comments: usize,
        /// Another project's task, which only the older app's board opens (task rust-nav-polish); `None`: one Rust's
        /// /tasks/<id> opens.
        #[serde(default)]
        project: Option<String>,
        /// Its labels and who it is assigned to, as a Linear card shows them on the board (task rust-board-linear).
        #[serde(default)]
        labels: Vec<String>,
        #[serde(default)]
        assignee: Option<String>,
        /// The board's drag moves it to another column (task rust-board-linear-order): a task Rust's board view places.
        #[serde(default)]
        movable: bool,
        /// Where it opens when that is not its task: a comment with no task opens on its page (task comments-by-state).
        #[serde(default)]
        href: Option<String>,
    },
    /// Cards side by side, as many to a row as fit: its children the cards. Only where a card was approved (drafts): a list
    /// of things is a `list` (@nha, 2026-11-09: « Cards are not the default UI element »; dash-gate's cards_only_where_approved).
    Cards,
    /// The default list: its children's cards drawn as rows, one under the other, each its title, its line and its link.
    List,
    /// One thing to read or do, Home's unit (task rust-nav-polish; @nha, 2026-11-08: « initial contents should be though
    /// about »): its title, one line under it, and where it opens. `href`: a path of this app or an http(s) URL, else none.
    /// `reading`: its title not known yet, the card in its loading state, the same element its title fills (task
    /// rust-today-doc; @nha, 2026-11-09: « This is a card loading state. same element. always start from the design system »).
    Card {
        title: String,
        line: String,
        #[serde(default)]
        href: Option<String>,
        #[serde(default)]
        reading: bool,
    },
    /// The board: its children the status groups, drawn as columns side by side. `view`: the views' name its cards follow
    /// (dash_views), so the page takes each of its diffs as a patch (task rust-view-patches).
    Board {
        #[serde(default)]
        view: Option<String>,
    },
    /// A queue or a list of records, a row each (components::Table). A row's `href` is checked as a card's.
    Table {
        columns: Vec<String>,
        rows: Vec<components::TableRow>,
    },
    /// Links in a row, the one shown marked: an account switcher, a pager. Navigation is links, never cards (@nha, 2026-11-09,
    /// on /mail: « cards are nonsense here »). An href is checked as a card's.
    Links {
        links: Vec<Link>,
    },
    /// A file sent to a path of this app: its picker, then its button (task dash-settings-backup). `action` is a local path.
    Upload {
        action: String,
        label: String,
        accept: String,
    },
    /// One button that posts to a path of this app: a step he confirms (task dash-settings-backup).
    Action {
        action: String,
        label: String,
    },
    /// The board's three views as links, the one shown marked: the board, the list, the dependency graph.
    TaskViews {
        current: TaskView,
        /// The project the board and the list are filtered to, kept by their links (task rust-nav-polish).
        #[serde(default)]
        project: Option<String>,
    },
    /// A titled part of a page, its children the part.
    Section {
        title: String,
        /// For agents alone: its text form whole, nothing of it drawn for him (task rust-tasks-runs-text-off-html; @nha,
        /// 2026-11-09: « that whole text is what ? for the agent ? don't show on the human pane »).
        #[serde(default)]
        agents: bool,
    },
    /// Someone's words as written, their line breaks kept.
    Quote {
        text: String,
    },
    /// A diagram, its text form its Mermaid source, which a browser draws (components::Diagram).
    Diagram {
        source: String,
    },
    /// One mail of a conversation (task rust-mail-thread): folded to its line unless `open`, then its words, or its HTML
    /// framed with no script and nothing remote (components::Frame). `files`: each attachment's name and where it downloads.
    Mail {
        from: String,
        to: String,
        at: String,
        text: String,
        #[serde(default)]
        html: Option<String>,
        #[serde(default)]
        open: bool,
        #[serde(default)]
        files: Vec<(String, String)>,
    },
    /// One conversation of /mail's list (components::ThreadRow, /design?tab=mail's thread row): its sender, subject,
    /// snippet, date and whether it is unread, and where it opens, checked as a card's (task rust-mail-design-parts).
    ThreadRow {
        from: String,
        subject: String,
        snippet: String,
        at: String,
        unread: bool,
        #[serde(default)]
        href: Option<String>,
    },
    /// His words and one button that posts them to `action`, a path of this app, with the `hidden` fields: a thread's
    /// reply box.
    Compose {
        action: String,
        send: String,
        placeholder: String,
        #[serde(default)]
        hidden: Vec<(String, String)>,
    },
    /// One access on its provider's card, its status a chip before its name (task rust-tools-human; @nha, 2026-11-09:
    /// « /tools unreadable for a human. »): in the text, one record line, its fields a tab apart, the table kept. `source`: where it was
    /// found (bin/secret, a CLI's config, tfvars, GitHub), its place's label in the HTML and the text's last field.
    Access {
        name: String,
        status: String,
        usage: String,
        note: String,
        place: String,
        source: String,
    },
    /// A document, drawn by its type's template (task rust-doc-types): the page gives its fields, never its layout.
    Document(documents::Doc),
    /// A document of the one model drawn by the one component in a view (rust/CLAUDE.md rule 7, task rust-one-doc-component):
    /// `doc` as canvist saves it, `anchors` the page's comments on its words.
    DocView {
        doc: serde_json::Value,
        #[serde(default)]
        view: documents::View,
        #[serde(default)]
        anchors: Vec<documents::Anchor>,
    },
    /// A tool's card, its logo and name, its children its accesses (task rust-tools-themes). `reading`: how many accesses its
    /// provider has yet to answer for, drawn as grey lines in a greyed card; 0 once it has.
    Tool {
        name: String,
        #[serde(default)]
        reading: usize,
    },
    /// A part of the page that fills itself from the page's URL while a card in it is reading, with no reload.
    Live,
    /// A section whose data is still being read: grey lines, filled in place when it lands (components::Skeleton). Its key is
    /// the filled element's, so the page swaps one for the other; `what` names what is read.
    Skeleton {
        what: String,
    },
}

/// A section still being read, as a tree element: its text form says so, its HTML is grey lines that fill in place (task
/// rust-skeleton-live). A page puts it under the key its filled element will have.
pub fn skeleton(what: &str) -> Value {
    json!({"type": "skeleton", "props": {"what": what}})
}

/// A page's one source (rust/CLAUDE.md rule 4). The server's `drawn` makes it HTML or text: no page picks its own form (task
/// rust-page-forms).
pub enum Drawing {
    /// A UI tree: its HTML the Dioxus component's (`screen::page`), its text `text`'s.
    Tree { title: String, tree: String },
    /// A page a component of its own draws whole, as /design: its text is its words (`forms::text_of`).
    Page(String),
}

/// A record's field in the text: a tab or a line break inside it, as a provider's error text holds, reads as a space, so a
/// record stays one line and its fields a tab apart (/code-review of rust-page-forms).
fn field(text: &str) -> String {
    text.replace(['\t', '\r', '\n'], " ")
}

/// A record's line: its fields a tab apart, the one separator of the text (a name holds « , », « : » and « | »).
fn record<S: AsRef<str>>(fields: &[S]) -> String {
    fields
        .iter()
        .map(|f| field(f.as_ref()))
        .collect::<Vec<_>>()
        .join("\t")
}

/// A view of the open tasks at /tasks (task rust-nav-board; @nha, 2026-11-08: « /tasks should have the board with the board
/// view too by default »). `/graph` answers as the graph's own URL.
#[derive(Clone, Copy, Deserialize, serde::Serialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum TaskView {
    Board,
    List,
    Graph,
}

impl TaskView {
    pub const ALL: [TaskView; 3] = [TaskView::Board, TaskView::List, TaskView::Graph];

    /// `?layout=`'s word, `None` for anything else: the board.
    pub fn of(layout: Option<&str>) -> TaskView {
        match layout {
            Some("list") => TaskView::List,
            Some("graph") => TaskView::Graph,
            _ => TaskView::Board,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            TaskView::Board => "Board",
            TaskView::List => "List",
            TaskView::Graph => "Graph",
        }
    }

    /// Its icon in the design system's set, beside its name in the views' toggle.
    pub(crate) fn icon(self) -> &'static str {
        use crate::design::families as i;
        match self {
            TaskView::Board => i::BOARD,
            TaskView::List => i::LIST,
            TaskView::Graph => i::GRAPH,
        }
    }

    /// Its URL, the board and the list kept to `project`; the graph is Rust's plan, whole.
    pub fn href(self, project: Option<&str>) -> String {
        let (path, on) = match self {
            TaskView::Board => ("/tasks", true),
            TaskView::List => ("/tasks?layout=list", true),
            TaskView::Graph => ("/tasks?layout=graph", false),
        };
        match project.filter(|_| on) {
            Some(p) => format!(
                "{path}{}project={}",
                if path.contains('?') { '&' } else { '?' },
                segment(p)
            ),
            None => path.into(),
        }
    }
}

/// A card's line in a text form: its title, its line, where it opens.
fn card_text(title: &str, line: &str, href: Option<&str>) -> String {
    // a record as every other: its fields a tab apart, an empty one left out (task rust-page-forms)
    let fields: Vec<&str> = [Some(title), Some(line), href]
        .into_iter()
        .flatten()
        .filter(|f| !f.is_empty())
        .collect();
    record(&fields)
}

/// The views' line in a text form: each name and URL, the one shown marked.
/// One link of a `links` element.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Link {
    pub name: String,
    pub href: String,
    #[serde(default)]
    pub current: bool,
}

fn views_text(current: TaskView, project: Option<&str>) -> String {
    let each = TaskView::ALL.map(|v| {
        let shown = if v == current { " (shown)" } else { "" };
        format!("{} {}{shown}", v.name(), v.href(project))
    });
    format!("Views: {}", each.join(", "))
}

/// Words as one URL segment: a plugin gives words, and only this crate makes a URL of them.
fn segment(words: &str) -> String {
    let mut path = String::new();
    for byte in words.bytes() {
        match byte {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                path.push(byte as char);
            }
            _ => path += &format!("%{byte:02X}"),
        }
    }
    path
}

/// A task's page: Rust's own, or another project's on the older app's board.
pub fn task_href(id: &str, project: Option<&str>) -> String {
    match project {
        None => format!("/tasks/{}", segment(id)),
        Some(p) => format!("/projects/{}?issue={}", segment(p), segment(id)),
    }
}

/// A task in a `task-graph`. `says`: one line on its state, the why of a waiting one. `waits`: ids in the same graph.
#[derive(Deserialize)]
struct GraphTask {
    id: String,
    state: TaskState,
    says: String,
    #[serde(default)]
    waits: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
enum TaskState {
    Next,
    Running,
    Waiting,
    /// Merged: a wait on it is met.
    Finished,
    /// Finished any other way: it holds what waits on it.
    Ended,
}

/// A graph's tasks as every form draws them: a merged one left out with each wait on it, met, so the page shows what is still to
/// do (task rust-graph-queue-edges). An ended one stays: it holds what waits on it. Done here, as the tree is read, so the check,
/// the HTML, the window and the text agree.
fn drawn<'de, D: serde::Deserializer<'de>>(from: D) -> Result<Vec<GraphTask>, D::Error> {
    let mut tasks = Vec::<GraphTask>::deserialize(from)?;
    let merged: HashSet<String> = (tasks.iter())
        .filter(|t| matches!(t.state, TaskState::Finished))
        .map(|t| t.id.clone())
        .collect();
    tasks.retain(|t| !merged.contains(&t.id));
    for task in &mut tasks {
        task.waits.retain(|w| !merged.contains(w));
    }
    Ok(tasks)
}

impl TaskState {
    fn class(&self) -> &'static str {
        match self {
            TaskState::Next => "next",
            TaskState::Running => "running",
            TaskState::Waiting => "waiting",
            TaskState::Finished => "finished",
            TaskState::Ended => "ended",
        }
    }
}

/// How deep a tree may nest. The flat map parses at any depth, and a deep chain would overflow `walk`'s, `told`'s or
/// `screen::node`'s stack, all run after `check`: an abort no
/// `catch_unwind` stops.
const DEPTH_LIMIT: usize = 64;

/// Tasks one graph may draw: the 50 open ones the cap allows, what the older app offers beside them, and what they name. It keeps
/// `columns`' scan, quadratic, small whatever a tree says. A screen past it says so in place of its graph: a refused tree loses the page.
pub const GRAPH_LIMIT: usize = 500;

const NODE_WIDTH: usize = 300;
const NODE_HEIGHT: usize = 52;
const GAP_X: usize = 60;
const GAP_Y: usize = 16;
const PAD: usize = 8;
const LANE: usize = 8;
/// Room left and right of the columns: a loop's arrow leaves the last one and enters the first through it.
const MARGIN_X: usize = GAP_X;
/// The x's a long arrow may take in a gap, from 8 in, past the gap's middle, where curves bend.
const LANE_XS: usize = 43;

/// Each task's column: one past the furthest of what it waits on, so an arrow runs left to right. Each pass takes a task whose
/// waits all have a column. A loop has none (a merge of two replicas can make one, and it must not stop the page), so it takes a
/// task on the loop: that task's waits inside the loop then count for nothing, and what only waits on the loop stays right of it.
/// ponytail: rows keep the given order, no crossing reduction, until a real graph reads as a tangle.
/// Answers each task's column and the index of each task it waits on.
fn columns(tasks: &[GraphTask]) -> Result<(Vec<usize>, Vec<Vec<usize>>), String> {
    if tasks.len() > GRAPH_LIMIT {
        return Err(format!(
            "a graph of {} tasks is past {GRAPH_LIMIT}",
            tasks.len()
        ));
    }
    let mut at = HashMap::new();
    for (i, task) in tasks.iter().enumerate() {
        if at.insert(task.id.as_str(), i).is_some() {
            return Err(format!("{} appears twice", task.id));
        }
    }
    let mut waits = vec![];
    for task in tasks {
        let named = task.waits.iter().map(|w| {
            let found = at.get(w.as_str()).copied();
            found.ok_or(format!("{} waits on {w}, not in the graph", task.id))
        });
        waits.push(named.collect::<Result<Vec<usize>, String>>()?);
    }
    let mut column: Vec<Option<usize>> = vec![None; tasks.len()];
    for _ in 0..tasks.len() {
        let free = |i: &usize| column[*i].is_none();
        let settled = |i: &usize| waits[*i].iter().all(|w| column[*w].is_some());
        let next = (0..tasks.len()).filter(free).find(settled);
        let next = next.or((0..tasks.len())
            .find(free)
            .map(|i| on_a_loop(i, &waits, &column)));
        let Some(next) = next else { break };
        let after = waits[next].iter().filter_map(|w| column[*w]).max();
        column[next] = Some(after.map_or(0, |c| c + 1));
    }
    Ok((column.into_iter().flatten().collect(), waits))
}

/// From a free task, when none is settled: each free task has a free wait, so the walk repeats, and where it repeats is on a loop.
fn on_a_loop(from: usize, waits: &[Vec<usize>], column: &[Option<usize>]) -> usize {
    let mut been = HashSet::new();
    let mut at = from;
    while been.insert(at) {
        let Some(wait) = waits[at].iter().find(|w| column[**w].is_none()) else {
            break;
        };
        at = *wait;
    }
    at
}

/// The graph: an SVG of the arrows under HTML nodes, each size here and each colour in page.css, so it needs no script. Words are
/// HTML as in ui-spike's diagram.rs: Blitz draws an inline svg as one image with no font for its text, and a reader reads a node.
fn graph_svg(dom_id: &str, tasks: &[GraphTask]) -> Result<String, String> {
    let (column, waits) = columns(tasks)?;
    let mut used = vec![0; column.iter().max().map_or(0, |c| c + 1)];
    let mut spots = vec![(0, 0); tasks.len()];
    // What runs heads its column, the rest in the given order: a reader finds the live work first.
    let mut order: Vec<usize> = (0..tasks.len()).collect();
    order.sort_by_key(|&i| !matches!(tasks[i].state, TaskState::Running));
    for i in order {
        let c = column[i];
        spots[i] = (
            MARGIN_X + c * (NODE_WIDTH + GAP_X),
            PAD + used[c] * (NODE_HEIGHT + GAP_Y),
        );
        used[c] += 1;
    }
    let width = MARGIN_X * 2 + (used.len() * (NODE_WIDTH + GAP_X)).saturating_sub(GAP_X);
    let below = PAD + used.iter().max().unwrap_or(&0) * (NODE_HEIGHT + GAP_Y);
    let long = |i: usize, from: usize| column[i] != column[from] + 1;
    let long_arrows: usize = (waits.iter().enumerate())
        .map(|(i, w)| w.iter().filter(|from| long(i, **from)).count())
        .sum();
    let mut lanes = 0;
    let mut edges = String::new();
    let mut nodes = String::new();
    for (i, task) in tasks.iter().enumerate() {
        let (x, y) = spots[i];
        let (id, says) = (escaped(&task.id), escaped(&task.says));
        for &from in &waits[i] {
            let (x1, y1) = (spots[from].0 + NODE_WIDTH, spots[from].1 + NODE_HEIGHT / 2);
            let (x2, y2) = (x, y + NODE_HEIGHT / 2);
            // To the next column, a curve through the gap. Any further, or back round a loop, it would run behind a node and read
            // as feeding it: it goes down to a lane of its own under them all, and through each gap on an x of its own, none the
            // gap's middle, where curves bend: on one shared line, two arrows would read as one.
            // ponytail: x's shared by the whole graph, so past LANE_XS long arrows two may share one; per gap if a real graph gets there.
            let line = if !long(i, from) {
                let bend = x1.midpoint(x2);
                format!("C{bend} {y1} {bend} {y2} {x2} {y2}")
            } else {
                let lane = below + lanes * LANE;
                let off = 8 + lanes * LANE_XS / long_arrows;
                let off = if off < GAP_X / 2 { off } else { off + 2 };
                lanes += 1;
                let (out, into) = (x1 + off, x2 - off);
                format!("H{out}V{lane}H{into}V{y2}H{x2}")
            };
            edges += &format!(
                r##"<path class="edge" data-from="{}" data-to="{id}" d="M{x1} {y1}{line}" marker-end="url(#graph-arrow)"/>"##,
                escaped(&tasks[from].id)
            );
        }
        nodes += &format!(
            r#"<div class="node {}" data-task="{id}" style="left:{x}px;top:{y}px;width:{NODE_WIDTH}px;height:{NODE_HEIGHT}px" title="{id}: {says}"><div class="name">{id}</div><div class="says">{says}</div></div>"#,
            task.state.class(),
        );
    }
    let height = below + lanes * LANE + PAD;
    Ok(format!(
        r#"<div id="{dom_id}" class="task-graph"><div class="canvas" style="width:{width}px;height:{height}px"><svg aria-hidden="true" width="{width}" height="{height}" viewBox="0 0 {width} {height}"><defs><marker id="graph-arrow" viewBox="0 0 8 8" refX="8" refY="4" markerWidth="8" markerHeight="8" orient="auto"><path class="arrow" d="M0 0L8 4L0 8z"/></marker></defs>{edges}</svg>{nodes}</div></div>"#
    ))
}

/// The element `id` names, once each walk checks it: never nested past `DEPTH_LIMIT`, never seen twice. `walk` and `told`
/// share it, so the page and the text refuse the same trees.
fn checked<'a>(
    tree: &'a Tree,
    id: &'a str,
    seen: &mut HashSet<&'a str>,
    depth: usize,
) -> Result<&'a Node, String> {
    if depth > DEPTH_LIMIT {
        return Err(format!("{id} is nested past {DEPTH_LIMIT} levels"));
    }
    if !seen.insert(id) {
        return Err(format!("{id} appears twice"));
    }
    let node = tree.elements.get(id).ok_or(format!("no element {id}"))?;
    // an access's HTML is its fields alone: a child would show in the text only
    if matches!(node.element, Element::Access { .. }) && !node.children.is_empty() {
        return Err(format!("{id}: an access holds no children"));
    }
    // a mail and a reply box draw their props alone; a reply goes to a path of this app only
    if matches!(
        node.element,
        Element::Mail { .. } | Element::Compose { .. } | Element::ThreadRow { .. }
    ) && !node.children.is_empty()
    {
        return Err(format!(
            "{id}: a mail, a thread row or a reply box holds no children"
        ));
    }
    if let Element::Compose { action, .. } = &node.element
        && (!action.starts_with('/') || action.starts_with("//") || action.contains('\\'))
    {
        return Err(format!("{id}: a reply box posts to a path of this app"));
    }
    if matches!(node.element, Element::DocView { .. }) && !node.children.is_empty() {
        return Err(format!("{id}: a document holds no children"));
    }
    // a document's template lays out its fields alone
    if let Element::Document(doc) = &node.element {
        if !node.children.is_empty() {
            return Err(format!("{id}: a document holds no children"));
        }
        doc.check(id)?;
    }
    // a table draws its rows alone, each as many cells as it has columns
    if let Element::Table { columns, rows } = &node.element {
        if !node.children.is_empty() {
            return Err(format!("{id}: a table holds no children"));
        }
        if let Some(row) = rows.iter().find(|r| r.cells.len() != columns.len()) {
            return Err(format!(
                "{id}: a row of {} cells under {} columns",
                row.cells.len(),
                columns.len()
            ));
        }
    }
    // a reading card's HTML is its grey lines alone: an access under it would show in the text only
    if matches!(node.element, Element::Skeleton { .. }) && !node.children.is_empty() {
        return Err(format!(
            "{id}: a skeleton holds no children: its grey lines stand for them"
        ));
    }
    if matches!(node.element, Element::Tool { reading: 1.., .. }) && !node.children.is_empty() {
        return Err(format!("{id}: a reading tool holds no accesses yet"));
    }
    Ok(node)
}

/// The walk that checks the tree under `id` whole: `seen` refuses a cycle, which would never end, and an element with two
/// parents, which would repeat its id; a graph past its limit, with an id twice or a wait on no task, by `columns`. It
/// writes no HTML: a page's is `screen::page`'s, drawn
/// with the design system's components (task rust-line-cap-main cut the hand-written copy).
fn walk<'a>(
    tree: &'a Tree,
    id: &'a str,
    seen: &mut HashSet<&'a str>,
    depth: usize,
) -> Result<(), String> {
    let node = checked(tree, id, seen, depth)?;
    for child in &node.children {
        walk(tree, child, seen, depth + 1)?;
    }
    if let Element::TaskGraph { tasks } = &node.element {
        columns(tasks)?;
    }
    Ok(())
}

/// A UI tree given as JSON text, checked whole: what `screen::page` draws once this passes.
fn check(tree: &str) -> Result<Tree, String> {
    let tree: Tree = serde_json::from_str(tree).map_err(|e| e.to_string())?;
    walk(&tree, &tree.root, &mut HashSet::new(), 0)?;
    Ok(tree)
}

/// Whether a UI tree can be drawn: its shape is checked, nothing of it returned.
pub fn check_tree(tree: &str) -> Result<(), String> {
    check(tree).map(|_| ())
}

/// The text of a UI tree, for an agent: drawn from the same tree as the HTML, never written twice (rust/CLAUDE.md point 4;
/// @nha, 2026-11-08: « don't write twice obviously, make sure there is say html->text. »). A line an element in reading order,
/// plain: a part's title after a blank line, a record's fields a tab apart. The one text every page answers (task
/// rust-page-forms).
pub fn text(tree: &str) -> Result<String, String> {
    let tree: Tree = serde_json::from_str(tree).map_err(|e| e.to_string())?;
    let mut lines = vec![];
    told(&tree, &tree.root, &mut HashSet::new(), 0, &mut lines)?;
    Ok(lines.join("\n") + "\n")
}

fn told<'a>(
    tree: &'a Tree,
    id: &'a str,
    seen: &mut HashSet<&'a str>,
    depth: usize,
    lines: &mut Vec<String>,
) -> Result<(), String> {
    let node = checked(tree, id, seen, depth)?;
    match &node.element {
        // the words searched, as the HTML's field shows them back (/code-review of rust-inbox-search-place)
        Element::SearchBar { value, .. } if !value.is_empty() => {
            lines.push(format!("Search: {value}"))
        }
        Element::Stack
        | Element::SearchBar { .. }
        | Element::Board { .. }
        | Element::Cards
        | Element::List
        | Element::Live => {}
        // a card with no name yet (a part that knows no tool) says nothing an agent could use (/code-review of rust-tools-themes)
        Element::Tool { name, .. } if name.is_empty() => {}
        Element::Tool { name, reading: 0 } => lines.push(format!("\n{name}")),
        Element::Tool { name, reading } => lines.push(format!("\n{name}\t{reading} reading")),
        Element::Skeleton { what } => lines.push(format!("{what}: being read.")),
        Element::Card {
            line,
            href,
            reading: true,
            ..
        } => lines.push(card_text("being read", line, card_href(href))),
        Element::Card {
            title, line, href, ..
        } => lines.push(card_text(title, line, card_href(href))),
        // its head, then a record a row, its link last as a card's, empty when it has none: every record one shape
        Element::Table { columns, rows } => {
            lines.push(record(&[&columns[..], &["Link".into()]].concat()));
            lines.extend(rows.iter().map(|r| {
                let link = card_href(&r.href).unwrap_or_default().to_string();
                record(&[&r.cells[..], &[link]].concat())
            }))
        }
        Element::TaskViews { current, project } => {
            lines.push(views_text(*current, project.as_deref()))
        }
        Element::Upload { action, label, .. } | Element::Action { action, label } => {
            lines.push(format!("{label}: POST {action}"))
        }
        Element::Links { links } => {
            let each = links.iter().map(|l| {
                format!(
                    "{} {}{}",
                    l.name,
                    l.href,
                    if l.current { " (shown)" } else { "" }
                )
            });
            lines.push(each.collect::<Vec<_>>().join(", "))
        }
        Element::Heading { text } | Element::Text { text } | Element::Quote { text } => {
            lines.push(text.clone())
        }
        Element::Diagram { source } => lines.push(format!("Mermaid:\n{}", source.trim_end())),
        Element::TaskGraph { tasks } => {
            // refused as the HTML refuses it: a wait on a task not in the graph
            columns(tasks)?;
            lines.extend(tasks.iter().map(|t| {
                let mut fields = vec![t.id.clone(), t.state.class().into(), t.says.clone()];
                if !t.waits.is_empty() {
                    fields.push(format!("waits on {}", t.waits.join(", ")));
                }
                record(&fields)
            }))
        }
        Element::IssueGroup { status, title } => {
            let name = title.as_deref().unwrap_or(status.name());
            lines.push(format!("\n{name} ({})", node.children.len()))
        }
        Element::IssueRow {
            id,
            title,
            says,
            comments,
            labels,
            assignee,
            href,
            ..
        } => {
            let mut fields = vec![id.clone(), title.clone(), says.clone()];
            if *comments > 0 {
                fields.push(format!("{comments} comments"));
            }
            fields.extend(labels.iter().map(|l| format!("#{l}")));
            fields.extend(assignee.iter().map(|a| format!("@{a}")));
            // where it opens when that is not its task: an agent follows it as he does (task comments-by-state)
            fields.extend(href.iter().cloned());
            lines.push(record(&fields))
        }
        Element::Section { title, .. } => lines.push(format!("\n{title}")),
        Element::Access {
            name,
            status,
            usage,
            note,
            place,
            source,
        } => lines.push(record(&[name, status, usage, note, place, source])),
        Element::Document(doc) => {
            lines.extend(documents::forms(id, doc).1.lines().map(String::from))
        }
        Element::DocView { doc, view, anchors } => {
            let text = documents::anchored_forms(doc.clone(), *view, anchors.clone()).1;
            lines.extend(text.lines().map(String::from))
        }
        // every mail whole, folded or not: an agent reads the conversation, his HTML's words when it has no text
        Element::Mail {
            from,
            to,
            at,
            text,
            html,
            files,
            ..
        } => {
            lines.push(format!("\n{from} · {at} · to {to}"));
            let words = match html {
                Some(html) if text.trim().is_empty() => forms::text_of(html),
                _ => text.clone(),
            };
            lines.extend(words.trim_end().lines().map(String::from));
            lines.extend(files.iter().map(|(name, href)| record(&[name, href])));
        }
        Element::Compose { action, send, .. } => {
            lines.push(format!("\n{send}: POST {action}, his words as text"))
        }
        // a record as a table row's: its fields, then its link
        Element::ThreadRow {
            from,
            subject,
            snippet,
            at,
            unread,
            href,
        } => {
            let read = if *unread { "unread" } else { "read" };
            let link = card_href(href).unwrap_or_default();
            let fields = [from, subject, snippet, at].map(String::as_str);
            lines.push(record(&[&fields[..], &[read, link]].concat()))
        }
    }
    for child in &node.children {
        told(tree, child, seen, depth + 1, lines)?;
    }
    Ok(())
}

/// dash's theme, generated from design.json and kept fresh by ui-spike's own test: one file, so the window and /design match.
const THEME: &str = include_str!("theme.css");
/// The theme is a class on the root, as the window sets it: a browser asks for dark in this one line.
const DARK: &str = "if(matchMedia('(prefers-color-scheme: dark)').matches)document.documentElement.className='app dark'";

fn tools_list(registry: &Registry) -> Value {
    let tools: Vec<Value> = registry
        .tools
        .iter()
        .map(|(name, _, tool)| {
            let schema: Value = serde_json::from_str(tool.input_schema())
                .unwrap_or_else(|_| json!({"type": "object"}));
            json!({"name": name, "description": tool.description(), "inputSchema": schema})
        })
        .collect();
    json!({"tools": tools})
}

/// A tool that fails answers `isError`, as MCP asks, so the agent reads why; an unknown tool is a protocol error.
fn tools_call(registry: &Registry, params: &Value) -> Result<Value, (i64, String)> {
    let name = params["name"]
        .as_str()
        .ok_or((-32602, "no tool name".to_string()))?;
    let tool = registry
        .tools
        .get(name)
        .ok_or((-32602, format!("no tool {name}")))?;
    let arguments = params
        .get("arguments")
        .map_or_else(|| "{}".to_string(), Value::to_string);
    let (text, is_error) = match tool.call(&arguments) {
        Ok(text) => (text, false),
        Err(e) => (e, true),
    };
    Ok(json!({"content": [{"type": "text", "text": text}], "isError": is_error}))
}

fn answer(registry: &Registry, method: &str, params: &Value) -> Result<Value, (i64, String)> {
    match method {
        "initialize" => Ok(json!({
            "protocolVersion": params["protocolVersion"].as_str().unwrap_or("2025-06-18"),
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "dash", "version": env!("CARGO_PKG_VERSION")},
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(tools_list(registry)),
        "tools/call" => tools_call(registry, params),
        other => Err((-32601, format!("no method {other}"))),
    }
}

fn failure(id: &Value, code: i64, text: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": text}})
}

/// The answer to one line, if it asks for one: a notification, or a client's response (we ask nothing), gets none.
fn respond(registry: &Registry, line: &[u8]) -> Option<Value> {
    let message: Value = match serde_json::from_slice(line) {
        Ok(message) => message,
        Err(e) => return Some(failure(&Value::Null, -32700, &e.to_string())),
    };
    if !message.is_object() {
        return Some(failure(&Value::Null, -32600, "a request is an object"));
    }
    let id = message.get("id")?;
    let method = message["method"].as_str()?;
    // Answering copies the id: nested, that copy on top of the parse passes the memory cap. JSON-RPC's id is never nested.
    if id.is_array() || id.is_object() {
        let text = "an id is a string or a number";
        return Some(failure(&Value::Null, -32600, text));
    }
    Some(match answer(registry, method, &message["params"]) {
        Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
        Err((code, text)) => failure(id, code, &text),
    })
}

/// One line's bytes, its newline apart. Small objects nested to the parser's depth, the costliest shape measured, peak at 140 MB
/// for 1 MB of line (4 MB of flat ones passed the memory cap), so this parses under the cap: a test in
/// crates/dash/tests/extensions.rs holds that.
pub const LINE_LIMIT: u64 = 1024 * 1024;

/// MCP over stdio until its input ends: one JSON message a line. Lines are read as bytes: a stray byte is that line's parse
/// error, not the end of the server, and a line past `LINE_LIMIT` is refused the same way, without being held.
pub fn serve_mcp(
    registry: &Registry,
    mut input: impl BufRead,
    mut output: impl Write,
) -> std::io::Result<()> {
    loop {
        let mut line = vec![];
        if (&mut input)
            .take(LINE_LIMIT + 1)
            .read_until(b'\n', &mut line)?
            == 0
        {
            return Ok(());
        }
        let response = if line.len() as u64 > LINE_LIMIT && line.last() != Some(&b'\n') {
            input.skip_until(b'\n')?;
            let text = format!("a line past {LINE_LIMIT} bytes");
            Some(failure(&Value::Null, -32600, &text))
        } else if line.trim_ascii().is_empty() {
            None
        } else {
            respond(registry, &line)
        };
        if let Some(response) = response {
            writeln!(output, "{response}")?;
            output.flush()?;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_core::registry::{Contribution, Tool};
    use dash_meter::memory::TestBudget;

    #[test]
    fn a_tree_renders_escaped_and_a_bad_one_is_refused() {
        let _budget = TestBudget::new();
        let html = screen::page(
            "t",
            r#"{"root":"page","elements":{
                "page":{"type":"stack","children":["bar","hint"]},
                "bar":{"type":"search-bar","props":{"placeholder":"a\"><script>"}},
                "hint":{"type":"text","props":{"text":"<b>&"}}}}"#,
        )
        .unwrap();
        // the whole tree, from the page's style to its end: nothing of it drawn twice or unescaped elsewhere
        assert!(
            html.ends_with(r#"</style><div id="page"><form class="form" id="bar" role="search" method="get" action="/search"><input class="input" name="q" placeholder="a&#34;&#62;&#60;script&#62;" type="search"/><button class="button">Search</button></form><p id="hint">&#60;b&#62;&#38;</p></div></div></body></html>"#),
            "{html}"
        );
        assert!(
            !html.contains(r#""><script>"#) && !html.contains("<b>&"),
            "{html}"
        );
        for (bad, says) in [
            (
                r#"{"root":"a","elements":{"a":{"type":"iframe"}}}"#,
                "iframe",
            ),
            (r#"{"root":"a","elements":{"a":{"type":"text"}}}"#, "props"),
            (
                r#"{"root":"a","elements":{"a":{"type":"text","props":{}}}}"#,
                "text",
            ),
            (r#"{"root":"none","elements":{}}"#, "no element none"),
            (
                r#"{"root":"a","elements":{"a":{"type":"stack","children":["a"]}}}"#,
                "twice",
            ),
            (
                r#"{"root":"a","elements":{"a":{"type":"stack","children":["b","b"]},"b":{"type":"stack"}}}"#,
                "twice",
            ),
            ("not json", "expected"),
        ] {
            let e = check(bad).err().unwrap_or_else(|| panic!("{bad} rendered"));
            assert!(e.contains(says), "{bad}: {e}");
        }
    }

    #[test]
    fn a_tree_nested_too_deep_is_refused() {
        let _budget = TestBudget::new();
        let chain = |depth: usize| -> String {
            let elements: Vec<String> = (0..depth)
                .map(|i| format!(r#""{i}":{{"type":"stack","children":["{}"]}}"#, i + 1))
                .collect();
            format!(
                r#"{{"root":"0","elements":{{{},"{depth}":{{"type":"stack"}}}}}}"#,
                elements.join(",")
            )
        };
        assert!(check(&chain(DEPTH_LIMIT)).is_ok());
        for depth in [DEPTH_LIMIT + 1, 20_000] {
            let e = check(&chain(depth)).err().expect("too deep");
            assert!(e.contains("nested past"), "{e}");
        }
    }

    /// The graph's SVG, once the tree around it is checked.
    fn graph(tasks: &str) -> Result<String, String> {
        let tree = format!(
            r#"{{"root":"g","elements":{{"g":{{"type":"task-graph","props":{{"tasks":[{tasks}]}}}}}}}}"#
        );
        match check(&tree)?.elements["g"].element {
            Element::TaskGraph { ref tasks } => graph_svg("g", tasks),
            _ => unreachable!("a task-graph"),
        }
    }

    fn task(id: &str, waits: &[&str]) -> String {
        let waits = waits
            .iter()
            .map(|w| format!(r#""{w}""#))
            .collect::<Vec<_>>();
        format!(
            r#"{{"id":"{id}","state":"waiting","says":"why","waits":[{}]}}"#,
            waits.join(",")
        )
    }

    #[test]
    fn a_graph_runs_each_arrow_from_what_a_task_waits_on_to_it() {
        let _budget = TestBudget::new();
        let svg = graph(&[task("b", &["a"]), task("a", &[])].join(",")).unwrap();
        assert!(svg.contains(r#"data-from="a" data-to="b""#), "{svg}");
        // a in the first column though listed last, b in the second.
        assert!(
            svg.contains(r#"data-task="a" style="left:60px;top:8px;"#),
            "{svg}"
        );
        assert!(
            svg.contains(r#"data-task="b" style="left:420px;top:8px;"#),
            "{svg}"
        );
    }

    #[test]
    fn an_arrow_past_the_next_column_runs_under_the_nodes_never_behind_one() {
        let _budget = TestBudget::new();
        let tasks = [task("a", &[]), task("b", &["a"]), task("c", &["a", "b"])];
        let svg = graph(&tasks.join(",")).unwrap();
        // One row of 52 at y 8, its gap of 16 under it: the lane at 76, the graph's height past it.
        assert!(
            svg.contains(r#"data-from="a" data-to="c" d="M360 34H368V76H772V34H780""#),
            "{svg}"
        );
        assert!(
            svg.contains(r#"data-from="b" data-to="c" d="M720 34C750 34"#),
            "{svg}"
        );
        assert!(svg.contains(r#"height="92""#), "{svg}");
        let lone = graph(&task("a", &[])).unwrap();
        assert!(
            lone.contains(r#"width="420" height="84""#),
            "no gap past the last column: {lone}"
        );
    }

    #[test]
    fn two_long_arrows_share_no_line() {
        let _budget = TestBudget::new();
        let tasks = [
            task("a", &[]),
            task("b", &[]),
            task("m", &["a"]),
            task("x", &["m", "a"]),
            task("y", &["m", "b"]),
        ];
        let svg = graph(&tasks.join(",")).unwrap();
        let drop_at = |edge: &str| {
            let at = svg.find(edge).expect(edge);
            let d = &svg[at..];
            let h = d.find('H').expect("a lane");
            d[h..d.find('V').expect("a lane")].to_string()
        };
        let a = drop_at(r#"data-from="a" data-to="x""#);
        let b = drop_at(r#"data-from="b" data-to="y""#);
        assert_ne!(a, b, "{svg}");
    }

    #[test]
    fn seven_long_arrows_from_one_column_share_no_line() {
        let _budget = TestBudget::new();
        let sources: Vec<String> = (0..7).map(|i| format!("s{i}")).collect();
        let names: Vec<&str> = sources.iter().map(String::as_str).collect();
        let mut tasks: Vec<String> = names.iter().map(|s| task(s, &[])).collect();
        tasks.push(task("m", &["s0"]));
        let mut waits = names.clone();
        waits.push("m");
        tasks.push(task("t", &waits));
        let svg = graph(&tasks.join(",")).unwrap();
        let drops: HashSet<String> = names
            .iter()
            .map(|s| {
                let edge = format!(r#"data-from="{s}" data-to="t""#);
                let d = &svg[svg.find(&edge).expect("the arrow")..];
                d[d.find('H').expect("a lane")..d.find('V').expect("a lane")].to_string()
            })
            .collect();
        assert_eq!(drops.len(), 7, "{svg}");
    }

    #[test]
    fn a_task_past_a_loop_stays_right_of_it() {
        let _budget = TestBudget::new();
        let tasks = [task("c", &["a"]), task("a", &["b"]), task("b", &["a"])];
        let svg = graph(&tasks.join(",")).unwrap();
        assert!(svg.contains(r#"data-task="a" style="left:60px;"#), "{svg}");
        assert!(svg.contains(r#"data-task="c" style="left:420px;"#), "{svg}");
    }

    #[test]
    fn a_merged_wait_is_left_out_and_a_task_that_ended_still_holds() {
        let _budget = TestBudget::new();
        let done = |id: &str, state: &str| {
            format!(r#"{{"id":"{id}","state":"{state}","says":"s","waits":[]}}"#)
        };
        let tasks = [
            done("old", "finished"),
            done("bad", "ended"),
            task("now", &["old", "bad"]),
        ];
        let svg = graph(&tasks.join(",")).unwrap();
        assert!(!svg.contains(r#""old""#), "{svg}");
        assert!(svg.contains(r#"class="edge" data-from="bad""#), "{svg}");
        assert!(svg.contains(r#"class="node ended""#), "{svg}");
    }

    /// Where a node sits: its column (by its left) and its row (by its top).
    fn spot(svg: &str, id: &str) -> (usize, usize) {
        let at = svg
            .find(&format!(r#"data-task="{id}" style="left:"#))
            .expect(id);
        let px = |from: &str| -> usize {
            let rest = &svg[at..][svg[at..].find(from).expect(from) + from.len()..];
            rest[..rest.find("px").expect("px")]
                .parse()
                .expect("a number")
        };
        (px("left:"), px("top:"))
    }

    /// Done-when 2 and 5 of rust-graph-queue-edges: tasks with real waits never draw as one column of cards; a merged task is
    /// left out; what runs heads column 0, though listed last; each dependent sits right of every task it waits on.
    #[test]
    fn real_waits_draw_columns_running_first_merged_left_out() {
        let _budget = TestBudget::new();
        let with = |id: &str, state: &str, waits: &[&str]| {
            let waits: Vec<String> = waits.iter().map(|w| format!(r#""{w}""#)).collect();
            format!(
                r#"{{"id":"{id}","state":"{state}","says":"s","waits":[{}]}}"#,
                waits.join(",")
            )
        };
        let edges: &[(&str, &[&str])] = &[
            ("ui-media", &[]),
            ("design-media", &["ui-media", "merged"]),
            ("crm-photo", &["design-media"]),
            ("lone", &[]),
            ("after-run", &["run"]),
        ];
        let mut tasks: Vec<String> = edges.iter().map(|(id, w)| with(id, "waiting", w)).collect();
        tasks.push(with("merged", "finished", &[]));
        tasks.push(with("run", "running", &[]));
        let svg = graph(&tasks.join(",")).unwrap();
        assert!(!svg.contains(r#"data-task="merged""#), "{svg}");
        assert!(!svg.contains(r#"data-from="merged""#), "{svg}");
        let lefts: HashSet<usize> = edges.iter().map(|(id, _)| spot(&svg, id).0).collect();
        assert!(lefts.len() > 1, "one column: {svg}");
        assert_eq!(spot(&svg, "run"), (MARGIN_X, PAD), "{svg}");
        for (id, waits) in edges {
            for wait in waits.iter().filter(|w| **w != "merged") {
                assert!(
                    spot(&svg, id).0 > spot(&svg, wait).0,
                    "{id} right of {wait}: {svg}"
                );
            }
        }
    }

    #[test]
    fn a_graph_escapes_its_words_and_keeps_long_ones_whole_for_css_to_clip() {
        let _budget = TestBudget::new();
        let long = "x".repeat(200);
        let tasks = format!(r#"{{"id":"{long}","state":"next","says":"<b>&\"","waits":[]}}"#,);
        let svg = graph(&tasks).unwrap();
        assert!(svg.contains(r#"class="node next""#), "{svg}");
        assert!(
            svg.contains(&format!("{long}: &lt;b&gt;&amp;&quot;")),
            "{svg}"
        );
        assert!(
            svg.contains(&format!(r#"<div class="name">{long}</div>"#)),
            "{svg}"
        );
        assert!(!svg.contains("<b>"), "{svg}");
    }

    #[test]
    fn a_loop_still_draws_and_a_bad_graph_is_refused_by_name() {
        let _budget = TestBudget::new();
        let looped = graph(&[task("a", &["b"]), task("b", &["a"]), task("c", &["c"])].join(","));
        let svg = looped.unwrap();
        for id in ["a", "b", "c"] {
            assert!(svg.contains(&format!(r#"data-task="{id}""#)), "{svg}");
        }
        let many = (0..=GRAPH_LIMIT).map(|i| task(&i.to_string(), &[]));
        for (bad, says) in [
            (task("a", &["gone"]), "a waits on gone"),
            (
                [task("a", &[]), task("a", &[])].join(","),
                "a appears twice",
            ),
            (many.collect::<Vec<_>>().join(","), "past 500"),
            (r#"{"id":"a","state":"odd","says":""}"#.to_string(), "odd"),
        ] {
            let e = graph(&bad).expect_err(says);
            assert!(e.contains(says), "{e}");
        }
    }

    #[test]
    fn a_group_counts_its_rows_and_a_row_opens_its_task() {
        let _budget = TestBudget::new();
        let html = screen::page(
            "t",
            r#"{"root":"g","elements":{
                "g":{"type":"issue-group","props":{"status":"in-progress"},"children":["a","b"]},
                "a":{"type":"issue-row","props":{"id":"a b/<c>","title":"<T>","status":"in-progress","says":"s&","comments":0}},
                "b":{"type":"issue-row","props":{"id":"b","title":"t","status":"in-progress","says":"","comments":2}}}}"#,
        )
        .unwrap();
        assert!(html.contains(r#"data-status="in-progress""#), "{html}");
        assert!(
            html.contains(r#"In Progress<span class="badge count">2</span>"#),
            "{html}"
        );
        assert!(html.contains(r#"href="/tasks/a%20b%2F%3Cc%3E""#), "{html}");
        assert!(html.contains(r#"data-task="a b/&#60;c&#62;""#), "{html}");
        assert!(
            html.contains(r#"<span class="title">&#60;T&#62;</span>"#),
            "{html}"
        );
        assert!(
            html.contains(r#"<span class="says">s&#38;</span>"#),
            "{html}"
        );
        assert_eq!(html.matches("data-comments").count(), 1, "{html}");
        assert!(html.contains(r#"data-comments="2">2 comments"#), "{html}");
    }

    /// Task rust-doc-types: a document is its template's, both forms refuse a link that is not GitHub's and a child.
    #[test]
    fn a_document_draws_by_its_template_and_both_forms_refuse_alike() {
        let _budget = TestBudget::new();
        let brief = |pr: &str, children: &str| {
            format!(
                r#"{{"root":"d","elements":{{"d":{{"type":"document","props":{{"kind":"task-brief","id":"t","title":"<T>","status":"todo","says":"s","why":"w","checks":[],"no_checks":"None written.","run":"Never run.","pr":"{pr}"}},"children":{children}}},"x":{{"type":"text","props":{{"text":"x"}}}}}}}}"#
            )
        };
        let html =
            screen::page("t", &brief("https://github.com/o/r/pull/12", "[]")).expect("drawn");
        assert!(
            html.contains(r#"</style><article id="d" class="doc doc-task-brief">"#),
            "{html}"
        );
        assert!(
            html.contains(r#"<h1 class="li-issue-title">&#60;T&#62;</h1>"#),
            "{html}"
        );
        assert!(
            html.contains(r#"<a class="li-tag li-pr" href="https://github.com/o/r/pull/12">"#),
            "{html}"
        );
        let told = text(&brief("https://github.com/o/r/pull/12", "[]")).expect("told");
        assert!(
            told.contains(
                "<T>\nw\nDone when\nNone written.\nLast run\nNever run.\nProperties\nTodo\ns\n"
            ),
            "{told}"
        );
        let elsewhere = brief("https://evil.example/pull/1", "[]");
        assert!(check(&elsewhere).is_err_and(|e| e.contains("not a GitHub link")));
        assert!(text(&elsewhere).is_err_and(|e| e.contains("not a GitHub link")));
        let parent = brief("https://github.com/o/r/pull/12", r#"["x"]"#);
        let refused = "d: a document holds no children".to_string();
        assert_eq!(check(&parent).err(), Some(refused.clone()));
        assert_eq!(text(&parent), Err(refused));
    }

    /// Done-when 2 of rust-page-forms, an access since rust-tools-human: one line in the text, a line break or a tab inside
    /// a field kept out; an access holds no children.
    #[test]
    fn a_record_tells_one_line() {
        let _budget = TestBudget::new();
        let access = |note: &str, children: &str| {
            json!({"root": "r", "elements": {
                "r": {"type": "access", "props": {"name": "gh", "status": "works", "usage": "a | b", "note": note, "place": "p", "source": "cli"}, "children": serde_json::from_str::<Value>(children).expect("children")},
                "t": {"type": "text", "props": {"text": "x"}}}})
            .to_string()
        };
        assert_eq!(
            text(&access("401\r\nretry", "[]")).expect("text"),
            "gh\tworks\ta | b\t401  retry\tp\tcli\n"
        );
        let tabbed = r#"{"root":"r","elements":{"r":{"type":"issue-row","props":{"id":"a","title":"t\tu\nv","status":"todo","says":"s","comments":0}}}}"#;
        assert_eq!(text(tabbed).expect("text"), "a\tt u v\ts\n");
        let stray = r#"{"root":"g","elements":{"g":{"type":"task-graph","props":{"tasks":[{"id":"x","state":"next","says":"go","waits":["y"]}]}}}}"#;
        assert!(
            check(stray).is_err() && text(stray).is_err(),
            "both forms refuse it"
        );
        let parent = access("n", r#"["t"]"#);
        let refused = "r: an access holds no children".to_string();
        assert_eq!(check(&parent).err(), Some(refused.clone()));
        assert_eq!(text(&parent), Err(refused));
    }

    #[test]
    fn a_tree_s_text_says_what_its_html_shows_unescaped_and_in_order() {
        let _budget = TestBudget::new();
        let tree = json!({"root": "page", "elements": {
            "page": {"type": "stack", "children": ["h", "part"]},
            "h": {"type": "heading", "props": {"text": "A <b>"}},
            "part": {"type": "section", "props": {"title": "Checks"}, "children": ["c", "q"]},
            "c": {"type": "text", "props": {"text": "a test"}},
            "q": {"type": "quote", "props": {"text": "one\ntwo"}}
        }})
        .to_string();
        assert_eq!(
            text(&tree).expect("text"),
            "A <b>\n\nChecks\na test\none\ntwo\n"
        );
        let looped = r#"{"root":"a","elements":{"a":{"type":"stack","children":["a"]}}}"#;
        assert!(text(looped).is_err_and(|e| e.contains("appears twice")));
    }

    #[test]
    fn a_page_carries_the_generated_theme_around_its_tree() {
        let _budget = TestBudget::new();
        let tree = r#"{"root":"h","elements":{"h":{"type":"heading","props":{"text":"<a>"}}}}"#;
        let html = screen::page("t<", tree).unwrap();
        assert!(html.starts_with("<!doctype html>"), "{html}");
        assert!(html.contains("<title>t&lt;</title>"), "{html}");
        assert!(html.contains(".app.dark {\n  --color-accent:"), "{html}");
        assert!(html.contains(r#"<h1 id="h">&#60;a&#62;</h1>"#), "{html}");
        assert!(screen::page("t", "not json").is_err());
    }

    struct Fails;

    impl Tool for Fails {
        fn description(&self) -> &str {
            "It fails"
        }
        fn input_schema(&self) -> &str {
            r#"{"type":"object"}"#
        }
        fn call(&self, arguments: &str) -> Result<String, String> {
            Err(format!("no, given {arguments}"))
        }
    }

    #[test]
    fn mcp_answers_each_request_and_no_notification() {
        let _budget = TestBudget::new();
        let mut registry = Registry::default();
        let c = Contribution {
            tools: vec![("fails".into(), Box::new(Fails))],
            ..Default::default()
        };
        registry.add("t", c).unwrap();
        let input = [
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26"}}"#,
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"fails","arguments":{"a":1}}}"#,
            r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"nope"}}"#,
            r#"{"jsonrpc":"2.0","id":5,"method":"nope"}"#,
            "not json",
            "[1]",
            r#"{"jsonrpc":"2.0","id":9,"result":{}}"#,
        ]
        .join("\n");
        let mut input = input.into_bytes();
        input.extend_from_slice(b"\n\xff\n");
        let mut output = vec![];
        serve_mcp(&registry, &input[..], &mut output).unwrap();
        let lines: Vec<Value> = String::from_utf8(output)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines.len(), 8, "a response gets no answer: {lines:?}");
        assert_eq!(lines[0]["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(lines[1]["result"]["tools"][0]["name"], "fails");
        assert_eq!(
            lines[1]["result"]["tools"][0]["inputSchema"]["type"],
            "object"
        );
        assert_eq!(lines[2]["result"]["isError"], true);
        assert_eq!(
            lines[2]["result"]["content"][0]["text"],
            r#"no, given {"a":1}"#
        );
        assert_eq!(lines[3]["error"]["code"], -32602);
        assert_eq!(lines[4]["error"]["code"], -32601);
        assert_eq!(lines[5]["error"]["code"], -32700);
        assert_eq!(lines[6]["error"]["code"], -32600);
        assert_eq!(
            lines[7]["error"]["code"], -32700,
            "a stray byte, and it served on"
        );
    }
}
