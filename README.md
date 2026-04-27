# amux

Friendly client for persistent local and remote terminal sessions.

## What It Is

`amux` is a Rust CLI for running persistent terminal sessions without exposing
tmux's interaction model.

The first implementation is intentionally tmux-backed:

- local sessions use the local tmux server
- remote sessions will use tmux on the remote host
- the user-facing controls stay the same across local and remote targets
- tmux is treated as the session kernel, not the UX

The product goal is a location-transparent terminal workspace:

```text
amux
  target: local
  target: ssh://devbox

session
  panes
  persistent PTYs
  attach/detach
  mouse-friendly split control
```

This repository is at the early MVP stage. The current CLI checks tmux
availability, lists local tmux sessions, creates detached local sessions, opens
an amux-rendered session view, and provides a mouse-friendly dashboard for
selecting sessions and controlling panes.

## Design Stance

- hide tmux mechanics from the user
- keep local and remote workflows under one model
- build the TUI like a focused application, not like a prefix-key multiplexer
- use existing terminal engines and PTY/session backends before writing new ones
- keep the state model explicit enough for future automation and harnesses

`amux` is not trying to be a terminal emulator in the first phase. The initial
target is a better client for tmux-backed terminal sessions.

## Quick Start

Build and test:

```bash
cargo test
cargo run -p amux -- doctor
```

List local sessions:

```bash
cargo run -p amux -- session list
```

Create a detached session that runs a command:

```bash
cargo run -p amux -- new towerlab --cwd /root/towerlab -- bash
```

Open a session:

```bash
cargo run -p amux -- attach towerlab
```

Open the dashboard:

```bash
cargo run -p amux --
```

If no sessions are running, the dashboard opens as a launcher. Press `Enter` or
click `Start Session` to create a shell session in the current directory and
open it immediately.

Dashboard controls:

```text
Mouse click        select a visible session or pane row
Mouse click        press New / Open / Right / Down / Close / Refresh
Wheel              move selection in the hovered list
Tab                switch keyboard focus between sessions and panes
Up / Down          move selection in the focused list
Enter              open selected session, or start the selected launcher option
Esc                quit or cancel command mode
Ctrl-A             enter command mode

Command mode:
n                  new session
a                  open selected session
v                  split selected pane right
h                  split selected pane down
x                  close selected pane
r                  refresh sessions
q                  quit dashboard
```

`amux attach` opens the amux session view. tmux stays behind the backend
boundary; amux renders panes, forwards normal input to the selected pane, and
uses `Ctrl-A` only for amux-level commands.

The session view preserves common ANSI colors from pane output and maps the
selected pane's tmux cursor position back to the visible terminal cursor.
Pane content is rendered inside an amux border; tmux is resized to the inner
content area so pane output and cursor coordinates stay aligned. The selected
pane's adjacent borders are highlighted. The outer frame uses rounded corners,
and split separators are drawn in tmux's gap cells without hard-connecting to
the outer frame.

Session view controls:

```text
Typing / Enter     send input to the selected pane
Mouse click        select a pane
Wheel              scroll the hovered pane history
Ctrl-A             enter command mode

Command mode:
v                  split selected pane right
h                  split selected pane down
x                  close selected pane
r                  refresh
q                  detach from the session view
Esc                cancel command mode
```

When the last pane exits, the session view closes and restores the terminal.

## Current CLI

```text
amux doctor
amux target list
amux session list
amux new <NAME> [--cwd <DIR>] [-- <COMMAND>...]
amux attach <NAME>
amux
```

Running `amux` with no subcommand opens the dashboard.

Only the local target is implemented today. Remote targets are part of the
first architectural goal, but the CLI is shaped so local and remote behavior can
share the same session model.

## Repository Layout

```text
crates/amux/
  src/cli.rs      command definitions and dispatch
  src/model.rs    target and session types
  src/tmux.rs     local tmux backend
  src/tui.rs      ratatui/crossterm dashboard
  src/main.rs     binary entrypoint

docs/
  architecture.md
  roadmap.md
  tui-debugging.md
```

## Development

```bash
cargo fmt
cargo test
cargo clippy --all-targets -- -D warnings
```

For TUI changes, also run the built binary in a real terminal or PTY and verify
mouse, resize, refresh, quit, dashboard open, and session-view input behavior.

## License

MIT
