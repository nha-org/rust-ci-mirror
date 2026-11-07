//! The design page in a real window. No flag: it opens for a person, the real pointer and keyboard live.
//!   --drive         runs script.rs, one JSON line per behaviour; exit 1 unless just the recorded ones fail
//!   --shots <dir>   with --drive, a PNG per shot, a separate run since a picture costs memory
//!   --size WxH      the window's size, 1200x800 by default: the evidence pictures use 1483x856
//!   --no-window     --drive on the software renderer, for a Mac that is locked: its pictures, never its numbers
//! A driven run drops the real pointer and keys, as slint-list does, so a person at the Mac cannot move its numbers.
use slint::winit_030::{EventResult, WinitWindowAccessor, winit::event::WindowEvent};
use slint::{ComponentHandle, LogicalSize, RenderingState};
use slint_design::script::{Driver, Tick};
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};
use ui_spike_common::meter::HIDDEN_EXIT;
use ui_spike_common::picture::{Picture, write_png};

/// No frame for this long: macOS does not show the window, so ui-measure --wait holds instead of counting a failed run.
const NEVER_SHOWN: Duration = Duration::from_secs(30);

fn flag(name: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

/// Slint's own read of what it drew, so no capture can show another window. Says nothing of what macOS put on the screen.
fn shot(app: &slint_design::App, dir: &Path, name: &str) {
    let buffer = app.window().take_snapshot().expect("snapshot");
    let rgba: Vec<u8> = buffer.as_bytes().to_vec();
    let picture = Picture { width: buffer.width(), height: buffer.height(), rgba };
    write_png(&dir.join(format!("{name}.png")), &picture);
}

fn finish(driver: &Driver) -> ! {
    for line in &driver.lines {
        println!("{line}");
    }
    if !driver.as_recorded() {
        eprintln!("refused: failed {:?}, not as recorded: {:?}", driver.failed(), driver.why());
        std::process::exit(1);
    }
    std::process::exit(0);
}

fn no_window(size: (u32, u32), dir: Option<PathBuf>) {
    use slint::platform::software_renderer::PremultipliedRgbaColor;
    let headless = slint_design::Headless::new();
    let app = slint_design::app();
    headless.size(size.0, size.1);
    app.show().expect("show");
    let mut pixels = vec![PremultipliedRgbaColor::default(); (size.0 * size.1) as usize];
    let mut driver = Driver::new();
    loop {
        headless.frame(&mut pixels, size.0);
        match driver.tick(&app) {
            Tick::Idle => {}
            Tick::Shot(name) => {
                if let Some(dir) = &dir {
                    shot(&app, dir, name);
                }
            }
            Tick::Done => finish(&driver),
        }
        std::thread::sleep(Duration::from_millis(8));
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let has = |name: &str| args.iter().any(|a| a == name);
    let size = flag("--size").and_then(|s| s.split_once('x').and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)))).unwrap_or((1200u32, 800u32));
    let dir = flag("--shots").map(PathBuf::from);
    if has("--no-window") {
        return no_window(size, dir);
    }
    ui_spike_common::meter::require_display();
    let app = slint_design::app();
    app.window().set_size(LogicalSize::new(size.0 as f32, size.1 as f32));
    if has("--drive") {
        let driver = Rc::new(RefCell::new(Driver::new()));
        let weak = app.as_weak();
        let on_frame = driver.clone();
        let last_frame = Rc::new(Cell::new(Instant::now()));
        let seen = last_frame.clone();
        let watch = slint::Timer::default();
        watch.start(slint::TimerMode::Repeated, Duration::from_secs(5), move || {
            if seen.get().elapsed() > NEVER_SHOWN {
                eprintln!("refused: macOS does not show this window, so its numbers would be false");
                std::process::exit(HIDDEN_EXIT);
            }
        });
        std::mem::forget(watch);
        app.window()
            .set_rendering_notifier(move |state, _| {
                if matches!(state, RenderingState::AfterRendering) {
                    last_frame.set(Instant::now());
                    let (weak, driver, dir) = (weak.clone(), on_frame.clone(), dir.clone());
                    // On the event loop, so the buffer swap that follows this notifier counts in the frame.
                    slint::Timer::single_shot(Duration::ZERO, move || {
                        let app = weak.upgrade().expect("window");
                        let tick = driver.borrow_mut().tick(&app);
                        match tick {
                            Tick::Idle => {}
                            Tick::Shot(name) => {
                                if let Some(dir) = &dir {
                                    shot(&app, dir, name);
                                }
                            }
                            Tick::Done => finish(&driver.borrow()),
                        }
                        app.window().request_redraw();
                    });
                }
            })
            .expect("a GPU window gives per-frame timing, or its numbers mean nothing");
        app.window().on_winit_window_event(|_, event| match event {
            WindowEvent::CursorMoved { .. } | WindowEvent::CursorEntered { .. } | WindowEvent::CursorLeft { .. } | WindowEvent::MouseInput { .. } | WindowEvent::MouseWheel { .. } | WindowEvent::KeyboardInput { .. } => EventResult::PreventDefault,
            // Covered, a GPU window stops drawing and the run stalls: every frame paints, as slint-list's.
            WindowEvent::Occluded(_) => EventResult::PreventDefault,
            _ => EventResult::Propagate,
        });
    }
    app.run().expect("run");
}
