//! What an mdview pane has open, read off its command line (§2.19).
//!
//! A pure function and nothing else: no client, no row, no clock. It takes the string iTerm2
//! reports as a session's `commandLine` and answers a base name or nothing, and every one of
//! its tests is a command line captured from a real mdview 0.6.0 under iTerm2 3.7.2 with
//! `oko-probe mdview` — the table §2.19 measured, verbatim.
//!
//! **The launch arguments are the open file, and that is a fact about mdview, not about
//! `commandLine`.** mdview takes exactly one operand, refuses a second, and nothing inside it
//! opens another file; §2.17 rejected this same variable for Helix because Helix moves off its
//! launch arguments. So this reads no screen and no user interface — iTerm2's record of the
//! process's arguments is structured, and §2.7 is not reopened. The day mdview can open a
//! second file without exiting, this is confidently wrong after the first switch, and the
//! remedy is for mdview to say which file it has open, not for Oko to read its screen.
//!
//! **`commandLine` is argv with quotes added, and this admits no escape.** Measured: an
//! argument carrying whitespace, `'` or `*` is double-quoted; one carrying `"`, `\` or `$` is
//! single-quoted; one carrying none is bare. What iTerm2 writes for an argument that needs
//! both kinds of quote was never measured, so it answers nothing rather than a name un-escaped
//! by a rule nobody checked — absence, never a wrong name, which is §2.17's principle kept.
//!
//! **A submodule of `iterm` rather than a root module**, for the reason `helix` is one:
//! declared at a binary's root it would have to be added to `oko` and `oko-probe` both, and
//! `oko-hook` — which declares `status` alone — must not reach it at all.

/// The one job name a command line is read for (§2.19).
///
/// `jobName` reads this for mdview reached by a full path, a symlink or `exec -a` alike — it is
/// the process's own name — while `commandLine`'s first word is argv[0]'s base name, which the
/// last two change. That disagreement is what [`open_file`]'s first rule is about.
pub const JOB_NAME: &str = "mdview";

/// argv[0] as a plain `mdview` writes it, and the one space iTerm2 puts after it.
///
/// **The only spelling that marks where argv[0] ends.** A symlink or an `exec -a` name may
/// itself contain a space, so a command line beginning any other way cannot be split into its
/// arguments with confidence, and answers nothing.
const PREFIX: &str = "mdview ";

/// The base name of the file an mdview command line names, or `None` (§2.19).
///
/// Five rules, each load-bearing: the literal `mdview ` prefix; exactly one argument after it,
/// bare, double-quoted or single-quoted, with no escape anywhere; not beginning with `-`; no
/// control character; and the answer is what follows the last `/`, an empty one being nothing.
///
/// **`None` is the only other answer, and it is final**: a command line cannot be covered for
/// a moment the way a screen can, so unlike Helix there is no "leave it as it was".
pub fn open_file(command_line: &str) -> Option<String> {
    let argument = one_argument(command_line.strip_prefix(PREFIX)?)?;
    // mdview exits on every such argument, so a live one never holds it — and a
    // `mdview --help` caught mid-exit must not read as a file called `--help`.
    if argument.starts_with('-') {
        return None;
    }
    // A tab is a legal file-name byte and iTerm2 double-quotes one faithfully, but no table cell
    // can draw it and the stream would carry it raw.
    if argument.chars().any(char::is_control) {
        return None;
    }
    // What follows the *last* `/` and nothing else: `dir/` is a directory, not a file called
    // `dir`, so this is deliberately not `src/iterm/helix.rs:base_name`.
    let name = argument.rsplit('/').next()?;
    (!name.is_empty()).then(|| name.to_string())
}

/// The whole of what follows argv[0], as argv holds it — or `None` unless it is exactly one
/// argument in one of the three shapes iTerm2 was measured to write.
///
/// **In each shape the text is argv verbatim, because no escape is admitted anywhere**: a bare
/// argument carries no whitespace, quote, `\`, `$` or backtick; a double-quoted one no `"`,
/// `\`, `$` or backtick inside; a single-quoted one no `'` inside. A second argument always
/// breaks one of those, since the space between two arguments is outside any quote.
fn one_argument(rest: &str) -> Option<&str> {
    if let Some(inner) = rest.strip_prefix('"') {
        let inner = inner.strip_suffix('"')?;
        return (!inner.contains(['"', '\\', '$', '`'])).then_some(inner);
    }
    if let Some(inner) = rest.strip_prefix('\'') {
        let inner = inner.strip_suffix('\'')?;
        return (!inner.contains('\'')).then_some(inner);
    }
    let bare =
        !rest.chars().any(|c| c.is_whitespace() || matches!(c, '"' | '\'' | '\\' | '$' | '`'));
    bare.then_some(rest)
}

#[cfg(test)]
mod tests {
    //! Every command line in the first two tests is one of §2.19's table, captured on
    //! 2026-09-21 from a real mdview 0.6.0 under iTerm2 3.7.2 with `oko-probe mdview` and
    //! checked byte for byte against the spec. **Not one of them is hand-typed**, and raw
    //! literals keep them verbatim — the quotes are iTerm2's, which is the whole thing under
    //! test. The rejections after them are constructed, and say which rule each one reaches.

    use super::*;

    fn file(name: &str) -> Option<String> {
        Some(name.to_string())
    }

    #[test]
    fn every_measured_command_line_names_its_file() {
        // The table's first row and its full-path row share `mdview plain.md`: eleven distinct
        // lines, not twelve.
        for (line, name) in [
            (r#"mdview plain.md"#, "plain.md"),
            (r#"mdview ../mdv/plain.md"#, "plain.md"),
            (r#"mdview ./-dash.md"#, "-dash.md"),
            (r#"mdview "with space.md""#, "with space.md"),
            (r#"mdview "sub dir/nested.md""#, "nested.md"),
            (r#"mdview "two  spaces.md""#, "two  spaces.md"),
            (r#"mdview "star*.md""#, "star*.md"),
            (r#"mdview "it's.md""#, "it's.md"),
            (r#"mdview 'q"uote.md'"#, r#"q"uote.md"#),
            (r#"mdview 'back\slash.md'"#, r#"back\slash.md"#),
            (r#"mdview 'd$x.md'"#, "d$x.md"),
        ] {
            assert_eq!(open_file(line), file(name), "{line}");
        }
    }

    #[test]
    fn a_renamed_argv0_answers_nothing() {
        // `jobName` read `mdview` for both; `commandLine`'s first word is argv[0]'s base name.
        assert_eq!(open_file(r#"mdv-link plain.md"#), None);
        assert_eq!(open_file(r#"fakename plain.md"#), None);
    }

    #[test]
    fn a_second_argument_answers_nothing() {
        assert_eq!(open_file("mdview a.md b.md"), None);
        assert_eq!(open_file(r#"mdview "a b.md" c.md"#), None);
    }

    #[test]
    fn a_flag_is_never_a_file() {
        assert_eq!(open_file("mdview --help"), None);
        // Quoted or not: the rule is about the argument, not about how iTerm2 wrote it.
        assert_eq!(open_file(r#"mdview "-x y.md""#), None);
    }

    #[test]
    fn a_control_character_answers_nothing() {
        assert_eq!(open_file("mdview \"tab\tx.md\""), None);
    }

    #[test]
    fn an_escape_is_never_unescaped() {
        // Whatever iTerm2 writes for an argument needing both quotes, it needs an escape inside
        // one of them — and a name un-escaped by an unmeasured rule is a guess.
        assert_eq!(open_file(r#"mdview "back\slash.md""#), None);
        assert_eq!(open_file(r#"mdview 'it'\''s.md'"#), None);
    }

    #[test]
    fn an_empty_base_name_is_nothing() {
        assert_eq!(open_file("mdview dir/"), None);
        // Nor is no argument at all, or a shell's own command line.
        assert_eq!(open_file("mdview"), None);
        assert_eq!(open_file("mdview "), None);
        assert_eq!(open_file("-zsh"), None);
    }
}
