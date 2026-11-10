//! The design page's behaviours, as script.rs's: each family opens, a state forced by a class draws what the real
//! pseudo-class draws, and the Linear and Notion screens work. Each picture beside its evidence is a shot here.
use super::FAMILIES;
use crate::script::{At, Step, background, click, drag, exists, shadow, shown, text, typed};
use std::cell::RefCell;
use std::rc::Rc;

/// None since the page's root focuses a clicked button.
pub const RECORDED_FAILURES: [&str; 0] = [];

const FAMILY_TABS: [(&str, &str, &str); 6] = [
    ("family: layout", "#f-layout", "family-layout"),
    ("family: data", "#f-data", "family-data"),
    ("family: forms", "#f-forms", "family-forms"),
    ("family: feedback", "#f-feedback", "family-feedback"),
    ("family: mail", "#f-mail", "family-mail"),
    ("family: board", "#f-board", "family-board"),
];

fn shots_down(name: &'static str, second: &'static str) -> Vec<Step> {
    let mut s = vec![
        Step::Wait(2),
        Step::Shot(name),
        Step::Move(At::Sel("#ds-main")),
        Step::Wheel(700.0),
        Step::Wait(4),
        Step::Shot(second),
    ];
    // Through the whole family, as slint-design's script must: a memory figure is of what was drawn, so both draw the same.
    for _ in 0..18 {
        s.extend([Step::Wheel(500.0), Step::Wait(1)]);
    }
    s
}

fn width(d: &blitz_dom::BaseDocument, sel: &str) -> f64 {
    crate::script::rect(d, sel).map_or(0.0, |r| r.2)
}

/// Holds the colour of a selector so a later step compares against it.
fn remember(what: &'static str, sel: &'static str, into: Rc<RefCell<String>>) -> Step {
    Step::Expect(
        what,
        Box::new(move |d| {
            *into.borrow_mut() = background(d, sel);
            !into.borrow().is_empty()
        }),
    )
}

/// The real pseudo-class on the real control draws what the forced class beside it draws.
fn same_as_forced(what: &'static str, sel: &'static str, forced: &'static str) -> Step {
    Step::Expect(
        what,
        Box::new(move |d| {
            let (a, b) = (background(d, sel), background(d, forced));
            !a.is_empty() && a == b
        }),
    )
}

/// Two missing nodes read the same empty string: that is no match.
fn same_shadow(d: &blitz_dom::BaseDocument, a: &str, b: &str) -> bool {
    let (a, b) = (shadow(d, a), shadow(d, b));
    !a.is_empty() && a == b
}

/// Past its start and short of its place, on the side a drop left it: it starts the dropped card's height and a gap away, and
/// cards here are one height.
fn part_way(d: &blitz_dom::BaseDocument, sel: &str, side: f64) -> bool {
    let (Some(y), Some(card)) = (
        crate::script::shifted_y(d, sel),
        crate::script::rect(d, sel),
    ) else {
        return false;
    };
    (y * side) > 1.0 && (y * side) < card.3
}

const LINE: &str = "the window and the web draw the same components";
const PHRASE: &str = "the same components";
const SECOND: &str = "the window";

/// A press at one end of the phrase, a drag to the other, Blitz naming those words while the button is held, then the release.
fn picking(phrase: &'static str, backwards: bool, shot: Option<&'static str>) -> Vec<Step> {
    let start = LINE.find(phrase).expect("the phrase is in the line");
    let (first, last) = (
        At::Byte("#no-text-2", start),
        At::Byte("#no-text-2", start + phrase.len()),
    );
    let mut s = if backwards {
        drag(last, first, shot)
    } else {
        drag(first, last, shot)
    };
    let up = s.pop().expect("its release");
    s.push(Step::Expect(
        "Blitz names the words",
        Box::new(move |d| d.get_selected_text().as_deref() == Some(phrase)),
    ));
    s.push(up);
    s
}

/// The marked runs of the line, in its order.
fn marks(d: &blitz_dom::BaseDocument) -> Vec<String> {
    d.query_selector_all("#no-text-2 .no-mark")
        .map_or(vec![], |all| {
            all.iter()
                .filter_map(|id| d.get_node(*id))
                .map(|n| n.text_content())
                .collect()
        })
}

pub fn script() -> Vec<Step> {
    let mut s = vec![];
    for (name, tab, shot) in FAMILY_TABS {
        s.push(Step::Begin(name));
        s.extend(click(At::Sel(tab)));
        s.push(Step::Expect(
            "its sections are drawn",
            Box::new(move |d| {
                let slug = tab.trim_start_matches("#f-");
                let (_, _, _, sections) = FAMILIES.iter().find(|f| f.0 == slug).expect("a family");
                sections
                    .iter()
                    .all(|(id, _)| exists(d, &format!("#s-{id}")))
                    && exists(d, ".app.dark.ds-cell")
                    && exists(d, ".app.light.ds-cell")
            }),
        ));
        s.extend(shots_down(
            shot,
            Box::leak(format!("{shot}-2").into_boxed_str()),
        ));
        s.push(Step::End);
    }

    // By width, so a face that falls back fails: mono's i and W take one width, the hand's words are not the sans's.
    s.push(Step::Begin("fonts: three faces in one window"));
    s.extend(click(At::Sel("#f-layout")));
    s.push(Step::Expect(
        "a mono i is as wide as a mono W",
        Box::new(|d| {
            width(d, ".ds-narrow") > 0.0 && width(d, ".ds-narrow") == width(d, ".ds-wide")
        }),
    ));
    s.push(Step::Expect(
        "the hand draws other words than the sans",
        Box::new(|d| {
            width(d, ".ds-note") > 0.0
                && width(d, ".ds-sans") > 0.0
                && width(d, ".ds-note") != width(d, ".ds-sans")
        }),
    ));
    s.push(Step::WheelTo(".ds-note"));
    s.push(Step::Wait(4));
    s.push(Step::Shot("fonts"));
    s.push(Step::End);

    // Read from its picture: the window's renderer is not the one css_probe paints with, and the comparison is too coarse for a row.
    s.push(Step::Begin("css: a blur in the window"));
    s.push(Step::Expect(
        "the row is drawn",
        Box::new(|d| width(d, ".ds-blur") > 0.0),
    ));
    s.push(Step::WheelTo(".ds-blur"));
    s.push(Step::Wait(4));
    s.push(Step::Shot("css-blur"));
    s.push(Step::End);

    // The forms page, the first rows of Actions: the real states against the forced ones. Try-it rows are low on the page.
    s.push(Step::Begin(
        "forms: real :hover, :active and :focus draw the forced states",
    ));
    s.extend(click(At::Sel("#f-forms")));
    s.push(Step::Move(At::Sel("#ds-main")));
    s.push(Step::Wheel(320.0));
    s.push(Step::Wait(4));
    const TRY: &str = ".app.light .try .button";
    const FORCED_HOVER: &str = ".app.light .button.is-hover";
    const FORCED_PRESSED: &str = ".app.light .button.is-active";
    s.push(Step::Move(At::Sel(TRY)));
    s.push(same_as_forced(
        "the real :hover is the forced one",
        TRY,
        FORCED_HOVER,
    ));
    s.push(Step::Down);
    s.push(same_as_forced(
        "the real :active is the forced one",
        TRY,
        FORCED_PRESSED,
    ));
    s.push(Step::Shot("state-pressed"));
    s.push(Step::Up);
    s.push(Step::End);

    s.push(Step::Begin("forms: a click focuses a button"));
    s.extend(click(At::Sel(TRY)));
    s.push(Step::Expect(
        "it is the document's focus",
        Box::new(|d| crate::script::focused(d, TRY)),
    ));
    s.push(Step::Expect(
        "the real :focus rings it as the forced one",
        Box::new(|d| same_shadow(d, TRY, ".app.light .button.is-focus")),
    ));
    s.extend(click(At::Sel(".ds-title")));
    s.push(Step::Expect(
        "a press off it blurs",
        Box::new(|d| {
            !crate::script::focused(d, TRY) && !same_shadow(d, TRY, ".app.light .button.is-focus")
        }),
    ));
    s.push(Step::End);

    s.push(Step::Begin("forms: a click focuses a field"));
    s.push(Step::WheelTo(".app.light .try-field .input"));
    s.push(Step::Wait(4));
    s.extend(click(At::Sel(".app.light .try-field .input")));
    s.push(Step::Expect(
        "the real :focus rings it as the forced one",
        Box::new(|d| {
            same_shadow(
                d,
                ".app.light .try-field .input",
                ".app.light .input.is-focus",
            )
        }),
    ));
    s.push(Step::Wait(2));
    s.push(Step::Shot("state-focus"));
    s.extend(typed("1"));
    s.push(Step::Expect(
        "and it takes a key",
        Box::new(|d| crate::script::input_value(d, ".app.light .try-field .input") == "6501"),
    ));
    s.push(Step::End);

    // Each frame's colour on the way: an eased hover passes through a colour that is neither end.
    s.push(Step::Begin("forms: a transition eases"));
    s.push(Step::WheelTo(TRY));
    s.push(Step::Wait(4));
    s.push(Step::Move(At::In("#ds-main", 4.0, 4.0)));
    s.push(Step::Wait(30));
    let rest = Rc::new(RefCell::new(String::new()));
    s.push(remember("it has a colour", TRY, rest.clone()));
    let between = Rc::new(RefCell::new(false));
    s.push(Step::Move(At::Sel(TRY)));
    for _ in 0..12 {
        let (r, b) = (rest.clone(), between.clone());
        s.push(Step::Expect(
            "colour read",
            Box::new(move |d| {
                let now = background(d, TRY);
                if now != *r.borrow() && now != background(d, FORCED_HOVER) {
                    *b.borrow_mut() = true;
                }
                true
            }),
        ));
        s.push(Step::Wait(1));
    }
    s.push(Step::Expect(
        "a colour between the two ends",
        Box::new(move |_| *between.borrow()),
    ));
    s.push(Step::End);

    // The keyframe's frames: the angle keeps moving.
    s.push(Step::Begin("forms: a keyframe turns"));
    s.push(Step::WheelTo(".app.light .ds-spin"));
    s.push(Step::Wait(4));
    let seen = Rc::new(RefCell::new(std::collections::BTreeSet::new()));
    for _ in 0..12 {
        let keep = seen.clone();
        s.push(Step::Expect(
            "angle read",
            Box::new(move |d| {
                keep.borrow_mut()
                    .insert(crate::script::spin(d, ".app.light .ds-spin"));
                true
            }),
        ));
        s.push(Step::Wait(1));
    }
    s.push(Step::Expect(
        "the angle moved on the way",
        Box::new(move |_| seen.borrow().len() >= 6),
    ));
    s.push(Step::End);

    s.push(Step::Begin("theme: the page's toggle"));
    let light = Rc::new(RefCell::new(String::new()));
    s.push(remember("it has a colour", ".ds-root", light.clone()));
    s.extend(click(At::Sel("#ds-theme")));
    s.push(Step::Expect(
        "the root's colour changes",
        Box::new(move |d| {
            background(d, ".ds-root") != *light.borrow() && exists(d, ".app.dark.ds-root")
        }),
    ));
    s.push(Step::Wait(2));
    s.push(Step::Shot("family-forms-dark"));
    s.extend(click(At::Sel("#ds-theme")));
    s.push(Step::Expect(
        "and back",
        Box::new(|d| exists(d, ".app.light.ds-root")),
    ));
    s.push(Step::End);

    s.push(Step::Begin("linear: the board"));
    s.extend(click(At::Sel("#v-linear")));
    s.push(Step::Expect(
        "the cards are drawn",
        Box::new(|d| exists(d, "#li-card-478") && exists(d, "#li-card-457")),
    ));
    s.push(Step::Wait(2));
    s.push(Step::Move(At::In(".li-side", 20.0, 700.0)));
    s.push(Step::Wait(2));
    s.push(Step::Shot("linear-board"));
    s.push(Step::End);

    s.push(Step::Begin("linear: hover and drag"));
    let resting = Rc::new(RefCell::new(String::new()));
    let keep = resting.clone();
    s.push(Step::Expect(
        "it has a shadow",
        Box::new(move |d| {
            *keep.borrow_mut() = shadow(d, "#li-card-476");
            !keep.borrow().is_empty()
        }),
    ));
    s.push(Step::Move(At::Sel("#li-card-476")));
    s.push(Step::Expect(
        "the shadow rises",
        Box::new(move |d| shadow(d, "#li-card-476") != *resting.borrow()),
    ));
    s.push(Step::Wait(2));
    s.push(Step::Shot("linear-hover"));
    s.extend(drag(
        At::Sel("#li-card-476"),
        At::Sel("#li-col-1"),
        Some("linear-drag"),
    ));
    s.push(Step::Expect(
        "the card moved",
        Box::new(|d| text(d, "#li-count-0") == "4" && text(d, "#li-count-1") == "6"),
    ));
    s.push(Step::End);

    // The ghost's rect on each frame: it passes through places that are neither where it was let go nor its slot. 476 was dropped
    // first in its column, so it is 478's new neighbour; 473 sat under 478's old place.
    s.push(Step::Begin("linear: a card settles after a drop"));
    let mut carry = drag(
        At::Sel("#li-card-478"),
        At::In("#li-col-1", 180.0, 300.0),
        None,
    );
    let up = carry.pop().expect("its release");
    s.extend(carry);
    let path = Rc::new(RefCell::new(vec![]));
    let keep = path.clone();
    s.push(Step::Expect(
        "the card is carried",
        Box::new(move |d| {
            keep.borrow_mut()
                .extend(crate::script::rect(d, "#li-ghost"));
            !keep.borrow().is_empty()
        }),
    ));
    s.push(up);
    // Held until true, so the picture is of the way there, never of either end. Wide, as a slow frame can pass a narrow range for good.
    // Each frame's places are kept from here: a picture costs frames, and the way after it can be short.
    let eased = Rc::new(RefCell::new((vec![], vec![])));
    let (keep, seen) = (path.clone(), eased.clone());
    s.push(Step::Expect(
        "the picture is part-way",
        Box::new(move |d| {
            seen.borrow_mut()
                .0
                .extend(crate::script::shifted_y(d, "#li-card-473"));
            seen.borrow_mut()
                .1
                .extend(crate::script::shifted_y(d, "#li-card-476"));
            let (Some(ghost), Some(slot)) = (
                crate::script::rect(d, "#li-ghost"),
                crate::script::rect(d, "#li-card-478"),
            ) else {
                return false;
            };
            keep.borrow_mut().push(ghost);
            let from = keep.borrow()[0];
            let done = 1.0
                - (ghost.0 - slot.0).hypot(ghost.1 - slot.1)
                    / (from.0 - slot.0).hypot(from.1 - slot.1);
            (0.1..0.95).contains(&done)
                && crate::script::opacity(d, "#li-card-478") == Some(0.0)
                && part_way(d, "#li-card-473", 1.0)
                && part_way(d, "#li-card-476", -1.0)
        }),
    ));
    s.push(Step::Shot("linear-settling"));
    for _ in 0..30 {
        let (keep, seen) = (path.clone(), eased.clone());
        s.push(Step::Expect(
            "position read",
            Box::new(move |d| {
                keep.borrow_mut()
                    .extend(crate::script::rect(d, "#li-ghost"));
                seen.borrow_mut()
                    .0
                    .extend(crate::script::shifted_y(d, "#li-card-473"));
                seen.borrow_mut()
                    .1
                    .extend(crate::script::shifted_y(d, "#li-card-476"));
                true
            }),
        ));
        s.push(Step::Wait(1));
    }
    // A jump has two places, where it was and its slot; the part-way frame above is a third.
    s.push(Step::Expect(
        "each neighbour eases to its place, only ever closer",
        Box::new(move |_| {
            let seen = eased.borrow();
            [&seen.0, &seen.1].iter().all(|ys| {
                let mut places: Vec<i32> = ys.iter().map(|y: &f64| y.abs() as i32).collect();
                places.dedup();
                places.len() >= 3 && places.is_sorted_by(|a, b| a >= b) && places.last() == Some(&0)
            })
        }),
    ));
    s.push(Step::Expect(
        "it passed through 5 places on the way to its slot",
        Box::new(move |_| {
            // The first is where it was let go.
            let mut places: Vec<_> = path
                .borrow()
                .iter()
                .skip(1)
                .map(|r| (r.0 as i32, r.1 as i32))
                .collect();
            places.dedup();
            places.len() >= 5
        }),
    ));
    s.push(Step::Expect(
        "it lands on the card's own place",
        Box::new(|d| {
            let (Some(ghost), Some(slot)) = (
                crate::script::rect(d, "#li-ghost"),
                crate::script::rect(d, "#li-card-478"),
            ) else {
                return false;
            };
            (ghost.0 - slot.0).abs() <= 1.0 && (ghost.1 - slot.1).abs() <= 1.0
        }),
    ));
    s.push(Step::Expect(
        "then the ghost hides, the card in its place",
        Box::new(|d| {
            crate::script::opacity(d, "#li-ghost") == Some(0.0)
                && crate::script::opacity(d, "#li-card-478") == Some(1.0)
        }),
    ));
    s.push(Step::End);

    s.push(Step::Begin("linear: its dark theme"));
    s.extend(click(At::Sel("#ds-theme")));
    s.push(Step::Expect(
        "dark",
        Box::new(|d| exists(d, ".app.dark.ds-root")),
    ));
    s.push(Step::Wait(2));
    s.push(Step::Shot("linear-dark"));
    s.push(Step::End);

    // Of the words of #no-line-2. Each drag's ends come from Blitz's hit test, so they land on these bytes in any font.
    s.push(Step::Begin("notion: a drag picks « the same components »"));
    s.extend(click(At::Sel("#v-notion")));
    s.push(Step::Expect(
        "the page",
        Box::new(|d| text(d, "#no-text-2") == LINE),
    ));
    s.extend(picking(PHRASE, false, Some("notion-drag")));
    s.push(Step::Expect(
        "the toolbar opens, the pick on those words",
        Box::new(|d| {
            exists(d, "#no-tool") && text(d, "#no-text-2 .no-pick") == PHRASE && marks(d).is_empty()
        }),
    ));
    s.push(Step::Wait(2));
    s.push(Step::Shot("notion-toolbar"));
    s.push(Step::End);

    s.push(Step::Begin("notion: a click picks no words"));
    s.extend(click(At::Byte("#no-text-4", 3)));
    s.push(Step::Expect(
        "the toolbar goes",
        Box::new(|d| !exists(d, "#no-tool") && !exists(d, ".no-pick")),
    ));
    s.extend(drag(
        At::Byte("#no-text-2", 4),
        At::Byte("#no-text-3", 4),
        None,
    ));
    s.push(Step::Expect(
        "nor does a drag across two lines, whose selection goes",
        Box::new(|d| {
            !exists(d, "#no-tool")
                && !exists(d, ".no-pick")
                && d.get_text_selection_ranges().is_empty()
        }),
    ));
    let start = LINE.find(PHRASE).expect("the phrase is in the line");
    // One jump, as a fast hand: nothing lands inside the words on the way.
    s.extend([
        Step::Move(At::Byte("#no-text-2", start)),
        Step::Down,
        Step::Move(At::In("#no-line-2", 400.0, 13.0)),
        Step::Up,
    ]);
    s.push(Step::Expect(
        "a drag past the last word stops at it",
        Box::new(|d| exists(d, "#no-tool") && text(d, "#no-text-2 .no-pick") == PHRASE),
    ));
    // The press lands on the pick's own span, which must outlive it for Blitz to start the drag.
    s.extend(picking("same", false, None));
    s.push(Step::Expect(
        "a drag from inside a pick picks again",
        Box::new(|d| exists(d, "#no-tool") && text(d, "#no-text-2 .no-pick") == "same"),
    ));
    s.extend([
        Step::Move(At::Byte("#no-text-4", 3)),
        Step::RightDown,
        Step::RightUp,
    ]);
    s.push(Step::Wait(2));
    s.push(Step::Expect(
        "the right button leaves a pick open",
        Box::new(|d| exists(d, "#no-tool") && text(d, "#no-text-2 .no-pick") == "same"),
    ));
    s.extend(click(At::Byte("#no-text-4", 3)));
    s.extend([
        Step::Move(At::Byte("#no-text-4", 3)),
        Step::RightDown,
        Step::Move(At::Byte("#no-text-4", 12)),
        Step::Wait(2),
        Step::RightUp,
        Step::Wait(2),
    ]);
    s.push(Step::Expect(
        "nor does its drag open one",
        Box::new(|d| !exists(d, "#no-tool") && !exists(d, ".no-pick")),
    ));
    s.extend(drag(
        At::Byte("#no-text-2", start - 1),
        At::Byte("#no-text-2", start + PHRASE.len()),
        None,
    ));
    s.push(Step::Expect(
        "a pick leaves out its outer spaces",
        Box::new(|d| exists(d, "#no-tool") && text(d, "#no-text-2 .no-pick") == PHRASE),
    ));
    s.extend(drag(
        At::Byte("#no-text-2", start - 1),
        At::Byte("#no-text-2", start),
        None,
    ));
    s.push(Step::Expect(
        "a space alone picks nothing",
        Box::new(|d| !exists(d, "#no-tool") && !exists(d, ".no-pick")),
    ));
    s.extend(picking(PHRASE, true, None));
    s.push(Step::Expect(
        "a drag backwards picks them too",
        Box::new(|d| exists(d, "#no-tool") && text(d, "#no-text-2 .no-pick") == PHRASE),
    ));
    s.push(Step::End);

    s.push(Step::Begin(
        "notion: a comment marks only those words, with its replies",
    ));
    s.extend(click(At::Sel("#no-comment")));
    s.push(Step::Expect(
        "its composer quotes them",
        Box::new(|d| exists(d, "#no-input") && text(d, "#no-new .no-quote") == PHRASE),
    ));
    s.push(Step::Expect(
        "the placeholder is drawn",
        Box::new(|d| exists(d, "#no-new .ds-placeholder")),
    ));
    for words in [
        "here is a test comment",
        "and a first reply",
        "and a second",
    ] {
        s.extend(click(At::Sel("#no-input")));
        s.extend(typed(words));
        s.push(Step::Expect(
            "the placeholder goes",
            Box::new(|d| !exists(d, ".no-thread .ds-placeholder")),
        ));
        s.extend(click(At::Sel("#no-send")));
        s.push(Step::Wait(2));
    }
    s.push(Step::Expect(
        "three comments on that phrase",
        Box::new(|d| exists(d, "#no-said-0-2") && text(d, "#no-quote-0") == PHRASE),
    ));
    s.push(Step::Expect(
        "the mark is those words, the line whole",
        Box::new(|d| marks(d) == [PHRASE] && text(d, "#no-text-2") == LINE),
    ));
    s.push(Step::Move(At::In(".no-side", 20.0, 500.0)));
    s.push(Step::Wait(2));
    s.push(Step::Shot("notion-thread"));
    s.push(Step::End);

    // The line's text is now split in spans: its offsets must still count from its first word.
    s.push(Step::Begin("notion: a second phrase on the marked line"));
    s.extend(picking(SECOND, false, None));
    s.extend(click(At::Sel("#no-comment")));
    s.extend(click(At::Sel("#no-input")));
    s.extend(typed("another phrase"));
    // A press that slips 10 px still sends: that drag must not become a selection, which fires no click.
    s.extend(drag(
        At::In("#no-send", 6.0, 11.0),
        At::In("#no-send", 16.0, 11.0),
        None,
    ));
    s.push(Step::Expect(
        "two marks, each its words",
        Box::new(|d| {
            marks(d) == [SECOND, PHRASE]
                && text(d, "#no-quote-1") == SECOND
                && text(d, "#no-text-2") == LINE
        }),
    ));
    s.push(Step::Move(At::In(".no-side", 20.0, 500.0)));
    s.push(Step::Wait(2));
    s.push(Step::Shot("notion-two-phrases"));
    s.extend(click(At::Sel("#no-quote-0")));
    s.push(Step::Expect(
        "a click on the first thread reopens it",
        Box::new(|d| exists(d, "#no-thread-0 #no-input") && !exists(d, "#no-thread-1 #no-input")),
    ));
    s.push(Step::Move(At::Sel("#no-said-1-0")));
    s.extend(click(At::Sel("#no-said-1-0-more")));
    s.push(Step::Expect(
        "the other thread's ... opens its menu and leaves the composer",
        Box::new(|d| {
            exists(d, "#no-menu")
                && exists(d, "#no-thread-0 #no-input")
                && !exists(d, "#no-thread-1 #no-input")
        }),
    ));
    s.extend(click(At::Sel("#no-said-1-0-more")));
    s.push(Step::Expect(
        "and closes it",
        Box::new(|d| !exists(d, "#no-menu")),
    ));
    s.extend(drag(
        At::In("#no-said-0-0 .no-words", 30.0, 8.0),
        At::In("#no-said-0-0 .no-words", 90.0, 8.0),
        None,
    ));
    s.push(Step::Expect(
        "a selection off the lines stays, for Cmd+C",
        Box::new(|d| {
            !d.get_text_selection_ranges().is_empty()
                && d.get_selected_text().is_some_and(|t| !t.is_empty())
        }),
    ));
    s.extend(picking("same", false, None));
    s.push(Step::Expect(
        "a pick inside a mark shows",
        Box::new(|d| text(d, "#no-text-2 .no-pick") == "same" && exists(d, "#no-tool")),
    ));
    s.extend(click(At::Byte("#no-text-4", 3)));
    s.push(Step::Expect(
        "the press closes it",
        Box::new(|d| !exists(d, "#no-tool") && marks(d) == [SECOND, PHRASE]),
    ));
    s.push(Step::End);

    s.push(Step::Begin(
        "notion: the page's own comments, replies folded",
    ));
    s.extend(click(At::Sel("#no-add")));
    s.push(Step::Expect(
        "its composer",
        Box::new(|d| exists(d, "#no-page-input")),
    ));
    s.extend(click(At::Sel("#no-page-input")));
    s.extend(typed("comments on the whole page"));
    s.extend(click(At::Sel("#no-page-post")));
    s.push(Step::Expect(
        "posted, the thread folded",
        Box::new(|d| exists(d, "#no-page-0") && exists(d, "#no-unfold-0")),
    ));
    s.push(Step::Move(At::In(".no-side", 20.0, 500.0)));
    s.push(Step::Wait(2));
    s.push(Step::Shot("notion-page"));
    s.push(Step::End);

    s.push(Step::Begin("notion: a comment's menu"));
    s.push(Step::Move(At::Sel("#no-page-0")));
    s.push(Step::Expect(
        "its ... shows on hover",
        Box::new(|d| shown(d, "#no-page-0-more")),
    ));
    s.extend(click(At::Sel("#no-page-0-more")));
    s.push(Step::Expect(
        "the menu opens",
        Box::new(|d| exists(d, "#no-menu")),
    ));
    s.push(Step::Wait(2));
    s.push(Step::Shot("notion-menu"));
    s.push(Step::End);

    s.push(Step::Begin("notion: its light theme"));
    s.extend(click(At::Sel("#ds-theme")));
    s.push(Step::Expect(
        "light",
        Box::new(|d| exists(d, ".app.light.ds-root")),
    ));
    s.push(Step::Wait(2));
    s.push(Step::Shot("notion-light"));
    s.push(Step::End);
    s
}
