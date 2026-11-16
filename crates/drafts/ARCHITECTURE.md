# nha/drafts

A message being written, its checks and its send through a channel. Follows `rust/ARCHITECTURE.md`; the Keys and
Database below are what the crate registers and creates, held so by dash-gate's
`a_plugin_s_doc_names_what_it_registers_and_creates`.

## Keys

- **Command:** `draft import`, `draft new`, `draft show`, `draft rates`, `draft revise` (`src/command.rs`).
- **Post:** `/drafts/send/*`, `/drafts/undo/*`, `/drafts/correct/*` (`src/card.rs`).
- **Element:** `mail/thread/10-drafts`, its card under a mail thread (`src/card.rs`).
- **Page:** `/drafts`, `/drafts/*`, `/d/*` (`src/pages.rs`); /drafts follows the drafts view (a Follows beside it), its
  list redrawn whole at each diff.
- **Tool, Channel, ContextProvider, Job, ViewDef, Item:** none.

## Database

- **File:** `drafts.sqlite`, plain SQL through `host.open`; every write is one `host.transact`, whose changeset the
  host keeps in the same commit (`dash_store::plugin`), the file's one event stream.
- **Tables:** `drafts`, `latest`, `draft_sends`, each an id and a record, one JSON text (`src/table.rs`). `latest`'s
  `pass:drafts` row dates the binary's last whole read of the older app's drafts: a draft it left unchanged is not
  written again, and its pages read it as of then.
- **Its view:** `drafts` (dash-views), each draft with its status and its send's state; /drafts follows it.

## Reaches

- **channel:** the draft's own `channel` field (`email` today).
- **element:** `mail/compose`.
- **post:** the comments plugin's comment on an anchor, for a correction.
- **context:** `comments/open-on`, his open comments on a draft's page; `drafts/read-in`, the binary's read of a draft
  the view misses (one the older app wrote since the last pass).
- **host:** `open` (also the pages' read of `pass:drafts`), `transact`, `view`, `after`, `now_ms`, `arm`.

## Split

- **tool:** none; a send goes through a `Channel`.
- **ui:** `card.rs`, the card's `Tree` (the design system's `draft-card`); `list.rs`, /drafts' list (`draft-list`) and a
  draft's page.
- **rest:** `pages.rs`, `command.rs`, `latest.rs`, `send.rs`, `posting.rs`, `outbox.rs`, `table.rs`, `lib.rs`.

## Not yet

- The older app is read into drafts.sqlite by the binary's latest pass (`dash/src/latest.rs`, `import`, and `import_one`
  after his write through this app, or on a page's miss through `drafts/read-in`), never by the plugin: that glue goes
  with the older app (rust-move-bridge).
- `outbox.rs` implements `registry::Slot`; it becomes an `Element` under `today/`.
- `host.arm` is not in the target `Host`; it becomes a `Command`'s own confirmation step.
- `draft import` reads its folder with `std::fs`; it moves under `host.dir()`.
- A command run in a terminal writes the file itself, not through serve (section 5 point 3).
