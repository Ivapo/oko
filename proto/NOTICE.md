# Third-party notice — `api.proto`

`api.proto` in this directory is **not** Oko's work and is **not** covered by Oko's MIT
licence. It is vendored verbatim from:

| | |
|---|---|
| Project | [gnachman/iTerm2](https://github.com/gnachman/iTerm2) |
| Path | `proto/api.proto` |
| Commit | `f4ca0004` |
| sha256 | `6f1a4e753e9c150d29454e9f83dfe91cc8d49465dde5f5aa3bda75cbd4482e31` |
| Fetched | 2026-08-14 |
| Licence | **GPL-2.0**, as iTerm2 is licensed |

It is included as the **interface definition** for iTerm2's scripting API — the wire format
a client must speak to talk to iTerm2 at all — and is compiled at build time by `protox`
and `prost-build` in `build.rs`.

## Why it is vendored rather than fetched

The alternative is downloading it during `build.rs`, which would make the build depend on
the network and on a URL staying up, and would silently change the wire format under a
`cargo build`. Pinning it by commit and hash means a build is reproducible and a schema
change is a visible diff.

## If you are redistributing Oko

Oko's own source is MIT (see `../LICENSE`). This file is not. Consult iTerm2's licence
directly for what its terms require of you — this notice records provenance, and is not
legal advice.

**Oko is published to crates.io as `oko-iterm2`, declaring `MIT AND GPL-2.0`.** That
expression describes what the tarball contains rather than choosing between the two:
`cargo package` includes this directory, so `api.proto` ships inside the crate. Installing
and running the binaries distributes nothing; taking a *library* dependency on the crate
links this file's generated schema into another artifact. `oko --licenses` prints the five
facts above from an installed copy, since neither this file nor `LICENSE` lands anywhere a
person who typed `cargo install` will look.
