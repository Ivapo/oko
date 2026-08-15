// Oko — the dashboard tab. Not built yet.
//
// Phase 1 of specs/tab_dashboard_spec.md is a transport spike and deliberately produces
// no dashboard: its whole output is `src/bin/probe.rs` and the answers that probe writes
// back into the spec. The table arrives in Phase 2.

fn main() {
    eprintln!(
        "oko: the dashboard is Phase 2 of specs/tab_dashboard_spec.md and is not built yet.\n\
         Phase 1 ships the transport spike instead:\n\
         \n\
         \x20   cargo run --bin probe            identity, then the sessions of this window\n\
         \x20   cargo run --bin probe activate <session-id>\n\
         \x20   cargo run --bin probe watch      live variable-change notifications\n\
         \n\
         Setup for all three lives in rules/iterm-api.md."
    );
    std::process::exit(2);
}
