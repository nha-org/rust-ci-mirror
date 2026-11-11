use super::button::Button;
use dioxus::prelude::*;

/// The comment compass, as the older app's top bar carried it (`DashboardWeb.CommentThread.comment_page_button/1`; @nha,
/// 2026-11-09: « the menu is getting nice. It needs the compass still. »): a tap, then a tap on any element of the page, comments
/// on it. Its drawing is `DashboardWeb.Library.compass/1`'s; `data-pick-button` lets an older page's pick mark it pressed and
/// take a second tap as a cancel, as it did its own button. A Button of the design system (rust/CLAUDE.md rule 6). `id`: the
/// menu's, which forms.rs's COMPASS_JS binds; none on /design's Page comments, which draws it too (task
/// rust-design-page-comments-compose). Its name is also its text form's (`.text-only`): an icon says nothing to an agent.
pub fn compass(id: Option<&'static str>) -> Element {
    rsx! {
        Button { look: "icon", r#type: "button", id, "data-pick-button": "", aria_pressed: "false",
            title: "Comment on an element: tap it next", aria_label: "Comment on an element",
            span { class: "text-only", "Comment on an element" }
            svg { view_box: "0 0 24 24", width: "16", height: "16", fill: "none", stroke: "currentColor", stroke_width: "2",
                stroke_linecap: "round", stroke_linejoin: "round", "aria-hidden": "true",
                path { d: "M12 22a10 10 0 1 0 0-20 10 10 0 0 0 0 20z" }
                path { d: "M16 8l-2 6-6 2 2-6z" }
            }
        }
    }
}

/// The compass's pick bar, the older app's (comment_thread.ex's `data-pickbar`): shown while the compass picks, it says what
/// the next tap does, and takes the whole page instead of an element, or stops the pick (task rust-comment-files-filter).
/// forms.rs's COMPASS_JS binds its buttons and NAV_CSS shows it while the compass is pressed.
pub fn pick_bar() -> Element {
    rsx! {
        div { class: "pick-bar", role: "toolbar", aria_label: "Comment on",
            span { "Tap the element to comment on" }
            Button { look: "quiet", r#type: "button", "data-whole-page": "", "Whole page" }
            Button { look: "quiet", r#type: "button", "data-pick-cancel": "", "Cancel" }
        }
    }
}
