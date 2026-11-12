//! The picture diff for Slint, on its software renderer: no window, no GPU, so CI runs it.
use slint_list::Headless;
use ui_spike_common::picture::{Picture, cases, check};

#[test]
fn slint_list_matches_the_references() {
    let headless = Headless::new();
    let mut failed = vec![];
    for case in cases() {
        let picture = Picture {
            width: case.w,
            height: case.h,
            rgba: headless.render_once(case.dark, case.w, case.h, case.scroll),
        };
        failed.extend(check("slint", &case.name(), &picture));
    }
    assert!(failed.is_empty(), "{}", failed.join("\n"));
}
