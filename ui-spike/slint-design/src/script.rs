//! The page's script: dioxus-page's design script, the same behaviours under the same names in the same order, plus the four
//! that are part of the comparison. Input enters as window events, a part is found by its accessible-id and read live.
use crate::{App, FAMILIES};
use i_slint_backend_testing::{ElementHandle, ElementRoot};
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition};
use std::cell::RefCell;
use std::rc::Rc;
use ui_spike_common::meter::{Run, memory};

const PATIENCE: u32 = 120;
/// The longest family is under 9,000 px high.
const WHEELS_THROUGH_A_FAMILY: usize = 18;

/// What a run must fail. None: unlike Blitz, a click here focuses a button.
pub const RECORDED_FAILURES: [&str; 0] = [];

type Check = Box<dyn Fn(&App) -> bool>;

pub enum At {
    Id(&'static str),
    In(&'static str, f32, f32),
    /// A line of the Notion page, and a byte of its words.
    Byte(usize, usize),
}

pub enum Step {
    Begin(&'static str),
    End,
    Expect(&'static str, Check),
    Wait(u32),
    Shot(&'static str),
    Move(At),
    Down,
    Up,
    RightDown,
    RightUp,
    Text(String),
    Wheel(f32),
    WheelTo(&'static str),
}

pub fn find(app: &App, id: &str) -> Option<ElementHandle> {
    let id = id.to_string();
    app.root_element()
        .query_descendants()
        .match_predicate(move |e| e.accessible_id().as_deref() == Some(&id))
        .find_first()
}

pub fn exists(app: &App, id: &str) -> bool {
    find(app, id).is_some()
}

pub fn value(app: &App, id: &str) -> String {
    find(app, id)
        .and_then(|e| e.accessible_value())
        .map_or(String::new(), |v| v.to_string())
}

fn click(at: At) -> Vec<Step> {
    vec![Step::Move(at), Step::Down, Step::Up]
}

fn drag(from: At, to: At) -> Vec<Step> {
    vec![
        Step::Move(from),
        Step::Down,
        Step::Wait(1),
        Step::Move(to),
        Step::Wait(1),
        Step::Up,
    ]
}

fn typed(words: &str) -> Vec<Step> {
    words.chars().map(|c| Step::Text(c.to_string())).collect()
}

fn expect(what: &'static str, check: impl Fn(&App) -> bool + 'static) -> Step {
    Step::Expect(what, Box::new(check))
}

/// « hover 412 »: its state, what the frame's colour is, as a number.
fn state(app: &App, id: &str) -> String {
    value(app, id).split(' ').next().unwrap_or("").to_string()
}

fn level(app: &App, id: &str) -> String {
    value(app, id).rsplit(' ').next().unwrap_or("").to_string()
}

/// The sections on screen in both themes: each one's id appears twice, once per tree. ElementHandle skips what a clip hides, so the
/// script wheels through the page and adds up what it saw. One walk of the tree per read: a walk's time lands in the frame measured.
fn in_both_themes(app: &App, sections: &'static [&'static str]) -> Vec<&'static str> {
    let found = app
        .root_element()
        .query_descendants()
        .match_predicate(|e| e.accessible_id().is_some_and(|id| id.starts_with("s-")))
        .find_all();
    let ids: Vec<_> = found.iter().filter_map(|e| e.accessible_id()).collect();
    sections
        .iter()
        .copied()
        .filter(|s| {
            ids.iter()
                .filter(|id| id.strip_prefix("s-") == Some(*s))
                .count()
                == 2
        })
        .collect()
}

/// The real pseudo-state on the real control draws what the forced one beside it draws: same state, same colour.
fn same_as_forced(what: &'static str, id: &'static str, forced: &'static str) -> Step {
    expect(what, move |a| {
        let (x, y) = (value(a, id), value(a, forced));
        !x.is_empty() && x == y
    })
}

const LINE: &str = "the window and the web draw the same components";
const PHRASE: &str = "the same components";
const SECOND: &str = "the window";

/// A drag over those words of line 2, either way.
fn picking(phrase: &'static str, backwards: bool, shot: Option<&'static str>) -> Vec<Step> {
    let start = LINE.find(phrase).expect("the phrase is in the line");
    let (first, last) = (start, start + phrase.len());
    let (from, to) = if backwards {
        (last, first)
    } else {
        (first, last)
    };
    let mut s = vec![
        Step::Move(At::Byte(2, from)),
        Step::Down,
        Step::Wait(1),
        Step::Move(At::Byte(2, to)),
        Step::Wait(1),
    ];
    s.extend(shot.map(Step::Shot));
    s.push(Step::Up);
    s
}

/// The marks drawn on line 2, each with a width, in its order left to right: the elements, not the model behind them.
fn marks(app: &App) -> Vec<String> {
    let found = app
        .root_element()
        .query_descendants()
        .match_predicate(|e| {
            e.accessible_id()
                .is_some_and(|id| id.starts_with("no-mark-2-"))
        })
        .find_all();
    let mut drawn: Vec<(f32, String)> = found
        .iter()
        .filter(|e| e.size().width > 0.0)
        .map(|e| {
            (
                e.absolute_position().x,
                e.accessible_value()
                    .map_or(String::new(), |v| v.to_string()),
            )
        })
        .collect();
    drawn.sort_by(|a, b| a.0.total_cmp(&b.0));
    drawn.into_iter().map(|d| d.1).collect()
}

pub fn script() -> Vec<Step> {
    use At::*;
    let mut s = vec![];
    for (i, (slug, sections)) in FAMILIES.iter().enumerate() {
        let name: &'static str = Box::leak(format!("family: {slug}").into_boxed_str());
        let tab: &'static str = Box::leak(format!("f-{slug}").into_boxed_str());
        let shot: &'static str = Box::leak(format!("family-{slug}").into_boxed_str());
        let shot2: &'static str = Box::leak(format!("family-{slug}-2").into_boxed_str());
        s.push(Step::Begin(name));
        s.extend(click(Id(tab)));
        let sections: &'static [&'static str] = sections;
        s.push(expect("it opens", move |a| a.get_view() == i as i32));
        let seen = Rc::new(RefCell::new(std::collections::BTreeSet::new()));
        let read = |seen: &Rc<RefCell<std::collections::BTreeSet<&'static str>>>| {
            let keep = seen.clone();
            expect("sections read", move |a| {
                keep.borrow_mut().extend(in_both_themes(a, sections));
                true
            })
        };
        s.extend([
            Step::Wait(2),
            read(&seen),
            Step::Shot(shot),
            Step::Move(Id("ds-main")),
            Step::Wheel(700.0),
            Step::Wait(4),
            Step::Shot(shot2),
        ]);
        for _ in 0..WHEELS_THROUGH_A_FAMILY {
            s.push(read(&seen));
            s.push(Step::Wheel(500.0));
            s.push(Step::Wait(1));
        }
        let seen_all = seen.clone();
        s.push(expect("its sections are drawn in both themes", move |_| {
            seen_all.borrow().len() == sections.len()
        }));
        s.push(Step::End);
    }

    s.push(Step::Begin("fonts: three faces in one window"));
    s.extend(click(Id("f-layout")));
    s.push(expect("sans, mono and hand draw differently", |a| {
        let v = value(a, "faces");
        let w: Vec<i32> = v.split('|').filter_map(|x| x.parse().ok()).collect();
        // Mono's two words have the same width on a Mac, where Menlo is. The sans and the hand differ from each other.
        w.len() == 4 && w[0] != w[3] && (!cfg!(target_os = "macos") || w[1] == w[2])
    }));
    s.push(Step::End);

    s.push(Step::Begin(
        "forms: real :hover, :active and :focus draw the forced states",
    ));
    s.extend(click(Id("f-forms")));
    s.push(Step::Move(Id("ds-main")));
    s.push(Step::WheelTo("try"));
    s.push(Step::Wait(4));
    s.push(Step::Move(Id("try")));
    s.push(Step::Wait(20));
    s.push(same_as_forced(
        "the real :hover is the forced one",
        "try",
        "forced-hover",
    ));
    s.push(Step::Down);
    s.push(Step::Wait(20));
    s.push(same_as_forced(
        "the real :active is the forced one",
        "try",
        "forced-active",
    ));
    s.push(Step::Shot("state-pressed"));
    s.push(Step::Up);
    s.push(Step::End);

    s.push(Step::Begin("forms: a click focuses a button"));
    s.extend(click(Id("try")));
    s.push(expect("the real :focus rings it as the forced one", |a| {
        value(a, "try").contains("focus") && value(a, "forced-focus").contains("focus")
    }));
    s.push(Step::End);

    s.push(Step::Begin("forms: a click focuses a field"));
    s.push(Step::WheelTo("try-field"));
    s.push(Step::Wait(4));
    s.extend(click(Id("try-field")));
    s.push(expect(
        "the real :focus rings it as the forced one, under a real hover",
        |a| value(a, "try-field").starts_with("focus hover "),
    ));
    s.push(Step::Wait(2));
    s.push(Step::Shot("state-focus"));
    s.extend(typed("1"));
    s.push(expect("and it takes a key", |a| {
        value(a, "try-field").ends_with("6501")
    }));
    s.push(Step::End);

    // Each frame's colour on the way: an eased hover passes through a colour that is neither end.
    s.push(Step::Begin("forms: a transition eases"));
    s.push(Step::WheelTo("try"));
    s.push(Step::Wait(4));
    s.push(Step::Move(In("ds-main", 4.0, 4.0)));
    s.push(Step::Wait(30));
    let rest = Rc::new(RefCell::new(String::new()));
    let keep = rest.clone();
    s.push(expect("it has a colour", move |a| {
        *keep.borrow_mut() = level(a, "try");
        !keep.borrow().is_empty()
    }));
    let between = Rc::new(RefCell::new(false));
    s.push(Step::Move(Id("try")));
    for _ in 0..12 {
        let (r, b) = (rest.clone(), between.clone());
        s.push(expect("colour read", move |a| {
            let now = level(a, "try");
            if state(a, "try") == "hover" && now != *r.borrow() && now != level(a, "forced-hover") {
                *b.borrow_mut() = true;
            }
            true
        }));
        s.push(Step::Wait(1));
    }
    s.push(expect("a colour between the two ends", move |_| {
        *between.borrow()
    }));
    s.push(Step::End);

    // The keyframe's frames: the angle keeps moving.
    s.push(Step::Begin("forms: a keyframe turns"));
    s.push(Step::WheelTo("spin"));
    s.push(Step::Wait(4));
    let seen = Rc::new(RefCell::new(std::collections::BTreeSet::new()));
    for _ in 0..12 {
        let keep = seen.clone();
        s.push(expect("angle read", move |a| {
            keep.borrow_mut().insert(value(a, "spin"));
            true
        }));
        s.push(Step::Wait(1));
    }
    s.push(expect("the angle moved on the way", move |_| {
        seen.borrow().len() >= 6
    }));
    s.push(Step::End);

    s.push(Step::Begin("theme: the page's toggle"));
    s.extend(click(Id("ds-theme")));
    s.push(expect("the root's colour changes", |a| a.get_dark()));
    s.push(Step::Wait(2));
    s.push(Step::Shot("family-forms-dark"));
    s.extend(click(Id("ds-theme")));
    s.push(expect("and back", |a| !a.get_dark()));
    s.push(Step::End);

    s.push(Step::Begin("linear: the board"));
    s.extend(click(Id("v-linear")));
    s.push(expect("the cards are drawn", |a| {
        exists(a, "li-card-478") && exists(a, "li-card-457")
    }));
    s.push(Step::Wait(2));
    s.push(Step::Shot("linear-board"));
    s.push(Step::End);

    s.push(Step::Begin("linear: hover and drag"));
    s.push(Step::Move(Id("li-card-476")));
    s.push(expect("the shadow rises", |a| {
        value(a, "li-card-476").ends_with("hover")
    }));
    s.push(Step::Wait(20));
    s.push(Step::Shot("linear-hover"));
    s.push(Step::Down);
    s.push(Step::Move(In("li-card-476", 60.0, 20.0)));
    s.push(Step::Wait(1));
    s.push(Step::Move(In("li-col-1", 160.0, 120.0)));
    s.push(Step::Wait(1));
    s.push(Step::Move(In("li-col-1", 150.0, 110.0)));
    s.push(expect("its column is tinted", |a| {
        value(a, "li-col-1") == "over"
    }));
    s.push(Step::Wait(16));
    s.push(Step::Shot("linear-drag"));
    s.push(Step::Up);
    s.push(expect("the card moved", |a| {
        value(a, "li-count-0") == "4" && value(a, "li-count-1") == "6"
    }));
    s.push(Step::End);

    // Its position on each frame: it passes through places that are neither where it was let go nor its slot.
    s.push(Step::Begin("linear: a card settles after a drop"));
    s.push(Step::Wait(40));
    s.push(Step::Move(Id("li-card-478")));
    s.push(Step::Down);
    s.push(Step::Move(In("li-card-478", 60.0, 30.0)));
    s.push(Step::Wait(1));
    s.push(Step::Move(In("li-col-1", 180.0, 300.0)));
    s.push(Step::Wait(2));
    s.push(Step::Up);
    let path = Rc::new(RefCell::new(vec![]));
    for _ in 0..30 {
        let keep = path.clone();
        s.push(expect("position read", move |a| {
            keep.borrow_mut().push(value(a, "li-card-478"));
            true
        }));
        s.push(Step::Wait(1));
    }
    s.push(expect(
        "it passed through 5 places on the way to its slot",
        move |_| {
            let mut places = path.borrow().clone();
            places.dedup();
            places.len() >= 5
        },
    ));
    s.push(Step::End);

    s.push(Step::Begin("linear: its dark theme"));
    s.extend(click(Id("ds-theme")));
    s.push(expect("dark", |a| a.get_dark()));
    s.push(Step::Wait(2));
    s.push(Step::Shot("linear-dark"));
    s.push(Step::End);

    s.push(Step::Begin("notion: a drag picks « the same components »"));
    s.extend(click(Id("v-notion")));
    s.push(expect("the page", |a| exists(a, "no-line-2")));
    s.push(Step::Wait(2));
    s.extend(picking(PHRASE, false, Some("notion-drag")));
    s.push(expect("the toolbar opens, the pick on those words", |a| {
        exists(a, "no-tool") && a.get_picked_phrase() == PHRASE && marks(a).is_empty()
    }));
    s.push(Step::Wait(2));
    s.push(Step::Shot("notion-toolbar"));
    s.push(Step::End);

    s.push(Step::Begin("notion: a click picks no words"));
    s.extend(click(Byte(4, 3)));
    s.push(expect("the toolbar goes", |a| !exists(a, "no-tool")));
    // Each line is its own single-line TextInput and holds the pointer: Dioxus's drag across two lines picks nothing, this one reads
    // the pointer's x alone and stays on its first line.
    s.extend(drag(Byte(2, 4), Byte(3, 30)));
    s.push(expect("a drag across two lines stays on its first", |a| {
        exists(a, "no-tool") && LINE[4..].starts_with(a.get_picked_phrase().as_str())
    }));
    s.extend(click(Byte(4, 3)));
    s.push(expect("a click closes it", |a| !exists(a, "no-tool")));
    let start = LINE.find(PHRASE).expect("the phrase is in the line");
    // One jump, as a fast hand: nothing lands inside the words on the way.
    s.extend([
        Step::Move(Byte(2, start)),
        Step::Down,
        Step::Move(In("no-line-2", 500.0, 13.0)),
        Step::Up,
    ]);
    s.push(expect("a drag past the last word stops at it", |a| {
        exists(a, "no-tool") && a.get_picked_phrase() == PHRASE
    }));
    s.extend(picking("same", false, None));
    s.push(expect("a drag from inside a pick picks again", |a| {
        exists(a, "no-tool") && a.get_picked_phrase() == "same"
    }));
    s.extend([
        Step::Move(Byte(4, 3)),
        Step::RightDown,
        Step::RightUp,
        Step::Wait(2),
    ]);
    s.push(expect("the right button leaves a pick open", |a| {
        exists(a, "no-tool") && a.get_picked_phrase() == "same"
    }));
    s.extend(click(Byte(4, 3)));
    s.extend([
        Step::Move(Byte(4, 3)),
        Step::RightDown,
        Step::Move(Byte(4, 12)),
        Step::Wait(2),
        Step::RightUp,
        Step::Wait(2),
    ]);
    s.push(expect("nor does its drag open one", |a| {
        !exists(a, "no-tool")
    }));
    s.extend(drag(Byte(2, start - 1), Byte(2, start + PHRASE.len())));
    s.push(expect("a pick leaves out its outer spaces", |a| {
        exists(a, "no-tool") && a.get_picked_phrase() == PHRASE
    }));
    // A press here so soon after the last would be a double click, which TextInput makes a word selection: free, and not this test.
    s.push(Step::Wait(70));
    s.extend(drag(Byte(2, start - 1), Byte(2, start)));
    s.push(expect("a space alone picks nothing", |a| {
        !exists(a, "no-tool")
    }));
    s.extend(picking(PHRASE, true, None));
    s.push(expect("a drag backwards picks them too", |a| {
        exists(a, "no-tool") && a.get_picked_phrase() == PHRASE
    }));
    s.push(Step::End);

    s.push(Step::Begin(
        "notion: a comment marks only those words, with its replies",
    ));
    s.extend(click(Id("no-comment")));
    s.push(expect("its composer quotes them", |a| {
        exists(a, "no-input") && value(a, "no-new-quote") == PHRASE
    }));
    for words in [
        "here is a test comment",
        "and a first reply",
        "and a second",
    ] {
        s.extend(click(Id("no-input")));
        s.extend(typed(words));
        s.push(expect("the placeholder goes", |a| {
            !a.get_draft().is_empty()
        }));
        s.extend(click(Id("no-send")));
        s.push(Step::Wait(2));
    }
    s.push(expect("three comments on that phrase", |a| {
        exists(a, "no-said-0-2") && value(a, "no-quote-0") == PHRASE
    }));
    s.push(expect("the mark is those words", |a| marks(a) == [PHRASE]));
    s.push(Step::Wait(2));
    s.push(Step::Shot("notion-thread"));
    s.push(Step::End);

    s.push(Step::Begin("notion: a second phrase on the marked line"));
    // From its end, past the line's first letter: a press half a pixel into that letter landed after it here (not traced into Slint).
    s.extend([
        Step::Move(Byte(2, SECOND.len())),
        Step::Down,
        Step::Wait(1),
        Step::Move(In("no-line-2", 2.0, 13.0)),
        Step::Wait(1),
        Step::Up,
    ]);
    s.extend(click(Id("no-comment")));
    s.extend(click(Id("no-input")));
    s.extend(typed("another phrase"));
    // A press that slips 10 px still sends.
    s.extend(drag(In("no-send", 6.0, 11.0), In("no-send", 16.0, 11.0)));
    s.push(expect("two marks, each its words", |a| {
        marks(a) == [SECOND, PHRASE] && value(a, "no-quote-1") == SECOND
    }));
    s.push(Step::Wait(2));
    s.push(Step::Shot("notion-two-phrases"));
    s.extend(click(Id("no-quote-0")));
    s.push(expect("a click on the first thread reopens it", |a| {
        a.get_composing() == 0 && exists(a, "no-input")
    }));
    s.push(Step::Move(Id("no-said-1-0")));
    s.extend(click(Id("no-said-1-0-more")));
    s.push(expect(
        "the other thread's ... opens its menu and leaves the composer",
        |a| exists(a, "no-menu") && a.get_composing() == 0,
    ));
    s.extend(click(Id("no-said-1-0")));
    s.push(expect("and a click closes it", |a| !exists(a, "no-menu")));
    s.extend(picking("same", false, None));
    s.push(expect("a pick inside a mark shows", |a| {
        a.get_picked_phrase() == "same" && exists(a, "no-tool")
    }));
    s.extend(click(Byte(4, 3)));
    s.push(expect("the press closes it", |a| {
        !exists(a, "no-tool") && marks(a) == [SECOND, PHRASE]
    }));
    s.push(Step::End);

    s.push(Step::Begin(
        "notion: the page's own comments, replies folded",
    ));
    s.extend(click(Id("no-add")));
    s.push(expect("its composer", |a| exists(a, "no-page-input")));
    s.extend(click(Id("no-page-input")));
    s.extend(typed("comments on the whole page"));
    s.extend(click(Id("no-page-post")));
    s.push(expect("posted, the thread folded", |a| {
        exists(a, "no-page-0") && exists(a, "no-unfold-0")
    }));
    s.push(Step::Wait(2));
    s.push(Step::Shot("notion-page"));
    s.push(Step::End);

    s.push(Step::Begin("notion: a comment's menu"));
    s.push(Step::Move(Id("no-page-0")));
    s.push(expect("its ... shows on hover", |a| {
        exists(a, "no-page-0-more")
    }));
    s.extend(click(Id("no-page-0-more")));
    s.push(expect("the menu opens", |a| exists(a, "no-menu")));
    s.push(Step::Wait(2));
    s.push(Step::Shot("notion-menu"));
    s.push(Step::End);

    s.push(Step::Begin("notion: its light theme"));
    // A Slint popup takes the click that closes it: the second one reaches the button.
    s.extend(click(Id("ds-theme")));
    s.push(expect("the click outside closes the menu", |a| {
        !exists(a, "no-menu")
    }));
    s.extend(click(Id("ds-theme")));
    s.push(expect("light", |a| !a.get_dark()));
    s.push(Step::Wait(2));
    s.push(Step::Shot("notion-light"));
    s.push(Step::End);
    s
}

#[derive(Default)]
struct Behaviour {
    name: &'static str,
    run: Run,
    waited: u32,
    worst: u32,
    failed: Option<&'static str>,
}

pub enum Tick {
    Idle,
    Shot(&'static str),
    Done,
}

pub struct Driver {
    steps: std::vec::IntoIter<Step>,
    now: Option<Step>,
    wait: u32,
    current: Behaviour,
    pointer: LogicalPosition,
    /// While a target is off screen: which way the wheel goes, and where the page stood.
    wheel_by: f32,
    last_scroll: f32,
    pub lines: Vec<String>,
    pub results: Vec<(&'static str, bool)>,
}

impl Driver {
    pub fn new() -> Self {
        let mut steps = script().into_iter();
        let now = steps.next();
        Self {
            steps,
            now,
            wait: 0,
            current: Behaviour::default(),
            pointer: LogicalPosition::new(0.0, 0.0),
            wheel_by: -300.0,
            last_scroll: f32::NAN,
            lines: vec![],
            results: vec![],
        }
    }

    pub fn failed(&self) -> Vec<&'static str> {
        self.results.iter().filter(|r| !r.1).map(|r| r.0).collect()
    }

    pub fn as_recorded(&self) -> bool {
        self.failed() == RECORDED_FAILURES
    }

    /// Why each failed, for the person reading a refused run.
    pub fn why(&self) -> Vec<&String> {
        self.lines
            .iter()
            .filter(|l| l.contains("\"ok\":false"))
            .collect()
    }

    fn at(&mut self, app: &App, at: &At) -> Option<LogicalPosition> {
        if let At::Byte(line, byte) = at {
            let (depth, words, _) = crate::LINES[*line];
            app.set_probe(words[..*byte].into());
            let e = find(app, &format!("no-line-{line}"))?;
            let (p, size) = (e.absolute_position(), e.size());
            let bullet = if depth == 0 { 0.0 } else { 16.0 };
            // Half a pixel in: byte 0 is the TextInput's own left edge.
            return Some(LogicalPosition::new(
                p.x + bullet + app.get_probe_width() + 0.5,
                p.y + size.height / 2.0,
            ));
        }
        let (id, dx, dy) = match at {
            At::Id(id) => (id, None, None),
            At::In(id, x, y) => (id, Some(*x), Some(*y)),
            At::Byte(..) => unreachable!("handled above"),
        };
        let e = find(app, id)?;
        let (p, size) = (e.absolute_position(), e.size());
        Some(LogicalPosition::new(
            p.x + dx.unwrap_or(size.width / 2.0),
            p.y + dy.unwrap_or(size.height / 2.0),
        ))
    }

    fn button(&self, app: &App, down: bool, button: PointerEventButton) {
        let position = self.pointer;
        app.window().dispatch_event(if down {
            WindowEvent::PointerPressed { position, button }
        } else {
            WindowEvent::PointerReleased { position, button }
        });
    }

    /// One presented frame: at most one input per frame, so the frame times are of work the frame really did.
    pub fn tick(&mut self, app: &App) -> Tick {
        self.current.run.frame();
        loop {
            let Some(step) = self.now.take() else {
                return Tick::Done;
            };
            if self.current.failed.is_some() && !matches!(step, Step::End) {
                self.now = self.steps.next();
                continue;
            }
            match step {
                Step::Begin(name) => {
                    self.current = Behaviour {
                        name,
                        ..Default::default()
                    };
                    self.current.run.frame();
                }
                Step::End => self.end(),
                Step::Expect(what, check) => {
                    if check(app) {
                        self.current.worst = self.current.worst.max(self.current.waited);
                        self.current.waited = 0;
                    } else if self.current.waited >= PATIENCE {
                        self.current.failed = Some(what);
                    } else {
                        self.current.waited += 1;
                        self.now = Some(Step::Expect(what, check));
                        return Tick::Idle;
                    }
                }
                Step::Wait(n) => {
                    if self.wait < n {
                        self.wait += 1;
                        self.now = Some(Step::Wait(n));
                        return Tick::Idle;
                    }
                    self.wait = 0;
                }
                Step::Shot(name) => {
                    self.now = self.steps.next();
                    return Tick::Shot(name);
                }
                Step::Move(at) => {
                    self.now = self.steps.next();
                    let Some(position) = self.at(app, &at) else {
                        self.current.failed = Some("its target is not on the page");
                        continue;
                    };
                    self.pointer = position;
                    app.window()
                        .dispatch_event(WindowEvent::PointerMoved { position });
                    return Tick::Idle;
                }
                Step::WheelTo(id) => {
                    let Some(e) = find(app, id) else {
                        // Off screen: no part is found, so wheel on. At an end, where the page stops moving, the other way.
                        let scroll = app.get_scroll();
                        if scroll == self.last_scroll {
                            self.wheel_by = -self.wheel_by;
                        }
                        self.last_scroll = scroll;
                        self.current.waited += 1;
                        if self.current.waited >= PATIENCE {
                            self.current.failed = Some("its target is not on the page");
                            self.now = self.steps.next();
                            continue;
                        }
                        self.now = Some(Step::WheelTo(id));
                        app.window().dispatch_event(WindowEvent::PointerScrolled {
                            position: self.pointer,
                            delta_x: 0.0,
                            delta_y: self.wheel_by,
                        });
                        return Tick::Idle;
                    };
                    self.current.waited = 0;
                    self.now = self.steps.next();
                    // Low on the window: the forced rows a Try-it row is compared with stand above it, and a part off screen cannot be read.
                    let window = app.window();
                    let delta_y = window.size().to_logical(window.scale_factor()).height * 0.7
                        - e.absolute_position().y;
                    app.window().dispatch_event(WindowEvent::PointerScrolled {
                        position: self.pointer,
                        delta_x: 0.0,
                        delta_y,
                    });
                    return Tick::Idle;
                }
                Step::Wheel(y) => {
                    self.now = self.steps.next();
                    app.window().dispatch_event(WindowEvent::PointerScrolled {
                        position: self.pointer,
                        delta_x: 0.0,
                        delta_y: -y,
                    });
                    return Tick::Idle;
                }
                Step::Down | Step::Up | Step::RightDown | Step::RightUp => {
                    self.now = self.steps.next();
                    let button = if matches!(step, Step::Down | Step::Up) {
                        PointerEventButton::Left
                    } else {
                        PointerEventButton::Right
                    };
                    self.button(app, matches!(step, Step::Down | Step::RightDown), button);
                    return Tick::Idle;
                }
                Step::Text(t) => {
                    self.now = self.steps.next();
                    app.window().dispatch_event(WindowEvent::KeyPressed {
                        text: t.clone().into(),
                    });
                    app.window()
                        .dispatch_event(WindowEvent::KeyReleased { text: t.into() });
                    return Tick::Idle;
                }
            }
            self.now = self.steps.next();
        }
    }

    fn end(&mut self) {
        let b = std::mem::take(&mut self.current);
        let fast = b.run.fast_frames();
        self.results.push((b.name, b.failed.is_none()));
        let why = b.failed.map_or("null".to_string(), |w| format!("\"{w}\""));
        self.lines.push(format!(
            "{{\"behaviour\":\"{}\",\"ok\":{},\"why_not\":{why},\"frames\":{},\"fast_frames\":{fast},\"worst_frames_to_show\":{},\"peak_mb\":{:.1},\"frame_ms\":{}}}",
            b.name,
            b.failed.is_none(),
            b.run.frames(),
            b.worst,
            memory().peak_mb,
            b.run.frame_ms_json(),
        ));
    }
}

impl Default for Driver {
    fn default() -> Self {
        Self::new()
    }
}
