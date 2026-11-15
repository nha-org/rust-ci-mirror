# nha/drafts

A message being written, its checks and its send through a channel. Follows `rust/ARCHITECTURE.md`; the Keys and
Database below are what the crate registers and creates.

## Keys

- **Command:** `draft import`, `draft new`, `draft show`, `draft rates` (`src/command.rs`).
- **Post:** `/drafts/send/*`, `/drafts/undo/*` (`src/card.rs`).
- **Element:** `mail/thread/10-drafts`, its card under a mail thread (`src/card.rs`).
- **Page, Tool, Channel, ContextProvider, Job, ViewDef, Item:** none.

## Database

- **File:** `drafts.sqlite`.
- **Tables:** `drafts`, `latest`, `draft_sends`.

## Reaches

- **channel:** the draft's own `channel` field (`email` today).
- **element:** `mail/compose`.
- **host:** `open`, `after`, `now_ms`, `arm`.

## Split

- **tool:** none; a send goes through a `Channel`.
- **ui:** `card.rs`, the card's `Tree`.
- **rest:** `command.rs`, `latest.rs`, `send.rs`, `posting.rs`, `outbox.rs`, `lib.rs`.

## Not yet

- The /drafts page lives in the binary (`dash/src/home.rs`); it moves here as a `Page`.
- `static ONE` in `card.rs` serialises read-then-write; it goes once `OpLog::write_if` exists.
- `outbox.rs` implements `registry::Slot`; it becomes an `Element` under `today/`.
- `host.arm` is not in the target `Host`; it becomes a `Command`'s own confirmation step.
- `draft import` reads its folder with `std::fs`; it moves under `host.dir()`.
