//! The behaviours, as data: the window (main.rs) and the no-window test feed the same steps through their own input path.
//! A check reads the live document, never a flag the page sets for the test, so a pass means the DOM really changed.
use blitz_dom::{BaseDocument, Node};
use std::cell::RefCell;
use std::rc::Rc;
use ui_spike_common::meter::{Run, memory};

/// A check that has not held after this many frames (1 s at 120 Hz) fails its behaviour.
const PATIENCE: u32 = 120;

#[derive(Clone)]
pub enum At {
    /// The middle of an element.
    Sel(&'static str),
    /// An offset from an element's top left.
    In(&'static str, f64, f64),
    /// Where Blitz's own hit test puts this byte of an element's text.
    Byte(&'static str, usize),
    /// A fraction of the way between two places.
    Lerp(Box<At>, Box<At>, f64),
}

pub type Check = Box<dyn Fn(&BaseDocument) -> bool>;

pub enum Step {
    Begin(&'static str),
    Move(At),
    Down,
    Up,
    RightDown,
    RightUp,
    Text(String),
    Backspace,
    Wheel(f64),
    /// A wheel that brings this element 300 px under the window's top.
    WheelTo(&'static str),
    /// Holds until the check passes, or fails the behaviour.
    Expect(&'static str, Check),
    /// A picture of the window, once the compositor has the last frame.
    Shot(&'static str),
    /// Frames with no input.
    Wait(u32),
    End,
}

/// What the input path must deliver this frame.
pub enum Input {
    Move(f64, f64),
    Down,
    Up,
    RightDown,
    RightUp,
    Text(String),
    Backspace,
    Wheel(f64),
}

pub enum Tick {
    Input(Input),
    Shot(&'static str),
    Idle,
    Done,
}

fn node<'a>(doc: &'a BaseDocument, sel: &str) -> Option<&'a Node> {
    doc.get_node(doc.query_selector(sel).ok()??)
}

pub fn text(doc: &BaseDocument, sel: &str) -> String {
    node(doc, sel).map_or(String::new(), |n| n.text_content())
}

pub fn exists(doc: &BaseDocument, sel: &str) -> bool {
    node(doc, sel).is_some()
}

pub(crate) fn background(doc: &BaseDocument, sel: &str) -> String {
    node(doc, sel)
        .and_then(|n| n.primary_styles().map(|s| format!("{:?}", s.clone_background_color())))
        .unwrap_or_default()
}

/// Laid out with a size: a node hidden by `display: none` is still in the document, so `exists` cannot say.
pub(crate) fn shown(doc: &BaseDocument, sel: &str) -> bool {
    rect(doc, sel).is_some_and(|r| r.2 > 0.0 && r.3 > 0.0)
}

/// Its computed box-shadow, for comparing two states.
pub(crate) fn shadow(doc: &BaseDocument, sel: &str) -> String {
    node(doc, sel).and_then(|n| n.primary_styles().map(|s| format!("{:?}", s.clone_box_shadow()))).unwrap_or_default()
}

/// A node's computed transform: a running keyframe changes it frame by frame.
pub(crate) fn spin(doc: &BaseDocument, sel: &str) -> String {
    node(doc, sel).and_then(|n| n.primary_styles().map(|s| format!("{:?}", s.clone_transform()))).unwrap_or_default()
}

pub(crate) fn opacity(doc: &BaseDocument, sel: &str) -> Option<f32> {
    node(doc, sel).and_then(|n| n.primary_styles().map(|s| s.clone_opacity()))
}

pub(crate) fn input_value(doc: &BaseDocument, sel: &str) -> String {
    node(doc, sel)
        .and_then(|n| n.element_data())
        .and_then(|e| e.text_input_data())
        .map_or(String::new(), |d| d.editor.raw_text().to_string())
}

/// (left, top, width, height) in CSS px.
pub(crate) fn rect(doc: &BaseDocument, sel: &str) -> Option<(f64, f64, f64, f64)> {
    let n = node(doc, sel)?;
    let p = n.absolute_position(0.0, 0.0);
    let s = n.final_layout().size;
    Some((p.x as f64, p.y as f64, s.width as f64, s.height as f64))
}

fn middle(doc: &BaseDocument, sel: &str) -> Option<(f64, f64)> {
    rect(doc, sel).map(|r| (r.0 + r.2 / 2.0, r.1 + r.3 / 2.0))
}

fn resolve(doc: &BaseDocument, at: &At) -> Option<(f64, f64)> {
    match at {
        At::Sel(sel) => middle(doc, sel),
        At::In(sel, x, y) => rect(doc, sel).map(|r| (r.0 + x, r.1 + y)),
        At::Byte(sel, byte) => {
            let r = rect(doc, sel)?;
            let y = r.1 + r.3 / 2.0;
            (0..=r.2.ceil() as u32).map(|i| r.0 + i as f64).find(|x| doc.find_text_position(*x as f32, y as f32).is_some_and(|p| p.1 == *byte)).map(|x| (x, y))
        }
        At::Lerp(a, b, f) => {
            let (a, b) = (resolve(doc, a)?, resolve(doc, b)?);
            Some((a.0 + (b.0 - a.0) * f, a.1 + (b.1 - a.1) * f))
        }
    }
}

pub(crate) fn click(at: At) -> Vec<Step> {
    vec![Step::Move(at), Step::Down, Step::Up]
}

pub(crate) fn expect_text(what: &'static str, sel: &'static str, want: String) -> Step {
    Step::Expect(what, Box::new(move |d| text(d, sel) == want))
}

/// Presses, carries over 24 frames, releases.
pub(crate) fn drag(from: At, to: At, shot: Option<&'static str>) -> Vec<Step> {
    let mut steps = vec![Step::Move(from.clone()), Step::Down];
    for i in 1..=24 {
        steps.push(Step::Move(At::Lerp(Box::new(from.clone()), Box::new(to.clone()), i as f64 / 24.0)));
    }
    steps.push(Step::Wait(2));
    if let Some(name) = shot {
        steps.push(Step::Shot(name));
    }
    steps.push(Step::Up);
    steps
}

pub(crate) fn typed(chars: &str) -> Vec<Step> {
    chars.chars().map(|c| Step::Text(c.to_string())).collect()
}

/// Its own behaviour: a failed nav is named, and never swallows the next one's Begin.
pub(crate) fn nav(name: &'static str, id: &'static str, thing: &'static str) -> Vec<Step> {
    let mut s = vec![Step::Begin(name)];
    s.extend(click(At::Sel(id)));
    s.push(Step::Expect("the screen opens", Box::new(move |d| exists(d, thing))));
    s.push(Step::Wait(2));
    s.push(Step::End);
    s
}

/// N rounds of: on, its colour changes, off, its colour comes back. `also` must hold while on.
fn hover(sel: &'static str, rest: At, shot: &'static str, also: Rc<dyn Fn(&BaseDocument) -> bool>) -> Vec<Step> {
    let resting = Rc::new(RefCell::new(String::new()));
    let keep = resting.clone();
    let mut s = vec![
        Step::Move(rest.clone()),
        Step::Wait(2),
        Step::Expect("it has a colour", Box::new(move |d| {
            *keep.borrow_mut() = background(d, sel);
            !keep.borrow().is_empty()
        })),
    ];
    for round in 1..=20 {
        s.push(Step::Move(At::Sel(sel)));
        let (r, also) = (resting.clone(), also.clone());
        s.push(Step::Expect("the :hover style", Box::new(move |d| background(d, sel) != *r.borrow() && also(d))));
        if round == 20 {
            s.push(Step::Wait(2));
            s.push(Step::Shot(shot));
        }
        s.push(Step::Move(rest.clone()));
        let r = resting.clone();
        s.push(Step::Expect("hover ends", Box::new(move |d| background(d, sel) == *r.borrow())));
    }
    s
}

fn shape_middle(id: &str) -> (f64, f64) {
    let (shapes, _) = crate::scene();
    let s = shapes.iter().find(|s| s.id == id).expect("the shape is in the scene");
    (s.x + s.w / 2.0, s.y + s.h / 2.0)
}

fn board(s: &mut Vec<Step>) {
    use Step::*;
    s.extend(nav("board: opens", "#nav-board", "#card-101"));

    s.push(Begin("board: hover"));
    s.extend(hover("#card-101", At::Sel(".bar-title"), "board-hover", Rc::new(|_| true)));
    s.push(End);

    s.push(Begin("board: click, typing, dialog"));
    // A real hand moves a pixel between press and release: still a click, and nothing moves.
    s.push(Move(At::In("#card-101", 20.0, 20.0)));
    s.push(Down);
    s.push(Move(At::In("#card-101", 21.0, 20.0)));
    s.push(Up);
    s.push(Expect("a click with a pixel of jitter still opens", Box::new(|d| exists(d, "#dialog"))));
    s.push(expect_text("and moves nothing", "#drops", "moved: 0".into()));
    s.extend(click(At::Sel("#issue-cancel")));
    s.push(Expect("it closes", Box::new(|d| !exists(d, "#dialog"))));
    let mut title = "Trace the window's memory".to_string();
    for round in 1..=8 {
        s.extend(click(At::Sel("#card-101")));
        s.push(Expect("a click opens its dialog", Box::new(|d| exists(d, "#dialog"))));
        s.extend(click(At::Sel("#issue-title-input")));
        s.extend(typed("!"));
        title.push('!');
        let want = title.clone();
        s.push(Expect("the key reaches the input", Box::new(move |d| input_value(d, "#issue-title-input") == want)));
        if round == 8 {
            s.push(Wait(40));
            s.push(Shot("board-dialog"));
        }
        s.extend(click(At::Sel("#issue-save")));
        s.push(Expect("it closes", Box::new(|d| !exists(d, "#dialog"))));
        let want = title.clone();
        s.push(Expect("the card shows the new title", Box::new(move |d| text(d, "#card-101").contains(&want))));
    }
    // Behind the backdrop: nothing fires, and the click closes it.
    s.extend(click(At::Sel("#card-101")));
    s.push(Expect("a click opens its dialog", Box::new(|d| exists(d, "#dialog"))));
    s.extend(click(At::Sel("#nav-mail")));
    s.push(Expect("the backdrop takes the click", Box::new(|d| !exists(d, "#dialog") && exists(d, "#card-101"))));
    // The bar too: its z-index, against Blitz's unclipped hit test, once lifted it above the backdrop.
    s.extend(click(At::Sel("#card-101")));
    s.push(Expect("a click opens its dialog", Box::new(|d| exists(d, "#dialog"))));
    s.extend(click(At::Sel(".bar-title")));
    s.push(Expect("the backdrop covers the bar", Box::new(|d| !exists(d, "#dialog"))));
    s.push(End);

    s.push(Begin("board: dialog animation"));
    s.extend(click(At::Sel("#card-102")));
    s.push(Expect("opacity between 0 and 1 on the way in", Box::new(|d| opacity(d, "#dialog").is_some_and(|o| o > 0.0 && o < 1.0))));
    s.push(Expect("past a third of the way", Box::new(|d| opacity(d, "#dialog").is_some_and(|o| o > 0.35))));
    s.push(Shot("board-dialog-animating"));
    s.push(Expect("opacity reaches 1", Box::new(|d| opacity(d, "#dialog") == Some(1.0))));
    s.extend(click(At::Sel("#issue-cancel")));
    s.push(Expect("it closes", Box::new(|d| !exists(d, "#dialog"))));
    s.push(End);

    s.push(Begin("board: drag and drop"));
    let in_progress = |d: &BaseDocument| text(d, "#col-2").contains("AAA-105");
    for round in 1..=10 {
        s.extend(drag(At::Sel("#card-105"), At::Sel("#col-2"), (round == 10).then_some("board-drag")));
        s.push(Expect("the card lands in In Progress", Box::new(in_progress)));
        s.extend(drag(At::Sel("#card-105"), At::Sel("#col-1"), None));
        s.push(Expect("and comes back", Box::new(move |d| !in_progress(d) && text(d, "#col-1").contains("AAA-105"))));
        s.push(expect_text("each drop counts", "#drops", format!("moved: {}", round * 2)));
    }
    // A sloppy click: past the drag threshold, released on the card it pressed. Nothing moves, no dialog opens.
    s.extend(drag(At::In("#card-104", 20.0, 20.0), At::In("#card-104", 30.0, 20.0), None));
    s.push(Wait(2));
    s.push(expect_text("a release on its own card moves nothing", "#drops", "moved: 20".into()));
    s.push(Expect(
        "and it keeps its place",
        Box::new(|d| {
            let col = text(d, "#col-1");
            col.find("AAA-104") < col.find("AAA-105") && !exists(d, "#dialog")
        }),
    ));
    s.push(Wait(2));
    s.push(Shot("board"));
    s.push(End);

    s.push(Begin("board: html5 drag"));
    s.extend(drag(At::Sel("#html5"), At::In("#html5", 60.0, 60.0), None));
    s.push(expect_text("ondragstart fires", "#html5", "HTML5 drag starts: 1".into()));
    s.push(End);

    s.push(Begin("board: dark theme"));
    let resting = Rc::new(RefCell::new(String::new()));
    let keep = resting.clone();
    s.push(Expect("the card has a colour", Box::new(move |d| {
        *keep.borrow_mut() = background(d, "#card-103");
        !keep.borrow().is_empty()
    })));
    s.extend(click(At::Sel("#theme-toggle")));
    let r = resting.clone();
    s.push(Expect("design.json's dark colours", Box::new(move |d| background(d, "#card-103") != *r.borrow())));
    s.push(Wait(2));
    s.push(Shot("board-dark"));
    s.extend(click(At::Sel("#theme-toggle")));
    s.push(Expect("and back", Box::new(move |d| background(d, "#card-103") == *resting.borrow())));
    s.push(End);
}

fn diagram(s: &mut Vec<Step>) {
    use Step::*;
    s.extend(nav("diagram: opens", "#nav-diagram", "#canvas"));
    let (shapes, _) = crate::scene();
    let on = |id: &str| {
        let (x, y) = shape_middle(id);
        At::In("#canvas", x, y)
    };

    s.push(Begin("diagram: hover"));
    for round in 0..40 {
        let shape = &shapes[round % shapes.len()];
        s.push(Move(on(&shape.id)));
        s.push(expect_text("the shape under the pointer", "#diagram-hover", format!("under the pointer: {}", shape.label)));
        if round == 39 {
            s.push(Wait(2));
            s.push(Shot("diagram-hover"));
        }
    }
    s.push(End);

    s.push(Begin("diagram: click"));
    for round in 0..40 {
        let shape = &shapes[round % shapes.len()];
        s.extend(click(on(&shape.id)));
        let want = format!("selected: {} at {:.0},{:.0}", shape.label, shape.x, shape.y);
        s.push(expect_text("the shape under the click", "#diagram-status", want));
    }
    s.push(End);

    s.push(Begin("diagram: drag a shape"));
    let shape = &shapes[0];
    let (x, y) = shape_middle(&shape.id);
    let (home, away) = (At::In("#canvas", x, y), At::In("#canvas", x + 80.0, y + 120.0));
    for round in 0..10 {
        s.extend(drag(home.clone(), away.clone(), (round == 9).then_some("diagram-drag")));
        let want = format!("selected: {} at {:.0},{:.0}", shape.label, shape.x + 80.0, shape.y + 120.0);
        s.push(expect_text("it lands where it was dropped", "#diagram-status", want));
        s.extend(drag(away.clone(), home.clone(), None));
        let want = format!("selected: {} at {:.0},{:.0}", shape.label, shape.x, shape.y);
        s.push(expect_text("and goes back", "#diagram-status", want));
    }
    s.push(Wait(2));
    s.push(Shot("diagram"));
    s.push(End);

    s.push(Begin("diagram: typing, dialog"));
    let mut label = shape.label.clone();
    for round in 1..=8 {
        s.extend(click(home.clone()));
        s.extend(click(At::Sel("#rename")));
        s.push(Expect("the dialog opens", Box::new(|d| exists(d, "#dialog"))));
        s.extend(click(At::Sel("#label-input")));
        s.extend(typed("!"));
        label.push('!');
        let want = label.clone();
        s.push(Expect("the key reaches the input", Box::new(move |d| input_value(d, "#label-input") == want)));
        if round == 8 {
            s.push(Wait(40));
            s.push(Shot("diagram-dialog"));
        }
        s.extend(click(At::Sel("#label-save")));
        let want = label.clone();
        s.push(Expect("the label changes", Box::new(move |d| !exists(d, "#dialog") && text(d, "#canvas").contains(&want))));
    }
    s.push(End);

    s.push(Begin("diagram: a shape's own onclick"));
    s.extend(click(home.clone()));
    s.push(Expect("a path's own onclick fires", Box::new(|d| !text(d, "#inner-clicks").ends_with(": 0"))));
    s.push(End);
}

fn mail(s: &mut Vec<Step>) {
    use Step::*;
    s.extend(nav("mail: opens", "#nav-mail", "#list"));

    s.push(Begin("mail: hover"));
    s.extend(hover("#line-1", At::Sel(".bar-title"), "mail-hover", Rc::new(|d| {
        // Its actions appear, its date goes: the rule on .line:hover reached the children.
        rect(d, "#line-1 .actions").is_some_and(|r| r.2 > 0.0)
    })));
    s.push(End);

    s.push(Begin("mail: click"));
    for round in 1..=30usize {
        s.extend(click(At::Sel("#star-1")));
        let stars = round % 2;
        s.extend(click(At::Sel("#tick-2")));
        s.push(expect_text("star and tick", "#mail-state", format!("{stars} starred, {stars} ticked, 0 sent")));
        if round == 29 {
            s.push(Wait(2));
            s.push(Shot("mail-click"));
        }
    }
    s.push(End);

    s.push(Begin("mail: typing"));
    s.extend(click(At::Sel("#mail-search")));
    s.extend(typed("Row 4999"));
    s.push(expect_text("the search filters 100,000 rows", "#mail-count", "11 lines".into()));
    s.push(Wait(2));
    s.push(Shot("mail-search"));
    for _ in 0.."Row 4999".len() {
        s.push(Backspace);
    }
    s.push(expect_text("clearing it brings them back", "#mail-count", "100000 lines".into()));
    s.push(End);

    s.push(Begin("mail: dialog"));
    for round in 1..=8 {
        s.extend(click(At::Sel("#compose")));
        s.push(Expect("it opens", Box::new(|d| exists(d, "#dialog"))));
        s.extend(click(At::Sel("#draft-input")));
        s.extend(typed("hello"));
        s.push(expect_text("the draft fills", "#draft-echo", "5 characters".into()));
        if round == 8 {
            s.push(Wait(40));
            s.push(Shot("mail-dialog"));
        }
        s.extend(click(At::Sel("#send")));
        s.push(Expect("it closes", Box::new(|d| !exists(d, "#dialog"))));
        s.push(Expect("it counts", Box::new(move |d| text(d, "#mail-state").ends_with(&format!("{round} sent")))));
    }
    s.push(End);

    s.push(Begin("mail: wheel"));
    s.push(Move(At::Sel("#list")));
    for _ in 0..60 {
        s.push(Wheel(700.0));
    }
    s.push(Expect("the lines follow the wheel", Box::new(|_| crate::scroll_top() >= 60.0 * 700.0 && crate::rows_caught_up())));
    s.push(Wait(2));
    s.push(Shot("mail"));
    s.push(End);

    // Blitz's hit test ignores overflow clipping: without the bar's z-index in page.css, a row scrolled half out of the list
    // takes this click where it is clipped, and the search box never gets the focus.
    s.push(Begin("mail: search while scrolled"));
    s.extend(click(At::Sel("#mail-search")));
    s.extend(typed("Row 4999"));
    s.push(Expect("the matching lines are drawn", Box::new(|d| text(d, "#mail-count") == "11 lines" && exists(d, "#line-4999"))));
    for _ in 0.."Row 4999".len() {
        s.push(Backspace);
    }
    s.push(expect_text("clearing it brings them back", "#mail-count", "100000 lines".into()));
    s.push(Expect("from the top", Box::new(|d| exists(d, "#line-0"))));
    s.push(End);
}

fn comments(s: &mut Vec<Step>) {
    use Step::*;
    s.extend(nav("comments: opens", "#nav-comments", "#block-1"));

    s.push(Begin("comments: hover"));
    s.extend(hover("#block-1", At::Sel(".bar-title"), "comments-hover", Rc::new(|d| {
        rect(d, "#add-1").is_some_and(|r| r.2 > 0.0)
    })));
    s.push(End);

    s.push(Begin("comments: click, typing"));
    for round in 1..=8 {
        s.push(Move(At::Sel("#block-1")));
        s.push(Wait(2));
        s.extend(click(At::Sel("#add-1")));
        s.push(Expect("the composer opens", Box::new(|d| exists(d, "#comment-input"))));
        s.extend(click(At::Sel("#comment-input")));
        s.extend(typed("Agreed"));
        s.push(Expect("the key reaches the input", Box::new(|d| input_value(d, "#comment-input") == "Agreed")));
        if round == 8 {
            s.push(Wait(2));
            s.push(Shot("comments-typing"));
        }
        s.extend(click(At::Sel("#comment-post")));
        let want = format!("{} open, 0 resolved", round + 1);
        s.push(expect_text("the thread lands in the rail", "#thread-count", want));
    }
    s.push(Wait(2));
    s.push(Shot("comments"));
    s.push(End);

    s.push(Begin("comments: dialog"));
    for round in 1..=8u32 {
        let (open, id): (&'static str, _) = (Box::leak(format!("#resolve-{round}").into_boxed_str()), round);
        s.extend(click(At::Sel(open)));
        s.push(Expect("it opens", Box::new(|d| exists(d, "#dialog"))));
        if round == 8 {
            s.push(Wait(40));
            s.push(Shot("comments-dialog"));
        }
        s.extend(click(At::Sel("#resolve-yes")));
        let want = format!("{} open, {id} resolved", 9 - id);
        s.push(expect_text("the thread leaves the rail", "#thread-count", want));
    }
    s.push(End);
}

/// Each screen's behaviours, in as many rounds as give its frame times more than a handful.
pub fn script() -> Vec<Step> {
    let mut s = vec![];
    board(&mut s);
    diagram(&mut s);
    mail(&mut s);
    comments(&mut s);
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

/// The two the window fails in every run, recorded in section 9: if Blitz gains either, a run says so and the page can use it.
pub const RECORDED_FAILURES: [&str; 2] = ["board: html5 drag", "diagram: a shape's own onclick"];

pub struct Driver {
    steps: std::vec::IntoIter<Step>,
    now: Option<Step>,
    wait: u32,
    current: Behaviour,
    /// Each behaviour's JSON line.
    pub lines: Vec<String>,
    /// (name, passed)
    pub results: Vec<(&'static str, bool)>,
    recorded: &'static [&'static str],
}

impl Driver {
    pub fn failed(&self) -> Vec<&'static str> {
        self.results.iter().filter(|r| !r.1).map(|r| r.0).collect()
    }

    pub fn new() -> Self {
        Self::with(script(), &RECORDED_FAILURES)
    }

    /// The design system page's script.
    pub fn design() -> Self {
        Self::with(crate::design::script::script(), &crate::design::script::RECORDED_FAILURES)
    }

    fn with(steps: Vec<Step>, recorded: &'static [&'static str]) -> Self {
        let mut steps = steps.into_iter();
        let now = steps.next();
        Self {
            steps,
            now,
            wait: 0,
            current: Behaviour::default(),
            lines: vec![],
            results: vec![],
            recorded,
        }
    }

    /// True when just the recorded behaviours failed: a run that fails more, or fewer, says so.
    pub fn as_recorded(&self) -> bool {
        self.failed() == self.recorded
    }

    /// One presented frame: at most one input per frame, so the frame times are of work the frame really did.
    pub fn tick(&mut self, doc: &BaseDocument) -> Tick {
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
                    self.current = Behaviour { name, ..Default::default() };
                    self.current.run.frame();
                }
                Step::End => self.end(),
                Step::Expect(what, check) => {
                    if check(doc) {
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
                Step::WheelTo(sel) => {
                    self.now = self.steps.next();
                    let Some(r) = rect(doc, sel) else {
                        self.current.failed = Some("its target is not on the page");
                        continue;
                    };
                    return Tick::Input(Input::Wheel(r.1 - 300.0));
                }
                Step::Move(at) => {
                    self.now = self.steps.next();
                    let Some((x, y)) = resolve(doc, &at) else {
                        self.current.failed = Some("its target is not on the page");
                        continue;
                    };
                    return Tick::Input(Input::Move(x, y));
                }
                input => {
                    self.now = self.steps.next();
                    return Tick::Input(match input {
                        Step::Down => Input::Down,
                        Step::Up => Input::Up,
                        Step::RightDown => Input::RightDown,
                        Step::RightUp => Input::RightUp,
                        Step::Text(t) => Input::Text(t),
                        Step::Backspace => Input::Backspace,
                        Step::Wheel(y) => Input::Wheel(y),
                        _ => unreachable!("handled above"),
                    });
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
