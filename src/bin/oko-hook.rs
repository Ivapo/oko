//! The hook Claude Code runs. Phase 3 of `specs/tab_dashboard_spec.md`.
//!
//!     oko-hook                    read one hook event on stdin, write one status file
//!     oko-hook --print-settings   print the registration block for ~/.claude/settings.json
//!
//! It runs *inside the pane*, which is the whole point: it is the one place where Claude
//! Code's session id and iTerm2's `TERM_SESSION_ID` are both readable, and §2.4's join is
//! made by writing them into the same file.
//!
//! **It is silent on both streams and exits 0 on every path**, including its own failures.
//! For `UserPromptSubmit` and `SessionStart`, Claude Code adds plain-text stdout to Claude's
//! context — a stray `echo` here would prepend text to the user's prompt in every Claude
//! session on the machine. And for `SessionStart` and `SessionEnd` it shows *stderr* to the
//! user, so a diagnostic there is just as visible. Set `OKO_HOOK_DEBUG` to get the errors
//! in `~/.oko/hook.log` instead.
//!
//! A binary rather than a shell script because parsing this JSON in shell needs `jq`, which
//! is present here and guaranteed nowhere.

use std::io::{Read, Write};

use anyhow::{Context, Result, bail};
use time::OffsetDateTime;

// `status` and nothing else: this binary has no `iterm` reference at all, and giving it
// one would compile perfectly and show up only as three extra tests (§4, Phase 8 check 3).
#[path = "../status.rs"]
mod status;

use status::{Entry, Status, remove_if_owned, write};

/// The two things it does. Claude Code runs this binary; a human runs it once, to get the
/// block to paste — so those are the only two lines that belong here.
const USAGE: &str = "\
oko-hook — the hook Claude Code runs, so Oko's status column has something to show.

usage:
  oko-hook                    read one hook event on stdin and write one status file
  oko-hook --print-settings   print the registration block for ~/.claude/settings.json
  oko-hook --help, -h         print this

Claude Code invokes the first form itself; you never run it by hand. Merge the second
form's output into ~/.claude/settings.json as a top-level `hooks` key, then restart any
Claude session you want in the table.

It is silent on both streams and exits 0 on every path, including its own failures — for
some events Claude Code feeds a hook's stdout into Claude's context, and shows its stderr
to the user. Set OKO_HOOK_DEBUG=1 to send errors to ~/.oko/hook.log instead.";

fn main() {
    // Nothing this program can fail at is worth showing a human mid-session (§2.3).
    if let Err(e) = run() {
        log_debug(&format!("{e:#}"));
    }
}

fn run() -> Result<()> {
    // Ahead of the stdin read, which is the whole point: matched on nothing but
    // `--print-settings`, this binary answered `--help` by *blocking* in `read_to_string`.
    // A hang is a worse first contact than a wrong answer, because nothing on screen says
    // what happened.
    if matches!(std::env::args().nth(1).as_deref(), Some("--help" | "-h")) {
        println!("{USAGE}");
        return Ok(());
    }

    if std::env::args().nth(1).as_deref() == Some("--print-settings") {
        println!("{}", settings_block()?);
        return Ok(());
    }

    let mut stdin = String::new();
    std::io::stdin().read_to_string(&mut stdin).context("reading the hook event on stdin")?;
    let event: serde_json::Value =
        serde_json::from_str(&stdin).context("parsing the hook event as JSON")?;

    // No iTerm2 identity to join on — Claude Code in Terminal.app, or under tmux. Write no
    // file at all, rather than one nothing can ever match or sweep (OQ-4).
    let Some(iterm_session_id) = iterm_session_id() else {
        return Ok(());
    };
    let claude_session_id =
        string(&event, "session_id").context("the hook event carries no session_id")?.to_string();
    let hook_event_name =
        string(&event, "hook_event_name").context("the hook event carries no hook_event_name")?;

    match action(hook_event_name, &event) {
        Some(Action::Write(status, tool)) => write(&Entry {
            iterm_session_id,
            claude_session_id,
            status,
            at: OffsetDateTime::now_utc(),
            tool,
        }),
        Some(Action::Delete) => remove_if_owned(&iterm_session_id, &claude_session_id),
        // An event we do not register for, or one whose matcher let through something we do
        // not act on. Doing nothing is the correct answer, not an error.
        None => Ok(()),
    }
}

/// What one hook firing does to the status file.
///
/// `Write` carries the tool in flight as well as the status, because [`write`] builds a
/// whole `Entry` and renames it over the file — there is no "leave that field alone", so
/// every event that writes has to decide it (§2.12).
enum Action {
    Write(Status, Option<String>),
    Delete,
}

/// §2.3's table, which is the referent for this function and the thing to change first.
///
/// The matchers in [`settings_block`] already narrow most of these; the checks here are the
/// second line of defence, and the `Notification` split cannot be done by matcher at all —
/// its two rows write opposite statuses, so only `notification_type` tells them apart.
///
/// **The tool in flight is one rule over that whole table: `PreToolUse` sets it, every other
/// event clears it** (§2.12). Enumerating ten answers would leave the eleventh event to a
/// guess; this way the rule is total by construction and stays total when a row is added.
fn action(hook_event_name: &str, event: &serde_json::Value) -> Option<Action> {
    // Every arm below writes no tool. The one that does is spelled out on its own.
    let idle = |status| Some(Action::Write(status, None));

    match hook_event_name {
        // `compact` fires on auto-compaction *mid-turn*: registered bare it would tell a
        // human "this agent is done, go prompt it" while it works. `fork` is excluded as
        // ambiguous rather than wrong — the next real event corrects it either way.
        "SessionStart" => match string(event, "source")? {
            "startup" | "resume" | "clear" => idle(Status::Ready),
            _ => None,
        },

        // The one event that records a tool: it starts one, and nothing has finished it.
        // A `working` carrying a tool ages against `OKO_TOOL_STALE_AFTER` instead, so a
        // quiet fifteen-minute build reports `working` rather than `stale`.
        "PreToolUse" => {
            Some(Action::Write(Status::Working, string(event, "tool_name").map(str::to_owned)))
        }

        // A tool finished, or a prompt was submitted, or auto mode denied a call and the
        // agent runs on. `PreToolUse` above is also what bounds the deny path: nothing fires
        // when a *human* answers no, so the next tool call is what clears a stale `waiting`.
        // `PostToolUse` is what clears the tool, which is the whole of "not in flight".
        "UserPromptSubmit" | "PostToolUse" | "PermissionDenied" => idle(Status::Working),

        // Nine notification types exist and only these six mean anything to a row.
        // `idle_prompt` is the one that must never reach `waiting`: it fires about a minute
        // after every turn, and subscribed bare it would make `ready` unreachable and stop
        // `waiting` meaning *this agent is blocked*.
        "Notification" => match string(event, "notification_type")? {
            "permission_prompt"
            | "agent_needs_input"
            | "elicitation_dialog"
            | "elicitation_url_dialog" => idle(Status::Waiting),
            "elicitation_complete" | "elicitation_response" => idle(Status::Working),
            _ => None,
        },

        // The turn is over, however it ended. `Stop` does not fire on a user interrupt and
        // an API error fires `StopFailure` instead — the interrupt is the hole OQ-4 (c)'s
        // staleness rule exists for, not something an eleventh registration fixes.
        "Stop" | "StopFailure" => idle(Status::Ready),

        "SessionEnd" => Some(Action::Delete),

        _ => None,
    }
}

/// The UUID after the colon in `TERM_SESSION_ID` — the join key (§2.4).
///
/// `ITERM_SESSION_ID` is read second, the **same two names in the same order** as
/// `src/iterm/watch.rs:resolve_own_session`. A pane exporting only the second would
/// otherwise join on Oko's side while this silently wrote nothing.
fn iterm_session_id() -> Option<String> {
    let raw = std::env::var("TERM_SESSION_ID")
        .or_else(|_| std::env::var("ITERM_SESSION_ID"))
        .ok()
        .filter(|s| !s.is_empty())?;
    // Observed shape: `w0t2p0:F79BC39C-…`. The `wNtMpK` prefix is a position and is
    // deliberately not the key — positions change when tabs are reordered.
    let uuid = raw.split_once(':').map_or(raw.as_str(), |(_, uuid)| uuid);
    (!uuid.is_empty()).then(|| uuid.to_string())
}

fn string<'a>(event: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    event.get(key)?.as_str()
}

/// The block to paste into `~/.claude/settings.json`.
///
/// Oko does not edit that file itself: it is the user's global configuration, outside this
/// repo, and a program that rewrites it is a program that can corrupt it.
fn settings_block() -> Result<String> {
    let exe = std::env::current_exe().context("finding this binary's own path")?;
    let exe = exe.canonicalize().unwrap_or(exe);
    let Some(command) = exe.to_str() else {
        bail!("this binary's path is not valid UTF-8, so it cannot go in a JSON settings file");
    };

    // `timeout` is 5 seconds everywhere except SessionEnd, whose hooks share a 1.5-second
    // budget that a longer per-hook timeout would *raise*. The default is 600 s, which is a
    // long time to hold up every tool call if a home directory ever hangs.
    let hook = serde_json::json!([{ "type": "command", "command": command, "timeout": 5 }]);
    let brief = serde_json::json!([{ "type": "command", "command": command }]);
    let any_tool =
        |name: &str| (name.to_string(), serde_json::json!([{ "matcher": "*", "hooks": hook }]));
    let unmatched = |name: &str| (name.to_string(), serde_json::json!([{ "hooks": hook }]));

    // Each event matches on a different field: tool events on `tool_name`, `Notification`
    // on `notification_type`, `SessionStart` on `source`. `UserPromptSubmit` and `Stop`
    // support no matcher at all. Every matcher below is load-bearing (§2.3).
    let hooks = serde_json::Value::Object(serde_json::Map::from_iter([
        (
            "SessionStart".to_string(),
            serde_json::json!([{ "matcher": "startup|resume|clear", "hooks": hook }]),
        ),
        unmatched("UserPromptSubmit"),
        any_tool("PreToolUse"),
        any_tool("PostToolUse"),
        any_tool("PermissionDenied"),
        (
            "Notification".to_string(),
            serde_json::json!([{
                "matcher": "permission_prompt|agent_needs_input|elicitation_dialog|elicitation_url_dialog|elicitation_complete|elicitation_response",
                "hooks": hook,
            }]),
        ),
        unmatched("Stop"),
        unmatched("StopFailure"),
        ("SessionEnd".to_string(), serde_json::json!([{ "hooks": brief }])),
    ]));

    serde_json::to_string_pretty(&serde_json::json!({ "hooks": hooks }))
        .context("rendering the settings block")
}

/// Errors go here, and only when asked for: every other channel is visible to a human
/// mid-session.
fn log_debug(message: &str) {
    if std::env::var_os("OKO_HOOK_DEBUG").is_none() {
        return;
    }
    let Ok(dir) = status::oko_dir() else {
        return;
    };
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    if let Ok(mut file) =
        std::fs::OpenOptions::new().create(true).append(true).open(dir.join("hook.log"))
    {
        let now = OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_default();
        let _ = writeln!(file, "{now} {message}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(name: &str, extra: serde_json::Value) -> serde_json::Value {
        let mut v = serde_json::json!({ "session_id": "c1", "hook_event_name": name });
        for (k, value) in extra.as_object().unwrap() {
            v[k] = value.clone();
        }
        v
    }

    fn written(name: &str, extra: serde_json::Value) -> Option<(Status, Option<String>)> {
        match action(name, &event(name, extra)) {
            Some(Action::Write(status, tool)) => Some((status, tool)),
            _ => None,
        }
    }

    #[test]
    fn only_pre_tool_use_records_a_tool() {
        assert_eq!(
            written("PreToolUse", serde_json::json!({ "tool_name": "Bash" })),
            Some((Status::Working, Some("Bash".to_string())))
        );

        // §2.12's rule is total over §2.3's table, so this list is that table minus the one
        // event above and minus `SessionEnd`, which deletes rather than writes.
        let tool = serde_json::json!({ "tool_name": "Bash" });
        let notification =
            |kind: &str| serde_json::json!({ "notification_type": kind.to_string() });
        let others = [
            ("SessionStart", serde_json::json!({ "source": "startup" }), Status::Ready),
            ("UserPromptSubmit", serde_json::json!({}), Status::Working),
            // These two carry a `tool_name` of their own and must still clear: a tool that
            // finished, or a call auto mode refused, is exactly "not in flight".
            ("PostToolUse", tool.clone(), Status::Working),
            ("PermissionDenied", tool, Status::Working),
            ("Notification", notification("permission_prompt"), Status::Waiting),
            ("Notification", notification("elicitation_complete"), Status::Working),
            ("Stop", serde_json::json!({}), Status::Ready),
            ("StopFailure", serde_json::json!({}), Status::Ready),
        ];
        for (name, extra, status) in others {
            assert_eq!(written(name, extra), Some((status, None)), "{name}");
        }
    }

    #[test]
    fn a_pre_tool_use_without_a_name_falls_back_to_the_ordinary_clock() {
        assert_eq!(written("PreToolUse", serde_json::json!({})), Some((Status::Working, None)));
    }

    #[test]
    fn idle_prompt_still_writes_nothing() {
        // The matcher that stops `ready` becoming unreachable, checked here too because
        // this function is the only filter that can split the two `Notification` rows.
        let e = event("Notification", serde_json::json!({ "notification_type": "idle_prompt" }));
        assert!(action("Notification", &e).is_none());
        let e = event("SessionStart", serde_json::json!({ "source": "compact" }));
        assert!(action("SessionStart", &e).is_none());
    }
}
