//! Scrolls the list end to end in a real window with synthetic wheel events, then prints section 9's numbers. This is
//! dioxus_native::launch_cfg's body, shorter, because the meter must wrap the application to see each frame.
use blitz_dom::{DocumentConfig, build_single_font_ctx};
use blitz_shell::{BlitzShellProxy, WindowConfig, create_default_event_loop};
use dioxus_native::{
    DioxusDocument, DioxusNativeApplication, DioxusNativeWindowRenderer, LogicalSize,
    WindowAttributes,
};
use ui_spike_common::meter::Run;
use ui_spike_common::{HEADER_H, INTER, ROW_H, ROWS, SCROLL_STEP};
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalPosition;
use winit::event::{MouseScrollDelta, PointerSource, TouchPhase, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::window::WindowId;

const SIZE: (f64, f64) = (1200.0, 800.0);

struct Metered {
    app: DioxusNativeApplication,
    run: Run,
    visible: bool,
    ended: bool,
}

impl ApplicationHandler for Metered {
    fn can_create_surfaces(&mut self, el: &dyn ActiveEventLoop) {
        self.app.can_create_surfaces(el);
    }

    fn window_event(&mut self, el: &dyn ActiveEventLoop, id: WindowId, event: WindowEvent) {
        // A redraw can arrive after el.exit(): the same run reported twice.
        if self.ended {
            return;
        }
        // The real pointer never reaches the run: moving it out of the window stopped the scroll, and a real wheel shortened
        // it (@nha, 2026-11-05: « if I move the mouse out it stops », « if I scroll it goes faster »). Only this meter's own
        // events, sent straight to the app below, drive the list.
        if matches!(
            event,
            WindowEvent::PointerEntered { .. }
                | WindowEvent::PointerLeft { .. }
                | WindowEvent::PointerMoved { .. }
                | WindowEvent::PointerButton { .. }
                | WindowEvent::MouseWheel { .. }
        ) {
            return;
        }
        // Behind a lock screen the window reads as occluded and Blitz skips its paint while the frames keep counting:
        // 2 ms frames and a 21 MB peak measured nothing. Every frame is told it is visible, so each one paints.
        let event = if matches!(event, WindowEvent::Occluded(_)) {
            WindowEvent::Occluded(false)
        } else {
            event
        };
        if !self.visible {
            self.visible = true;
            self.app.window_event(el, id, WindowEvent::Occluded(false));
        }
        let redraw = matches!(event, WindowEvent::RedrawRequested);
        self.app.window_event(el, id, event);
        if !redraw {
            return;
        }
        // blitz-shell's redraw does not poll the vdom: a paint before the next poll shows the old rows.
        if !blitz_list::rows_caught_up() {
            return;
        }
        self.run.frame();
        let top = blitz_list::scroll_top();
        if top + SIZE.1 - HEADER_H + 1.0 >= ROWS as f64 * ROW_H {
            self.run.report(
                &format!("blitz-{}", env!("BLITZ_RENDERER")),
                (top / ROW_H) as usize,
            );
            self.ended = true;
            el.exit();
            return;
        }
        if self.run.frames() == 1 {
            // Over the list at scale 1 and at scale 2.
            let position = PhysicalPosition::new(300.0, 300.0);
            let source = PointerSource::Mouse;
            self.app.window_event(
                el,
                id,
                WindowEvent::PointerMoved {
                    device_id: None,
                    position,
                    primary: true,
                    source,
                },
            );
        }
        let delta = MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, -SCROLL_STEP));
        self.app.window_event(
            el,
            id,
            WindowEvent::MouseWheel {
                device_id: None,
                delta,
                phase: TouchPhase::Moved,
            },
        );
    }

    fn resumed(&mut self, el: &dyn ActiveEventLoop) {
        self.app.resumed(el);
    }

    fn suspended(&mut self, el: &dyn ActiveEventLoop) {
        self.app.suspended(el);
    }

    fn destroy_surfaces(&mut self, el: &dyn ActiveEventLoop) {
        self.app.destroy_surfaces(el);
    }

    fn proxy_wake_up(&mut self, el: &dyn ActiveEventLoop) {
        self.app.proxy_wake_up(el);
    }

    fn about_to_wait(&mut self, el: &dyn ActiveEventLoop) {
        self.app.about_to_wait(el);
    }

    fn new_events(&mut self, el: &dyn ActiveEventLoop, cause: winit::event::StartCause) {
        self.app.new_events(el, cause);
    }
}

/// One frame per step, full window at scale 2, no display: the numbers a locked Mac can still give.
fn headless(dark: bool) {
    let (w, h) = (SIZE.0 as u32 * 2, SIZE.1 as u32 * 2);
    let mut doc = blitz_list::Headless::new(dark, w, h, 2.0);
    let mut run = Run::default();
    doc.render();
    run.frame();
    while blitz_list::scroll_top() + SIZE.1 - HEADER_H + 1.0 < ROWS as f64 * ROW_H {
        let before = blitz_list::scroll_top();
        doc.scroll_by(SCROLL_STEP);
        assert!(
            blitz_list::scroll_top() > before,
            "a wheel step left the scroller at {before}"
        );
        doc.render();
        run.frame();
    }
    run.report(
        "blitz-headless-vello-cpu",
        (blitz_list::scroll_top() / ROW_H) as usize,
    );
}

fn main() {
    let dark = std::env::args().any(|a| a == "--dark");
    if std::env::args().any(|a| a == "--headless") {
        return headless(dark);
    }
    ui_spike_common::meter::require_display();
    let event_loop = create_default_event_loop();
    let (proxy, queue) = BlitzShellProxy::new(event_loop.create_proxy());
    let doc = DioxusDocument::new(
        blitz_list::vdom(dark),
        DocumentConfig {
            font_ctx: Some(build_single_font_ctx(INTER)),
            ..Default::default()
        },
    );
    let attributes =
        // Fixed size: WINDOW_ROWS builds the rows of an 800 px window only.
        WindowAttributes::default()
            .with_surface_size(LogicalSize::new(SIZE.0, SIZE.1))
            .with_resizable(false);
    let config = WindowConfig::with_attributes(
        Box::new(doc) as _,
        DioxusNativeWindowRenderer::new(),
        attributes,
    );
    let app = DioxusNativeApplication::new(proxy, queue, config);
    event_loop
        .run_app(Metered {
            app,
            run: Run::default(),
            visible: false,
            ended: false,
        })
        .expect("run");
}
