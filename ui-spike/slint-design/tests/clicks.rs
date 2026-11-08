//! What a click must leave alone. One test in its own binary: one platform a process.
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition};
use slint_design::script::{find, value};

#[test]
fn a_click_loses_nothing() {
    let headless = slint_design::Headless::new();
    let app = slint_design::app();
    headless.size(1200, 800);
    app.show().expect("show");
    app.set_view(6);
    let button = PointerEventButton::Left;
    let drag_to = |x: f32, y: f32| {
        let at = find(&app, "li-card-478").expect("the card").absolute_position();
        let position = LogicalPosition::new(at.x + 60.0, at.y + 30.0);
        let to = LogicalPosition::new(x, y);
        app.window().dispatch_event(WindowEvent::PointerMoved { position });
        app.window().dispatch_event(WindowEvent::PointerPressed { position, button });
        app.window().dispatch_event(WindowEvent::PointerMoved { position: to });
        app.window().dispatch_event(WindowEvent::PointerReleased { position: to, button });
        // The settle's 240 ms: a card still out of the board's clip cannot be read.
        std::thread::sleep(std::time::Duration::from_millis(300));
        slint::platform::update_timers_and_animations();
    };
    drag_to(1100.0, 300.0);
    assert!(value(&app, "li-card-478").starts_with("0 "), "over Hidden columns: {}", value(&app, "li-card-478"));
    drag_to(100.0, 300.0);
    assert!(value(&app, "li-card-478").starts_with("0 "), "over the sidebar: {}", value(&app, "li-card-478"));
    drag_to(700.0, 20.0);
    assert!(value(&app, "li-card-478").starts_with("0 "), "over the top bar: {}", value(&app, "li-card-478"));
    drag_to(700.0, 300.0);
    assert!(value(&app, "li-card-478").starts_with("1 "), "over In Progress: {}", value(&app, "li-card-478"));

    // Typed words survive a new comment.
    app.set_draft("typed".into());
    app.invoke_comment();
    assert_eq!(app.get_draft(), "typed");
    // Leaving the view that holds a pick closes its toolbar.
    app.set_picked(3);
    app.set_view(0);
    slint::platform::update_timers_and_animations();
    assert_eq!(app.get_picked(), -1);
}
