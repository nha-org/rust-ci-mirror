//! dash-sdk, the one crate of ours a plugin depends on (ARCHITECTURE.md section 2; @nha, 2026-11-15: « Plugins can only
//! depend on a specific shared plugin sdk crate. »): traits, plain data types and the registrations' shapes, each `dyn` or a
//! `fn` pointer, never a generic. Its root is its whole surface; each name is here because a plugin needs it, never copied
//! from dash-core name for name. dash-core implements it on the host side.
mod host;
mod ops;
mod plugin;
#[cfg(any(test, feature = "memory-log"))]
mod testing;
mod time;

pub use host::{Host, Later, Recurring, SignIn, Tick, Transport};
pub use ops::{FieldWrite, KEY_SEPARATOR, Op, OpLog, Value, WallClock};
pub use plugin::{
    ANCHORED, Answer, Asked, COMMENT_ON_ANCHOR, Channel, Check, Command, ContextProvider, Draw,
    Drawing, Element, HTML, JSON, Line, Message, Post, Row, Run, Slot, TEXT, Tree, View, channel,
    context, element, form_decoded, on_anchor, param, placed, post, segment_decoded,
};
// a tree's props, built without a crate of their own
pub use serde_json::json;
#[cfg(any(test, feature = "memory-log"))]
pub use testing::{MemoryLog, StandIn};
#[cfg(any(test, feature = "testing"))]
pub use testing::{dash_meter, dash_test};
pub use time::{MONTHS, date, days, is_day, iso, stamp};
