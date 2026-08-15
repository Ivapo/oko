//! Oko — "the eye": a dashboard tab inside iTerm2 that shows what every other tab in the
//! window is doing.
//!
//! The library half is the iTerm2 client. Two binaries share it: `oko`, the dashboard
//! itself, and `probe`, the headless diagnostic that Phase 1 of
//! `specs/tab_dashboard_spec.md` left behind.

pub mod iterm;
