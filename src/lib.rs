//! Oko — "the eye": a dashboard tab inside iTerm2 that shows what every other tab in the
//! window is doing.
//!
//! The library half is the iTerm2 client and the status file. Three binaries share it:
//! `oko`, the dashboard itself; `oko-hook`, which Claude Code runs on its own events; and
//! `probe`, the headless diagnostic that Phase 1 of `specs/tab_dashboard_spec.md` left
//! behind.

pub mod iterm;
pub mod status;
