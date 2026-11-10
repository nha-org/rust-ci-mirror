//! The picture diff for Blitz, on vello's CPU renderer: no window, no GPU, so CI runs it. Its renders are the shared
//! references (UPDATE_UI_REFS=1 writes them).
use blitz_list::Headless;
use ui_spike_common::picture::{Picture, cases, check};

#[test]
fn blitz_list_matches_the_references() {
    let mut failed = vec![];
    for case in cases() {
        let mut doc = Headless::new(case.dark, case.w, case.h, 1.0);
        doc.scroll_by(case.scroll);
        let picture = Picture {
            width: case.w,
            height: case.h,
            rgba: doc.render().to_vec(),
        };
        failed.extend(check("blitz", &case.name(), &picture));
    }
    assert!(failed.is_empty(), "{}", failed.join("\n"));
}
