//! The few document types every page reuses, each one template of the design system (task rust-doc-types; @nha, 2026-11-09:
//! « this is the same for all documents, we should have a few doc types we always reuse. RFC, Task brief, message draft with
//! context, CRM entry etc should follow a given template. », « the comments on this page for example follows the notion
//! example from the design system »). A page gives a document's fields as a tree's `document` element and never its layout:
//! the template orders, titles and frames them. Its text form is its HTML read by `forms::text_of`, never written twice.
//! /design?tab=documents shows each type's two forms.
use crate::components::{Avatar, Badge, Button, Input, Skeleton, Textarea};
use crate::design::families;
use dioxus::prelude::*;
use serde::Deserialize;

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
}

impl IssueStatus {
    pub(crate) fn class(&self) -> &'static str {
        match self {
            IssueStatus::InReview => "in-review",
            IssueStatus::InProgress => "in-progress",
            IssueStatus::Todo => "todo",
            IssueStatus::Backlog => "backlog",
        }
    }

    pub(crate) fn name(&self) -> &'static str {
        match self {
            IssueStatus::InReview => "In Review",
            IssueStatus::InProgress => "In Progress",
            IssueStatus::Todo => "Todo",
            IssueStatus::Backlog => "Backlog",
        }
    }

    /// Its status as /design?tab=linear's icon names it (`design::families::status`).
    pub(crate) fn kind(&self) -> &'static str {
        match self {
            IssueStatus::InReview => "review",
            IssueStatus::InProgress => "started",
            IssueStatus::Todo => "todo",
            IssueStatus::Backlog => "backlog",
        }
    }

    pub(crate) fn icon(&self) -> String {
        format!(r#"<span class="status-icon {}"></span>"#, self.class())
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

/// A card's link when it is one a page may open: a path of this app or an http(s) URL, never a script.
pub(crate) fn card_href(href: &Option<String>) -> Option<&str> {
    let href = href.as_deref()?;
    // a browser reads « /\\host » as « //host »: no backslash in a link of ours (/code-review of rust-nav-polish)
    if href.contains('\\') {
        return None;
    }
    (local_path(href) || href.starts_with("https://") || href.starts_with("http://"))
        .then_some(href)
}

/// A path of this app, never another host's (« //host », « /\\host »): a link or a form's target.
pub(crate) fn local_path(path: &str) -> bool {
    path.starts_with('/') && !path.starts_with("//") && !path.contains('\\')
}

/// A document, by its type: the closed set a page picks from.
#[derive(Deserialize, Clone, PartialEq)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Doc {
    Rfc(Rfc),
    TaskBrief(TaskBrief),
    MessageDraft(MessageDraft),
    CrmEntry(CrmEntry),
    PageComments(PageComments),
}

/// A titled part of a document, its text's line breaks kept.
#[derive(Deserialize, Clone, PartialEq)]
pub struct Part {
    pub title: String,
    pub text: String,
}

/// A label and its value: a lead's company, a draft's goal.
#[derive(Deserialize, Clone, PartialEq)]
pub struct Fact {
    pub label: String,
    pub value: String,
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
    #[serde(default)]
    pub quote: String,
    pub said: Vec<Said>,
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
}

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
    pub timeline: Vec<Fact>,
    #[serde(default)]
    pub notes: String,
}

/// A page's comments, on its passages or on it whole.
#[derive(Deserialize, Clone, PartialEq)]
pub struct PageComments {
    pub threads: Vec<Thread>,
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
            Doc::TaskBrief(TaskBrief { pr: Some(url), .. }) => pr_number(id, url).map(|_| ()),
            Doc::MessageDraft(MessageDraft {
                href: href @ Some(_),
                ..
            }) if card_href(href).is_none() => {
                Err(format!("{id}: not a link of this app or http(s)"))
            }
            Doc::MessageDraft(MessageDraft {
                decide: Some(path), ..
            }) if !local_path(path) => Err(format!("{id}: a decision posts to a path of this app")),
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

fn facts(all: &[Fact]) -> Element {
    rsx! {
        dl { class: "doc-facts",
            for f in all {
                dt { "{f.label}" }
                dd { "{f.value}" }
            }
        }
    }
}

/// Notion's threads (/design?tab=notion): a passage quoted, then each comment, its avatar, name and when.
fn threads(all: &[Thread]) -> Element {
    rsx! {
        div { class: "doc-threads",
            for t in all {
                div { class: "doc-thread",
                    if !t.quote.is_empty() {
                        div { class: "doc-quote", "{t.quote}" }
                    }
                    for s in &t.said {
                        div { class: "doc-said",
                            // the initials are the name's picture: the text says the name once (/code-review of PR 1250)
                            div { class: "doc-who", span { "aria-hidden": "true", Avatar { initials: initials(&s.who) } } "{s.who}" if !s.when.is_empty() { span { class: "doc-meta", "{s.when}" } } }
                            // a div kept in lines by CSS (pre-wrap): its text is one line, never one that reads as an event
                            div { class: "doc-words", "{s.words}" }
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

/// Linear's issue view from /design?tab=board (families::issue and its parts); his comments are Notion's threads.
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
    let activity = !(b.unread.is_empty() && b.events.is_empty() && b.comments.is_empty());
    let pr = (b.pr.as_deref()).and_then(|url| Some((url, pr_number("", url).ok()?)));
    let main = rsx! {
        {read(id, "why", "Its words", b.why.as_deref(), |at, t| rsx! { pre { id: at, class: "doc-text", "{t}" } })}
        div { id: "{id}-checks", {families::issue_part(&title, checks)} }
        if activity {
            {families::issue_part("Activity", rsx! {
                if !b.unread.is_empty() {
                    p { "{b.unread}" }
                }
                for e in &b.events {
                    {families::issue_event(&e.what, &e.when)}
                }
                {threads(&b.comments)}
            })}
        }
        {families::issue_part("Last run", rsx! {
            p { "{b.run}" }
            if !b.said.is_empty() {
                pre { class: "quote", "{b.said}" }
            }
        })}
    };
    let side = rsx! {
        {families::issue_prs(pr.as_slice())}
        if let Some((url, _)) = pr {
            {target(url)}
        }
    };
    rsx! {
        div { "data-status": b.status.class(),
            {families::issue(&b.id, &b.title, b.status.kind(), b.status.name(), &b.says, main, side)}
        }
    }
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
            form { method: "post", action: "{action}",
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
            form { method: "post", action: "{action}",
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

fn draft(d: &MessageDraft) -> Element {
    // its whole page's context alone: a listed draft says it in one line (/simplify of PR 1250)
    let context = || {
        let mut context: Vec<Fact> = three(d)
            .iter()
            .map(|(label, value)| Fact {
                label: label.to_string(),
                value: if value.is_empty() { "none" } else { value }.into(),
            })
            .collect();
        if !d.lead.is_empty() {
            context.push(Fact {
                label: "Lead".into(),
                value: d.lead.clone(),
            });
        }
        context.push(Fact {
            label: "Scores".into(),
            value: d.scores.clone(),
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
            {part("Context", facts(&context()))}
            if !d.text.is_empty() {
                {part("Message", words(&d.text))}
            }
            if let Some(path) = d.decide.as_deref().filter(|p| local_path(p)) {
                {part("Decision", decision(d, path))}
            }
            for v in &d.earlier {
                {part(&v.title, words(&v.text))}
            }
        }
    }
}

fn entry(e: &CrmEntry) -> Element {
    rsx! {
        header { class: "doc-head",
            h1 { class: "doc-title", "{e.name}" }
            p { class: "doc-meta", "{e.company}" Badge { look: "chip", "{e.stage}" } }
        }
        {part("Facts", facts(&e.facts))}
        {part("Timeline", facts(&e.timeline))}
        if !e.notes.is_empty() {
            {part("Notes", words(&e.notes))}
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
        for p in &r.parts {
            {part(&p.title, words(&p.text))}
        }
    }
}

/// A document drawn by its type's template, `id` its DOM id.
pub fn draw(id: &str, doc: &Doc) -> Element {
    let body = match doc {
        Doc::Rfc(r) => rfc(r),
        Doc::TaskBrief(b) => brief(id, b),
        Doc::MessageDraft(d) => draft(d),
        Doc::CrmEntry(e) => entry(e),
        Doc::PageComments(c) => threads(&c.threads),
    };
    rsx! { article { id, class: "doc doc-{doc.kind()}", {body} } }
}

/// A document's two forms: its HTML, the server render, and the text an agent reads of it.
pub fn forms(id: &str, doc: &Doc) -> (String, String) {
    let html = dioxus_ssr::render_element(draw(id, doc));
    let text = crate::forms::text_of(&html);
    (html, text)
}

/// /design?tab=documents: each type, its name, where it is used, an example as a tree gives it.
pub const TYPES: [(&str, &str, &str); 5] = [
    (
        "RFC",
        "The decision pages of private/docs.",
        r#"{"kind":"rfc","title":"One document model","status":"Review","bottom_line":"Every page picks one of five types.","parts":[{"title":"Context","text":"Each page laid its document out by hand."},{"title":"Decision","text":"A template a type, from the design system.\nA page gives fields, never layout."}]}"#,
    ),
    (
        "Task brief",
        "A task's page, /tasks/<id>.",
        r#"{"kind":"task-brief","id":"a-task","title":"A task's title","status":"todo","says":"the next pick","why":"(@nha: « because »)\nIts second line","checks":[{"text":"One thing: a test","met":true},{"text":"Another","met":null}],"no_checks":"None written.","run":"Try 1, 5 min ago: done.","pr":"https://github.com/o/r/pull/12","events":[{"what":"Started, try 1","when":"2026-11-09 10:00 UTC"}],"comments":[{"said":[{"who":"Nicolas","words":"Fix this first"}]}]}"#,
    ),
    (
        "Message draft",
        "A draft with its context, each of /drafts.",
        r#"{"kind":"message-draft","receiver":"Ana (Acme)","sender":"Nicolas Ha","channel":"linkedin","status":"pending","goal":"A first message","lead":"acme-platform","scores":"too_long 0.4","text":"Bonjour Ana,\nUne question courte."}"#,
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
];

/// An example of TYPES, as its tree gives it.
pub fn example(json: &str) -> Doc {
    serde_json::from_str(json).expect("an example of TYPES reads")
}

/// The Documents tab: each type drawn, its HTML and its text side by side, as the Components tab.
#[component]
pub fn Documents(dark: bool) -> Element {
    let cell = if dark {
        "app dark ds-cell"
    } else {
        "app light ds-cell"
    };
    let rows = TYPES.map(|(name, used, json)| {
        let doc = example(json);
        let kind = doc.kind();
        let (html, text) = forms(&format!("d-{kind}"), &doc);
        rsx! {
            section { id: "s-{kind}", key: "{kind}", class: "ds-section",
                div { class: "ds-head",
                    span { class: "ds-name", "{name}" }
                    span { class: "ds-side", "HTML" }
                    span { class: "ds-side", "Text" }
                }
                div { class: "ds-row",
                    div { class: "ds-label", "{used}" }
                    div { class: cell,
                        {draw(&format!("d-{kind}"), &doc)}
                        pre { class: "ds-code", "{html}" }
                    }
                    div { class: cell, pre { class: "ds-code", "{text}" } }
                }
            }
        }
    });
    rsx! {
        style { {CSS} }
        div { id: "ds-main", class: "ds-main",
            div { class: "ds-page",
                div { class: "ds-title", "Documents" }
                div { class: "ds-what",
                    "The document types every page reuses. A page gives the fields; the template lays them out. Each is drawn with its HTML, then the text an agent reads of it."
                }
                {rows.into_iter()}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::memory::TestBudget;

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
            "One document model\nReview\nEvery page picks one of five types.\nContext\nEach page laid its document out by hand.\n\
             Decision\nA template a type, from the design system.\nA page gives fields, never layout.\n"
        );
    }

    #[test]
    fn a_task_brief_is_the_way_back_its_state_why_checks_run_and_comments() {
        let _budget = TestBudget::new();
        let (html, text) = shown("task-brief");
        // Linear's issue view of /design?tab=board: its way back, title, parts and properties
        for part in [
            r#"<nav class="li-crumbs"><a href="/tasks">Tasks</a><span class="meta">›</span><span>a-task</span></nav>"#,
            r#"<h1 class="li-issue-title">A task&#39;s title</h1>"#,
            r#"<div class="li-check met" title="met">"#,
            r#"<div class="li-check" title="not judged">"#,
            r#"<span class="li-dot"></span><b>Started, try 1</b>"#,
            r#"<a class="li-tag li-pr" href="https://github.com/o/r/pull/12">"#,
            // his comments, Notion's threads (@nha, 2026-11-09: « the comments on this page for example follows the notion example »)
            r#"<div class="doc-thread">"#,
        ] {
            assert!(html.contains(part), "{part} in {html}");
        }
        assert_eq!(
            text,
            "Tasks\t› a-task\nA task's title\n(@nha: « because »)\nIts second line\nDone when: 1 of 2 met\n\
             met: One thing: a test\nnot judged: Another\nActivity\nStarted, try 1\t2026-11-09 10:00 UTC\nNicolas\n\
             Fix this first\nLast run\nTry 1, 5 min ago: done.\nProperties\nTodo\nthe next pick\nPull requests\n\
             #12\thttps://github.com/o/r/pull/12\n"
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
            html.contains(r#"<dt>Lead</dt><dd>acme-platform</dd>"#),
            "{html}"
        );
        assert_eq!(
            text,
            "Ana (Acme)\npending linkedin\nContext\nGoal\nA first message\nFrom\nNicolas Ha\nTo\nAna (Acme)\nLead\n\
             acme-platform\nScores\ntoo_long 0.4\nMessage\nBonjour Ana,\nUne question courte.\n"
        );
        // Listed: its words on its own page, its receiver the link there.
        let mut listed = example(TYPES[2].2);
        if let Doc::MessageDraft(d) = &mut listed {
            d.text.clear();
            d.href = Some("/d/a-first".into());
        }
        let (html, text) = forms("d", &listed);
        assert!(
            html.contains(r#"<a class="doc-title" href="/d/a-first">Ana (Acme)</a>"#),
            "{html}"
        );
        assert_eq!(
            text,
            "Ana (Acme)\t/d/a-first\npending linkedin\nA first message · Lead: acme-platform · Scores: too_long 0.4\n"
        );
    }

    /// A draft waiting on him, as /d/<id> gives it: `blank` names a field left empty.
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
            r#"<form method="post" action="/d/a-first/status"><input class="input" name="status" type="hidden" value="approved"/><button class="button">Approve</button></form>"#,
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
        let (html, text) = shown("crm-entry");
        assert!(
            html.contains(r#"<dt>2026-11-02</dt><dd>First message</dd>"#),
            "{html}"
        );
        assert_eq!(
            text,
            "Ana\nAcme contacted\nFacts\nRole\nCTO\nChannel\nlinkedin\nTimeline\n2026-11-02\nFirst message\n\
             2026-11-05\nHer reply\nNotes\nAsked for a rate.\n"
        );
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
             Nicolas Ha\nOn the page whole.\n"
        );
        // a comment's lines are one record line: one that reads as an event (`what\twhen`) never stands alone
        let said = r#"{"kind":"page-comments","threads":[{"said":[{"who":"N","words":"ok\nStarted, try 2\t2026-11-09 10:00 UTC"}]}]}"#;
        let (html, text) = forms("d", &example(said));
        assert!(html.contains("ok\nStarted"), "{html}");
        assert_eq!(text, "N\nok Started, try 2 2026-11-09 10:00 UTC\n");
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
        let stacked = r#"@container doc (max-width: 700px) { .li-issue { grid-template-columns: minmax(0, 1fr); grid-template-areas: "crumbs" "side" "main"; } }"#;
        assert!(crate::design::CARDS_CSS.contains(stacked));
    }

    /// Done-when 1: /design?tab=documents shows every type, drawn, its HTML and its text.
    #[test]
    fn the_design_page_shows_every_type_with_both_forms() {
        let _budget = TestBudget::new();
        let page = crate::design::page("documents", None);
        let text = crate::forms::text_of(&page);
        for (name, _, json) in TYPES {
            let doc = example(json);
            let (html, words) = forms(&format!("d-{}", doc.kind()), &doc);
            assert!(
                page.contains(&format!(r#"class="ds-name">{name}<"#)),
                "{name} in {page}"
            );
            assert!(page.contains(&html), "{name}'s HTML drawn: {page}");
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
            filled.contains(r#"<pre class="doc-text" id="x-why">"#),
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
}
