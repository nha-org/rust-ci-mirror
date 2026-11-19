//! The live parts' base (task rust-js-port-small): what a component placed live writes in the page and what its renderer
//! gives it. Below the components and `part`, which draws them again, so neither loops back.
use dioxus::prelude::*;
use std::{future::Future, pin::Pin, rc::Rc};

/// The placeholder's class: the nth on a page is where its renderer moves the nth part drawn live.
pub const PLACE: &str = "live-part";
/// The attribute of a part drawn live, its place's number: the renderer moves it into that placeholder.
pub const GRAFT: &str = "data-graft";

/// What a copy asks its renderer: `text`, after the page's origin when `origin`; on a failure, the element `select` names
/// selected, so he copies it by hand.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Clip {
    pub text: String,
    pub origin: bool,
    pub select: Option<String>,
}

/// What a copy did: written to the clipboard; not, its `select` selected for him to copy by hand; or neither.
#[derive(Clone, Copy, Debug, PartialEq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Copied {
    Yes,
    Selected,
    No,
}

/// A copy written, answered with what it did.
pub type Clips = Rc<dyn Fn(Clip) -> Pin<Box<dyn Future<Output = Copied>>>>;

/// What runs a page's parts gives them: the window (dash's live.rs, the Mac's clipboard) or a liveview page (the browser's,
/// through liveview's eval). `submit` sends the form of that id, as its button would (a menu's digit). `graft` moves the nth
/// part, once mounted, into its place: the window's; the web's liveview client moves them itself.
#[derive(Clone)]
pub struct PartHost {
    pub copy: Clips,
    pub submit: Rc<dyn Fn(&str)>,
    pub graft: Option<Rc<dyn Fn(usize, MountedEvent)>>,
}

/// A component where the page draws it, live: `picture` in its place, its props beside it, `part` naming its kind, for the
/// renderer that runs it (`part::Part` reads them back).
pub fn placed(part: &str, props: &impl serde::Serialize, picture: Element) -> Element {
    let mut json = serde_json::to_value(props).unwrap_or_default();
    if let Some(fields) = json.as_object_mut() {
        fields.insert("part".into(), part.into());
    }
    // a span: the text form reads it as a word, so a row of copies stays one line
    rsx! { span { class: PLACE, "data-part": "{json}", {picture} } }
}
