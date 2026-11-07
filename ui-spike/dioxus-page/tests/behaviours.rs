//! The window's script with no window, on Linux CI too. Pointer and key events enter one layer lower, as UiEvents, and
//! time is simulated, so frame times mean nothing here: the numbers come from `dioxus-page --drive` in a real window.
use blitz_dom::{Document, DocumentConfig, build_single_font_ctx};
use blitz_traits::events::*;
use blitz_traits::shell::{ColorScheme, Viewport};
use dioxus::prelude::Key;
#[cfg(not(target_os = "macos"))]
use dioxus::prelude::NamedKey;
use dioxus_native_dom::DioxusDocument;
use dioxus_page::script::{Driver, Input, Tick};
use dioxus_page::{Screen, WINDOW, vdom};
use ui_spike_common::INTER;

fn coords(x: f64, y: f64) -> PointerCoords {
    PointerCoords { page_x: x as f32, page_y: y as f32, screen_x: x as f32, screen_y: y as f32, client_x: x as f32, client_y: y as f32 }
}

fn pointer(at: (f64, f64), button: MouseEventButton, buttons: MouseEventButtons) -> BlitzPointerEvent {
    BlitzPointerEvent {
        id: BlitzPointerId::Mouse,
        is_primary: true,
        coords: coords(at.0, at.1),
        button,
        buttons,
        mods: Default::default(),
        details: Default::default(),
        element: Default::default(),
        active_pointers: Default::default(),
    }
}

fn key(key: Key, text: Option<String>) -> UiEvent {
    UiEvent::KeyDown(BlitzKeyEvent {
        key,
        code: Default::default(),
        modifiers: Default::default(),
        location: Default::default(),
        is_auto_repeating: false,
        is_composing: false,
        state: KeyState::Pressed,
        text: text.map(Into::into),
    })
}

#[test]
fn the_script_passes_and_fails_where_the_window_does() {
    let mut doc = DioxusDocument::new(
        vdom(Screen::Board),
        DocumentConfig {
            font_ctx: Some(build_single_font_ctx(INTER)),
            viewport: Some(Viewport::new(WINDOW.0 as u32, WINDOW.1 as u32, 1.0, ColorScheme::Light)),
            ..Default::default()
        },
    );
    doc.initial_build();
    let mut driver = Driver::new();
    let (mut t, mut at, mut held) = (0.0, (0.0, 0.0), MouseEventButtons::None);
    loop {
        for _ in 0..8 {
            if !doc.poll(None) {
                break;
            }
        }
        t += 1.0 / 120.0;
        doc.inner_mut().resolve(t);
        let tick = driver.tick(&doc.inner());
        match tick {
            Tick::Done => break,
            Tick::Idle | Tick::Shot(_) => {}
            Tick::Input(Input::Move(x, y)) => {
                at = (x, y);
                doc.handle_ui_event(UiEvent::PointerMove(pointer(at, MouseEventButton::Main, held)));
            }
            Tick::Input(Input::Down) => {
                held = MouseEventButtons::Primary;
                doc.handle_ui_event(UiEvent::PointerDown(pointer(at, MouseEventButton::Main, held)));
            }
            Tick::Input(Input::Up) => {
                held = MouseEventButtons::None;
                doc.handle_ui_event(UiEvent::PointerUp(pointer(at, MouseEventButton::Main, held)));
            }
            Tick::Input(Input::Wheel(y)) => doc.handle_ui_event(UiEvent::Wheel(BlitzWheelEvent {
                delta: BlitzWheelDelta::Pixels(0.0, -y),
                coords: coords(at.0, at.1),
                buttons: held,
                mods: Default::default(),
                element: Default::default(),
            })),
            Tick::Input(Input::Text(c)) => doc.handle_ui_event(key(Key::Character(c.clone().into()), Some(c))),
            #[cfg(target_os = "macos")]
            Tick::Input(Input::Backspace) => doc.handle_ui_event(UiEvent::AppleStandardKeybinding("deleteBackward:".into())),
            #[cfg(not(target_os = "macos"))]
            Tick::Input(Input::Backspace) => doc.handle_ui_event(key(Key::Named(NamedKey::Backspace), None)),
        }
    }
    println!("{}", driver.lines.join("\n"));
    assert_eq!(driver.failed(), dioxus_page::script::RECORDED_FAILURES);
    assert_eq!(driver.results.len(), 24);
}

#[test]
fn the_theme_is_design_json_s() {
    let source = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../private/dashboard/assets/css/theme.css");
    // rust/ alone rides to the CI mirror, so there this has nothing to compare.
    let Ok(source) = std::fs::read_to_string(source) else {
        return;
    };
    let want = dioxus_page::theme::from_dash(&source);
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/src/theme.css");
    if std::env::var_os("UPDATE_THEME").is_some() {
        std::fs::write(path, &want).expect("write");
    }
    assert_eq!(std::fs::read_to_string(path).expect("read"), want, "UPDATE_THEME=1 cargo test");
}

#[test]
fn the_same_components_render_to_html() {
    let mut vdom = vdom(Screen::Board);
    vdom.rebuild_in_place();
    let html = dioxus_ssr::render(&vdom);
    assert!(html.contains("id=\"card-101\"") && html.contains("Trace the window"), "{html}");
}

#[test]
fn the_scene_is_read_from_its_file() {
    let (shapes, arrows) = dioxus_page::scene();
    assert_eq!((shapes.len(), arrows.len()), (5, 4));
    assert!(shapes.iter().all(|s| !s.label.is_empty()), "{shapes:?}");
}
