//! The four screens in a real window. No flag: it opens for a person, the real pointer and keyboard live.
//!   --shots <dir>   the same opened window, its first screen captured once drawn, then it exits: proof that this mode draws,
//!                   so exit 1 when the capture is not the live document
//!   --drive         runs script.rs in the window, one JSON line per behaviour; exit 1 unless just the recorded two fail
//!   --drive --shots <dir>  the same, and a PNG per behaviour: a separate run, since a picture costs memory; exit 1 on any
//!                   capture that is not the live document
//!   --measure       scrolls the 100,000 rows end to end, one JSON line as ui-measure takes it
//!   --design        the design system page instead of the four screens: alone, for a person; with --drive, its own script
//!   --size WxH      that page's window size, 1200x800 by default as the plain run: the evidence pictures use 1483x856
//! A driven run drops the real pointer and keys, as blitz-list does, so a person at the Mac cannot move its numbers.
use blitz_dom::{Document, DocumentConfig, build_single_font_ctx};
use blitz_shell::{BlitzApplication, BlitzShellEvent, BlitzShellProxy, View, WindowConfig};
use blitz_traits::events::{BlitzKeyEvent, KeyState, UiEvent};
use dioxus::prelude::Key;
#[cfg(not(target_os = "macos"))]
use dioxus::prelude::NamedKey;
use dioxus_native::{DioxusDocument, DioxusNativeWindowRenderer, LogicalSize, WindowAttributes};
use dioxus_page::script::{Driver, Input, Tick};
use dioxus_page::{Screen, WINDOW, rows_caught_up, scroll_top, vdom};
use std::path::PathBuf;
use ui_spike_common::meter::{HIDDEN_EXIT, Run};
use ui_spike_common::picture::{Picture, write_png};
use ui_spike_common::{DESIGN_FONT, HEADER_H, INTER, ROW_H, ROWS, SCROLL_STEP};
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalPosition;
use winit::event::{ButtonSource, ElementState, MouseButton, MouseScrollDelta, PointerSource, TouchPhase, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
#[cfg(target_os = "macos")]
use winit::platform::macos::EventLoopBuilderExtMacOS;
use winit::window::{WindowId, WindowLevel};

enum Mode {
    /// With a folder: capture the first screen, then exit.
    Open(Option<PathBuf>),
    Measure(Run),
    Drive(Box<Driver>, Option<PathBuf>),
}

struct App {
    app: BlitzApplication<DioxusNativeWindowRenderer>,
    mode: Mode,
    built: bool,
    visible: bool,
    hidden: u32,
    frames_seen: u32,
    ended: bool,
    pointer: PhysicalPosition<f64>,
    bad_shots: u32,
}

type Renderer = DioxusNativeWindowRenderer;

impl App {
    fn send(&mut self, el: &dyn ActiveEventLoop, id: WindowId, event: WindowEvent) {
        self.app.window_event(el, id, event);
    }

    fn button(&mut self, el: &dyn ActiveEventLoop, id: WindowId, state: ElementState, button: MouseButton) {
        let event = WindowEvent::PointerButton {
            device_id: None,
            state,
            position: self.pointer,
            primary: true,
            button: ButtonSource::Mouse(button),
            is_macos_activation_click: false,
        };
        self.send(el, id, event);
    }

    fn input(&mut self, el: &dyn ActiveEventLoop, id: WindowId, input: Input) {
        match input {
            Input::Move(x, y) => {
                let scale = self.app.windows[&id].window.scale_factor();
                self.pointer = PhysicalPosition::new(x * scale, y * scale);
                let event = WindowEvent::PointerMoved {
                    device_id: None,
                    position: self.pointer,
                    primary: true,
                    source: PointerSource::Mouse,
                };
                self.send(el, id, event);
            }
            Input::Down => self.button(el, id, ElementState::Pressed, MouseButton::Left),
            Input::Up => self.button(el, id, ElementState::Released, MouseButton::Left),
            Input::RightDown => self.button(el, id, ElementState::Pressed, MouseButton::Right),
            Input::RightUp => self.button(el, id, ElementState::Released, MouseButton::Right),
            Input::Wheel(y) => {
                let delta = MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, -y));
                self.send(el, id, WindowEvent::MouseWheel { device_id: None, delta, phase: TouchPhase::Moved });
            }
            // winit's KeyEvent cannot be built outside winit: this is the UiEvent View::handle_winit_event makes of one.
            Input::Text(t) => self.key(id, Key::Character(t.clone().into()), Some(t)),
            // On macOS a real Backspace reaches Blitz this way, through winit's standard key binding, never as a key.
            #[cfg(target_os = "macos")]
            Input::Backspace => {
                self.app.windows.get_mut(&id).expect("window").handle_apple_standard_keybinding("deleteBackward:");
                self.app.proxy.send_event(BlitzShellEvent::Poll { window_id: id });
            }
            #[cfg(not(target_os = "macos"))]
            Input::Backspace => self.key(id, Key::Named(NamedKey::Backspace), None),
        }
    }

    fn key(&mut self, id: WindowId, key: Key, text: Option<String>) {
        let event = BlitzKeyEvent {
            key,
            code: Default::default(),
            modifiers: Default::default(),
            location: Default::default(),
            is_auto_repeating: false,
            is_composing: false,
            state: KeyState::Pressed,
            text: text.map(Into::into),
        };
        let view = self.app.windows.get_mut(&id).expect("window");
        view.downcast_doc_mut::<DioxusDocument>().handle_ui_event(UiEvent::KeyDown(event));
        self.app.proxy.send_event(BlitzShellEvent::Poll { window_id: id });
    }

    fn after_redraw(&mut self, el: &dyn ActiveEventLoop, id: WindowId) {
        let view = self.app.windows.get_mut(&id).expect("window");
        // Hidden reads add up over the run, never reset: the case below read visible 1 time in 16. The first frames do
        // not count, as AppKit's first occlusion state comes late. Ends with the code a locked Mac gets; ui-measure --wait holds.
        self.frames_seen += 1;
        if self.frames_seen > 60 && !matches!(self.mode, Mode::Open(_)) && !capture::on_screen(&*view.window) {
            self.hidden += 1;
        }
        if self.hidden > 30 {
            eprintln!("refused: macOS does not show this window, so its numbers would be false");
            std::process::exit(HIDDEN_EXIT);
        }
        match &mut self.mode {
            Mode::Open(None) => {}
            Mode::Open(Some(dir)) => {
                // A few frames, so the compositor holds what was drawn.
                if self.frames_seen < 30 {
                    view.request_redraw();
                } else {
                    let dir = dir.clone();
                    if !write_shot(view, &dir, "open") {
                        eprintln!("refused: the opened window's capture is not the live document");
                        std::process::exit(1);
                    }
                    self.ended = true;
                    el.exit();
                }
            }
            Mode::Measure(run) => {
                // blitz-shell's redraw does not poll the vdom: a paint before the next poll shows the old rows.
                if !rows_caught_up() {
                    return;
                }
                run.frame();
                let top = scroll_top();
                if top + WINDOW.1 - HEADER_H + 1.0 >= ROWS as f64 * ROW_H {
                    run.report("dioxus-page-mail-vello-hybrid", (top / ROW_H) as usize);
                    self.ended = true;
                    el.exit();
                    return;
                }
                let first = run.frames() == 1;
                if first {
                    self.input(el, id, Input::Move(WINDOW.0 - 300.0, 300.0));
                }
                self.input(el, id, Input::Wheel(SCROLL_STEP));
            }
            Mode::Drive(driver, shots) => {
                let tick = driver.tick(&view.doc.inner());
                view.request_redraw();
                match tick {
                    Tick::Idle => {}
                    Tick::Input(input) => self.input(el, id, input),
                    Tick::Shot(name) => {
                        if let Some(dir) = shots
                            && !write_shot(view, dir, name)
                        {
                            self.bad_shots += 1;
                        }
                    }
                    Tick::Done => {
                        let Mode::Drive(driver, _) = &self.mode else { unreachable!() };
                        for line in &driver.lines {
                            println!("{line}");
                        }
                        // ui-measure discards a run on this exit: numbers from a script that did not run are not numbers.
                        if !driver.as_recorded() {
                            eprintln!("refused: failed {:?}, not as recorded", driver.failed());
                            std::process::exit(1);
                        }
                        if self.bad_shots > 0 {
                            eprintln!("refused: {} captures are not the live document", self.bad_shots);
                            std::process::exit(1);
                        }
                        self.ended = true;
                        el.exit();
                    }
                }
            }
        }
    }
}

/// Not blank is not drawn: the window's pixels are compared with the live document painted by the CPU renderer. A real
/// capture of another screen scored 0.90 to 0.94, a dragged card 0.986, the dark theme 0.04, the same screen 1.000.
const SAME_SCREEN: f64 = 0.99;

/// The window's own pixels when macOS gives them, else the live document painted by the CPU renderer: the file name says
/// which, so no picture passes for what it is not. True when the window's own pixels show the live document.
fn write_shot(view: &mut View<Renderer>, dir: &std::path::Path, name: &str) -> bool {
    let painted = paint(view);
    let (picture, how, matches) = match capture::window(&*view.window).filter(|p| !blank(p)) {
        Some(p) => {
            let matches = matching(&p, &painted);
            (p, "window", Some(matches))
        }
        None => (painted, "painted", None),
    };
    write_png(&dir.join(format!("{name}.{how}.png")), &picture);
    let score = matches.map_or("null".to_string(), |m| format!("{m:.3}"));
    eprintln!("{{\"shot\":\"{name}\",\"source\":\"{how}\",\"matches_live_document\":{score}}}");
    matches.is_some_and(|m| m >= SAME_SCREEN)
}

/// A capture of a window that drew nothing is its title bar over one flat colour (it looked fine by size alone until read).
fn blank(p: &Picture) -> bool {
    let row = |y: u32| &p.rgba[(y * p.width * 4) as usize..((y + 1) * p.width * 4) as usize];
    let body: Vec<&[u8]> = (p.height / 8..p.height).step_by(16).map(row).collect();
    let first = &body[0][p.width as usize * 2..p.width as usize * 2 + 4];
    body.iter().all(|r| r.chunks(4).skip(16).take(p.width as usize - 32).all(|c| c == first))
}

/// The share of the document's pixels the capture shows, within a colour step per channel. The capture has the title bar
/// on top, so it is compared from the bottom up.
fn matching(window: &Picture, doc: &Picture) -> f64 {
    let (w, h) = (window.width.min(doc.width) as usize, window.height.min(doc.height) as usize);
    let at = |p: &Picture, x: usize, y: usize| {
        let i = ((p.height as usize - 1 - y) * p.width as usize + x) * 4;
        [p.rgba[i], p.rgba[i + 1], p.rgba[i + 2]]
    };
    let same = (0..h)
        .flat_map(|y| (0..w).map(move |x| (x, y)))
        .filter(|&(x, y)| at(window, x, y).iter().zip(at(doc, x, y)).all(|(a, b)| a.abs_diff(b) <= 24))
        .count();
    same as f64 / (w * h) as f64
}

fn paint(view: &mut View<Renderer>) -> Picture {
    use anyrender::ImageRenderer;
    let mut doc = view.doc.inner_mut();
    let (width, height) = doc.viewport().window_size;
    let scale = doc.viewport().scale_f64();
    let mut renderer = anyrender_vello_cpu::VelloCpuImageRenderer::new(width, height);
    let mut rgba = vec![0; (width * height * 4) as usize];
    renderer.render(|scene| blitz_paint::paint_scene(scene, &mut doc, scale, width, height, 0, 0), &mut rgba);
    Picture { width, height, rgba }
}

#[cfg(target_os = "macos")]
mod capture {
    //! CGWindowListCreateImage on this process's own window: screencapture hangs in this sandbox, waiting on a permission.
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use std::ffi::{c_char, c_void};
    use ui_spike_common::picture::Picture;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Rect(f64, f64, f64, f64);

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGWindowListCreateImage(rect: Rect, list: u32, window: u32, image: u32) -> *const c_void;
        fn CGImageGetWidth(i: *const c_void) -> usize;
        fn CGImageGetHeight(i: *const c_void) -> usize;
        fn CGImageGetBytesPerRow(i: *const c_void) -> usize;
        fn CGImageGetDataProvider(i: *const c_void) -> *const c_void;
        fn CGDataProviderCopyData(p: *const c_void) -> *const c_void;
        fn CGImageRelease(i: *const c_void);
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFDataGetBytePtr(d: *const c_void) -> *const u8;
        fn CFRelease(o: *const c_void);
    }
    #[link(name = "objc")]
    unsafe extern "C" {
        fn sel_registerName(name: *const c_char) -> *const c_void;
        fn objc_msgSend();
    }

    fn ns_window(window: &dyn winit::window::Window) -> Option<*const c_void> {
        let RawWindowHandle::AppKit(handle) = window.window_handle().ok()?.as_raw() else {
            return None;
        };
        unsafe {
            let send: unsafe extern "C" fn(*const c_void, *const c_void) -> *const c_void =
                std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
            Some(send(handle.ns_view.as_ptr(), sel_registerName(c"window".as_ptr())))
        }
    }

    /// Whether macOS shows the window at all: NSWindowOcclusionStateVisible. Without it frames are not paced and its numbers
    /// are false, on a Mac the lock flag calls unlocked (2026-11-06: occlusionState 8192 and isOnActiveSpace 0 on 15 of 16
    /// reads, floating and on every Space; 2,316 of 2,506 frames under 4 ms, blitz-list's own run 7,237 of 7,429, a blank capture).
    pub fn on_screen(window: &dyn winit::window::Window) -> bool {
        let Some(ns_window) = ns_window(window) else { return true };
        unsafe {
            let send: unsafe extern "C" fn(*const c_void, *const c_void) -> usize =
                std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
            send(ns_window, sel_registerName(c"occlusionState".as_ptr())) & 2 != 0
        }
    }

    /// canJoinAllSpaces with fullScreenAuxiliary: drawn wherever he is working.
    pub fn on_every_space(window: &dyn winit::window::Window) {
        let Some(ns_window) = ns_window(window) else { return };
        unsafe {
            let send: unsafe extern "C" fn(*const c_void, *const c_void, usize) =
                std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
            send(ns_window, sel_registerName(c"setCollectionBehavior:".as_ptr()), 1 | 1 << 8);
        }
    }

    pub fn window(window: &dyn winit::window::Window) -> Option<Picture> {
        let ns_window = ns_window(window)?;
        unsafe {
            let send_int: unsafe extern "C" fn(*const c_void, *const c_void) -> isize =
                std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
            let number = send_int(ns_window, sel_registerName(c"windowNumber".as_ptr()));
            // CGRectNull, kCGWindowListOptionIncludingWindow, kCGWindowImageBoundsIgnoreFraming
            let null = Rect(f64::INFINITY, f64::INFINITY, 0.0, 0.0);
            let image = CGWindowListCreateImage(null, 8, number as u32, 1);
            if image.is_null() {
                return None;
            }
            let (w, h, stride) = (CGImageGetWidth(image), CGImageGetHeight(image), CGImageGetBytesPerRow(image));
            let data = CGDataProviderCopyData(CGImageGetDataProvider(image));
            let bytes = std::slice::from_raw_parts(CFDataGetBytePtr(data), stride * h);
            // BGRA, a row padded to its stride.
            let mut rgba = Vec::with_capacity(w * h * 4);
            for row in bytes.chunks(stride) {
                for p in row[..w * 4].chunks(4) {
                    rgba.extend([p[2], p[1], p[0], 255]);
                }
            }
            CFRelease(data);
            CGImageRelease(image);
            Some(Picture { width: w as u32, height: h as u32, rgba })
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod capture {
    pub fn window(_: &dyn winit::window::Window) -> Option<ui_spike_common::picture::Picture> {
        None
    }

    pub fn on_every_space(_: &dyn winit::window::Window) {}

    pub fn on_screen(_: &dyn winit::window::Window) -> bool {
        true
    }
}

impl ApplicationHandler for App {
    fn can_create_surfaces(&mut self, el: &dyn ActiveEventLoop) {
        self.app.can_create_surfaces(el);
        if !self.built {
            self.built = true;
            for view in self.app.windows.values_mut() {
                if !matches!(self.mode, Mode::Open(_)) {
                    let _ = view.window.set_cursor_hittest(false);
                    capture::on_every_space(&*view.window);
                }
                view.downcast_doc_mut::<DioxusDocument>().initial_build();
                view.request_redraw();
            }
        }
    }

    fn window_event(&mut self, el: &dyn ActiveEventLoop, id: WindowId, event: WindowEvent) {
        if self.ended {
            return;
        }
        if matches!(self.mode, Mode::Open(None)) {
            return self.app.window_event(el, id, event);
        }
        if matches!(self.mode, Mode::Open(_)) {
            let redraw = matches!(event, WindowEvent::RedrawRequested);
            self.app.window_event(el, id, event);
            if redraw {
                self.after_redraw(el, id);
            }
            return;
        }
        if matches!(
            event,
            WindowEvent::PointerEntered { .. }
                | WindowEvent::PointerLeft { .. }
                | WindowEvent::PointerMoved { .. }
                | WindowEvent::PointerButton { .. }
                | WindowEvent::MouseWheel { .. }
                | WindowEvent::KeyboardInput { .. }
                | WindowEvent::ModifiersChanged(_)
        ) {
            return;
        }
        // As blitz-list: a window read as covered skips each paint while its frames keep counting.
        let event = if matches!(event, WindowEvent::Occluded(_)) { WindowEvent::Occluded(false) } else { event };
        if !self.visible {
            self.visible = true;
            self.app.window_event(el, id, WindowEvent::Occluded(false));
        }
        let redraw = matches!(event, WindowEvent::RedrawRequested);
        self.app.window_event(el, id, event);
        if redraw {
            self.after_redraw(el, id);
        }
    }

    fn proxy_wake_up(&mut self, el: &dyn ActiveEventLoop) {
        self.app.proxy_wake_up(el);
    }

    fn destroy_surfaces(&mut self, el: &dyn ActiveEventLoop) {
        self.app.destroy_surfaces(el);
    }

    #[cfg(target_os = "macos")]
    fn macos_handler(&mut self) -> Option<&mut dyn winit::platform::macos::ApplicationHandlerExtMacOS> {
        self.app.macos_handler()
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let shots = args.iter().position(|a| a == "--shots").map(|i| PathBuf::from(&args[i + 1]));
    if let Some(dir) = &shots {
        std::fs::create_dir_all(dir).expect("shots folder");
    }
    let design = args.iter().any(|a| a == "--design");
    if design && args.iter().any(|a| a == "--measure") {
        eprintln!("refused: --measure scrolls the mail list, which the design page does not have");
        std::process::exit(2);
    }
    // The design page's alone: the four screens' list builds the rows of an 800 px window.
    let size = args.iter().position(|a| a == "--size").filter(|_| design).map_or(WINDOW, |i| {
        let (w, h) = args.get(i + 1).and_then(|s| s.split_once('x')).expect("--size WxH");
        (w.parse().expect("--size width"), h.parse().expect("--size height"))
    });
    let mode = if args.iter().any(|a| a == "--measure") {
        Mode::Measure(Run::default())
    } else if args.iter().any(|a| a == "--drive") {
        Mode::Drive(Box::new(if design { Driver::design() } else { Driver::new() }), shots)
    } else {
        Mode::Open(shots)
    };
    ui_spike_common::meter::require_display();
    let open = matches!(mode, Mode::Open(_));
    // create_default_event_loop's body, as a driven run must not take the keyboard from a person working.
    let mut builder = EventLoop::builder();
    #[cfg(target_os = "macos")]
    builder.with_activate_ignoring_other_apps(open);
    let event_loop = builder.build().expect("event loop");
    event_loop.set_control_flow(ControlFlow::Wait);
    let (proxy, queue) = BlitzShellProxy::new(event_loop.create_proxy());
    let doc = DioxusDocument::new(
        if design {
            dioxus_page::design::vdom()
        } else {
            vdom(if matches!(mode, Mode::Measure(_)) { Screen::Mail } else { Screen::Board })
        },
        DocumentConfig {
            font_ctx: Some(build_single_font_ctx(if design { DESIGN_FONT } else { INTER })),
            ..Default::default()
        },
    );
    doc.vdom.provide_root_context(doc.inner.clone());
    // Fixed size: the list builds the rows of an 800 px window only.
    let attributes = WindowAttributes::default()
        .with_title(if design { "Design system: Dioxus on design.json" } else { "rust-ui-tree: Dioxus on design.json" })
        .with_surface_size(LogicalSize::new(size.0, size.1))
        // The design page holds no 800 px list, so it can follow a window a person resizes.
        .with_resizable(design && open)
        .with_active(open)
        // A driven window floats on every Space (capture::on_every_space), never taking the keyboard, and lets clicks through.
        .with_window_level(if open { WindowLevel::Normal } else { WindowLevel::AlwaysOnTop });
    let mut app = BlitzApplication::new(proxy, queue);
    app.add_window(WindowConfig::with_attributes(Box::new(doc) as _, DioxusNativeWindowRenderer::new(), attributes));
    event_loop
        .run_app(App { app, mode, built: false, visible: false, hidden: 0, frames_seen: 0, ended: false, pointer: PhysicalPosition::new(0.0, 0.0), bad_shots: 0 })
        .expect("run");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(width: u32, height: u32, c: u8) -> Picture {
        Picture { width, height, rgba: [c, c, c, 255].repeat((width * height) as usize) }
    }

    #[test]
    fn a_capture_is_compared_under_its_title_bar_and_can_fail() {
        let doc = flat(4, 4, 200);
        let mut window = flat(4, 6, 200);
        // The title bar: the top 2 rows, which the document does not have.
        window.rgba[..4 * 4 * 2].fill(0);
        assert_eq!(matching(&window, &doc), 1.0);
        assert_eq!(matching(&flat(4, 6, 0), &doc), 0.0);
        // One bottom row wrong of 4.
        let mut half = flat(4, 6, 200);
        half.rgba[4 * 4 * 5..].fill(0);
        assert_eq!(matching(&half, &doc), 0.75);
    }
}
