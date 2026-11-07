//! The design page's behaviours, as script.rs's: each family opens, a state forced by a class draws what the real
//! pseudo-class draws, and the Linear and Notion screens work. Each picture beside its evidence is a shot here.
use super::FAMILIES;
use crate::script::{At, Step, background, click, drag, exists, shadow, shown, text, typed};
use std::cell::RefCell;
use std::rc::Rc;

/// A click does not focus a button, though it does a field: a ring on a button is untested from the keyboard here. Recorded so
/// a Blitz that changes says so.
pub const RECORDED_FAILURES: [&str; 1] = ["forms: a click focuses a button"];

const FAMILY_TABS: [(&str, &str, &str); 6] = [
    ("family: layout", "#f-layout", "family-layout"),
    ("family: data", "#f-data", "family-data"),
    ("family: forms", "#f-forms", "family-forms"),
    ("family: feedback", "#f-feedback", "family-feedback"),
    ("family: mail", "#f-mail", "family-mail"),
    ("family: board", "#f-board", "family-board"),
];

fn shots_down(name: &'static str, second: &'static str) -> Vec<Step> {
    vec![Step::Wait(2), Step::Shot(name), Step::Move(At::Sel("#ds-main")), Step::Wheel(700.0), Step::Wait(4), Step::Shot(second)]
}

/// Holds the colour of a selector so a later step compares against it.
fn remember(what: &'static str, sel: &'static str, into: Rc<RefCell<String>>) -> Step {
    Step::Expect(what, Box::new(move |d| {
        *into.borrow_mut() = background(d, sel);
        !into.borrow().is_empty()
    }))
}

/// The real pseudo-class on the real control draws what the forced class beside it draws.
fn same_as_forced(what: &'static str, sel: &'static str, forced: &'static str) -> Step {
    Step::Expect(what, Box::new(move |d| {
        let (a, b) = (background(d, sel), background(d, forced));
        !a.is_empty() && a == b
    }))
}

/// Two missing nodes read the same empty string: that is no match.
fn same_shadow(d: &blitz_dom::BaseDocument, a: &str, b: &str) -> bool {
    let (a, b) = (shadow(d, a), shadow(d, b));
    !a.is_empty() && a == b
}

pub fn script() -> Vec<Step> {
    let mut s = vec![];
    for (name, tab, shot) in FAMILY_TABS {
        s.push(Step::Begin(name));
        s.extend(click(At::Sel(tab)));
        s.push(Step::Expect("its sections are drawn", Box::new(move |d| {
            let slug = tab.trim_start_matches("#f-");
            let (_, _, _, sections) = FAMILIES.iter().find(|f| f.0 == slug).expect("a family");
            sections.iter().all(|(id, _)| exists(d, &format!("#s-{id}"))) && exists(d, ".app.dark.ds-cell") && exists(d, ".app.light.ds-cell")
        })));
        s.extend(shots_down(shot, Box::leak(format!("{shot}-2").into_boxed_str())));
        s.push(Step::End);
    }

    // The forms page, the first rows of Actions: the real states against the forced ones. Try-it rows are low on the page.
    s.push(Step::Begin("forms: real :hover, :active and :focus draw the forced states"));
    s.extend(click(At::Sel("#f-forms")));
    s.push(Step::Move(At::Sel("#ds-main")));
    s.push(Step::Wheel(320.0));
    s.push(Step::Wait(4));
    const TRY: &str = ".app.light .try .button";
    const FORCED_HOVER: &str = ".app.light .button.is-hover";
    const FORCED_PRESSED: &str = ".app.light .button.is-active";
    s.push(Step::Move(At::Sel(TRY)));
    s.push(same_as_forced("the real :hover is the forced one", TRY, FORCED_HOVER));
    s.push(Step::Down);
    s.push(same_as_forced("the real :active is the forced one", TRY, FORCED_PRESSED));
    s.push(Step::Shot("state-pressed"));
    s.push(Step::Up);
    s.push(Step::End);

    s.push(Step::Begin("forms: a click focuses a button"));
    s.extend(click(At::Sel(TRY)));
    s.push(Step::Expect("the real :focus rings it as the forced one", Box::new(|d| same_shadow(d, TRY, ".app.light .button.is-focus"))));
    s.push(Step::End);

    s.push(Step::Begin("forms: a click focuses a field"));
    s.push(Step::WheelTo(".app.light .try-field .input"));
    s.push(Step::Wait(4));
    s.extend(click(At::Sel(".app.light .try-field .input")));
    s.push(Step::Expect("the real :focus rings it as the forced one", Box::new(|d| same_shadow(d, ".app.light .try-field .input", ".app.light .input.is-focus"))));
    s.push(Step::Wait(2));
    s.push(Step::Shot("state-focus"));
    s.extend(typed("1"));
    s.push(Step::Expect("and it takes a key", Box::new(|d| crate::script::input_value(d, ".app.light .try-field .input") == "6501")));
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
        s.push(Step::Expect("colour read", Box::new(move |d| {
            let now = background(d, TRY);
            if now != *r.borrow() && now != background(d, FORCED_HOVER) {
                *b.borrow_mut() = true;
            }
            true
        })));
        s.push(Step::Wait(1));
    }
    s.push(Step::Expect("a colour between the two ends", Box::new(move |_| *between.borrow())));
    s.push(Step::End);

    s.push(Step::Begin("theme: the page's toggle"));
    let light = Rc::new(RefCell::new(String::new()));
    s.push(remember("it has a colour", ".ds-root", light.clone()));
    s.extend(click(At::Sel("#ds-theme")));
    s.push(Step::Expect("the root's colour changes", Box::new(move |d| background(d, ".ds-root") != *light.borrow() && exists(d, ".app.dark.ds-root"))));
    s.push(Step::Wait(2));
    s.push(Step::Shot("family-forms-dark"));
    s.extend(click(At::Sel("#ds-theme")));
    s.push(Step::Expect("and back", Box::new(|d| exists(d, ".app.light.ds-root"))));
    s.push(Step::End);

    s.push(Step::Begin("linear: the board"));
    s.extend(click(At::Sel("#v-linear")));
    s.push(Step::Expect("the cards are drawn", Box::new(|d| exists(d, "#li-card-478") && exists(d, "#li-card-457"))));
    s.push(Step::Wait(2));
    s.push(Step::Move(At::In(".li-side", 20.0, 700.0)));
    s.push(Step::Wait(2));
    s.push(Step::Shot("linear-board"));
    s.push(Step::End);

    s.push(Step::Begin("linear: hover and drag"));
    let resting = Rc::new(RefCell::new(String::new()));
    let keep = resting.clone();
    s.push(Step::Expect("it has a shadow", Box::new(move |d| {
        *keep.borrow_mut() = shadow(d, "#li-card-476");
        !keep.borrow().is_empty()
    })));
    s.push(Step::Move(At::Sel("#li-card-476")));
    s.push(Step::Expect("the shadow rises", Box::new(move |d| shadow(d, "#li-card-476") != *resting.borrow())));
    s.push(Step::Wait(2));
    s.push(Step::Shot("linear-hover"));
    s.extend(drag(At::Sel("#li-card-476"), At::Sel("#li-col-1"), Some("linear-drag")));
    s.push(Step::Expect("the card moved", Box::new(|d| text(d, "#li-count-0") == "4" && text(d, "#li-count-1") == "6")));
    s.push(Step::End);

    s.push(Step::Begin("linear: its dark theme"));
    s.extend(click(At::Sel("#ds-theme")));
    s.push(Step::Expect("dark", Box::new(|d| exists(d, ".app.dark.ds-root"))));
    s.push(Step::Wait(2));
    s.push(Step::Shot("linear-dark"));
    s.push(Step::End);

    s.push(Step::Begin("notion: pick a line, its toolbar"));
    s.extend(click(At::Sel("#v-notion")));
    s.push(Step::Expect("the page", Box::new(|d| exists(d, "#no-line-2"))));
    s.extend(click(At::Sel("#no-line-2")));
    s.push(Step::Expect("the toolbar opens", Box::new(|d| exists(d, "#no-tool") && exists(d, "#no-line-2.selected"))));
    s.push(Step::Wait(2));
    s.push(Step::Shot("notion-toolbar"));
    s.push(Step::End);

    s.push(Step::Begin("notion: a thread and its replies"));
    s.extend(click(At::Sel("#no-comment")));
    s.push(Step::Expect("its composer", Box::new(|d| exists(d, "#no-input"))));
    s.push(Step::Expect("the placeholder is drawn", Box::new(|d| exists(d, "#no-new .ds-placeholder"))));
    for words in ["here is a test comment", "and a first reply", "and a second"] {
        s.extend(click(At::Sel("#no-input")));
        s.extend(typed(words));
        s.push(Step::Expect("the placeholder goes", Box::new(|d| !exists(d, ".no-thread .ds-placeholder"))));
        s.extend(click(At::Sel("#no-send")));
        s.push(Step::Wait(2));
    }
    s.push(Step::Expect("three comments, the line marked", Box::new(|d| exists(d, "#no-said-0-2") && exists(d, "#no-line-2.marked"))));
    s.push(Step::Move(At::In(".no-side", 20.0, 500.0)));
    s.push(Step::Wait(2));
    s.push(Step::Shot("notion-thread"));
    s.push(Step::End);

    s.push(Step::Begin("notion: the page's own comments, replies folded"));
    s.extend(click(At::Sel("#no-add")));
    s.push(Step::Expect("its composer", Box::new(|d| exists(d, "#no-page-input"))));
    s.extend(click(At::Sel("#no-page-input")));
    s.extend(typed("comments on the whole page"));
    s.extend(click(At::Sel("#no-page-post")));
    s.push(Step::Expect("posted, the thread folded", Box::new(|d| exists(d, "#no-page-0") && exists(d, "#no-unfold-0"))));
    s.push(Step::Move(At::In(".no-side", 20.0, 500.0)));
    s.push(Step::Wait(2));
    s.push(Step::Shot("notion-page"));
    s.push(Step::End);

    s.push(Step::Begin("notion: a comment's menu"));
    s.push(Step::Move(At::Sel("#no-page-0")));
    s.push(Step::Expect("its ... shows on hover", Box::new(|d| shown(d, "#no-page-0-more"))));
    s.extend(click(At::Sel("#no-page-0-more")));
    s.push(Step::Expect("the menu opens", Box::new(|d| exists(d, "#no-menu"))));
    s.push(Step::Wait(2));
    s.push(Step::Shot("notion-menu"));
    s.push(Step::End);

    s.push(Step::Begin("notion: its light theme"));
    s.extend(click(At::Sel("#ds-theme")));
    s.push(Step::Expect("light", Box::new(|d| exists(d, ".app.light.ds-root"))));
    s.push(Step::Wait(2));
    s.push(Step::Shot("notion-light"));
    s.push(Step::End);
    s
}
