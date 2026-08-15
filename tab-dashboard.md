# Tab Dashboard (Oko)

## Idea

**Name:** Oko — "the eye" in Russian.

A dashboard tab inside iTerm2. This tab shows the status of every other tab in the same window. For a plain tab, it shows the current directory and the running process. For a tab that runs Claude Code, it shows a status: working, waiting for input, or ready for a prompt. You select a row and press Enter to jump to that tab.

This is a lighter alternative to herdr, a tool similar to tmux, built for managing multiple agent sessions. This design does not spawn or manage sessions. It only observes tabs that you already open yourself in iTerm2.

## Design

### Tab enumeration

The dashboard connects to the iTerm2 Python API. This API is a local service, exposed by iTerm2 itself. It reports the list of windows, tabs, and sessions, in real time. The dashboard uses this list to build its rows.

### Status for a plain tab

For each session, the iTerm2 API exposes two live values: the current working directory, and the name of the process running in the foreground. The dashboard reads these two values directly. No extra setup is needed for a plain tab.

### Status for a Claude Code tab

If the foreground process name matches Claude Code, the dashboard checks a status file for that session. Claude Code hooks write this file.

A hook is a command that Claude Code runs on a specific event. Three events matter here:

- **UserPromptSubmit** — a new prompt was sent.
- **Notification** — Claude Code needs input, or needs permission for a tool.
- **Stop** — Claude Code finished its turn, and is ready for the next prompt.

Each hook writes one line to a small status file, tied to the session ID. Claude Code decides when each hook runs, and Claude Code runs the hook command. We only write the hook script, and read the file it writes. The dashboard never queries Claude Code directly.

### Session identity

Claude Code passes a session ID to each hook, in JSON, on standard input. iTerm2 also sets an environment variable for each pane, named TERM_SESSION_ID. The hook script reads both values, and writes them to the status file together. This lets the dashboard match a status update to the exact iTerm tab.

### Navigation

Each row in the dashboard holds a session ID. On Enter, the dashboard calls the iTerm2 API function that activates that session. This switches the focused tab, and raises the iTerm window if needed.

### Why a dashboard tab, and not a side panel

A true side panel needs a split pane, placed inside one specific tab. A split pane exists only inside that one tab. It does not appear in other tabs, unless the split is repeated in every tab, with a separate running copy of the dashboard in each one.

A dashboard tab avoids this cost. One single copy of the program runs, in one fixed tab. It stays reachable from any other tab, at the cost of not staying visible at the same time as your work.

## Tech stack

- **Language** — Rust.
- **TUI library** — ratatui.
- **iTerm2 Python API** — the official scripting interface for iTerm2. It gives live access to windows, tabs, sessions, and session variables.
- **Claude Code hooks** — configured once, in Claude Code's settings file. Global settings apply the hooks to every project.

### Why hooks, and not screen content

The iTerm2 API can also return the visible text of a pane. A program could search this text for patterns, such as a spinner or a permission prompt. This method needs no change to Claude Code. But it can break easily: a small change to the Claude Code interface can break the pattern match. Hooks avoid this problem, because they send structured data, not raw screen text.

## Rough scope (not yet started)

- [ ] Connect to the iTerm2 Python API, and list all sessions in the current window
- [ ] Read the current working directory and the foreground process name, per session
- [ ] Write a hook script for UserPromptSubmit, Notification, and Stop
- [ ] Register these hooks in Claude Code's global settings file
- [ ] Read the status files, and merge them with the plain-tab data
- [ ] Render the list as a table, inside a dedicated dashboard tab
- [ ] Add Enter-to-navigate, using the iTerm2 API's session-activate function

## Open questions

- What is the exact foreground process name for a Claude Code session? This name decides whether the dashboard treats a tab as a Claude Code tab.
- Should the dashboard watch for a closed session, so a stale status file does not linger?
- Does the dashboard need a refresh loop, or can the iTerm2 API push updates as they happen?

## Status

Idea stage — design discussed, no code written yet.
