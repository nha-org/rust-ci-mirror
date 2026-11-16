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

/// Registers an entry with its origin stamped: `submit! { Page { path: .., .. } }` or `submit! { Command::usage("dash x") }`
/// is `inventory::submit!` with `origin: module_path!()` added, the registering module's path as the compiler writes it.
/// A plugin's identity is that origin, never its name typed by hand or read from a URL (rfc-superapp-plugins section 3;
/// ARCHITECTURE.md section 6 point 2). dash-gate's `an_entry_s_origin_is_stamped_never_typed` refuses a typed one.
#[macro_export]
macro_rules! submit {
    ($($ty:ident)::+ { $($field:tt)* }) => {
        ::inventory::submit! { $($ty)::+ { origin: ::core::module_path!(), $($field)* } }
    };
    ($($made:ident)::+ ( $($arg:tt)* )) => {
        ::inventory::submit! { $($made)::+ ( ::core::module_path!(), $($arg)* ) }
    };
}

pub use host::{Connection, Host, Later, Recurring, Row, SignIn, Tick, Transport, View};
pub use ops::{FieldWrite, KEY_SEPARATOR, Op, OpLog, Value, WallClock};
pub use plugin::{
    ANCHORED, Answer, Asked, COMMENT_ON_ANCHOR, Channel, Check, Command, ContextProvider, Draw,
    Drawing, Element, Follows, HTML, JSON, Line, Message, Page, Post, Reads, Run, Slot, TEXT, Tree,
    channel, context, element, form_decoded, lines, on_anchor, param, placed, post,
    segment_decoded,
};
// a plugin's SQL (`Host::open`): its params, rows and errors, without a dependency of its own
pub use rusqlite;
// a tree's props, built without a crate of their own
pub use serde_json::json;
#[cfg(any(test, feature = "memory-log"))]
pub use testing::{MemoryLog, StandIn};
#[cfg(any(test, feature = "testing"))]
pub use testing::{dash_meter, dash_test};
pub use time::{MONTHS, date, days, is_day, iso, stamp};
