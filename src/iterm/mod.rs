//! The iTerm2 client: the transport Phase 1 proved, and the row model Phase 2 needs.
//!
//! The split is between `client` — frames, requests, subscriptions, one connection — and
//! `watch`, which is everything that knows what a *row* is: which window is ours, which
//! sessions are in it, in which tab, and what changed.
//!
//! Setup — enabling the API, how authorization works — is in `rules/iterm-api.md`.

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
    Cmd, Event, Placed, Row, Snapshot, Watcher, flatten, own_tty, resolve_own_session,
    row_variables,
};
