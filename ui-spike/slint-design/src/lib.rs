//! The design system page in Slint on slintcn: /design's six families, Linear cards and Notion comments, on design.json's tokens.
//! The same page as dioxus-page's --design, to decide between the two (@nha, 2026-11-07: « I think we want to do the same design
//! system with slintcn actually and compare. Cannot decide as-is. »). The window is main.rs, the steps are script.rs.
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, WindowAdapter};
use slint::{ComponentHandle, Model, ModelRc, PhysicalSize, SharedString, VecModel};
use std::cell::RefCell;
use std::rc::Rc;

pub mod script;

slint::include_modules!();

/// DesignLive's @families, as dioxus-page's FAMILIES, in order: slug, sections. A test keeps the two the same.
pub const FAMILIES: [(&str, &[&str]); 6] = [
    ("layout", &["guidelines", "heroui", "shell", "bell", "shared", "cards", "colors"]),
    ("data", &["data", "table", "charts", "stats", "timeline", "lists", "records", "knowledge"]),
    ("forms", &["buttons", "group", "field", "text"]),
    ("feedback", &["stage", "wait", "chips", "reading", "comments", "comments-principles", "handoff"]),
    ("mail", &["mail"]),
    ("board", &["board", "issues", "tasks", "work-card", "agents"]),
];

const CARDS: [[(i32, &str, &str); 5]; 2] = [
    [
        (478, "A store file edited outside the app is caught, never silently synced", "dashboard"),
        (476, "The precommit passes at any load, and each slow test is named with its own timing line", "dashboard"),
        (473, "The one poll classes every pane: working, blocked, done, shell, crashed", "orchestration"),
        (472, "Every decision page reads clear enough to decide in one pass", "dashboard"),
        (469, "A command lists every open step that waits, task or not", "dashboard"),
    ],
    [
        (457, "Publish the three project pages and their videos on the site", "site"),
        (455, "The window's memory traced to its largest three owners", "rust"),
        (451, "Every button that writes answers in under a second, with visible feedback", "dashboard"),
        (449, "Scoring credits what a person can do, not only what they did", "dashboard"),
        (447, "A reviewer started outside the check claims its PR, so no second one starts", "orchestration"),
    ],
];

pub(crate) const LINES: [(i32, &str, bool); 11] = [
    (0, "2026:", false),
    (1, "One tree, two renderers", true),
    (2, "the window and the web draw the same components", false),
    (2, "theme.css generated from design.json, shared by both", false),
    (2, "each behaviour is checked against the live document", false),
    (1, "Memory", false),
    (1, "Frame time", false),
    (1, "Fonts", true),
    (2, "one face a window", false),
    (2, "matches on family name, then falls back", false),
    (1, "Selection", false),
];

/// A drop's new order: the card goes first in its new column, each card's slot is its place there.
fn moved(cards: &[LinearCard], id: i32, to: i32) -> Vec<LinearCard> {
    let mut out = cards.to_vec();
    let Some(card) = out.iter_mut().find(|c| c.id == id) else { return out };
    if card.column == to {
        return out;
    }
    card.column = to;
    card.slot = -1;
    for column in 0..2 {
        let mut here: Vec<&mut LinearCard> = out.iter_mut().filter(|c| c.column == column).collect();
        here.sort_by_key(|c| c.slot);
        for (slot, card) in here.into_iter().enumerate() {
            card.slot = slot as i32;
        }
    }
    out
}

struct Thread {
    line: usize,
    start: usize,
    end: usize,
    said: Vec<String>,
    /// Older replies folded, as Notion does when the thread loses focus.
    folded: bool,
}

#[derive(Default)]
struct Notes {
    /// In the order they began: an index names one for good.
    threads: Vec<Thread>,
    /// The selection: a line and its bytes.
    selection: Option<(usize, usize, usize)>,
    /// The selection the open composer was opened on: a later click must not move the comment.
    commenting: Option<(usize, usize, usize)>,
    page: Vec<String>,
}

/// The words picked, without their outer spaces: None when only spaces were. The offsets are the TextInput's own, so on a char boundary.
fn trimmed(line: &str, start: usize, end: usize) -> Option<(usize, usize)> {
    let picked = &line[start..end];
    let words = picked.trim();
    (!words.is_empty()).then(|| {
        let from = start + (picked.len() - picked.trim_start().len());
        (from, from + words.len())
    })
}

/// The threads on the rail, by their place in the page, each keeping its index.
fn publish(app: &App, notes: &Notes) {
    let said = |id: String, words: &str| NotionSaid { id: id.into(), words: words.into() };
    let mut order: Vec<usize> = (0..notes.threads.len()).collect();
    order.sort_by_key(|&t| (notes.threads[t].line, notes.threads[t].start));
    let threads: Vec<NotionThread> = order
        .iter()
        .map(|&t| {
            let thread = &notes.threads[t];
            let last = thread.said.len() - 1;
            let hide = thread.folded && thread.said.len() > 2;
            let shown: Vec<NotionSaid> = thread.said.iter().enumerate().filter(|(i, _)| !hide || *i == 0 || *i == last).map(|(i, w)| said(format!("no-said-{t}-{i}"), w)).collect();
            NotionThread {
                index: t as i32,
                quote: LINES[thread.line].1[thread.start..thread.end].into(),
                said: ModelRc::new(VecModel::from(shown)),
                folded_out: if hide { thread.said.len() as i32 - 2 } else { 0 },
            }
        })
        .collect();
    // Row by row: a repeated thread keeps its instance, so an open reply keeps its focus.
    let model = app.get_threads();
    let model = model.as_any().downcast_ref::<VecModel<NotionThread>>().expect("app() sets a VecModel");
    for (i, thread) in threads.into_iter().enumerate() {
        if i < model.row_count() {
            model.set_row_data(i, thread);
        } else {
            model.push(thread);
        }
    }
    let first = notes.threads.iter().map(|t| t.line).chain(notes.commenting.filter(|_| app.get_composing_new()).map(|c| c.0)).min();
    app.set_rail_line(first.unwrap_or(0) as i32);
    let page: Vec<NotionSaid> = notes.page.iter().enumerate().map(|(i, w)| said(format!("no-page-{i}"), w)).collect();
    app.set_page_comments(ModelRc::new(VecModel::from(page)));
}

/// A line's marks, each what stands before it and its words, in the line's order. Row by row, so the other lines keep their TextInput.
fn mark_line(notes: &Notes, lines: &VecModel<NotionLine>, line: usize) {
    let mut row = lines.row_data(line).expect("a line");
    let mut here: Vec<&Thread> = notes.threads.iter().filter(|t| t.line == line).collect();
    here.sort_by_key(|t| t.start);
    let marks: Vec<NotionMark> = here.iter().map(|t| NotionMark { before: row.words[..t.start].into(), phrase: row.words[t.start..t.end].into() }).collect();
    row.marks = ModelRc::new(VecModel::from(marks));
    lines.set_row_data(line, row);
}

/// The app with its data and what Slint cannot do alone: a model it cannot insert in, a string it cannot slice.
pub fn app() -> App {
    let app = App::new().expect("window");
    let cards: Vec<LinearCard> = CARDS
        .iter()
        .enumerate()
        .flat_map(|(column, cards)| {
            cards.iter().enumerate().map(move |(slot, (id, title, tag))| LinearCard {
                id: *id,
                title: (*title).into(),
                tag: (*tag).into(),
                column: column as i32,
                slot: slot as i32,
            })
        })
        .collect();
    let model = Rc::new(VecModel::from(cards));
    app.set_cards(ModelRc::from(model.clone()));
    let weak = app.as_weak();
    app.on_dropped(move |id, to| {
        let now: Vec<LinearCard> = model.iter().collect();
        // Row by row: a repeated element keeps its instance, so its x and y animate from where it was let go.
        for (i, card) in moved(&now, id, to).into_iter().enumerate() {
            model.set_row_data(i, card);
        }
        let count = |c: i32| model.iter().filter(|x| x.column == c).count() as i32;
        let counts = VecModel::from(vec![count(0), count(1)]);
        weak.upgrade().expect("app").set_counts(ModelRc::new(counts));
    });

    let lines: Vec<NotionLine> = LINES.iter().map(|(depth, words, link)| NotionLine { depth: *depth, words: (*words).into(), link: *link, marks: ModelRc::default() }).collect();
    let lines = Rc::new(VecModel::from(lines));
    app.set_lines(ModelRc::from(lines.clone()));
    app.set_threads(ModelRc::new(VecModel::<NotionThread>::default()));
    let notes = Rc::new(RefCell::new(Notes::default()));

    let weak = app.as_weak();
    let held = notes.clone();
    let all = lines.clone();
    app.on_selected(move |line, start, end| {
        let app = weak.upgrade().expect("app");
        let mut notes = held.borrow_mut();
        let l = line as usize;
        let words = all.row_data(l).expect("a line").words;
        let Some((start, end)) = trimmed(&words, start as usize, end as usize) else {
            // The line that holds the pick lost it. Another line's cursor closes nothing it does not hold.
            if notes.selection.is_some_and(|s| s.0 == l) {
                notes.selection = None;
                app.set_picked(-1);
            }
            return;
        };
        notes.selection = Some((l, start, end));
        app.set_picked_before(words[..start].into());
        app.set_picked_phrase(words[start..end].into());
        app.set_picked_depth(all.row_data(l).expect("a line").depth);
        app.set_picked(line);
    });

    let weak = app.as_weak();
    let held = notes.clone();
    app.on_comment(move || {
        let app = weak.upgrade().expect("app");
        // Words being typed are never thrown away by a new comment, as by a click on a quote.
        if !app.get_draft().is_empty() {
            return;
        }
        let mut notes = held.borrow_mut();
        notes.commenting = notes.selection;
        app.set_composing(-1);
        app.set_composing_new(true);
        app.set_new_quote(app.get_picked_phrase());
        app.set_draft(SharedString::new());
        app.set_picked(-1);
        publish(&app, &notes);
    });

    let weak = app.as_weak();
    let held = notes.clone();
    let all = lines.clone();
    app.on_post(move || {
        let app = weak.upgrade().expect("app");
        let words = app.get_draft();
        if words.is_empty() {
            return;
        }
        let mut notes = held.borrow_mut();
        if app.get_composing_new() {
            let Some((line, start, end)) = notes.commenting else { return };
            notes.threads.push(Thread { line, start, end, said: vec![words.to_string()], folded: false });
            mark_line(&notes, &all, line);
            app.set_composing(notes.threads.len() as i32 - 1);
            app.set_composing_new(false);
        } else if let Some(thread) = notes.threads.get_mut(app.get_composing() as usize) {
            thread.said.push(words.to_string());
        }
        app.set_draft(SharedString::new());
        publish(&app, &notes);
    });

    let weak = app.as_weak();
    let held = notes.clone();
    app.on_post_page(move || {
        let app = weak.upgrade().expect("app");
        let words = app.get_page_draft();
        if !words.is_empty() {
            let mut notes = held.borrow_mut();
            notes.page.push(words.to_string());
            app.set_page_draft(SharedString::new());
            publish(&app, &notes);
        }
    });

    let weak = app.as_weak();
    let held = notes.clone();
    app.on_fold_threads(move || {
        let app = weak.upgrade().expect("app");
        let mut notes = held.borrow_mut();
        for t in notes.threads.iter_mut() {
            t.folded = true;
        }
        app.set_composing_new(false);
        publish(&app, &notes);
    });

    let weak = app.as_weak();
    let held = notes.clone();
    app.on_unfold(move |t| {
        let mut notes = held.borrow_mut();
        notes.threads[t as usize].folded = false;
        publish(&weak.upgrade().expect("app"), &notes);
    });

    let weak = app.as_weak();
    app.on_reopen(move |t| {
        let app = weak.upgrade().expect("app");
        // Words being typed are never thrown away by a click on a quote.
        if t == app.get_composing() || !app.get_draft().is_empty() {
            return;
        }
        let mut notes = notes.borrow_mut();
        notes.threads[t as usize].folded = false;
        app.set_composing_new(false);
        app.set_composing(t);
        publish(&app, &notes);
    });
    app
}

/// The window with no display, on Slint's software renderer: what the test and --no-window share.
pub struct Headless {
    pub window: Rc<MinimalSoftwareWindow>,
}

struct HeadlessPlatform(Rc<MinimalSoftwareWindow>);

impl Platform for HeadlessPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.0.clone())
    }
}

impl Headless {
    pub fn new() -> Self {
        let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
        slint::platform::set_platform(Box::new(HeadlessPlatform(window.clone()))).expect("platform");
        Self { window }
    }

    /// One frame's work, as a window's: timers and animations move, then a whole draw.
    pub fn frame(&self, pixels: &mut [slint::platform::software_renderer::PremultipliedRgbaColor], width: u32) {
        slint::platform::update_timers_and_animations();
        self.window.request_redraw();
        self.window.draw_if_needed(|renderer| {
            renderer.render(pixels, width as usize);
        });
    }

    pub fn size(&self, width: u32, height: u32) {
        self.window.set_size(PhysicalSize::new(width, height));
    }
}

impl Default for Headless {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(id: i32, column: i32, slot: i32) -> LinearCard {
        LinearCard { id, title: "".into(), tag: "".into(), column, slot }
    }

    #[test]
    fn a_drop_goes_first_in_its_column_and_closes_the_gap() {
        let after = moved(&[card(1, 0, 0), card(2, 0, 1), card(3, 1, 0)], 1, 1);
        let at = |id| after.iter().find(|c| c.id == id).map(|c| (c.column, c.slot)).expect("a card");
        assert_eq!((at(1), at(2), at(3)), ((1, 0), (0, 0), (1, 1)));
        assert_eq!(moved(&after, 1, 1), after, "a drop in its own column moves nothing");
    }

    #[test]
    fn a_pick_leaves_out_its_outer_spaces() {
        assert_eq!(trimmed("the window and", 3, 11), Some((4, 10)));
        assert_eq!(trimmed("the window and", 3, 4), None);
    }

    /// The same families as dioxus-page, whose own test keeps them DesignLive's.
    #[test]
    fn the_families_are_dioxus_pages() {
        let theirs = include_str!("../../../crates/surface/src/design/mod.rs");
        let theirs = theirs.split("pub const FAMILIES").nth(1).and_then(|t| t.split("\n];").next()).expect("their list");
        // Each family's slug is the first string since the last list closed; its sections are in the list that follows.
        let blocks: Vec<&str> = theirs.split("&[").collect();
        let read: Vec<(&str, Vec<&str>)> = blocks
            .windows(2)
            .map(|b| {
                let slug = b[0].rsplit(']').next().and_then(|head| head.split('"').nth(1)).expect("a slug");
                let list = b[1].split(']').next().expect("a list");
                (slug, list.split("(\"").skip(1).map(|s| s.split('"').next().expect("an id")).collect())
            })
            .collect();
        let ours: Vec<(&str, Vec<&str>)> = FAMILIES.iter().map(|(slug, sections)| (*slug, sections.to_vec())).collect();
        assert_eq!(ours, read);
    }
}
