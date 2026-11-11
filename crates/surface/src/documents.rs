//! The few document types every page reuses, each one template of the design system (task rust-doc-types; @nha, 2026-11-09:
//! « this is the same for all documents, we should have a few doc types we always reuse. RFC, Task brief, message draft with
//! context, CRM entry etc should follow a given template. », « the comments on this page for example follows the notion
//! example from the design system »). A page gives a document's fields as a tree's `document` element and never its layout:
//! the template orders, titles and frames them. Its text form is its HTML read by `forms::text_of`, never written twice.
//! /design?tab=documents shows each type's two forms.
use crate::components::{
    Attachment, AttachmentGroup, Avatar, Badge, Button, ButtonLink, Copies, CopyButton, CopyText,
    Fold, Form, Input, Kbd, KbdGroup, PropertyMenu, Skeleton, Textarea, Timeline, TimelineEvent,
    Toolbar, ToolbarButton, ToolbarGroup, ToolbarSeparator, card_href, initials_of, is_picture,
    local_path,
};
pub use crate::design::notion_doc::Line;
use crate::design::{families, notion_doc};
// canvist_model 0.2.1 owns a block's attr `id` (json.rs ID_KEY): its JSON writes the block's own id there and its read drops
// ours, with no error. A DOM anchor goes in another attr, `dom` (task rust-one-doc-component spent turns on it, 2026-11-10).
use canvist_model::{BlockId, Document};
use dioxus::prelude::*;
use serde::Deserialize;
use serde_json::Value;

/// A document's looks, which the page around it and /design share.
pub const CSS: &str = include_str!("documents.css");

/// A status as the Linear view groups open tasks by it, first to last.
#[derive(Deserialize, Clone, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum IssueStatus {
    InReview,
    InProgress,
    Todo,
    Backlog,
    /// Finished: a comment's task shows it on /comments (task comments-by-state); no board column holds it.
    Done,
}

impl IssueStatus {
    pub(crate) fn class(&self) -> &'static str {
        match self {
            IssueStatus::InReview => "in-review",
            IssueStatus::InProgress => "in-progress",
            IssueStatus::Todo => "todo",
            IssueStatus::Backlog => "backlog",
            IssueStatus::Done => "done",
        }
    }

    pub(crate) fn name(&self) -> &'static str {
        match self {
            IssueStatus::InReview => "In Review",
            IssueStatus::InProgress => "In Progress",
            IssueStatus::Todo => "Todo",
            IssueStatus::Backlog => "Backlog",
            IssueStatus::Done => "Done",
        }
    }

    /// Its status as /design?tab=linear's icon names it (`design::families::status`).
    pub(crate) fn kind(&self) -> &'static str {
        match self {
            IssueStatus::InReview => "review",
            IssueStatus::InProgress => "started",
            IssueStatus::Todo => "todo",
            IssueStatus::Backlog => "backlog",
            IssueStatus::Done => "done",
        }
    }
}

/// A check's verdict in words, for the text form: `families::issue_check` draws its own mark and class.
pub(crate) fn verdict(met: Option<bool>) -> &'static str {
    match met {
        Some(true) => "met",
        Some(false) => "not met",
        None => "not judged",
    }
}

/// A pull request's number, or the refusal of a link that is not GitHub's: both forms refuse it alike.
pub(crate) fn pr_number<'a>(id: &str, url: &'a str) -> Result<&'a str, String> {
    let Some(path) = url.strip_prefix("https://github.com/") else {
        return Err(format!("{id}: not a GitHub link"));
    };
    // owner/repo/pull/<number>, nothing after: a trailing slash or a repo's link draws no « # » (/code-review of the merge)
    match path.split('/').collect::<Vec<_>>()[..] {
        [_, _, "pull", n] if !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) => Ok(n),
        _ => Err(format!("{id}: not a pull request's link")),
    }
}

/// A document, by its type: the closed set a page picks from.
#[derive(Deserialize, Clone, PartialEq)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Doc {
    Rfc(Rfc),
    // the two with a page's many fields boxed, so the others stay small (clippy's large_enum_variant)
    TaskBrief(Box<TaskBrief>),
    MessageDraft(Box<MessageDraft>),
    CrmEntry(CrmEntry),
    PageComments(PageComments),
    Skill(Skill),
}

/// A titled part of a document, its text's line breaks kept.
#[derive(Deserialize, Clone, PartialEq)]
pub struct Part {
    pub title: String,
    pub text: String,
}

/// A label and its value: a lead's company, a draft's goal; its value a link where `href` says one (an ad's Source).
#[derive(Deserialize, Clone, PartialEq)]
pub struct Fact {
    pub label: String,
    pub value: String,
    #[serde(default)]
    pub href: Option<String>,
}

/// One comment: who, when (empty when not known), the words.
#[derive(Deserialize, Clone, PartialEq)]
pub struct Said {
    pub who: String,
    #[serde(default)]
    pub when: String,
    pub words: String,
}

/// Comments on one passage, Notion's thread: the passage quoted (empty: on the page as a whole), then each comment.
#[derive(Deserialize, Clone, PartialEq)]
pub struct Thread {
    /// Its anchor on the page, `c-<comment id>`, so a link to it lands on it (task rust-comment-answer-on-page); empty: none.
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub quote: String,
    pub said: Vec<Said>,
    /// Still waiting on its answer: the rail's Open filter keeps it (task rust-comment-files-filter).
    #[serde(default)]
    pub open: bool,
    /// Its attached images and PDFs, each its name, where it opens and its MIME type, drawn as `Attachment`s.
    #[serde(default)]
    pub files: Vec<(String, String, String)>,
}

/// A decision page: its status, its bottom line, its sections.
#[derive(Deserialize, Clone, PartialEq)]
pub struct Rfc {
    pub title: String,
    pub status: String,
    pub bottom_line: String,
    pub parts: Vec<Part>,
}

/// One check of a done-when. `met`: `None` before a judgement.
#[derive(Deserialize, Clone, PartialEq)]
pub struct CheckLine {
    pub text: String,
    pub met: Option<bool>,
}

/// A line of a task's Activity: what happened, and when.
#[derive(Deserialize, Clone, PartialEq)]
pub struct Event {
    pub what: String,
    pub when: String,
}

/// A task's page, Linear's issue view (task rust-task-page-linear): the way back, its title, why, its checks, its Activity
/// (its history, then his comments), its last run; its status and pull request among its properties.
#[derive(Deserialize, Clone, PartialEq)]
pub struct TaskBrief {
    pub id: String,
    pub title: String,
    pub status: IssueStatus,
    pub says: String,
    /// None while the older app has not answered: grey lines that fill in place (task rust-skeleton-live).
    #[serde(default)]
    pub why: Option<String>,
    pub checks: Vec<CheckLine>,
    /// What the checks part says when there are none: none written, or the list not read; None while it is read.
    #[serde(default)]
    pub no_checks: Option<String>,
    pub run: String,
    #[serde(default)]
    pub pr: Option<String>,
    /// The last run's own words.
    #[serde(default)]
    pub said: String,
    /// Its history: each write of its status.
    #[serde(default)]
    pub events: Vec<Event>,
    /// What its Activity says first when its history is not read.
    #[serde(default)]
    pub unread: String,
    #[serde(default)]
    pub comments: Vec<Thread>,
    /// Its priority as the list files it, 1 urgent to 4 low; none when not set. The properties below are Linear's issue
    /// sidebar, as board_live.ex's issue page draws them (task rust-board-buttons-new-issue).
    #[serde(default)]
    pub priority: Option<u8>,
    /// The day it is due.
    #[serde(default)]
    pub deadline: Option<String>,
    /// Its assignee, labels, project, cycle and reminder, each his menu; none until the list was read.
    #[serde(default)]
    pub sidebar: Option<Box<Sidebar>>,
    /// Its place among the board's cards, its previous and next a step away, each its page's link.
    #[serde(default)]
    pub position: Option<Position>,
    /// Its status is his to set: a task the board view places (dash tasks.rs `Board::move_to`).
    #[serde(default)]
    pub movable: bool,
    /// The due days his menu offers, each its name and its date (Linear's: tomorrow, end of this week, in one or two weeks).
    #[serde(default)]
    pub due_days: Vec<(String, String)>,
}

/// What an issue's sidebar menus show and offer (task rust-issue-sidebar-menus), as the older app's issue page: each value set,
/// and each menu's choices, a value and its name.
#[derive(Deserialize, Clone, PartialEq, Default)]
pub struct Sidebar {
    /// Who runs it, Linear's assignee: one of `owners`' values.
    #[serde(default)]
    pub owner: String,
    #[serde(default)]
    pub labels: Vec<String>,
    /// Its project, the epic it belongs to.
    #[serde(default)]
    pub epic: Option<String>,
    /// Its cycle, a Monday.
    #[serde(default)]
    pub cycle: Option<String>,
    /// The day it comes back to him, Linear's « Remind me ».
    #[serde(default)]
    pub remind: Option<String>,
    #[serde(default)]
    pub owners: Vec<(String, String)>,
    /// The labels he sets, the list's set but the agent's.
    #[serde(default)]
    pub label_set: Vec<String>,
    #[serde(default)]
    pub epics: Vec<String>,
    /// Each cycle offered, its Monday and its name with its dates.
    #[serde(default)]
    pub cycles: Vec<(String, String)>,
    /// Each reminder day offered, its date and its name.
    #[serde(default)]
    pub remind_days: Vec<(String, String)>,
}

/// A task's place among the board's open tasks, Linear's « 42 / 2193 » with its two steps.
#[derive(Deserialize, Clone, PartialEq)]
pub struct Position {
    pub at: usize,
    pub of: usize,
    #[serde(default)]
    pub previous: Option<String>,
    #[serde(default)]
    pub next: Option<String>,
}

/// Where a task page's property menus post (dash main.rs's `property_set`): `/tasks/<id>/set`, a form of one field.
pub fn set_path(id: &str) -> String {
    format!("{}/set", crate::words::task_href(id, None))
}

/// The statuses his status menu moves a task between: the board's columns.
const COLUMNS: [IssueStatus; 4] = [
    IssueStatus::Backlog,
    IssueStatus::Todo,
    IssueStatus::InProgress,
    IssueStatus::InReview,
];
/// The priorities his menu sets, 1 to 4 as the list files them, then none (Linear's « No priority »).
const PRIORITIES: [u8; 5] = [1, 2, 3, 4, 0];

/// A message before it goes, with what it is written from: its goal, who sends it, the lead, its scores. `text` empty: the
/// draft listed, its words on its own page at `href`.
#[derive(Deserialize, Clone, PartialEq)]
pub struct MessageDraft {
    pub receiver: String,
    pub sender: String,
    pub channel: String,
    pub status: String,
    pub goal: String,
    #[serde(default)]
    pub lead: String,
    pub scores: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub href: Option<String>,
    /// Its page's path while it waits on his decision: Approve and Request corrections post there (task rust-draft-approve).
    #[serde(default)]
    pub decide: Option<String>,
    /// Its earlier versions on its own page, oldest first: each its title and words.
    #[serde(default)]
    pub earlier: Vec<Part>,
    /// His open comments on its page: Approve waits until they are applied, as the older page's « Approve with my corrections ».
    #[serde(default)]
    pub open_comments: usize,
    /// The other moves its status offers (Reject, I sent it, Back to review), each posted as Approve is.
    #[serde(default)]
    pub moves: Vec<Move>,
    /// The ad it answers, from its archive (task rust-draft-posting): Title, Company, Location, Contract, then Source, linked.
    #[serde(default)]
    pub posting: Vec<Fact>,
    /// The ad's text, folded under its facts.
    #[serde(default)]
    pub ad: String,
    /// Where a form draft is sent: its Application form's link.
    #[serde(default)]
    pub apply: Option<String>,
    /// Its lead's last events, newest first, each its day and words, linked to its source (else the lead's page): on a fresh
    /// lead, the search note that found it.
    #[serde(default)]
    pub recent: Vec<Fact>,
    /// Its id, said in State's Status line.
    #[serde(default)]
    pub id: String,
    /// « Copy the text » under the message: a draft he sends himself (task rust-draft-state-edit).
    #[serde(default)]
    pub copy: bool,
    /// Its own page while its text is open to his edit: Change opens it there with `?edit=1`, Save posts his words as `body`
    /// to `decide`/body, the older app's write of his edit, kept as a version with his name.
    #[serde(default)]
    pub change: Option<String>,
    /// The page was opened with `?edit=1`: the message is the field.
    #[serde(default)]
    pub editing: bool,
    /// Why this text: each reason it says what it says (the record's :context).
    #[serde(default)]
    pub why: Vec<String>,
    /// State: where it goes (the record's :where), whether his bold reaches the reader, each decision's day.
    #[serde(default, rename = "where")]
    pub place: String,
    #[serde(default)]
    pub bold: String,
    #[serde(default)]
    pub dates: Vec<Fact>,
    /// While it is open, where « Goes out on » is saved, and the day it holds (none: with the next outbox).
    #[serde(default)]
    pub schedule: Option<String>,
    #[serde(default)]
    pub send_on: String,
    /// Waits: each step it waits on, still open, linked; none, it is actionable. `meanwhile`: what may go before.
    #[serde(default)]
    pub waits: Vec<Fact>,
    #[serde(default)]
    pub meanwhile: String,
    /// What it waits on was not read: Waits says so, never « waits on nothing ».
    #[serde(default)]
    pub waits_unread: bool,
    /// A form whose fields show formatting: Copy the text keeps the stars of his bold, as State's Bold says.
    #[serde(default)]
    pub keep_bold: bool,
    /// Whether State and Waits were read: a listed draft and the table's fallback copy draw neither.
    #[serde(default)]
    pub stated: bool,
}

/// A draft's move: the status it posts and its button's words.
#[derive(Deserialize, Clone, PartialEq)]
pub struct Move {
    pub status: String,
    pub label: String,
}

/// A contact or a lead: its name, company and stage, its facts, what happened when, his notes.
#[derive(Deserialize, Clone, PartialEq)]
pub struct CrmEntry {
    pub name: String,
    pub company: String,
    pub stage: String,
    pub facts: Vec<Fact>,
    /// Each event: its day (2026-11-02, a time after it allowed) as `label`, what happened as `value`.
    pub timeline: Vec<Fact>,
    #[serde(default)]
    pub notes: String,
    /// The day its dates count from, 2026-11-10; none: the clock's today.
    #[serde(default)]
    pub today: Option<String>,
}

/// A page's comments, on its passages or on it whole.
#[derive(Deserialize, Clone, PartialEq)]
pub struct PageComments {
    pub threads: Vec<Thread>,
}

/// A SKILL.md (@nha, 2026-11-09: « needs an extra "skill" imo. as a document. »): its frontmatter's name and description, when
/// an agent reaches for it, then its sections.
#[derive(Deserialize, Clone, PartialEq)]
pub struct Skill {
    pub name: String,
    pub description: String,
    /// Empty: a skill of one block, no `##` heading.
    #[serde(default)]
    pub parts: Vec<Part>,
}

impl Doc {
    /// Its kind as the tree names it: its class and /design's anchor.
    pub fn kind(&self) -> &'static str {
        match self {
            Doc::Rfc(_) => "rfc",
            Doc::TaskBrief(_) => "task-brief",
            Doc::MessageDraft(_) => "message-draft",
            Doc::CrmEntry(_) => "crm-entry",
            Doc::PageComments(_) => "page-comments",
            Doc::Skill(_) => "skill",
        }
    }

    /// Still being read: the page draws its refill loop (screen::Screen).
    pub fn busy(&self) -> bool {
        match self {
            Doc::TaskBrief(b) => b.why.is_none() || (b.checks.is_empty() && b.no_checks.is_none()),
            _ => false,
        }
    }

    /// The refusals both forms share: a pull request that is not GitHub's, a link that is not one a page may open.
    pub(crate) fn check(&self, id: &str) -> Result<(), String> {
        match self {
            Doc::TaskBrief(b) => {
                b.pr.as_deref()
                    .map_or(Ok(()), |url| pr_number(id, url).map(|_| ()))
            }
            Doc::MessageDraft(d) if d.href.is_some() && card_href(&d.href).is_none() => {
                Err(format!("{id}: not a link of this app or http(s)"))
            }
            Doc::MessageDraft(d) if d.decide.as_deref().is_some_and(|p| !local_path(p)) => {
                Err(format!("{id}: a decision posts to a path of this app"))
            }
            _ => Ok(()),
        }
    }
}

/// Initials of a name: its words' first letters, two at most.
fn initials(who: &str) -> String {
    who.split_whitespace()
        .filter_map(|w| w.chars().next())
        .take(2)
        .collect()
}

/// A status's dot, the issue rows' (screen.rs); lib.rs's string walk draws `IssueStatus::icon` until it retires.
pub(crate) fn status_icon(status: &IssueStatus) -> Element {
    rsx! { span { class: "status-icon {status.class()}" } }
}

/// A link's target, for the text form alone: an agent reads where it opens, a person sees the link (components.css .text-only).
fn target(href: &str) -> Element {
    rsx! { span { class: "text-only", "{href}" } }
}

fn part(title: &str, body: Element) -> Element {
    rsx! { section { class: "doc-part", h2 { "{title}" } {body} } }
}

fn words(text: &str) -> Element {
    rsx! { pre { class: "doc-text", "{text}" } }
}

/// How DocView lays a document out: its view is its style, the markup one (rust/CLAUDE.md rule 7).
#[derive(Clone, Copy, PartialEq, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum View {
    /// A page: its blocks in reading order, a section's private note a callout under its words.
    #[default]
    Notion,
    /// A message: each section's private note beside its words, as a draft waits on him.
    Gmail,
    /// A chat: each public section a message bubble, in order; a private note is never drawn.
    Chat,
}

impl View {
    fn class(self) -> &'static str {
        match self {
            View::Notion => "notion",
            View::Gmail => "gmail",
            View::Chat => "chat",
        }
    }
}

/// The editor's script, once a page: the field's words go in `text` at Send, an empty field sends nothing; Undo and Redo (their
/// buttons, ⌘Z, ⇧⌘Z) walk the field's own history, never the page's, whose undo stack every editable on it shares (a task
/// page's description and comments are DocViews too; /code-review); Discard empties it as one step Undo brings back; Preview
/// shows it read only; ⌘Enter sends; a page back from the browser's cache shows its words without the placeholder. The window
/// runs no script and sends no POST form (crates/ui's InWindow), so a Composer there posts nothing, as the textarea did not
/// either. ponytail: a step per input event, so Undo goes back a letter at a time; group by word if he asks.
pub(crate) const EDITOR_JS: &str = r#"(()=>{if(window.dashEditor)return;window.dashEditor=1;
const on=(t,f,c)=>document.addEventListener(t,f,c);
const ed=e=>e.target.closest&&e.target.closest(".editor");
const field=f=>f.querySelector(".editor-field .dv");
const words=f=>field(f).innerText.trim();
const sync=f=>f.querySelector(".editor-field").toggleAttribute("data-empty",!words(f));
const book=new WeakMap();
const past=f=>{if(!book.has(f))book.set(f,{steps:[""],at:0});return book.get(f)};
const keep=f=>{const h=past(f),t=field(f).innerText;if(t===h.steps[h.at])return;h.steps.length=h.at+1;h.steps.push(t);h.at++;sync(f)};
const go=(f,by)=>{const h=past(f),to=h.at+by;if(to<0||to>=h.steps.length)return;h.at=to;const d=field(f);d.textContent=h.steps[to];d.focus();getSelection().selectAllChildren(d);getSelection().collapseToEnd();sync(f)};
const mark=(f,k)=>{const d=field(f);d.focus();const s=getSelection(),t=s.rangeCount&&d.contains(s.anchorNode)?s.toString():"";
const wrap=(l,r,e)=>l+(t||e)+r;const line=p=>{let n=0;return(t||"").split("\n").map(l=>l.trim()?p(n++)+l:l).join("\n")||p(0)};
const out={bold:()=>wrap("**","**","bold text"),italic:()=>wrap("*","*","italic text"),link:()=>wrap("[","](https://)","link text"),
ul:()=>line(()=>"- "),ol:()=>line(i=>i+1+". "),quote:()=>line(()=>"> ")}[k];if(out)document.execCommand("insertText",false,out())};
on("input",e=>{const f=ed(e);if(f)keep(f)});
on("click",e=>{const b=e.target.closest&&e.target.closest(".editor button");if(!b)return;const f=b.closest(".editor");
if(b.matches(".editor-undo"))go(f,-1);else if(b.matches(".editor-redo"))go(f,1);
else if(b.matches(".editor-discard")){field(f).textContent="";keep(f);field(f).focus()}
else if(b.dataset.format)mark(f,b.dataset.format)
else if(b.matches(".editor-preview")){const p=b.getAttribute("aria-pressed")!=="true";b.setAttribute("aria-pressed",p);field(f).contentEditable=p?"false":"plaintext-only";f.classList.toggle("is-preview",p)}});
on("keydown",e=>{const f=ed(e);if(!f||!(e.metaKey||e.ctrlKey))return;const k=e.key.toLowerCase();
if(k==="enter"){e.preventDefault();f.closest("form").requestSubmit()}
else if(k==="z"||k==="y"){e.preventDefault();go(f,e.shiftKey||k==="y"?1:-1)}});
on("submit",e=>{const f=e.target.querySelector(".editor");if(!f)return;const t=words(f);if(!t){e.preventDefault();field(f).focus();return}e.target.elements.text.value=t},true);
addEventListener("pageshow",()=>document.querySelectorAll(".editor").forEach(keep))})()"#;

/// What a comment attaches, as a file picker's `accept`: images and PDFs, the older app's list (`POST /compass` refuses any other).
pub const ATTACHABLE: &str = ".png,.jpg,.jpeg,.gif,.webp,.pdf";
/// Each of ATTACHABLE's extensions and the MIME type its file is served as: the one table `POST /compass` checks against.
pub const ATTACHABLE_KINDS: [(&str, &str); 6] = [
    ("png", "image/png"),
    ("jpg", "image/jpeg"),
    ("jpeg", "image/jpeg"),
    ("gif", "image/gif"),
    ("webp", "image/webp"),
    ("pdf", "application/pdf"),
];

/// The documents editor, the box he writes a message in (task compose-uses-doc-editor; @nha, 2026-11-09: « the input should be a
/// component reused from documents. same for the input controls. This gives us consistent shorcuts rich controls and content,
/// preview etc. as well as the same history controls. »): the Notion-style field, DocView editable as the Notion tab's lines
/// are, its toolbar of Undo, Redo and Preview with their keys, and Discard and `send` inside the field. Its words post to
/// `action`, a path of this app, as `text`, with the `hidden` fields: a thread's reply, /'s Ask, a task's comment. The field is
/// named by `placeholder`. The keys' caps are drawn for him, kept out of the text form (aria-hidden): an agent reads the controls.
#[component]
pub fn Composer(
    #[props(into)] id: String,
    #[props(into)] action: String,
    #[props(into)] send: String,
    #[props(into)] placeholder: String,
    #[props(default)] hidden: Vec<(String, String)>,
    #[props(default)] fields: Vec<Field>,
    #[props(default)] format: bool,
    #[props(default)] attach: bool,
) -> Element {
    let hidden = hidden.into_iter().map(|(name, value)| {
        rsx! { Input { r#type: "hidden", name: "{name}", value: "{value}" } }
    });
    // a new mail's From, To, Cc, Bcc and Subject, each a labelled line above the words, as Gmail's window has them
    let fields = fields.into_iter().map(|f| {
        let shown = f.name.is_empty();
        rsx! {
            label { class: "composer-field",
                span { "{f.label}" }
                if shown { span { class: "value", "{f.value}" } }
                else { Input { r#type: if f.name == "subject" { "text" } else { "email" }, name: "{f.name}", value: "{f.value}",
                    multiple: (f.name != "subject").then_some(true), required: f.required.then_some(true) } }
            }
        }
    });
    rsx! {
        Form { id: id.clone(), action, file: attach,
            {hidden}
            {fields}
            {editor(&id, &send, &placeholder, format)}
            // the browser's own picker: it names the files chosen, no script (task rust-comment-files-filter)
            if attach {
                label { class: "composer-attach",
                    span { "Attach images or PDFs" }
                    Input { r#type: "file", name: "files", accept: ATTACHABLE, multiple: true }
                }
            }
            script { dangerous_inner_html: EDITOR_JS }
        }
    }
}

/// One line of a new mail's head (Composer's `fields`): what it is named in the form, its label, its value. No name: shown, never
/// sent (the From his switch picked); `required`: the form is not sent without it.
#[derive(Clone, PartialEq, Deserialize, serde::Serialize, Debug)]
pub struct Field {
    #[serde(default)]
    pub name: String,
    pub label: String,
    #[serde(default)]
    pub value: String,
    #[serde(default)]
    pub required: bool,
}

/// Gmail's formatting bar (@nha, 2026-10-31, on /mail: « we need the richer controls besides attachment (bold etc.) »): each
/// button marks the selection the Markdown way, which the mail's HTML is made from (dash_mail's `html_of`), its text sent as typed.
const FORMATS: [(&str, &str, &str); 6] = [
    ("bold", "Bold", "B"),
    ("italic", "Italic", "I"),
    ("link", "Insert link", "🔗"),
    ("ul", "Bulleted list", "•"),
    ("ol", "Numbered list", "1."),
    ("quote", "Quote", "❝"),
];

/// The editor every compose box draws, Composer's and the compass's alike, so no box draws a field of its own: the hidden
/// `text` EDITOR_JS fills at Send, the toolbar, and the field with Discard and `send` inside it.
fn editor(id: &str, send: &str, placeholder: &str, format: bool) -> Element {
    let doc = notion_doc::from_text("");
    let marks = FORMATS.map(|(mark, title, face)| {
        let (mark, title) = (Some(mark), Some(title));
        rsx! { ToolbarButton { look: "editor-format", mark, title, "{face}" } }
    });
    rsx! {
        Input { r#type: "hidden", name: "text", value: "" }
        div { class: "editor",
            Toolbar { id: "{id}-tools", label: "Editor", look: "editor-tools",
                if format {
                    ToolbarGroup { {marks.into_iter()} }
                    ToolbarSeparator {}
                }
                ToolbarButton { look: "editor-undo", "Undo" span { "aria-hidden": "true", KbdGroup { Kbd { "⌘" } Kbd { "Z" } } } }
                ToolbarButton { look: "editor-redo", "Redo" span { "aria-hidden": "true", KbdGroup { Kbd { "⇧" } Kbd { "⌘" } Kbd { "Z" } } } }
                ToolbarSeparator {}
                ToolbarButton { look: "editor-preview", "Preview" }
            }
            div { class: "editor-field", "data-empty": true, "data-placeholder": "{placeholder}",
                DocView { doc, view: View::Notion, editable: true, said: placeholder.to_string() }
                div { class: "editor-controls",
                    Button { r#type: "button", look: "quiet editor-discard", "Discard" }
                    Button { look: "editor-send", title: "{send} (⌘Enter)", "{send}" }
                }
            }
        }
    }
}

/// The compass's comment box, `quote` the words the compass picked: the menu's template holds it empty, /design's Page
/// comments draws it open (task rust-design-page-comments-compose; @nha, 2026-11-10: « the compass element and the box I type
/// in should be here as well »). Its words are the documents editor's, as every compose box's (task compose-uses-doc-editor),
/// then Cancel, which closes it, and what the send says. Here, beside Composer: components cannot read DocView (a module
/// loop). Its look is documents.css's `.doc-compose`, written once for both.
pub fn comment_box(quote: &str) -> Element {
    rsx! {
        div { class: "doc-compose",
            blockquote { "{quote}" }
            {editor("dash-compose", "Send", "Your comment", false)}
            div {
                Button { look: "quiet", r#type: "button", value: "cancel", "Cancel" }
                " "
                span { role: "status" }
            }
        }
    }
}

/// A comment anchored on a document's text: the words it quotes, and its thread's id on the page's rail (`c-<id>`).
#[derive(Clone, PartialEq, Deserialize, serde::Serialize)]
pub struct Anchor {
    pub quote: String,
    pub thread: String,
}

/// The one component every document is drawn with (task rust-one-doc-component; @nha, 2026-11-09: « we need to have _one_
/// model, _one_ component that can turn into views depending on the UI. »): `doc` a canvist_model document as saved
/// (design::notion_doc), `view` its style, never a second component. Each of `anchors` marks the first passage its quote
/// is found in, at (block id, byte range) of the model, tied to its thread by `data-thread`: a comment sits on the words it
/// is about (@nha, 2026-11-09: « this should be a notion-like doc. which allows comments naturally. »). `editable`: edited in
/// place in the browser, kept in its memory, never saved, as the Notion tab's lines (task rust-notion-doc-editable); plain text
/// only, so a paste brings no markup. `said`: its title while editable, which names it, « not saved » unless given (a Composer's
/// words are sent: its placeholder).
#[component]
pub fn DocView(
    doc: Value,
    view: View,
    #[props(default)] anchors: Vec<Anchor>,
    #[props(default)] editable: bool,
    #[props(default)] said: Option<String>,
) -> Element {
    match notion_doc::load(&doc, 0) {
        Ok(doc) => {
            let at = anchored(&doc, &anchors);
            // it says so where it is edited: a reload brings the saved words back
            let edit = editable.then_some("plaintext-only");
            let (role, multi, said) = match editable {
                true => (
                    Some("textbox"),
                    Some("true"),
                    Some(said.unwrap_or_else(|| "Edited here only, not saved".into())),
                ),
                false => (None, None, None),
            };
            rsx! {
                div { class: "dv {view.class()}", contenteditable: edit, role, "aria-multiline": multi, title: said,
                    {blocks(&doc, None, view, &at)}
                }
            }
        }
        Err(e) => rsx! { p { class: "dv", "This document does not read: {e}" } },
    }
}

/// Where each anchor lands: the first text block holding its quote, and the quote's byte range there. A quote found nowhere
/// (the words changed since) anchors nowhere: its thread stays on the page whole, in the rail.
pub fn anchored<'a>(
    doc: &Document,
    anchors: &'a [Anchor],
) -> Vec<(BlockId, std::ops::Range<usize>, &'a str)> {
    (anchors.iter())
        .filter(|a| !a.quote.trim().is_empty())
        .filter_map(|a| {
            let quote = a.quote.trim();
            doc.text_blocks().find_map(|b| {
                let at = b.inline()?.as_str().find(quote)?;
                Some((b.id(), at..at + quote.len(), a.thread.as_str()))
            })
        })
        .collect()
}

type At<'a> = [(BlockId, std::ops::Range<usize>, &'a str)];

fn blocks(doc: &Document, parent: Option<BlockId>, view: View, at: &At) -> Element {
    rsx! {
        for id in doc.children(parent).iter().copied() {
            {block(doc, id, view, at)}
        }
    }
}

/// A line's words, run by run: a link mark opens its target (its href read in the text form), an anchored passage is
/// marked for its thread.
fn line(doc: &Document, id: BlockId, at: &At) -> Element {
    let Some(text) = doc.block(id).and_then(|b| b.inline()) else {
        return rsx! {};
    };
    let link = doc.schema().mark_type("link");
    let mine: Vec<_> = at.iter().filter(|(b, ..)| *b == id).collect();
    // each run one piece, a link its one <a>; cut inside where an anchor starts or ends, so a passage is marked whole
    let mut runs = vec![];
    for run in text.runs() {
        let (start, end) = (run.range.start as usize, run.range.end as usize);
        let mut cuts = vec![start, end];
        for (_, r, _) in &mine {
            cuts.extend(
                [r.start, r.end]
                    .into_iter()
                    .filter(|c| (start..end).contains(c)),
            );
        }
        cuts.sort_unstable();
        cuts.dedup();
        let href = (link.and_then(|l| run.marks.get(l)))
            .and_then(|m| m.attrs.get("href")?.as_str().map(str::to_string))
            .filter(|h| local_path(h));
        // a tab drawn as a space: in the text form a tab parts an event's words from its time (`what\twhen`), so no line of a
        // document reads as one; one byte for one, so the anchors' ranges hold (/code-review of rust-task-brief-linear)
        let cut: Vec<(String, Option<&str>)> = (cuts.windows(2))
            .map(|w| {
                let thread = mine
                    .iter()
                    .find(|(_, r, _)| r.start <= w[0] && w[1] <= r.end);
                (
                    text.as_str()[w[0]..w[1]].replace('\t', " "),
                    thread.map(|t| t.2),
                )
            })
            .collect();
        runs.push((href, cut));
    }
    let words = |cut: &[(String, Option<&str>)]| {
        rsx! {
            for (words, thread) in cut.iter() {
                match thread {
                    Some(thread) => rsx! { mark { class: "dv-anchor", "data-thread": "{thread}", "{words}" } },
                    None => rsx! { "{words}" },
                }
            }
        }
    };
    let last = runs.iter().rev().find_map(|r| r.0.clone());
    rsx! {
        pre { class: "doc-text",
            for (href, cut) in &runs {
                match href {
                    Some(href) => rsx! { a { href: "{href}", {words(cut)} } },
                    None => words(cut),
                }
            }
            if let Some(href) = last { {target(&href)} }
        }
    }
}

fn block(doc: &Document, id: BlockId, view: View, at: &At) -> Element {
    let Some(b) = doc.block(id) else {
        return rsx! {};
    };
    let text = b.inline().map_or("", |t| t.as_str());
    match &*doc.schema().block(b.kind()).name {
        "title" => rsx! { h1 { class: "doc-title", "{text}" } },
        "line" => line(doc, id, at),
        "note" => {
            if view == View::Chat {
                rsx! {}
            } else {
                rsx! { aside { class: "dv-note", span { class: "dv-private", "Private" } "{text}" } }
            }
        }
        "section" => rsx! { section { class: "dv-section", {blocks(doc, Some(id), view, at)} } },
        _ => {
            let attr = |k: &str| b.attrs().get(k).cloned();
            let dom = attr("dom")
                .and_then(|v| v.as_str().map(str::to_string))
                .unwrap_or_default();
            let reading =
                attr("reading").is_some_and(|v| matches!(v, canvist_model::Value::Bool(true)));
            // still reading: the design system's loading state under the item's id, which the page's LIVE_JS fills in place
            // (task rust-today-doc; @nha, 2026-11-09: « This is a card loading state. same element. »)
            match (reading, dom.is_empty()) {
                (true, _) => {
                    rsx! { Skeleton { id: dom, what: "", lines: 1, look: "dv-item", span { class: "text-only", "being read" } {blocks(doc, Some(id), view, at)} } }
                }
                (false, true) => {
                    rsx! { div { class: "dv-item", {blocks(doc, Some(id), view, at)} } }
                }
                (false, false) => {
                    rsx! { div { id: dom, class: "dv-item", {blocks(doc, Some(id), view, at)} } }
                }
            }
        }
    }
}

/// Whether a line of it is still being read: the page then runs its LIVE_JS, which fills it in place.
pub fn reading(doc: &Value) -> bool {
    doc["content"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|b| b["attrs"]["reading"] == true)
}

/// What a message sends: its public lines alone, never a private note (rust/CLAUDE.md rule 7). A send of a draft reads this.
pub fn sent(doc: &Value) -> Result<String, String> {
    notion_doc::sent(doc).map_err(|e| e.to_string())
}

/// A page of lines as the one model holds it, /today's: its title, then an item a line.
pub fn outline(title: &str, lines: &[Line]) -> Value {
    notion_doc::outline(title, lines)
}

/// A document's two forms in one view: its HTML, the server render, and the text an agent reads of it.
pub fn view_forms(doc: Value, view: View) -> (String, String) {
    anchored_forms(doc, view, vec![])
}

/// Its two forms with comments anchored on its text.
pub fn anchored_forms(doc: Value, view: View, anchors: Vec<Anchor>) -> (String, String) {
    let html = dioxus_ssr::render_element(rsx! { DocView { doc, view, anchors } });
    let text = crate::words::text_of(&html);
    (html, text)
}

fn facts(all: &[Fact]) -> Element {
    rsx! {
        dl { class: "doc-facts",
            for f in all {
                dt { "{f.label}" }
                dd {
                    // a link a page may open, never a script's; the text form names where it goes when its words do not
                    match card_href(&f.href) {
                        Some(href) => rsx! {
                            a { href: "{href}", "{f.value}" }
                            if f.value != href { {target(href)} }
                        },
                        None => rsx! { "{f.value}" },
                    }
                }
            }
        }
    }
}

/// Notion's threads (/design?tab=notion): a passage quoted, then each comment, its avatar, name and when, its words drawn by
/// the one DocView (rust/CLAUDE.md rule 7), `editable` in place where the page allows it. `on_page`: each a page's comment,
/// which names what it is on, its passage or else the whole page (task rust-comment-files-filter); a task's Activity is on the
/// task alone.
pub(crate) fn threads(all: &[Thread], editable: bool, on_page: bool) -> Element {
    rsx! {
        div { class: "doc-threads",
            for t in all {
                div { class: "doc-thread", id: (!t.id.is_empty()).then(|| t.id.clone()),
                    if !t.quote.is_empty() {
                        div { class: "doc-quote", "{t.quote}" }
                    } else if on_page {
                        div { class: "doc-quote", "Whole page" }
                    }
                    for s in &t.said {
                        div { class: "doc-said",
                            // the initials are the name's picture: the text says the name once (/code-review of PR 1250)
                            div { class: "doc-who", span { "aria-hidden": "true", Avatar { initials: initials(&s.who) } } "{s.who}" if !s.when.is_empty() { span { class: "doc-meta", "{s.when}" } } }
                            // each paragraph a block, its line breaks kept
                            div { class: "doc-words", DocView { doc: notion_doc::from_text(&s.words), view: View::Notion, editable } }
                        }
                    }
                    if !t.files.is_empty() {
                        AttachmentGroup {
                            for (i, (name, href, kind)) in t.files.iter().enumerate() {
                                Attachment { id: "{t.id}-file-{i}", name: name.clone(), kind: kind.clone(), href: href.clone(),
                                    picture: is_picture(kind) }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// A field drawn by `draw` under the id `<article>-<key>`, or the Skeleton under that same id while it is read, so the refill
/// loop swaps one for the other in place (task rust-skeleton-live); its text form says so (`.text-only`).
fn read(
    id: &str,
    key: &str,
    what: &str,
    field: Option<&str>,
    draw: impl FnOnce(String, &str) -> Element,
) -> Element {
    let at = format!("{id}-{key}");
    match field {
        Some(text) => draw(at, text),
        None => rsx! {
            Skeleton { id: at, what,
                span { class: "text-only", "{what}: being read." }
            }
        },
    }
}

/// Linear's issue view from /design?tab=board (families::issue and its parts; @nha, 2026-11-10: « not matching the design yet
/// (see elixir/linear as example) »): its title, its description, its checks and last run, then its Activity ending on a
/// comment box, its properties beside. The description and each comment are DocView's, editable in place; the box posts to
/// the compass as a comment on /tasks/<id>, which the page's rail then lists (forms::notes).
fn brief(id: &str, b: &TaskBrief) -> Element {
    let judged = b.checks.iter().filter(|c| c.met.is_some()).count();
    let met = b.checks.iter().filter(|c| c.met == Some(true)).count();
    let title = if judged > 0 {
        format!("Done when: {met} of {} met", b.checks.len())
    } else {
        "Done when".to_string()
    };
    // the skeleton under its own id, so the refill swaps the whole part around it, `<id>-checks`: its title and its checks
    let checks = if b.checks.is_empty() {
        read(
            id,
            "checks-read",
            "Its checks",
            b.no_checks.as_deref(),
            |_, t| rsx! { p { "{t}" } },
        )
    } else {
        rsx! {
            for c in &b.checks {
                {families::issue_check(c.met, verdict(c.met), &c.text)}
            }
        }
    };
    let pr = (b.pr.as_deref()).and_then(|url| Some((url, pr_number("", url).ok()?)));
    let main = rsx! {
        {read(id, "why", "Its words", b.why.as_deref(), |at, t| rsx! {
            div { id: at, DocView { doc: notion_doc::from_text(t), view: View::Notion, editable: true } }
        })}
        div { id: "{id}-checks", {families::issue_part(&title, checks)} }
        {families::issue_part("Last run", rsx! {
            p { "{b.run}" }
            if !b.said.is_empty() {
                pre { class: "quote", "{b.said}" }
            }
        })}
        {families::issue_part("Activity", rsx! {
            if !b.unread.is_empty() {
                p { "{b.unread}" }
            }
            for e in &b.events {
                {families::issue_event(&e.what, &e.when)}
            }
            {threads(&b.comments, true, false)}
            Composer { id: "{id}-comment", action: "/compass", send: "Comment", placeholder: "Leave a comment...",
                hidden: vec![("page".to_string(), format!("/tasks/{}", b.id))] }
        })}
    };
    // his menus, as board_live.ex's issue page: each pick posts one field to /tasks/<id>/set (task rust-board-buttons-new-issue)
    let action = set_path(&b.id);
    let status = rsx! { {families::status(b.status.kind())} span { "{b.status.name()}" } };
    let state = match b.movable {
        true => rsx! {
            PropertyMenu { id: "{id}-status", action: action.clone(), field: "status", label: "Change status", current: status,
                choices: COLUMNS.iter().map(|c| (c.class().to_string(), c.name().to_string(), *c == b.status)).collect() }
        },
        false => rsx! { div { class: "li-prop", {status} } },
    };
    let priority = b.priority.unwrap_or(0);
    let side = b.sidebar.as_deref();
    let due = match &b.deadline {
        Some(day) => format!("Due {day}"),
        None => "No due date".to_string(),
    };
    let side = rsx! {
        PropertyMenu { id: "{id}-priority", action: action.clone(), field: "priority", label: "Set priority",
            current: families::issue_priority(priority),
            choices: PRIORITIES.iter().map(|p| {
                let value = if *p == 0 { "none".to_string() } else { p.to_string() };
                (value, families::priority_name(*p).to_string(), *p == priority)
            }).collect() }
        if let Some(s) = side {
            {assignee_menu(id, &action, s)}
            {cycle_menu(id, &action, s)}
        }
        PropertyMenu { id: "{id}-due", action: action.clone(), field: "due", label: "Set due date",
            current: rsx! { {families::icon(families::CALENDAR)} span { "{due}" } },
            choices: due_choices(b) }
        if let Some(s) = side {
            {remind_menu(id, &action, s)}
        }
        {families::issue_prs(pr.as_slice())}
        if let Some((url, _)) = pr {
            {target(url)}
        }
        if let Some(s) = side {
            h3 { class: "li-side-head", "Labels" }
            div { class: "li-tags",
                for label in &s.labels {
                    Badge { look: "li-tag", "{label}" }
                }
                PropertyMenu { id: "{id}-labels", action: action.clone(), field: "labels", label: "Change or add labels...",
                    many: true, current: families::icon(families::PLUS),
                    choices: s.label_set.iter().map(|l| (l.clone(), l.clone(), s.labels.contains(l))).collect() }
            }
            h3 { class: "li-side-head", "Project" }
            PropertyMenu { id: "{id}-project", action: action.clone(), field: "project", label: "Add to project...",
                current: rsx! { {families::icon(families::BOARD)} span { {s.epic.as_deref().unwrap_or("Add to project")} } },
                choices: std::iter::once(("none".to_string(), "No project".to_string(), s.epic.is_none()))
                    .chain(s.epics.iter().map(|e| (e.clone(), e.clone(), s.epic.as_ref() == Some(e))))
                    .collect() }
        }
    };
    // its key and its place among the board's issues at the way back's right end, as Linear's « 42 / 2193 ↓ ↑ »
    let step = |to: &Option<String>, label: &'static str, path: &'static str| {
        to.as_ref().map(|href| {
            rsx! { a { class: "li-step", href: "{href}", title: label, "aria-label": label,
            span { class: "text-only", "{label}: {href}" } {families::icon(path)} } }
        })
    };
    let bar = rsx! {
        span { class: "meta li-crumb-key", "{b.id}" }
        if let Some(p) = &b.position {
            span { class: "meta li-position", "{p.at} / {p.of}" }
            {step(&p.next, "Next issue", families::DOWN)}
            {step(&p.previous, "Previous issue", families::UP)}
        }
    };
    let tools = rsx! { Copies {
        CopyButton { id: "{id}-copy-link", label: "Copy link", text: crate::words::task_href(&b.id, None), origin: true,
            icon: families::icon(families::LINK) }
        CopyButton { id: "{id}-copy-id", label: "Copy ID", text: b.id.clone(), icon: families::icon(families::DOC) }
        CopyButton { id: "{id}-copy-branch", label: "Copy branch name", text: format!("task/{}", b.id),
            icon: families::icon(families::CODE) }
    } };
    rsx! {
        div { "data-status": b.status.class(),
            {families::issue(&b.title, &b.says, bar, tools, state, main, side)}
        }
        {crate::components::menu_filter()}
    }
}

/// Linear's assignee menu, the task's owner: no « No assignee », none is claude (task rust-issue-sidebar-menus).
fn assignee_menu(id: &str, action: &str, s: &Sidebar) -> Element {
    let name = s
        .owners
        .iter()
        .find(|(v, _)| *v == s.owner)
        .map_or(s.owner.as_str(), |(_, n)| n.as_str());
    rsx! {
        PropertyMenu { id: "{id}-assignee", action, field: "assignee", label: "Assign to...", initials: true, numbered: true,
            current: rsx! { span { "aria-hidden": "true", Avatar { initials: initials_of(name), look: "li-who" } } span { "{name}" } },
            choices: s.owners.iter().map(|(v, n)| (v.clone(), n.clone(), *v == s.owner)).collect() }
    }
}

/// Linear's cycle menu: none, then each cycle offered with its dates and when.
fn cycle_menu(id: &str, action: &str, s: &Sidebar) -> Element {
    let held = s.cycle.as_ref().map(|c| {
        let named = s
            .cycles
            .iter()
            .find(|(v, _)| v == c)
            .map_or(c.as_str(), |(_, n)| n.as_str());
        named.split(" · ").next().unwrap_or(named).to_string()
    });
    rsx! {
        PropertyMenu { id: "{id}-cycle", action, field: "cycle", label: "Add to cycle...",
            current: rsx! { {families::icon(families::CYCLE)} span { {held.unwrap_or_else(|| "Add to cycle".into())} } },
            choices: std::iter::once(("none".to_string(), "No cycle".to_string(), s.cycle.is_none()))
                .chain(s.cycles.iter().map(|(v, n)| (v.clone(), n.clone(), s.cycle.as_ref() == Some(v))))
                .collect() }
    }
}

/// Linear's « Remind me »: none, the presets, each with its date, then the day set when no preset is it: last, so each preset
/// keeps its digit whatever the task holds (/code-review of rust-issue-sidebar-menus).
fn remind_menu(id: &str, action: &str, s: &Sidebar) -> Element {
    let set = s.remind.as_ref();
    let mut choices = vec![("none".to_string(), "No reminder".to_string(), set.is_none())];
    choices.extend(
        s.remind_days
            .iter()
            .map(|(day, name)| (day.clone(), name.clone(), set == Some(day))),
    );
    if let Some(day) = set.filter(|d| !s.remind_days.iter().any(|(p, _)| p == *d)) {
        choices.push((day.clone(), format!("Remind {day}"), true));
    }
    let shown = set.map_or("Remind me".to_string(), |d| format!("Remind {d}"));
    rsx! {
        PropertyMenu { id: "{id}-remind", action, field: "remind", label: "Remind me...", numbered: true,
            current: rsx! { {families::icon(families::CLOCK)} span { "{shown}" } }, choices }
    }
}

/// His due menu's choices: the day set, when no preset is it, checked first, so the text form names the one set (/code-review of
/// PR 1395); then Linear's presets; then none, which clears it.
fn due_choices(b: &TaskBrief) -> Vec<(String, String, bool)> {
    let set = b.deadline.as_ref();
    let mut choices: Vec<(String, String, bool)> = vec![];
    if let Some(day) = set.filter(|d| !b.due_days.iter().any(|(_, p)| p == *d)) {
        choices.push((day.clone(), format!("Due {day}"), true));
    }
    choices.extend(
        b.due_days
            .iter()
            .map(|(name, day)| (day.clone(), format!("{name}, {day}"), set == Some(day))),
    );
    choices.push(("none".into(), "No due date".into(), set.is_none()));
    choices
}

/// The three things a draft needs before Approve, in his order (@nha, 2026-11-07: « the draft needs 3 things imo: -- the
/// context, goal -- the sender information -- the receiver information »): each its label and value.
fn three(d: &MessageDraft) -> [(&'static str, &str); 3] {
    [("Goal", &d.goal), ("From", &d.sender), ("To", &d.receiver)].map(|(l, v)| (l, v.trim()))
}

/// While it is pending, Approve, or in its place what holds it back (rfc-rust-plugins-ui section 1: « a draft missing one of
/// the three shows what is missing in place of Send »), and Request corrections: his words, the draft back to the agent (task
/// rust-draft-approve). Then its other moves. Each posts to the older app's draft write, `path`/status.
fn decision(d: &MessageDraft, path: &str) -> Element {
    let mut held: Vec<String> = three(d)
        .iter()
        .filter(|(_, v)| v.is_empty())
        .map(|(l, _)| match *l {
            "Goal" => "no goal".to_string(),
            "From" => "no sender".to_string(),
            _ => "no receiver".to_string(),
        })
        .collect();
    match d.open_comments {
        0 => {}
        1 => held.push("1 open comment to apply".into()),
        n => held.push(format!("{n} open comments to apply")),
    }
    let action = format!("{path}/status");
    let post = |status: &str, label: &str, look: &'static str| {
        rsx! {
            Form { action: action.clone(),
                Input { r#type: "hidden", name: "status", value: "{status}" }
                Button { look, "{label}" }
            }
        }
    };
    rsx! {
        if d.status == "pending" {
            if held.is_empty() {
                {post("approved", "Approve", "")}
            } else {
                p { "Not ready to approve: {held.join(\", \")}." }
            }
            Form { action: action.clone(),
                Input { r#type: "hidden", name: "status", value: "approved-pending-changes" }
                Textarea { name: "text", rows: "3", placeholder: "What to correct", required: true }
                Button { look: "quiet", "Request corrections" }
            }
        }
        for m in &d.moves {
            {post(&m.status, &m.label, "quiet")}
        }
    }
}

/// A lead's page, where its name links.
fn lead_href(lead: &str) -> String {
    format!("/o/{}", crate::segment(lead))
}

/// Its lead's last events as facts: each its day, its words linked to its source, else to the lead's page.
fn recent(d: &MessageDraft) -> Vec<Fact> {
    (d.recent.iter())
        .map(|e| Fact {
            href: e
                .href
                .clone()
                .or_else(|| (!d.lead.is_empty()).then(|| lead_href(&d.lead))),
            ..e.clone()
        })
        .collect()
}

/// An ad's text: its reader's « # title | company | … » first line a heading, the rest its words.
fn ad(text: &str) -> Element {
    let (first, rest) = text.split_once('\n').unwrap_or((text, ""));
    match first.strip_prefix("# ") {
        Some(head) => rsx! { h3 { "{head}" } {words(rest.trim_start_matches('\n'))} },
        None => words(text),
    }
}

/// The message, then what he does with it as it stands: Copy the text, a draft he sends himself; Change, a text still open to
/// his edit, the page drawn again with `?edit=1` and the text its field, Save posting it to the older app's write of his edit
/// (task rust-draft-state-edit; the older page's text_editor).
fn message(d: &MessageDraft) -> Element {
    let page = d.change.as_deref().filter(|p| local_path(p));
    let save = d.decide.as_deref().filter(|p| local_path(p));
    if let (true, Some(page), Some(save)) = (d.editing, page, save) {
        return rsx! {
            Form { id: "draft-change", action: "{save}/body",
                Textarea { name: "body", rows: "16", required: true, "aria-label": "The message", text: d.text.clone() }
                Button { "Save" }
                " "
                ButtonLink { look: "quiet", href: "{page}", "Cancel" }
            }
        };
    }
    // a plain field gets the words without the stars of his bold
    let plain = match d.keep_bold {
        true => d.text.clone(),
        false => d.text.replace("**", ""),
    };
    rsx! {
        div { id: "draft-text", DocView { doc: notion_doc::from_text(&d.text), view: View::Gmail } }
        if d.copy || page.is_some() {
            Copies {
                if d.copy { CopyText { id: "draft-copy", text: plain, select: "draft-text" } }
                if let (Some(page), Some(_)) = (page, save) {
                    ButtonLink { look: "quiet", href: "{page}?edit=1", "Change" }
                }
            }
        }
    }
}

/// « Why and context »'s one line, as the older page's: what it waits on, then its lead.
fn why_line(d: &MessageDraft) -> String {
    let wait = match d.waits.len() {
        _ if d.waits_unread => "What it waits on was not read".to_string(),
        0 => "Waits on nothing".to_string(),
        _ => format!(
            "Waits on {}",
            (d.waits.iter())
                .map(|w| w.value.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    };
    match d.lead.as_str() {
        "" => wait,
        lead => format!("{wait} · lead {lead}"),
    }
}

/// State, as the older page's: its status, channel and id, each decision's day, where it goes and whether his bold reaches the
/// reader; while it is open, the day it goes out, saved to `schedule` (empty: with the next outbox).
fn state(d: &MessageDraft) -> Element {
    let fact = |label: &str, value: &str| Fact {
        label: label.into(),
        value: value.into(),
        href: None,
    };
    let mut all = vec![fact(
        "Status",
        &[d.status.as_str(), &d.channel, &d.id]
            .iter()
            .filter(|w| !w.is_empty())
            .copied()
            .collect::<Vec<_>>()
            .join(" · "),
    )];
    all.extend(d.dates.iter().cloned());
    for (label, value) in [("Where", &d.place), ("Bold", &d.bold)] {
        if !value.is_empty() {
            all.push(fact(label, value));
        }
    }
    rsx! {
        {facts(&all)}
        if let Some(path) = d.schedule.as_deref().filter(|p| local_path(p)) {
            Form { id: "send-on", action: "{path}",
                label { class: "composer-field",
                    span { "Goes out on (empty: with the next outbox)" }
                    Input { r#type: "date", name: "send-on", value: "{d.send_on}" }
                }
                Button { "Save" }
            }
        } else if !d.send_on.is_empty() {
            {facts(&[fact("Goes out on", &d.send_on)])}
        }
    }
}

/// Waits, as the older page's: actionable, or in flight with each step it waits on, linked, and what may go meanwhile.
fn waits(d: &MessageDraft) -> Element {
    rsx! {
        if d.waits_unread {
            p { "Not read: what it waits on is not known (the page's note says why)." }
        } else if d.waits.is_empty() {
            p { Badge { look: "chip", "Actionable" } }
        } else {
            p { Badge { look: "chip", "In flight" } " waits on "
                for (i, w) in d.waits.iter().enumerate() {
                    if i > 0 { ", " }
                    match card_href(&w.href) {
                        Some(href) => rsx! { a { href: "{href}", "{w.value}" } },
                        None => rsx! { "{w.value}" },
                    }
                }
            }
            if !d.meanwhile.is_empty() {
                {facts(&[Fact { label: "Meanwhile".into(), value: d.meanwhile.clone(), href: None }])}
            }
        }
    }
}

fn draft(d: &MessageDraft) -> Element {
    // its whole page's context alone: a listed draft says it in one line (/simplify of PR 1250)
    let context = || {
        let mut context: Vec<Fact> = three(d)
            .iter()
            .map(|(label, value)| Fact {
                label: label.to_string(),
                value: if value.is_empty() { "none" } else { value }.into(),
                href: None,
            })
            .collect();
        if !d.lead.is_empty() {
            context.push(Fact {
                label: "Lead".into(),
                value: d.lead.clone(),
                href: Some(lead_href(&d.lead)),
            });
        }
        if let Some(url) = &d.apply {
            context.push(Fact {
                label: "Application form".into(),
                value: url.clone(),
                href: Some(url.clone()),
            });
        }
        context.push(Fact {
            label: "Scores".into(),
            value: d.scores.clone(),
            href: None,
        });
        context
    };
    let receiver = match d.receiver.trim() {
        "" => "no receiver",
        r => r,
    };
    rsx! {
        header { class: "doc-head",
            // filtered where it is drawn too, so no caller that skips `check` draws a script's link (/code-review of the merge)
            match card_href(&d.href) {
                Some(href) => rsx! { a { class: "doc-title", href: "{href}", "{receiver}" } {target(href)} },
                None => rsx! { h1 { class: "doc-title", "{receiver}" } },
            }
            p { class: "doc-meta", Badge { look: "chip", "{d.status}" } Badge { look: "li-tag", "{d.channel}" } }
        }
        // Listed, with somewhere to open it whole, its context in one line: a list of them reads at a glance.
        // Its lead is the context a list needs; its sender, always him, is on its own page (/code-review of PR 1250).
        if d.href.is_some() {
            p { class: "doc-line",
                "{d.goal}"
                if !d.lead.is_empty() { " · Lead: {d.lead}" }
                " · Scores: {d.scores}"
            }
        } else {
            // what it answers above it, as the older page's first screen (task rust-draft-posting)
            if !d.posting.is_empty() || !d.ad.is_empty() {
                {part("The ad", rsx! {
                    {facts(&d.posting)}
                    if !d.ad.is_empty() {
                        Fold { id: "ad-text", open: false, summary: rsx! { "The ad's text" }, {ad(&d.ad)} }
                    }
                })}
            }
            {part("Context", facts(&context()))}
            if !d.recent.is_empty() {
                {part("Recent", facts(&recent(d)))}
            }
            // read into the one model and drawn by its one component (task rust-one-doc-component)
            if !d.text.is_empty() {
                {part("Message", message(d))}
            }
            if let Some(path) = d.decide.as_deref().filter(|p| local_path(p)) {
                {part("Decision", decision(d, path))}
            }
            // folded under one line, as the older page's « Why and context »: what it waits on, then its lead
            if d.stated || !d.why.is_empty() {
                Fold { id: "why", open: false,
                    summary: rsx! { "Why and context " span { "{why_line(d)}" } },
                    if !d.why.is_empty() {
                        {part("Why this text", rsx! { for w in &d.why { p { class: "doc-text", "{w}" } } })}
                    }
                    if d.stated {
                        {part("State", state(d))}
                        {part("Waits", waits(d))}
                    }
                }
            }
            for v in &d.earlier {
                {part(&v.title, words(&v.text))}
            }
        }
    }
}

fn entry(e: &CrmEntry) -> Element {
    let today = e
        .today
        .as_deref()
        .and_then(dash_core::days)
        .unwrap_or_else(crate::components::today);
    rsx! {
        header { class: "doc-head",
            h1 { class: "doc-title", "{e.name}" }
            p { class: "doc-meta", "{e.company}" Badge { look: "chip", "{e.stage}" } }
        }
        {part("Facts", facts(&e.facts))}
        {part("Timeline", rsx! {
            Timeline {
                for f in &e.timeline {
                    TimelineEvent { date: f.label.clone(), today, "{f.value}" }
                }
            }
        })}
        if !e.notes.is_empty() {
            {part("Notes", words(&e.notes))}
        }
    }
}

/// Each titled part, its words' line breaks kept: an RFC's sections, a skill's.
fn parts(all: &[Part]) -> Element {
    rsx! {
        for p in all {
            {part(&p.title, words(&p.text))}
        }
    }
}

fn rfc(r: &Rfc) -> Element {
    rsx! {
        header { class: "doc-head",
            h1 { class: "doc-title", "{r.title}" }
            p { class: "doc-meta", Badge { look: "chip", "{r.status}" } }
            p { class: "doc-lead", b { "{r.bottom_line}" } }
        }
        {parts(&r.parts)}
    }
}

fn skill(k: &Skill) -> Element {
    rsx! {
        header { class: "doc-head",
            h1 { class: "doc-title", "{k.name}" }
            p { class: "doc-lead", "{k.description}" }
        }
        {parts(&k.parts)}
    }
}

/// A document drawn by its type's template, `id` its DOM id.
pub fn draw(id: &str, doc: &Doc) -> Element {
    let body = match doc {
        Doc::Rfc(r) => rfc(r),
        Doc::TaskBrief(b) => brief(id, b),
        Doc::MessageDraft(d) => draft(d),
        Doc::CrmEntry(e) => entry(e),
        Doc::PageComments(c) => threads(&c.threads, false, true),
        Doc::Skill(k) => skill(k),
    };
    rsx! { article { id, class: "doc doc-{doc.kind()}", {body} } }
}

/// An RFC page's section ids, in its order: each `<section>`'s or heading's id, as its links name them. A Rust task names one
/// as the layer it changes (task rust-task-names-layer).
pub fn section_ids(html: &str) -> Vec<String> {
    let mut ids: Vec<String> = vec![];
    for tag in html.split('<').skip(1) {
        let head = &tag[..tag.find('>').unwrap_or(tag.len())];
        let named = ["section ", "h2 ", "h3 ", "h4 "]
            .iter()
            .any(|t| head.starts_with(t));
        if let Some(id) = head
            .split(" id=\"")
            .nth(1)
            .and_then(|r| r.split('"').next())
            && named
            && !ids.iter().any(|i| i == id)
        {
            ids.push(id.to_string());
        }
    }
    ids
}

/// A document's two forms: its HTML, the server render, and the text an agent reads of it.
pub fn forms(id: &str, doc: &Doc) -> (String, String) {
    let html = dioxus_ssr::render_element(draw(id, doc));
    let text = crate::words::text_of(&html);
    (html, text)
}

/// /design?tab=documents: each type, its name, where it is used, an example as a tree gives it.
pub const TYPES: [(&str, &str, &str); 6] = [
    (
        "RFC",
        "The decision pages of private/docs.",
        r#"{"kind":"rfc","title":"One document model","status":"Review","bottom_line":"Every page picks one of six types.","parts":[{"title":"Context","text":"Each page laid its document out by hand."},{"title":"Decision","text":"A template a type, from the design system.\nA page gives fields, never layout."}]}"#,
    ),
    (
        "Task brief",
        "A task's page, /tasks/<id>.",
        r#"{"kind":"task-brief","id":"a-task","title":"A task's title","status":"todo","says":"the next pick","why":"(@nha: « because »)\nIts second line","checks":[{"text":"One thing: a test","met":true},{"text":"Another","met":null}],"no_checks":"None written.","run":"Try 1, 5 min ago: done.","pr":"https://github.com/o/r/pull/12","events":[{"what":"Started, try 1","when":"2026-11-09 10:00 UTC"}],"comments":[{"said":[{"who":"Nicolas","words":"Fix this first"}]}],"priority":2,"deadline":"2026-11-15","sidebar":{"owner":"rust","labels":["Design"],"epic":"rust-poc","cycle":"2026-11-09","remind":null,"owners":[["claude","Claude"],["nicolas","Nicolas Ha"],["rust","Rust"]],"label_set":["Bug","Design"],"epics":["crm","rust-poc"],"cycles":[["2026-11-09","Cycle Nov 9 - Nov 15 · Current"],["2026-11-16","Cycle Nov 16 - Nov 22 · Upcoming"]],"remind_days":[["2026-11-12","Tomorrow · Thu 12 Nov 2026"]]},"position":{"at":3,"of":9,"previous":"/tasks/z-task","next":"/tasks/b-task"},"movable":true,"due_days":[["Tomorrow","2026-11-12"],["In one week","2026-11-15"]]}"#,
    ),
    (
        "Message draft",
        "A draft with its context, each of /drafts.",
        r##"{"kind":"message-draft","receiver":"Ana (Acme)","sender":"Nicolas Ha","channel":"linkedin","status":"pending","goal":"A first message","lead":"acme-platform","scores":"too_long 0.4","text":"Bonjour Ana,\nUne question courte.","posting":[{"label":"Title","value":"Platform engineer"},{"label":"Company","value":"Acme"},{"label":"Location","value":"Remote (EU)"},{"label":"Contract","value":"Freelance"},{"label":"Source","value":"https://acme.example/jobs/1","href":"https://acme.example/jobs/1"}],"ad":"# Platform engineer | Acme | Remote (EU)\nSix months, Rust.","apply":"https://acme.example/jobs/1/apply","recent":[{"label":"2026-11-10","value":"Search of 2026-11-10: posted the day before","href":"https://acme.example/jobs/1"},{"label":"2026-11-09","value":"Noted"}]}"##,
    ),
    (
        "CRM entry",
        "A lead or a contact.",
        r#"{"kind":"crm-entry","name":"Ana","company":"Acme","stage":"contacted","facts":[{"label":"Role","value":"CTO"},{"label":"Channel","value":"linkedin"}],"timeline":[{"label":"2026-11-02","value":"First message"},{"label":"2026-11-05","value":"Her reply"}],"notes":"Asked for a rate."}"#,
    ),
    (
        "Page comments",
        "Comments on a page, as Notion's: a passage quoted, then each comment.",
        r#"{"kind":"page-comments","threads":[{"quote":"One tree, two renderers","said":[{"who":"Nicolas Ha","when":"Just now","words":"Both, really?"},{"who":"Claude","when":"Just now","words":"The window and the web."}]},{"said":[{"who":"Nicolas Ha","words":"On the page whole."}]}]}"#,
    ),
    (
        "Skill",
        "A SKILL.md, a repo's, the global ones or a plugin's: when an agent uses it, then its steps.",
        r#"{"kind":"skill","name":"mail-poll","description":"Use when checking the job-search mail since the last check.","parts":[{"title":"Steps","text":"1. Read the mail since the last check.\n2. File what matters on its lead."},{"title":"Never","text":"Notify him for a newsletter."}]}"#,
    ),
];

/// An example of TYPES, as its tree gives it.
pub fn example(json: &str) -> Doc {
    serde_json::from_str(json).expect("an example of TYPES reads")
}

/// /design?tab=documents: DocView's views, (slug, name, where it is used, the view), each drawing `example_message`.
pub const VIEWS: [(&str, &str, &str, View); 3] = [
    (
        "view-notion",
        "Notion view",
        "A document as a page: its blocks in order, a private note under its words.",
        View::Notion,
    ),
    (
        "view-gmail",
        "Gmail view",
        "A message draft's words, /drafts/<id>: each section's private note beside it.",
        View::Gmail,
    ),
    (
        "view-chat",
        "Chat view",
        "A message draft's words as chat bubbles: each public section a bubble, in order; private notes never drawn.",
        View::Chat,
    ),
];

/// The one document each view of VIEWS draws: made up, as rust/ rides to the CI mirror.
pub fn example_message() -> Value {
    notion_doc::message(&[
        ("Bonjour Ana,", ""),
        (
            "Une question courte : le poste est-il ouvert en remote ?",
            "Her post says hybrid; asking first saves a call.",
        ),
        ("Nicolas", "Signed as on every first message."),
    ])
}

/// A type of TYPES as /design draws it, its two forms: Page comments with the menu's compass and its empty comment box under
/// the threads, the components a page's comments are written with (task rust-design-page-comments-compose; @nha, 2026-11-10:
/// « the compass element and the box I type in should be here as well »). Only here: a page's own comments get the menu's.
pub fn designed(doc: &Doc) -> (String, String) {
    let id = format!("d-{}", doc.kind());
    if !matches!(doc, Doc::PageComments(_)) {
        return forms(&id, doc);
    }
    let html = dioxus_ssr::render_element(rsx! {
        {draw(&id, doc)}
        {crate::components::compass(None)}
        {comment_box("")}
    });
    let text = crate::words::text_of(&html);
    (html, text)
}

/// The Documents tab: each type as a family's section, light and dark, its two forms folded under it, as the Components tab.
#[component]
pub fn Documents(children: Element) -> Element {
    let views = VIEWS.map(|(slug, name, used, view)| {
        crate::design::families::specimen(
            slug,
            name,
            used,
            view_forms(example_message(), view),
            None,
        )
    });
    let rows = TYPES.map(|(name, used, json)| {
        let doc = example(json);
        crate::design::families::specimen(doc.kind(), name, used, designed(&doc), None)
    });
    rsx! {
        style { {CSS} }
        div { id: "ds-main", class: "ds-main",
            div { class: "ds-page",
                div { class: "ds-title", "Documents" }
                div { class: "ds-what",
                    "One editor draws every document, Notion style: select words to comment, + or / to add a block. Under it, the document types every page reuses: a page gives the fields, the template lays them out. Each is drawn light and dark; its HTML and the text an agent reads of it fold under it."
                }
                {children}
                {views.into_iter()}
                {rows.into_iter()}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::memory::TestBudget;

    /// Task compose-uses-doc-editor, done-when 1 to 5: /'s Ask, /design's Composer and the compass's box on every page's menu
    /// are the documents editor, its toolbar
    /// of Undo, Redo and Preview with their keys, its field the Notion-style DocView edited in place, Discard and Send inside
    /// it; no textarea and no input of its own but the hidden ones it posts. It fails the day compose draws its own input again.
    #[test]
    fn compose_is_the_documents_editor_on_home_design_and_the_compass() {
        let _budget = TestBudget::new();
        let tree = r#"{"root":"ask","elements":{"ask":{"type":"compose","props":{"action":"/compass","send":"Ask","placeholder":"Ask dash, or tell it what to do","hidden":[["page","/"]]}}}}"#;
        let home = crate::screen::page("Home", tree).expect("a page");
        let design = crate::design::page("mail", None, "");
        let design = &design[design.find(r#"id="s-composer""#).expect("the Composer row")..];
        // the compass's box, every page's menu: the template its script clones into its form
        let menu = crate::forms::with_menu("<html><body></body></html>", "/", "");
        let compass = &menu[menu
            .find(r#"<template id="dash-compose-form">"#)
            .expect("the compass's box")..];
        for (page, (open, close), send, named) in [
            (
                home.as_str(),
                ("<form", "</form>"),
                "Ask",
                "Ask dash, or tell it what to do",
            ),
            (
                design,
                ("<form", "</form>"),
                "Send",
                "Reply to Ada Lovelace",
            ),
            (
                compass,
                ("<template", "</template>"),
                "Send",
                "Your comment",
            ),
        ] {
            let form = &page[page.find(open).expect("a form")..];
            let form = &form[..form.find(close).expect("its end")];
            assert!(!form.contains("<textarea"), "no textarea: {form}");
            assert_eq!(
                form.matches("<input").count(),
                form.matches(r#"type="hidden""#).count(),
                "no input of its own: {form}"
            );
            let at = |s: &str| form.find(s).unwrap_or_else(|| panic!("{s}: {form}"));
            let tools = at(r#"class="toolbar editor-tools" aria-label="Editor""#);
            for part in ["editor-undo", "editor-redo", "editor-preview"] {
                assert!(tools < at(part), "{part} on the toolbar");
            }
            assert!(
                form.contains(r#"<kbd class="kbd">Z</kbd>"#),
                "its keys: {form}"
            );
            let field = at(r#"class="editor-field""#);
            // the Notion-style field, named by its placeholder
            let words = at(&format!(
                r#"class="dv notion" contenteditable="plaintext-only" role="textbox" aria-multiline="true" title="{named}">"#
            ));
            // the field's element whole, to its own closing tag: the words, Discard and Send are nested in it (done-when 4)
            let inside = &form[form[..field].rfind("<div").expect("its tag")..];
            let (mut depth, mut end) = (0, None);
            for (i, _) in inside.match_indices("div") {
                if inside[..i].ends_with('<') {
                    depth += 1;
                } else if inside[..i].ends_with("</") {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(i + "div>".len());
                        break;
                    }
                }
            }
            let inside = &inside[..end.expect("the field closes")];
            assert!(tools < field && field < words, "{form}");
            for control in [
                r#"class="button quiet editor-discard" type="button">Discard</button>"#,
                &format!(r#"class="button editor-send" title="{send} (⌘Enter)">{send}</button>"#),
            ] {
                assert!(
                    inside.contains(control),
                    "{control} nested in the field: {inside}"
                );
            }
            assert!(form.contains(r#"name="text" type="hidden""#), "{form}");
        }
    }

    fn shown(kind: &str) -> (String, String) {
        let (_, _, json) = TYPES
            .iter()
            .find(|t| example(t.2).kind() == kind)
            .expect(kind);
        forms("d", &example(json))
    }

    #[test]
    fn an_rfc_is_its_title_status_bottom_line_then_its_parts() {
        let _budget = TestBudget::new();
        let (html, text) = shown("rfc");
        assert!(html.starts_with(r#"<article id="d" class="doc doc-rfc"><header class="doc-head"><h1 class="doc-title">One document model</h1>"#), "{html}");
        assert!(
            html.contains(r#"<span class="badge chip">Review</span>"#),
            "{html}"
        );
        assert_eq!(
            text,
            "One document model\nReview\nEvery page picks one of six types.\nContext\nEach page laid its document out by hand.\n\
             Decision\nA template a type, from the design system.\nA page gives fields, never layout.\n"
        );
    }

    /// Task rust-task-brief-linear, done-when 1 and 2: Linear's layout, its title, description, properties beside, Activity
    /// ending on a comment box; the description and each comment are DocView's, editable in place.
    #[test]
    fn a_task_brief_is_the_way_back_its_state_why_checks_run_and_comments() {
        let _budget = TestBudget::new();
        let (html, text) = shown("task-brief");
        // Linear's issue view of /design?tab=board: its way back, title, parts and properties
        for part in [
            // the way back names its title, its key at the right end, as board_live.ex's issue bar (task rust-board-buttons-new-issue)
            r#"<nav class="li-crumbs"><a href="/tasks">Tasks</a><span class="meta">›</span><span class="li-crumb-title">A task&#39;s title</span><span class="meta li-crumb-key">a-task</span>"#,
            r#"<h1 class="li-issue-title">A task&#39;s title</h1>"#,
            r#"<div class="li-check met" title="met">"#,
            r#"<div class="li-check" title="not judged">"#,
            r#"<span class="li-dot"></span><b>Started, try 1</b>"#,
            r#"<a class="li-tag li-pr" href="https://github.com/o/r/pull/12">"#,
            // Linear's sidebar, as the Elixir issue page's: priority, assignee, cycle, due day, reminder, labels, project
            r#"<span>High</span></summary>"#,
            r#"<summary title="Assign to..."><span aria-hidden="true"><span class="avatar li-who">R</span></span><span>Rust</span></summary>"#,
            r#"<span>Cycle Nov 9 - Nov 15</span></summary>"#,
            r#"<span>Due 2026-11-15</span></summary>"#,
            r#"<span>Remind me</span></summary>"#,
            // its place among the board's issues and its two steps, as Linear's « 42 / 2193 ↓ ↑ »
            r#"<span class="meta li-crumb-key">a-task</span><span class="meta li-position">3 / 9</span>"#,
            r#"href="/tasks/b-task" title="Next issue""#,
            r#"href="/tasks/z-task" title="Previous issue""#,
            // Linear's round copy buttons, their script once
            r#"data-copy="/tasks/a-task" data-origin="true""#,
            r#"data-copy="a-task""#,
            r#"data-copy="task/a-task""#,
            // each property his menu, each pick a form posting its one field to /tasks/<id>/set
            r#"<details class="prop-menu" id="d-status">"#,
            r#"<details class="prop-menu" id="d-priority">"#,
            r#"<details class="prop-menu" id="d-due">"#,
            r#"action="/tasks/a-task/set""#,
            r#"name="status" type="hidden" value="in-review"/>"#,
            r#"aria-checked="true"><span class="text-only">todo:</span><span>Todo</span><span class="text-only">(set)</span></button>"#,
            r#"name="priority" type="hidden" value="1"/>"#,
            r#"aria-checked="true"><span class="text-only">2:</span><span>High</span><span class="text-only">(set)</span></button>"#,
            r#"name="due" type="hidden" value="2026-11-12"/>"#,
            r#"aria-checked="true"><span class="text-only">2026-11-15:</span><span>In one week, 2026-11-15</span><span class="text-only">(set)</span></button>"#,
            // task rust-issue-sidebar-menus: labels tick on and off, each other menu one value, each filtered as he types
            r#"<h3 class="li-side-head">Labels</h3><div class="li-tags"><span class="badge li-tag">Design</span><details class="prop-menu" id="d-labels"><summary title="Change or add labels..."><svg"#,
            r#"role="menuitemcheckbox" aria-checked="true"><span class="text-only">Design:</span><span>Design</span><span class="text-only">(set)</span></button>"#,
            r#"<h3 class="li-side-head">Project</h3><details class="prop-menu" id="d-project"><summary title="Add to project..."><svg"#,
            r#"<span>rust-poc</span></summary>"#,
            r#"name="assignee" type="hidden" value="nicolas"/>"#,
            // the assignee's and « Remind me »'s choices numbered from 0, as the older app's: the digit picks it (its own words
            // for dash parity, « NH Nicolas Ha 1 »), hidden from the text form
            r#"<span class="avatar li-who">NH</span></span><span>Nicolas Ha</span><span class="prop-menu-n" aria-hidden="true" data-n="1">1</span></button>"#,
            r#"<span class="text-only">none:</span><span>No reminder</span><span class="text-only">(set)</span><span class="prop-menu-n" aria-hidden="true" data-n="0">0</span></button>"#,
            r#"name="cycle" type="hidden" value="2026-11-16"/>"#,
            r#"name="remind" type="hidden" value="none"/>"#,
            r#"name="project" type="hidden" value="crm"/>"#,
            r#"placeholder="Add to project...""#,
            // his comments, Notion's threads (@nha, 2026-11-09: « the comments on this page for example follows the notion example »)
            r#"<div class="doc-thread">"#,
            // its description and each comment drawn by the one DocView, edited in place
            r#"<div id="d-why"><div class="dv notion" contenteditable="plaintext-only" role="textbox" aria-multiline="true" title="Edited here only, not saved"><section class="dv-section"><pre class="doc-text">(@nha: « because »)
Its second line</pre>"#,
            r#"<div class="doc-words"><div class="dv notion" contenteditable="plaintext-only" role="textbox" aria-multiline="true" title="Edited here only, not saved"><section class="dv-section"><pre class="doc-text">Fix this first</pre>"#,
            // Activity ends on his comment box, a comment on the task's page in the compass's queue
            r#"<form class="form" id="d-comment" method="post" action="/compass"><input class="input" name="page" type="hidden" value="/tasks/a-task"/>"#,
        ] {
            assert!(html.contains(part), "{part} in {html}");
        }
        assert_eq!(
            html.matches("if(window.dashCopy)").count(),
            1,
            "one script: {html}"
        );
        // eight menus, one filter script (/code-review of PR 1399)
        assert_eq!(
            html.matches("if(window.dashMenuFilter)").count(),
            1,
            "one filter script: {html}"
        );
        // a label's pick posts the state he wants: Design is on, so its pick turns it off
        assert!(
            html.contains(
                r#"value="Design"/><input class="input" name="on" type="hidden" value="0"/>"#
            ),
            "{html}"
        );
        assert_eq!(
            text,
            "Tasks\t› A task's title a-task 3 / 9\tNext issue: /tasks/b-task\tPrevious issue: /tasks/z-task\n\
             Copy link: /tasks/a-task\tCopy ID: a-task\tCopy branch name: task/a-task\n\
             A task's title\n(@nha: « because »)\nIts second line\nDone when: 1 of 2 met\n\
             met: One thing: a test\nnot judged: Another\nLast run\nTry 1, 5 min ago: done.\nActivity\n\
             Started, try 1\t2026-11-09 10:00 UTC\nNicolas\nFix this first\n\
             Undo\tRedo\nPreview\nDiscard\tComment\nProperties\nTodo\n\
             Change status: POST /tasks/a-task/set status=\nbacklog: Backlog\ntodo: Todo (set)\nin-progress: In Progress\n\
             in-review: In Review\nthe next pick\nHigh\nSet priority: POST /tasks/a-task/set priority=\n1: Urgent\n\
             2: High (set)\n3: Medium\n4: Low\nnone: No priority\n\
             Rust\nAssign to...: POST /tasks/a-task/set assignee=\nclaude: Claude\nnicolas: Nicolas Ha\nrust: Rust (set)\n\
             Cycle Nov 9 - Nov 15\nAdd to cycle...: POST /tasks/a-task/set cycle=\nnone: No cycle\n\
             2026-11-09: Cycle Nov 9 - Nov 15 · Current (set)\n2026-11-16: Cycle Nov 16 - Nov 22 · Upcoming\n\
             Due 2026-11-15\nSet due date: POST /tasks/a-task/set due=\n\
             2026-11-12: Tomorrow, 2026-11-12\n2026-11-15: In one week, 2026-11-15 (set)\nnone: No due date\n\
             Remind me\nRemind me...: POST /tasks/a-task/set remind=\nnone: No reminder (set)\n\
             2026-11-12: Tomorrow · Thu 12 Nov 2026\nPull requests\n#12\thttps://github.com/o/r/pull/12\n\
             Labels\nDesign\nChange or add labels...: POST /tasks/a-task/set labels= with on=1 to turn one on, on=0 off\n\
             Bug: Bug\nDesign: Design (set)\n\
             Project\nrust-poc\nAdd to project...: POST /tasks/a-task/set project=\nnone: No project\ncrm: crm\n\
             rust-poc: rust-poc (set)\n"
        );
    }

    /// /code-review of PR 1395: a due day no preset names is offered first, checked, so the text names the one set; none clears it.
    #[test]
    fn a_due_day_no_preset_names_is_the_one_set() {
        let _budget = TestBudget::new();
        let Doc::TaskBrief(mut b) = example(TYPES[1].2) else {
            panic!("a task brief")
        };
        b.deadline = Some("2026-12-01".into());
        let choices = due_choices(&b);
        assert_eq!(
            choices[0],
            ("2026-12-01".into(), "Due 2026-12-01".into(), true)
        );
        assert!(choices[1..].iter().all(|c| !c.2), "{:?}", choices);
        assert_eq!(choices.last().map(|c| c.0.as_str()), Some("none"));
        b.deadline = None;
        assert_eq!(
            due_choices(&b).last(),
            Some(&("none".into(), "No due date".into(), true))
        );
    }

    /// Done-when 2 of rust-one-doc-component, the Notion view: the document's blocks in order, a section's private note a
    /// callout under its words.
    #[test]
    fn the_notion_view_draws_each_block_in_order_its_note_under_its_words() {
        let _budget = TestBudget::new();
        let (html, text) = view_forms(example_message(), View::Notion);
        assert!(html.starts_with(r#"<div class="dv notion"><section class="dv-section"><pre class="doc-text">Bonjour Ana,</pre></section>"#), "{html}");
        assert!(html.contains(r#"<pre class="doc-text">Nicolas</pre><aside class="dv-note"><span class="dv-private">Private</span>Signed as on every first message.</aside>"#), "{html}");
        assert_eq!(
            text,
            "Bonjour Ana,\nUne question courte : le poste est-il ouvert en remote ?\nPrivate Her post says hybrid; asking first saves a call.\n\
             Nicolas\nPrivate Signed as on every first message.\n"
        );
    }

    /// Done-when 2, the Gmail view: the same document and the same component, its style alone differs, so each section's note
    /// sits beside its words (documents.css .dv.gmail).
    #[test]
    fn the_gmail_view_is_the_same_component_its_notes_beside_the_words() {
        let _budget = TestBudget::new();
        let (notion, notion_text) = view_forms(example_message(), View::Notion);
        let (gmail, gmail_text) = view_forms(example_message(), View::Gmail);
        assert_eq!(gmail, notion.replacen("dv notion", "dv gmail", 1));
        assert_eq!(gmail_text, notion_text);
        assert!(CSS.contains(".dv.gmail .dv-section { display: flex;"));
        assert!(CSS.contains(".dv.gmail .dv-section > .dv-note { flex: 1 1 0;"));
        // A draft's page draws its words through it, read from the older draft's text.
        let (html, _) = shown("message-draft");
        assert!(html.contains(r#"<h2>Message</h2><div id="draft-text"><div class="dv gmail"><section class="dv-section"><pre class="doc-text">Bonjour Ana,"#), "{html}");
    }

    /// Task rust-one-doc-component, done-when 4: a chat view is view code alone; each public section a bubble, in order, and no
    /// private note's words in its HTML or its text (CSS cannot hide words from the text form).
    #[test]
    fn the_chat_view_draws_each_public_section_in_order_and_never_a_private_note() {
        let _budget = TestBudget::new();
        let (html, text) = view_forms(example_message(), View::Chat);
        let (hello, ask, sign) = (
            html.find("Bonjour Ana,"),
            html.find("Une question courte"),
            html.find("Nicolas"),
        );
        assert!(hello.is_some() && hello < ask && ask < sign, "{html}");
        assert!(html.contains(r#"class="dv chat""#), "{html}");
        for private in [
            "Her post says hybrid",
            "Signed as on every first message",
            "Private",
        ] {
            assert!(
                !html.contains(private) && !text.contains(private),
                "{private}: {html}"
            );
        }
    }

    /// Done-when 5 of rust-one-doc-component: an outline page, /today's, is the one model drawn by the one component; a line
    /// opens its link (its target in the text form), a line still reading is the loading state under its id, and a comment
    /// sits on the words it quotes, at its block and byte range; a quote found nowhere marks nothing.
    #[test]
    fn an_outline_draws_links_loading_lines_and_comments_on_its_words() {
        let _budget = TestBudget::new();
        let doc = outline(
            "Today",
            &[
                Line {
                    id: "line-0",
                    says: "3 to send",
                    what: "drafts approved",
                    href: Some("/drafts?show=to-send"),
                    reading: false,
                },
                Line {
                    id: "line-1",
                    says: "",
                    what: "unread answers",
                    href: Some("/mail"),
                    reading: true,
                },
            ],
        );
        assert!(reading(&doc));
        let anchors = vec![
            Anchor {
                quote: "to send".into(),
                thread: "c-a1".into(),
            },
            Anchor {
                quote: "gone words".into(),
                thread: "c-b2".into(),
            },
        ];
        let (html, text) = anchored_forms(doc.clone(), View::Notion, anchors.clone());
        assert!(
            html.contains(r#"<div class="dv-item" id="line-0"><pre class="doc-text"><a href="/drafts?show=to-send">3 <mark class="dv-anchor" data-thread="c-a1">to send</mark></a><span class="text-only">/drafts?show=to-send</span></pre>"#),
            "{html}"
        );
        assert!(
            html.contains(r#"<div id="line-1" class="dv-item skeleton" aria-busy="true""#),
            "{html}"
        );
        assert!(!html.contains("c-b2"), "{html}");
        assert!(
            text.starts_with("Today\n3 to send\t/drafts?show=to-send\ndrafts approved\n"),
            "{text}"
        );
        // anchored on the model: the block holding the quote, and its bytes there
        let model = notion_doc::load(&doc, 0).expect("an outline fits the schema");
        let at = anchored(&model, &anchors);
        let block = model
            .block(at[0].0)
            .and_then(|b| b.inline())
            .map(|t| t.as_str());
        assert_eq!(
            (at.len(), block, at[0].1.clone()),
            (1, Some("3 to send"), 2..9)
        );
    }

    #[test]
    fn a_message_draft_is_its_receiver_then_its_context_then_its_words() {
        let _budget = TestBudget::new();
        let (html, text) = shown("message-draft");
        assert!(
            html.contains(r#"<h1 class="doc-title">Ana (Acme)</h1>"#),
            "{html}"
        );
        assert!(
            html.contains(r#"<dt>Lead</dt><dd><a href="/o/acme-platform">acme-platform</a><span class="text-only">/o/acme-platform</span></dd>"#),
            "{html}"
        );
        assert_eq!(
            text,
            "Ana (Acme)\npending linkedin\nThe ad\nTitle\nPlatform engineer\nCompany\nAcme\nLocation\nRemote (EU)\nContract\n\
             Freelance\nSource\nhttps://acme.example/jobs/1\nThe ad's text\nPlatform engineer | Acme | Remote (EU)\nSix months, \
             Rust.\nContext\nGoal\nA first message\nFrom\nNicolas Ha\nTo\nAna (Acme)\nLead\nacme-platform\t/o/acme-platform\n\
             Application form\nhttps://acme.example/jobs/1/apply\nScores\ntoo_long 0.4\nRecent\n2026-11-10\nSearch of \
             2026-11-10: posted the day before\thttps://acme.example/jobs/1\n2026-11-09\nNoted\t/o/acme-platform\nMessage\n\
             Bonjour Ana,\nUne question courte.\n"
        );
        // Listed: its words on its own page, its receiver the link there.
        let mut listed = example(TYPES[2].2);
        if let Doc::MessageDraft(d) = &mut listed {
            d.text.clear();
            d.href = Some("/drafts/a-first".into());
        }
        let (html, text) = forms("d", &listed);
        assert!(
            html.contains(r#"<a class="doc-title" href="/drafts/a-first">Ana (Acme)</a>"#),
            "{html}"
        );
        assert_eq!(
            text,
            "Ana (Acme)\t/drafts/a-first\npending linkedin\nA first message · Lead: acme-platform · Scores: too_long 0.4\n"
        );
    }

    /// Done-when 1 of rust-draft-posting: a draft on a lead with a posting shows its Title, Company, Location, Contract and
    /// Source, the Source linked, then the ad's text folded, its reader's first line a heading, all above its context.
    #[test]
    fn a_draft_on_a_posting_shows_its_facts_and_the_ads_text_folded() {
        let _budget = TestBudget::new();
        let (html, _) = shown("message-draft");
        for (label, value) in [
            ("Title", "Platform engineer"),
            ("Company", "Acme"),
            ("Location", "Remote (EU)"),
            ("Contract", "Freelance"),
        ] {
            assert!(
                html.contains(&format!("<dt>{label}</dt><dd>{value}</dd>")),
                "{label}: {html}"
            );
        }
        assert!(
            html.contains(r#"<dt>Source</dt><dd><a href="https://acme.example/jobs/1">https://acme.example/jobs/1</a></dd>"#),
            "{html}"
        );
        assert!(
            html.contains(r#"<details class="fold" id="ad-text"><summary>The ad&#39;s text</summary><h3>Platform engineer | Acme | Remote (EU)</h3><pre class="doc-text">Six months, Rust.</pre></details>"#),
            "{html}"
        );
        assert!(
            html.find("<h2>The ad</h2>") < html.find("<h2>Context</h2>"),
            "{html}"
        );
        // no posting, no part: a draft on no ad shows none of it
        let mut bare = example(TYPES[2].2);
        if let Doc::MessageDraft(d) = &mut bare {
            (d.posting, d.ad) = (vec![], String::new());
        }
        let (html, _) = forms("d", &bare);
        assert!(
            !html.contains("The ad") && !html.contains("ad-text"),
            "{html}"
        );
    }

    /// Done-when 2 of rust-draft-posting: it links the Application form, the lead's page and its search note (the lead's
    /// last event, linked to its source), an event with none linked to the lead's page; a script's link is never drawn.
    #[test]
    fn a_draft_links_its_application_form_its_leads_page_and_its_search_note() {
        let _budget = TestBudget::new();
        let (html, _) = shown("message-draft");
        for link in [
            r#"<dt>Application form</dt><dd><a href="https://acme.example/jobs/1/apply">"#,
            r#"<dt>Lead</dt><dd><a href="/o/acme-platform">acme-platform</a>"#,
            r#"<dt>2026-11-10</dt><dd><a href="https://acme.example/jobs/1">Search of 2026-11-10: posted the day before</a>"#,
            r#"<dt>2026-11-09</dt><dd><a href="/o/acme-platform">Noted</a>"#,
        ] {
            assert!(html.contains(link), "{link}: {html}");
        }
        let mut bad = example(TYPES[2].2);
        if let Doc::MessageDraft(d) = &mut bad {
            d.apply = Some("javascript:alert(1)".into());
        }
        let (html, _) = forms("d", &bad);
        assert!(
            html.contains("<dt>Application form</dt><dd>javascript:alert(1)</dd>"),
            "{html}"
        );
    }

    /// The draft of TYPES as /drafts/<id> gives it read now: its decisions, Change, State and Waits, `edit` its message's field.
    fn stated(edit: bool) -> MessageDraft {
        let Doc::MessageDraft(d) = example(TYPES[2].2) else {
            panic!("a message draft")
        };
        let mut d = *d;
        d.id = "a-first".into();
        d.decide = Some("/d/a-first".into());
        d.change = Some("/drafts/a-first".into());
        d.editing = edit;
        d.text = "Bonjour Ana,\n\nUne **question** courte.".into();
        d.copy = true;
        d.stated = true;
        d.lead = "acme-platform".into();
        d.why = vec![
            "Fit 5: RAG in production.".into(),
            "Her post asks for remote.".into(),
        ];
        d.place = "Easy Apply LinkedIn, its message field".into();
        d.bold = "Not checked: plain text until the form is checked (its :form-bold)".into();
        d.dates = vec![Fact {
            label: "Approved".into(),
            value: "2026-11-10".into(),
            href: None,
        }];
        d.schedule = Some("/drafts/a-first/set".into());
        d.send_on = "2026-11-20".into();
        d
    }

    /// Task rust-draft-state-edit, done-when 1: Copy the text copies the message whole, the stars of his bold left out, then says
    /// « Copied »; on a failure it says « Copy failed, the text is selected » and selects the message it names. A draft Claude
    /// sends has none.
    #[test]
    fn copy_the_text_copies_the_message_and_says_copied_or_selects_it() {
        let _budget = TestBudget::new();
        let (html, text) = forms("d", &Doc::MessageDraft(Box::new(stated(false))));
        assert!(
            html.contains(r#"<button class="button" type="button" data-label="Copy the text" id="draft-copy" data-copy="Bonjour Ana,

Une question courte." data-select="draft-text">Copy the text</button>"#),
            "{html}"
        );
        // what it selects on a failure is the message as drawn
        assert!(
            html.contains(r#"<div id="draft-text"><div class="dv gmail">"#),
            "{html}"
        );
        assert!(text.contains("\nCopy the text\tChange\n"), "{text:?}");
        // its script says either outcome in the button's words and, failed, selects the named text
        let script = html
            .split("<script>")
            .find(|s| s.contains("dashCopy"))
            .expect("the copy script");
        for said in [
            r#"say("Copied")"#,
            r#"say(label?"Copy failed, the text is selected":"Not copied")"#,
            "r.selectNodeContents(t)",
            "document.getElementById(b.dataset.select)",
            "b.textContent=w",
        ] {
            assert!(script.contains(said), "{said}: {script}");
        }
        // a form that shows formatting keeps his bold's stars, as State's Bold says (/code-review)
        let mut rich = stated(false);
        rich.keep_bold = true;
        let (html, _) = forms("d", &Doc::MessageDraft(Box::new(rich)));
        assert!(
            html.contains("data-copy=\"Bonjour Ana,\n\nUne **question** courte.\""),
            "{html}"
        );
        let mut ours = stated(false);
        ours.copy = false;
        let (html, _) = forms("d", &Doc::MessageDraft(Box::new(ours)));
        assert!(!html.contains("Copy the text"), "{html}");
    }

    /// Done-when 2: Change opens the message in place, its field holding the text, Save posting it as `body` to the older app's
    /// write of his edit, Cancel back to the page; a draft no longer open to edits offers no Change.
    #[test]
    fn change_opens_the_text_in_place_and_save_posts_it_to_his_edit() {
        let _budget = TestBudget::new();
        let (html, _) = forms("d", &Doc::MessageDraft(Box::new(stated(false))));
        assert!(
            html.contains(r#"<a class="button quiet" href="/drafts/a-first?edit=1">Change</a>"#),
            "{html}"
        );
        let (html, text) = forms("d", &Doc::MessageDraft(Box::new(stated(true))));
        assert!(
            html.contains(r#"<form class="form" id="draft-change" method="post" action="/d/a-first/body"><textarea class="textarea" aria-label="The message" name="body" required=true rows="16">Bonjour Ana,

Une **question** courte.</textarea><button class="button">Save</button> <a class="button quiet" href="/drafts/a-first">Cancel</a></form>"#),
            "{html}"
        );
        // in place: the field stands where the words were, no second copy of them, no Copy of a text being changed
        assert!(
            !html.contains(r#"id="draft-text""#) && !html.contains("Copy the text"),
            "{html}"
        );
        assert!(
            text.contains("Message\nBonjour Ana, Une **question** courte.\tSave\tCancel\n"),
            "{text:?}"
        );
        // a text opening on a blank line keeps it: a parser drops one newline after <textarea> (/code-review)
        let mut blank = stated(true);
        blank.text = "\nBonjour".into();
        let (html, _) = forms("d", &Doc::MessageDraft(Box::new(blank)));
        assert!(
            html.contains("rows=\"16\">\n\nBonjour</textarea>"),
            "{html}"
        );
        let mut closed = stated(false);
        closed.change = None;
        let (html, _) = forms("d", &Doc::MessageDraft(Box::new(closed)));
        assert!(!html.contains(">Change<"), "{html}");
    }

    /// Done-when 3: State shows Status (its status, channel and id), each decision's day, Where and Bold, then Goes out on, a
    /// date field holding its day that Save posts to /drafts/<id>/set; closed, the day is shown, never a field.
    #[test]
    fn state_shows_status_where_bold_and_goes_out_on() {
        let _budget = TestBudget::new();
        let (html, text) = forms("d", &Doc::MessageDraft(Box::new(stated(false))));
        assert!(
            text.contains(
                "State\nStatus\npending · linkedin · a-first\nApproved\n2026-11-10\nWhere\nEasy Apply LinkedIn, its message field\n\
                 Bold\nNot checked: plain text until the form is checked (its :form-bold)\n"
            ),
            "{text}"
        );
        assert!(
            html.contains(r#"<form class="form" id="send-on" method="post" action="/drafts/a-first/set"><label class="composer-field"><span>Goes out on (empty: with the next outbox)</span><input class="input" name="send-on" type="date" value="2026-11-20"/></label><button class="button">Save</button></form>"#),
            "{html}"
        );
        let mut closed = stated(false);
        closed.schedule = None;
        let (html, text) = forms("d", &Doc::MessageDraft(Box::new(closed)));
        assert!(!html.contains(r#"name="send-on""#), "{html}");
        assert!(text.contains("Goes out on\n2026-11-20\n"), "{text}");
    }

    /// Done-when 4: Why this text and Waits, folded under the older page's one line, « Why and context », what it waits on then
    /// its lead: actionable with no wait, else in flight with each step linked and what may go meanwhile.
    #[test]
    fn why_this_text_and_waits_show_as_the_older_page() {
        let _budget = TestBudget::new();
        let (html, text) = forms("d", &Doc::MessageDraft(Box::new(stated(false))));
        assert!(
            html.contains(r#"<details class="fold" id="why"><summary>Why and context <span>Waits on nothing · lead acme-platform</span></summary>"#),
            "{html}"
        );
        assert!(
            text.contains("Why this text\nFit 5: RAG in production.\nHer post asks for remote.\n"),
            "{text}"
        );
        assert!(text.contains("Waits\nActionable\n"), "{text}");
        let mut held = stated(false);
        held.waits = vec![Fact {
            label: "drafts:b".into(),
            value: "drafts:b".into(),
            href: Some("/drafts/b".into()),
        }];
        held.meanwhile = "the talk with Ana".into();
        let (html, text) = forms("d", &Doc::MessageDraft(Box::new(held)));
        assert!(
            html.contains("<span>Waits on drafts:b · lead acme-platform</span>"),
            "{html}"
        );
        assert!(html.contains(r#"<span class="badge chip">In flight</span> waits on <a href="/drafts/b">drafts:b</a>"#), "{html}");
        assert!(text.contains("Meanwhile\nthe talk with Ana\n"), "{text}");
        // unread, never « waits on nothing » (/code-review)
        let mut unread = stated(false);
        unread.waits_unread = true;
        let (_, text) = forms("d", &Doc::MessageDraft(Box::new(unread)));
        assert!(
            text.contains("Why and context What it waits on was not read · lead acme-platform\n"),
            "{text}"
        );
        assert!(
            text.contains("Waits\nNot read: what it waits on is not known"),
            "{text}"
        );
        assert!(
            !text.contains("Actionable") && !text.contains("Waits on nothing"),
            "{text}"
        );
        // as the older page names them, so dash parity finds each (done-when 5)
        let items: Vec<(crate::parity::Kind, String)> = crate::parity::items(&html)
            .into_iter()
            .map(|i| (i.kind, i.label))
            .collect();
        use crate::parity::Kind::*;
        for want in [
            (Section, "Why this text"),
            (Section, "State"),
            (Section, "Waits"),
            (Field, "Status"),
            (Field, "Where"),
            (Field, "Bold"),
            (Field, "Goes out on (empty: with the next outbox)"),
            (Control, "send-on"),
            (Control, "Save"),
            (Control, "Copy the text"),
        ] {
            assert!(
                items.iter().any(|(k, l)| *k == want.0 && l == want.1),
                "{want:?}: {items:?}"
            );
        }
    }

    /// A draft waiting on him, as /drafts/<id> gives it: `blank` names a field left empty.
    fn waiting(blank: &str) -> (String, String) {
        let mut doc = example(TYPES[2].2);
        if let Doc::MessageDraft(d) = &mut doc {
            d.decide = Some("/d/a-first".into());
            d.earlier = vec![Part {
                title: "Earlier: v1 on 2026-11-07 by claude".into(),
                text: "Bonjour Ana, une longue question.".into(),
            }];
            match blank {
                "goal" => d.goal = " ".into(),
                "sender" => d.sender.clear(),
                "receiver" => d.receiver = "\n".into(),
                _ => {}
            }
        }
        doc.check("d").expect("a path of this app");
        forms("d", &doc)
    }

    /// Done-when 1 and 2 of rust-draft-approve: its three things above its words, then Approve posting « approved » to the
    /// older app's draft write, Request corrections with his words, its earlier version readable.
    #[test]
    fn a_waiting_draft_shows_its_three_things_above_its_words_then_approve() {
        let _budget = TestBudget::new();
        let (html, text) = waiting("");
        let at = |what: &str| {
            text.find(what)
                .unwrap_or_else(|| panic!("{what} in {text}"))
        };
        assert!(at("Goal\nA first message") < at("Message\nBonjour"));
        assert!(at("From\nNicolas Ha") < at("Message\nBonjour"));
        assert!(at("To\nAna (Acme)") < at("Message\nBonjour"));
        assert!(
            text.ends_with(
                "Decision\nApprove\nRequest corrections\nEarlier: v1 on 2026-11-07 by claude\nBonjour Ana, une longue question.\n"
            ),
            "{text}"
        );
        for part in [
            r#"<form class="form" method="post" action="/d/a-first/status"><input class="input" name="status" type="hidden" value="approved"/><button class="button">Approve</button></form>"#,
            r#"<input class="input" name="status" type="hidden" value="approved-pending-changes"/><textarea class="textarea" name="text""#,
        ] {
            assert!(html.contains(part), "{part} in {html}");
        }
    }

    fn missing(blank: &str, named: &str) {
        let (html, text) = waiting(blank);
        assert!(!html.contains(">Approve<"), "{html}");
        assert!(
            text.contains(&format!(
                "Not ready to approve: {named}.\nRequest corrections"
            )),
            "{text}"
        );
    }

    /// Done-when 1 of rust-draft-approve: a missing goal is named in place of Approve.
    #[test]
    fn a_draft_with_no_goal_names_it_in_place_of_approve() {
        let _budget = TestBudget::new();
        missing("goal", "no goal");
    }

    /// Done-when 1 of rust-draft-approve: a missing sender is named in place of Approve.
    #[test]
    fn a_draft_with_no_sender_names_it_in_place_of_approve() {
        let _budget = TestBudget::new();
        missing("sender", "no sender");
    }

    /// Done-when 1 of rust-draft-approve: a missing receiver is named in place of Approve, and its title says so.
    #[test]
    fn a_draft_with_no_receiver_names_it_in_place_of_approve() {
        let _budget = TestBudget::new();
        missing("receiver", "no receiver");
        assert!(waiting("receiver").1.starts_with("no receiver\n"));
    }

    /// His open comments hold Approve back until applied (/code-review: a plain Approve sent the uncorrected text); the
    /// other moves its status offers post as Approve does.
    #[test]
    fn open_comments_hold_approve_back_and_other_moves_post_their_status() {
        let _budget = TestBudget::new();
        let mut doc = example(TYPES[2].2);
        if let Doc::MessageDraft(d) = &mut doc {
            d.decide = Some("/d/a-first".into());
            d.goal.clear();
            d.open_comments = 2;
            d.moves = vec![Move {
                status: "rejected".into(),
                label: "Reject".into(),
            }];
        }
        let (html, text) = forms("d", &doc);
        assert!(
            text.contains("Not ready to approve: no goal, 2 open comments to apply.\nRequest corrections\nReject\n"),
            "{text}"
        );
        assert!(
            html.contains(r#"value="rejected"/><button class="button quiet">Reject</button>"#),
            "{html}"
        );
        // approved: no Approve, no corrections, only its moves
        if let Doc::MessageDraft(d) = &mut doc {
            d.status = "approved".into();
            d.moves = vec![Move {
                status: "sent".into(),
                label: "I sent it".into(),
            }];
        }
        assert!(forms("d", &doc).1.ends_with("Decision\nI sent it\n"));
    }

    /// A decision posts to this app alone: another host's path is refused and never drawn.
    #[test]
    fn a_decision_posts_only_to_a_path_of_this_app() {
        let _budget = TestBudget::new();
        for path in [
            "//evil.example/x",
            "/\\evil.example",
            "https://evil.example/d",
        ] {
            let mut doc = example(TYPES[2].2);
            if let Doc::MessageDraft(d) = &mut doc {
                d.decide = Some(path.into());
            }
            assert!(doc.check("d").is_err(), "{path}");
            assert!(!forms("d", &doc).0.contains("<form"), "{path}");
        }
    }

    #[test]
    fn a_crm_entry_is_its_name_company_stage_facts_and_timeline() {
        let _budget = TestBudget::new();
        // the catalog's example counts from the clock's today; this one from a day given, so its words hold
        let json = TYPES
            .iter()
            .find(|t| t.0 == "CRM entry")
            .expect("a CRM entry")
            .2
            .replace(r#""notes":"#, r#""today":"2026-11-10","notes":"#);
        let (html, text) = forms("d", &example(&json));
        assert!(
            html.contains(r#"<h2>Timeline</h2><ol class="timeline"><li><details class="when"><summary><span>8 days ago</span><time datetime="2026-11-02">2026-11-02</time></summary></details><div>First message</div></li>"#),
            "{html}"
        );
        assert_eq!(
            text,
            "Ana\nAcme contacted\nFacts\nRole\nCTO\nChannel\nlinkedin\nTimeline\n8 days ago\t2026-11-02\nFirst message\n\
             5 days ago\t2026-11-05\nHer reply\nNotes\nAsked for a rate.\n"
        );
        // with no day given, the clock's: read before and after the render, so a midnight between them fails nothing
        let before = crate::components::today();
        let (live, _) = shown("crm-entry");
        let ago = |t| crate::components::ago("2026-11-02", t).expect("a day");
        let said = [before, crate::components::today()].map(|t| format!("<span>{}</span>", ago(t)));
        assert!(said.iter().any(|s| live.contains(s)), "{live}");
    }

    /// /design's Page comments draws the compass and the box he types in, the menu's own components, in its HTML and its text
    /// (task rust-design-page-comments-compose); a page's own comments document draws neither, the menu has them (/code-review).
    #[test]
    fn design_s_page_comments_draws_the_menu_s_compass_and_comment_box() {
        let _budget = TestBudget::new();
        let doc = example(
            TYPES
                .iter()
                .find(|t| t.0 == "Page comments")
                .expect("the type")
                .2,
        );
        let (html, text) = designed(&doc);
        let compass = dioxus_ssr::render_element(crate::components::compass(None));
        let compose = dioxus_ssr::render_element(comment_box(""));
        let menu = crate::forms::with_menu("<html><body></body></html>", "/", "");
        assert!(
            compass
                .starts_with(r#"<button class="button icon" aria-label="Comment on an element""#),
            "{compass}"
        );
        assert!(
            !compass.contains(" id="),
            "one #dash-compass a page, the menu's: {compass}"
        );
        assert!(
            menu.contains(&compose),
            "the menu's box is this one: {compose}"
        );
        assert!(
            !compose.contains("<textarea")
                && compose.contains(r#"class="dv notion" contenteditable="plaintext-only""#),
            "the documents editor, no textarea: {compose}"
        );
        assert!(
            html.ends_with(&format!("</article>{compass}{compose}")),
            "{html}"
        );
        assert!(
            text.ends_with(
                "On the page whole.\nComment on an element\nUndo\tRedo\nPreview\nDiscard\tSend\nCancel\n"
            ),
            "{text}"
        );
        let (drawn, _) = forms("d", &doc);
        assert!(
            !drawn.contains("Comment on an element") && !drawn.contains("doc-compose"),
            "{drawn}"
        );
    }

    /// The picker's `accept` and the table `POST /compass` checks are the same list (/code-review of rust-comment-files-filter).
    #[test]
    fn the_picker_accepts_what_a_comment_keeps() {
        let _budget = TestBudget::new();
        let listed: Vec<String> = ATTACHABLE_KINDS
            .iter()
            .map(|(e, _)| format!(".{e}"))
            .collect();
        assert_eq!(listed.join(","), ATTACHABLE);
    }

    #[test]
    fn page_comments_are_notion_s_threads_a_quote_then_each_comment() {
        let _budget = TestBudget::new();
        let (html, text) = shown("page-comments");
        assert!(
            html.contains(r#"<div class="doc-quote">One tree, two renderers</div>"#),
            "{html}"
        );
        assert!(
            html.contains(
                r#"<span aria-hidden="true"><span class="avatar">NH</span></span>Nicolas Ha<span class="doc-meta">Just now</span>"#
            ),
            "{html}"
        );
        assert_eq!(
            text,
            "One tree, two renderers\nNicolas Ha Just now\nBoth, really?\nClaude Just now\nThe window and the web.\n\
             Whole page\nNicolas Ha\nOn the page whole.\n"
        );
        // a comment's lines keep their breaks, its tabs spaces: no line of it reads as an event (`what\twhen`)
        let said = r#"{"kind":"page-comments","threads":[{"said":[{"who":"N","words":"ok\nStarted, try 2\t2026-11-09 10:00 UTC"}]}]}"#;
        let (html, text) = forms("d", &example(said));
        assert!(html.contains("ok\nStarted"), "{html}");
        assert_eq!(
            text,
            "Whole page\nN\nok\nStarted, try 2 2026-11-09 10:00 UTC\n"
        );
        // an indented snippet keeps its indentation; blank lines part paragraphs (/code-review of rust-task-brief-linear)
        let said = r#"{"kind":"page-comments","threads":[{"said":[{"who":"N","words":"\n\n    let x = 1;\n    y\n\n\n\nz"}]}]}"#;
        let (html, _) = forms("d", &example(said));
        assert!(html.contains(">    let x = 1;\n    y</pre>"), "{html}");
        assert_eq!(html.matches("class=\"dv-section\"").count(), 2, "{html}");
    }

    #[test]
    fn a_skill_is_its_name_when_to_use_it_then_its_sections() {
        let _budget = TestBudget::new();
        let (html, text) = shown("skill");
        assert!(html.starts_with(r#"<article id="d" class="doc doc-skill"><header class="doc-head"><h1 class="doc-title">mail-poll</h1><p class="doc-lead">Use when"#), "{html}");
        assert!(
            html.contains(r#"<section class="doc-part"><h2>Steps</h2>"#),
            "{html}"
        );
        assert_eq!(
            text,
            "mail-poll\nUse when checking the job-search mail since the last check.\nSteps\n1. Read the mail since the last check.\n\
             2. File what matters on its lead.\nNever\nNotify him for a newsletter.\n"
        );
        // a skill of one block, no `##` heading, still reads (/code-review of PR doc-type-skill)
        let (_, text) = forms(
            "d",
            &example(r#"{"kind":"skill","name":"x","description":"Use when y."}"#),
        );
        assert_eq!(text, "x\nUse when y.\n");
    }

    /// A template lays out by its own width: /design's narrow cell stacked the task brief's title a few letters a line
    /// while the screen was wide (the PM check's screenshot of /design?tab=documents).
    #[test]
    fn a_document_stacks_by_its_own_width_not_the_screen_s() {
        let _budget = TestBudget::new();
        assert!(
            CSS.contains(
                ".doc { display: block; max-width: 48rem; container: doc / inline-size; }"
            )
        );
        let stacked = r#"@container doc (max-width: 700px) { .li-issue { grid-template-columns: minmax(0, 1fr); grid-template-areas: "crumbs" "tools" "side" "main"; } }"#;
        assert!(crate::design::CARDS_CSS.contains(stacked));
    }

    /// Done-when 1: /design?tab=documents shows every type, drawn, its HTML and its text.
    #[test]
    fn the_design_page_shows_every_type_with_both_forms() {
        let _budget = TestBudget::new();
        let page = crate::design::page("documents", None, "");
        let text = crate::words::text_of(&page);
        for (name, used, json) in TYPES {
            let doc = example(json);
            let (html, words) = designed(&doc);
            // a family's section, as the Components tab's (task rust-design-components-fit)
            let section = crate::design::families::specimen_html(
                doc.kind(),
                name,
                used,
                (&html, &words),
                None,
            );
            assert!(page.contains(&section), "{section} in {page}");
            for line in words.lines() {
                assert!(text.contains(line), "{name}'s text {line:?} in {text}");
            }
        }
    }

    /// A field still read is a skeleton inside the element its filled drawing keeps by id, so the page's refill swaps the whole
    /// part, the checks' title too (/code-review of the merge with main).
    #[test]
    fn a_brief_being_read_swaps_whole_parts() {
        let _budget = TestBudget::new();
        let mut doc = example(TYPES[1].2);
        assert!(!doc.busy());
        let filled = dioxus_ssr::render_element(draw("x", &doc));
        let part = r#"<div id="x-checks"><section class="li-part"><h2 class="li-part-head">Done when: 1 of 2 met</h2>"#;
        assert!(filled.contains(part), "{filled}");
        assert!(
            filled.contains(
                r#"<div id="x-why"><div class="dv notion" contenteditable="plaintext-only" "#
            ),
            "{filled}"
        );
        if let Doc::TaskBrief(b) = &mut doc {
            (b.why, b.no_checks, b.checks) = (None, None, vec![]);
        }
        assert!(doc.busy());
        let (html, text) = forms("x", &doc);
        let part = r#"<div id="x-checks"><section class="li-part"><h2 class="li-part-head">Done when</h2><div id="x-checks-read" class="skeleton""#;
        assert!(html.contains(part), "{html}");
        assert!(
            html.contains(r#"<div id="x-why" class="skeleton""#),
            "{html}"
        );
        assert!(
            text.contains("Its words: being read.") && text.contains("Its checks: being read."),
            "{text}"
        );
    }

    #[test]
    fn a_pull_request_not_on_github_is_refused() {
        let _budget = TestBudget::new();
        let mut doc = example(TYPES[1].2);
        assert!(doc.check("b").is_ok());
        if let Doc::TaskBrief(b) = &mut doc {
            b.pr = Some("https://example.com/pull/1".into());
        }
        assert_eq!(doc.check("b"), Err("b: not a GitHub link".into()));
        // a pull request's link alone: a trailing slash or a repo's link would draw « # » (/code-review of the merge)
        for url in [
            "https://github.com/o/r/pull/12/",
            "https://github.com/o/r",
            "https://github.com/o/r/pull/x",
        ] {
            if let Doc::TaskBrief(b) = &mut doc {
                b.pr = Some(url.into());
            }
            assert_eq!(
                doc.check("b"),
                Err("b: not a pull request's link".into()),
                "{url}"
            );
        }
        // A draft's link is one a page may open, never a script or another host (/code-review).
        let mut draft = example(TYPES[2].2);
        for (href, fine) in [
            ("/d/x", true),
            ("https://x.io", true),
            ("javascript:alert(1)", false),
            ("//evil.host", false),
            ("/\\evil.host", false),
        ] {
            if let Doc::MessageDraft(d) = &mut draft {
                d.href = Some(href.into());
            }
            assert_eq!(draft.check("m").is_ok(), fine, "{href}");
            // and filtered where it is drawn, for a caller that skips `check`
            let html = dioxus_ssr::render_element(draw("m", &draft));
            assert_eq!(html.contains(&format!(r#"href="{href}""#)), fine, "{html}");
        }
    }

    /// rust-task-names-layer: an RFC's sections and headings are the layers a task names, a paragraph's id is none.
    #[test]
    fn an_rfc_s_sections_and_headings_are_its_section_ids() {
        let _budget = TestBudget::new();
        let html = r#"<section id="intent"><h2>1. Intent</h2><h3 id="core-rule">The rule</h3><p id="note">x</p></section>
<section class="s" id="dataflow"><h3 id="intent">Again</h3>"#;
        assert_eq!(section_ids(html), ["intent", "core-rule", "dataflow"]);
    }
}

/// A mail's HTML part from the words he typed with Composer's formatting bar (task rust-mail-parity): the design system draws
/// it, as every HTML of the app, and dash hands it to the mail plugin's send.
pub use marks::html_of as mail_html;

/// Composer's marks as a mail's HTML, ported from the older app's Dashboard.Webmail.compose_html: `**bold**`, `*italic*`,
/// `[text](https://…)`, lines of `- `, `1. ` or `> `, every other line as typed, the text escaped first. ponytail: the bar's
/// own marks only, no nesting; a Markdown library if he writes more.
mod marks {
    /// The HTML part of a mail whose text part is `text`.
    pub fn html_of(text: &str) -> String {
        let lines: Vec<&str> = text.lines().collect();
        let mut body = String::new();
        let mut at = 0;
        while at < lines.len() {
            let kind = block(lines[at]);
            let end = (at..lines.len())
                .find(|&i| block(lines[i]) != kind)
                .unwrap_or(lines.len());
            body.push_str(&block_html(kind, &lines[at..end]));
            at = end;
        }
        format!(r#"<div dir="ltr">{body}</div>"#)
    }

    #[derive(Clone, Copy, PartialEq)]
    enum Block {
        Bullets,
        Numbers,
        Quote,
        Text,
    }

    /// The list mark a line opens with, past its indent, and its length: `- `, `* ` or `12. `.
    fn list_mark(line: &str) -> Option<(Block, usize)> {
        let t = line.trim_start();
        let indent = line.len() - t.len();
        if t.starts_with("- ") || t.starts_with("* ") {
            return Some((Block::Bullets, indent + 2));
        }
        let digits = t.chars().take_while(char::is_ascii_digit).count();
        (digits > 0 && t[digits..].starts_with(". "))
            .then_some((Block::Numbers, indent + digits + 2))
    }

    fn block(line: &str) -> Block {
        match list_mark(line) {
            Some((kind, _)) => kind,
            None if line.starts_with('>') => Block::Quote,
            None => Block::Text,
        }
    }

    fn block_html(kind: Block, lines: &[&str]) -> String {
        let item = |l: &&str| format!("<li>{}</li>", inline(&l[list_mark(l).map_or(0, |m| m.1)..]));
        let quoted = |l: &&str| inline(l.strip_prefix("> ").or(l.strip_prefix('>')).unwrap_or(l));
        match kind {
            Block::Bullets => format!("<ul>{}</ul>", lines.iter().map(item).collect::<String>()),
            // the typed number starts the list: one split by blank lines reads 1, 2, 3 as its text does
            Block::Numbers => {
                let start: u64 = lines[0]
                    .trim_start()
                    .split('.')
                    .next()
                    .and_then(|n| n.parse().ok())
                    .unwrap_or(1);
                format!(
                    r#"<ol start="{start}">{}</ol>"#,
                    lines.iter().map(item).collect::<String>()
                )
            }
            Block::Quote => format!(
                "<blockquote>{}</blockquote>",
                lines.iter().map(quoted).collect::<Vec<_>>().join("<br>")
            ),
            Block::Text => {
                lines
                    .iter()
                    .map(|l| inline(l))
                    .collect::<Vec<_>>()
                    .join("<br>")
                    + "<br>"
            }
        }
    }

    fn escaped(text: &str) -> String {
        text.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
    }

    /// A line's links, each URL kept whole (a `*` in one is never italic), then its emphasis.
    fn inline(line: &str) -> String {
        let mut out = String::new();
        let mut rest = line;
        while let Some(open) = rest.find('[') {
            let link = rest[open + 1..]
                .split_once("](")
                .and_then(|(label, after)| {
                    let (url, tail) = after.split_once(')')?;
                    let safe = (url.starts_with("https://")
                        || url.starts_with("http://")
                        || url.starts_with("mailto:"))
                        && !url.contains(|c: char| c.is_whitespace() || "\"<>".contains(c));
                    (safe && !label.is_empty() && !label.contains('['))
                        .then_some((label, url, tail))
                });
            match link {
                Some((label, url, tail)) => {
                    out.push_str(&emphasis(&escaped(&rest[..open])));
                    out.push_str(&format!(
                        r#"<a href="{}">{}</a>"#,
                        escaped(url),
                        emphasis(&escaped(label))
                    ));
                    rest = tail;
                }
                None => {
                    out.push_str(&emphasis(&escaped(&rest[..=open])));
                    rest = &rest[open + 1..];
                }
            }
        }
        out + &emphasis(&escaped(rest))
    }

    /// `**bold**` then `*italic*`, each pair on one line: a mark between two letters or digits (« 5*3*2 », « file*name ») stays
    /// as typed, and an italic never crosses a bold's tag (« ***x*** »).
    fn emphasis(text: &str) -> String {
        let bold = pairs(text, "**", "b");
        pairs(&bold, "*", "i")
    }

    /// Each `mark`…`mark` in `text` as `<tag>`…`</tag>`, when what it holds starts and ends with no space.
    fn pairs(text: &str, mark: &str, tag: &str) -> String {
        let mut out = String::new();
        let mut rest = text;
        while let Some(open) = rest.find(mark) {
            let inside = &rest[open + mark.len()..];
            let wordy = |c: Option<char>| c.is_some_and(char::is_alphanumeric);
            let close = inside.find(mark).filter(|&c| {
                let held = &inside[..c];
                let (before, after) = (
                    rest[..open].chars().last(),
                    inside[c + mark.len()..].chars().next(),
                );
                !held.is_empty()
                    && !held.starts_with(' ')
                    && !held.ends_with(' ')
                    && !(wordy(before) || wordy(after))
                    && !held.contains(['<', '*'])
            });
            match close {
                Some(c) => {
                    out.push_str(&rest[..open]);
                    out.push_str(&format!("<{tag}>{}</{tag}>", &inside[..c]));
                    rest = &inside[c + mark.len()..];
                }
                None => {
                    out.push_str(&rest[..open + mark.len()]);
                    rest = inside;
                }
            }
        }
        out + rest
    }

    #[cfg(test)]
    mod tests {
        use super::html_of;

        /// Done-when 2's formatting bar, its send side: each mark the bar puts becomes the mail's HTML, the text escaped first.
        #[test]
        fn the_bar_s_marks_become_the_mail_s_html() {
            let _budget = dash_meter::memory::TestBudget::new();
            assert_eq!(
                html_of(
                    "Hi **Ada**, see *this* & [the plan](https://x.example/a*b*c).\n- one\n- two\n3. three\n4. four\n> said\n<b>"
                ),
                r#"<div dir="ltr">Hi <b>Ada</b>, see <i>this</i> &amp; <a href="https://x.example/a*b*c">the plan</a>.<br><ul><li>one</li><li>two</li></ul><ol start="3"><li>three</li><li>four</li></ol><blockquote>said</blockquote>&lt;b&gt;<br></div>"#
            );
            // /code-review: a mark between letters or digits stays as typed, and an italic never crosses a bold's tag
            assert_eq!(
                html_of("5*3*2 file*name*.txt"),
                r#"<div dir="ltr">5*3*2 file*name*.txt<br></div>"#
            );
            assert!(
                !html_of("***x***").contains("</b></i>"),
                "{}",
                html_of("***x***")
            );
            assert_eq!(
                html_of("[x](javascript:alert(1)) 2 * 3 * 4"),
                r#"<div dir="ltr">[x](javascript:alert(1)) 2 * 3 * 4<br></div>"#
            );
        }
    }
}
