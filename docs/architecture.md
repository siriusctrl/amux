# Architecture

`amux` starts as a friendly client for tmux-backed sessions.

The main boundary is:

```text
client/UI
  -> backend API
     -> tmux local backend
     -> tmux over SSH backend later
```

tmux owns process persistence in the first phase. `amux` owns the product model,
command vocabulary, UI, and future agent-aware state.

## Core Concepts

### Target

A target is the location where sessions run.

Current:

- `local`

Planned:

- `ssh://host`
- container or hosted development environments if they fit the same model

The client should not special-case interaction behavior based on target type.
Target-specific concerns belong in backend implementations.

### Session

A session is a persistent workspace entry. It can contain one or more panes and
it may keep running after the client disconnects.

In the tmux-backed implementation, a session maps to a tmux session.

### Pane

A pane is an interactive process surface. In the tmux-backed implementation, a
pane maps to a tmux pane. In future implementations, it may map to a direct PTY
managed by an `amuxd` process.

## Current Backend

`crates/amux/src/tmux.rs` shells out to `tmux` for the initial local backend.

Implemented operations:

- check tmux availability
- list sessions
- create detached sessions
- attach to sessions

This keeps the first version useful without committing to tmux as the permanent
backend.

## TUI Model

`crates/amux/src/tui.rs` uses `ratatui` and `crossterm`.

The UI should follow focused application behavior:

- visible status over hidden modes
- mouse and keyboard controls
- direct session selection
- minimal global key grammar
- no tmux prefix-key model

The TUI is a client surface. It should not embed tmux command details in UI
state.

## Non-Goals For The First Phase

- full terminal emulator implementation
- custom PTY daemon
- plugin system
- distributed orchestration
- agent-specific prompt/runtime logic

Those may become useful later, but they should be driven by a working local and
remote session client first.
