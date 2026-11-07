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
use ui_spike_common::picture::{Picture, write_png};
use ui_spike_common::{DESIGN_FONT, INTER};

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

/// Feeds a driver's script through UiEvents; `shots` paints each picture with the CPU renderer.
fn run(vdom: dioxus::prelude::VirtualDom, font: &[u8], size: (u32, u32), driver: &mut Driver, shots: Option<&std::path::Path>) {
    let mut doc = DioxusDocument::new(
        vdom,
        DocumentConfig {
            font_ctx: Some(build_single_font_ctx(font)),
            viewport: Some(Viewport::new(size.0, size.1, 1.0, ColorScheme::Light)),
            ..Default::default()
        },
    );
    doc.initial_build();
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
            Tick::Idle => {}
            Tick::Shot(name) => {
                if let Some(dir) = shots {
                    use anyrender::ImageRenderer;
                    let mut renderer = anyrender_vello_cpu::VelloCpuImageRenderer::new(size.0, size.1);
                    let mut rgba = vec![0; (size.0 * size.1 * 4) as usize];
                    let mut inner = doc.inner_mut();
                    renderer.render(|scene| blitz_paint::paint_scene(scene, &mut inner, 1.0, size.0, size.1, 0, 0), &mut rgba);
                    write_png(&dir.join(format!("{name}.painted.png")), &Picture { width: size.0, height: size.1, rgba });
                }
            }
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
}

#[test]
fn the_script_passes_and_fails_where_the_window_does() {
    let mut driver = Driver::new();
    run(vdom(Screen::Board), INTER, (WINDOW.0 as u32, WINDOW.1 as u32), &mut driver, None);
    assert_eq!(driver.failed(), dioxus_page::script::RECORDED_FAILURES);
    assert_eq!(driver.results.len(), 24);
}

/// `DESIGN_SHOTS=<dir>` writes its pictures, for reading the page on a locked Mac; the results page's come from the window.
#[test]
fn the_design_script_passes_and_fails_where_the_window_does() {
    let shots = std::env::var_os("DESIGN_SHOTS").map(std::path::PathBuf::from);
    if let Some(dir) = &shots {
        std::fs::create_dir_all(dir).expect("shots folder");
    }
    let mut driver = Driver::design();
    run(dioxus_page::design::vdom(), DESIGN_FONT, (1483, 856), &mut driver, shots.as_deref());
    assert!(driver.as_recorded(), "failed {:?}: {}", driver.failed(), driver.lines.join("\n"));
}

/// Every family and section of /design, in its order: DesignLive is the page this one proves can be styled.
#[test]
fn the_families_are_design_live_s() {
    let source = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../private/dashboard/lib/dashboard_web/live/design_live.ex");
    // rust/ alone rides to the CI mirror, so there this has nothing to compare.
    let Ok(source) = std::fs::read_to_string(source) else {
        return;
    };
    let block = &source[source.find("@families [").expect("@families")..source.find("@doc \"The families").expect("its end")];
    let theirs: Vec<(&str, &str)> = block
        .split("{\"")
        .skip(1)
        .map(|entry| {
            let mut quoted = entry.split('"');
            let id = quoted.next().expect("an id");
            quoted.next();
            (id, quoted.next().expect("a title"))
        })
        .collect();
    let ours: Vec<(&str, &str)> = dioxus_page::design::FAMILIES
        .iter()
        .flat_map(|(slug, title, _, sections)| std::iter::once((*slug, *title)).chain(sections.iter().copied()))
        .collect();
    assert_eq!(ours, theirs);
}

/// Every colour on the design page comes from design.json through theme.css: no hex, rgb or hsl of its own.
#[test]
fn the_design_css_has_no_colour_of_its_own() {
    let css = dioxus_page::design::DESIGN_CSS;
    assert!(css.contains("var(--color-primary)"), "the read is of the wrong file");
    let raw: Vec<&str> = css
        .lines()
        .filter(|l| !l.trim_start().starts_with("/*"))
        .filter(|l| l.contains("rgb(") || l.contains("rgba(") || l.contains("hsl(") || l.split(": #").count() > 1 || l.split(" #").skip(1).any(|c| c.starts_with(|x: char| x.is_ascii_hexdigit())))
        .collect();
    assert!(raw.is_empty(), "{raw:#?}");
}

#[test]
fn each_section_has_a_state_in_both_themes() {
    use dioxus_page::design::FAMILIES;
    for (i, (_, _, _, sections)) in FAMILIES.iter().enumerate() {
        let mut vdom = dioxus_page::design::family_vdom(i);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        for (id, _) in *sections {
            let from = html.find(&format!("id=\"s-{id}\"")).unwrap_or_else(|| panic!("no section {id}"));
            let rest = &html[from + 1..];
            let section = &rest[..rest.find("id=\"s-").unwrap_or(rest.len())];
            assert!(section.contains("app light ds-cell") && section.contains("app dark ds-cell"), "{id} draws no state");
        }
    }
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
