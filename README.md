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
availability, lists local tmux sessions, creates detached local sessions,
attaches to a session, and opens a mouse-friendly dashboard for selecting
sessions and controlling panes.

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

Attach to a session:

```bash
cargo run -p amux -- attach towerlab
```

Open the dashboard:

```bash
cargo run -p amux --
```

If no sessions are running, the dashboard opens as a launcher. Press `Enter` or
click `Start Session` to create a shell session in the current directory and
attach to it immediately.

Dashboard controls:

```text
q / Esc      quit
r            refresh sessions
Tab          switch keyboard focus between sessions and panes
j / Down     select next session or pane
k / Up       select previous session or pane
Enter        attach selected session, or start the selected launcher option
|            split selected pane right
-            split selected pane down
x            close selected pane
Mouse click  select a visible session or pane row
Mouse click  press New / Attach / Right / Down / Close / Refresh
Wheel        move selection in the hovered list
```

`amux attach` enables tmux mouse support for the selected session before
attaching, so tmux itself can use mouse selection and pane resizing while the
session is attached.

## Current CLI

```text
amux doctor
amux target list
amux session list
amux new <NAME> [--cwd <DIR>] [-- <COMMAND>...]
amux attach <NAME>
amux
amux tui
```

Running `amux` with no subcommand opens the dashboard. `amux tui` is kept as an
explicit alias.

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
mouse, resize, refresh, quit, and attach behavior.

## License

MIT
