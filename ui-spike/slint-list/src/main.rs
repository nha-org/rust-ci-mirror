//! Scrolls the list end to end in a real window one viewport a frame, then prints section 9's numbers.
use slint::{ComponentHandle, LogicalSize, RenderingState};
use std::cell::RefCell;
use std::rc::Rc;
use ui_spike_common::meter::Run;
use ui_spike_common::{ROW_H, SCROLL_STEP};

/// One frame per step, full window at scale 2, no display: the numbers a locked Mac can still give.
fn headless(dark: bool) {
    use slint::platform::software_renderer::PremultipliedRgbaColor;
    let (w, h) = (2400, 1600);
    let backend = slint_list::Headless::new();
    let app = slint_list::app(dark);
    backend.show(&app, w, h, 2.0);
    let mut pixels = vec![PremultipliedRgbaColor::default(); (w * h) as usize];
    let mut run = Run::default();
    backend.draw(&mut pixels, w);
    run.frame();
    loop {
        let (top, max) = (
            -f64::from(app.get_scroll_y()),
            f64::from(app.get_scroll_height()),
        );
        if top + 1.0 >= max {
            run.report("slint-headless-software", (top / ROW_H) as usize);
            return;
        }
        app.set_scroll_y(-((top + SCROLL_STEP).min(max)) as f32);
        backend.draw(&mut pixels, w);
        run.frame();
    }
}

fn main() {
    let dark = std::env::args().any(|a| a == "--dark");
    if std::env::args().any(|a| a == "--headless") {
        return headless(dark);
    }
    ui_spike_common::meter::require_display();
    let app = slint_list::app(dark);
    app.window().set_size(LogicalSize::new(1200.0, 800.0));
    let run = Rc::new(RefCell::new(Run::default()));
    let weak = app.as_weak();
    app.window()
        .set_rendering_notifier(move |state, _| {
            if !matches!(state, RenderingState::AfterRendering) {
                return;
            }
            let (run, weak) = (run.clone(), weak.clone());
            // On the event loop, so the buffer swap that follows this notifier counts in the frame.
            slint::Timer::single_shot(std::time::Duration::ZERO, move || {
                let app = weak.upgrade().expect("window");
                run.borrow_mut().frame();
                let top = -f64::from(app.get_scroll_y());
                let max = f64::from(app.get_scroll_height());
                if top + 1.0 >= max {
                    run.borrow().report("slint-femtovg", (top / ROW_H) as usize);
                    slint::quit_event_loop().expect("quit");
                    return;
                }
                // Slint's wheel path is a clamp over this property; a dispatched wheel event moved nothing here.
                app.set_scroll_y(-((top + SCROLL_STEP).min(max)) as f32);
            });
        })
        .expect("only FemtoVG is built with the window, and it gives this notifier");
    app.run().expect("run");
}
