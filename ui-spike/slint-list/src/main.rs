//! Scrolls the list end to end in a real window one viewport a frame, then prints section 9's numbers.
use slint::winit_030::{EventResult, WinitWindowAccessor, winit::event::WindowEvent};
use slint::{ComponentHandle, LogicalSize, RenderingState};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;
use ui_spike_common::meter::Run;
use ui_spike_common::{ROW_H, SCROLL_STEP};

/// The CPU window's step: meter::window_refusal refuses a run paced at this, since no paint then held the timer back.
const CPU_STEP_MS: u64 = 8;

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
    let ended = Cell::new(false);
    let step = Rc::new(move |renderer: &str| {
        // A second redraw before the loop quits would report the run twice.
        if ended.get() {
            return;
        }
        let app = weak.upgrade().expect("window");
        run.borrow_mut().frame();
        // Slint's wheel path is a clamp over this property; a dispatched wheel event moved nothing here. Set and ended from
        // the frame count, never from the property, so a real wheel on this window cannot shorten the run (@nha, 2026-11-05:
        // « if I scroll it goes faster »).
        let top = (run.borrow().frames() - 1) as f64 * SCROLL_STEP;
        let max = f64::from(app.get_scroll_height());
        if top + 1.0 >= max {
            ended.set(true);
            // Read back from the property this closure sets: it says the run stepped to its end, never that a paint showed it.
            let at = -f64::from(app.get_scroll_y());
            run.borrow().report(renderer, (at / ROW_H) as usize);
            slint::quit_event_loop().expect("quit");
            return;
        }
        app.set_scroll_y(-((top + SCROLL_STEP).min(max)) as f32);
    });
    let on_frame = step.clone();
    let notifier = app.window().set_rendering_notifier(move |state, _| {
        if matches!(state, RenderingState::AfterRendering) {
            let step = on_frame.clone();
            // On the event loop, so the buffer swap that follows this notifier counts in the frame.
            slint::Timer::single_shot(Duration::ZERO, move || step("slint-femtovg"));
        }
    });
    let cpu_step = match notifier {
        Ok(()) => None,
        Err(_) if std::env::var("SLINT_BACKEND").is_ok_and(|b| b.contains("software")) => {
            Some(step)
        }
        Err(e) => panic!("a GPU window gives per-frame timing, or its numbers mean nothing: {e:?}"),
    };
    let timer = slint::Timer::default();
    app.window()
        .on_winit_window_event(move |_, event| match event {
            // The real pointer never reaches the run, as in blitz-list (@nha, 2026-11-05: « if I scroll it goes faster »).
            WindowEvent::CursorMoved { .. }
            | WindowEvent::CursorEntered { .. }
            | WindowEvent::CursorLeft { .. }
            | WindowEvent::MouseInput { .. }
            | WindowEvent::MouseWheel { .. } => EventResult::PreventDefault,
            // Covered, a GPU window stops drawing, so no AfterRendering steps the run and it stalls to ui-measure's limit;
            // every frame paints, as blitz-list's Occluded(false) (/code-review of PR 1097)
            WindowEvent::Occluded(_) => EventResult::PreventDefault,
            // The CPU renderer has no notifier, and on the Mac its display link calls draw() directly, so winit's redraw fires
            // for the first paint alone: a loop waiting on it froze after one step (@nha, 2026-11-05: « slint-list is open and
            // not moving right ? »). First paint is read after that redraw; then a timer steps the scroll. Nothing here counts
            // later paints, so the frame gaps are this timer held back by each paint, no renderer's frame time.
            WindowEvent::RedrawRequested => {
                if let Some(step) = cpu_step.as_ref().filter(|_| !timer.running()) {
                    let step = step.clone();
                    timer.start(
                        slint::TimerMode::Repeated,
                        Duration::from_millis(CPU_STEP_MS),
                        move || step("slint-software"),
                    );
                }
                EventResult::Propagate
            }
            _ => EventResult::Propagate,
        });
    app.run().expect("run");
}
