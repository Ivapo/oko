//! The iTerm2 client: the transport Phase 1 proved, and the row model Phase 2 needs.
//!
//! The split is between `client` — frames, requests, subscriptions, one connection — and
//! `watch`, which is everything that knows what a *row* is: which window is ours, which
//! sessions are in it, in which tab, and what changed.
//!
//! Setup — enabling the API, how authorization works — is in `rules/iterm-api.md`.
//! **`#![allow(dead_code)]` is the cost of having no lib target.** These modules are
//! compiled into each binary that declares them (§2.16), and a binary crate seeds dead-code
//! analysis from `main` — so every item a given binary does not reach would warn. Three
//! binaries lose dead-code cover over this file, and nothing buys it back.
//!
//! `unused_imports` joins it here and only here: the `pub use` re-exports below stop being
//! public API the moment this module lives inside a bin crate.

#![allow(dead_code, unused_imports)]

// Generated from the vendored schema; its shape is iTerm2's business, not ours.
#[allow(clippy::all, clippy::pedantic)]
pub mod api {
    // prost writes one file per proto package; ours is `iterm2`.
    include!(concat!(env!("OUT_DIR"), "/iterm2.rs"));
}

mod client;
mod watch;

pub use client::{Client, socket_path};
pub use watch::{
    Cmd, Event, OKO_NAME, Placed, Row, Snapshot, Watcher, flatten, own_tty, resolve_own_session,
    row_variables,
};
