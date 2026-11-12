//! The window's script with no window, on Slint's software renderer, on Linux CI too. The numbers come from the real window.
use slint::platform::software_renderer::PremultipliedRgbaColor;
use slint_design::script::{Driver, Tick};
use std::time::Duration;

#[test]
fn the_script_runs_as_recorded() {
    let headless = slint_design::Headless::new();
    let app = slint_design::app();
    let (w, h) = (1200, 800);
    headless.size(w, h);
    slint::ComponentHandle::show(&app).expect("show");
    let mut pixels = vec![PremultipliedRgbaColor::default(); (w * h) as usize];
    let mut driver = Driver::new();
    let mut frame = 0;
    loop {
        // A draw a tenth of the frames: a debug build's software renderer would take the run past nextest's limit.
        frame += 1;
        if frame % 10 == 1 {
            headless.frame(&mut pixels, w);
        } else {
            slint::platform::update_timers_and_animations();
        }
        if matches!(driver.tick(&app), Tick::Done) {
            break;
        }
        // As the screen's pace: a transition is measured against the clock.
        std::thread::sleep(Duration::from_millis(8));
    }
    assert!(driver.as_recorded(), "{:#?}", driver.why());
    assert!(driver.results.len() >= 20, "{:?}", driver.results);
}
