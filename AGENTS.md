Principles for agents contributing to this repository.

## Mission

Build a friendly local and remote client for persistent terminal sessions.

The project has two goals:

1. Make tmux-backed persistence usable without tmux's interaction model.
2. Keep the client and state model clean enough for future automation and harnesses.

## Core Principles

1. **KISS**
   - Prefer the simplest solution that works.
   - Avoid premature abstraction.
   - If one clear module is enough, do not invent a framework.

2. **Do not leak tmux UX**
   - tmux may be the first backend, but it should not define the user-facing interaction model.
   - Do not require users to know prefix keys, pane ids, copy mode, or tmux config details.
   - Keep commands and UI phrased in `target`, `session`, and `pane` concepts.

3. **Local and remote share one model**
   - Local sessions and SSH-backed sessions should use the same client behavior.
   - Avoid local-only assumptions in shared types and UI state.
   - Make host/location differences explicit at backend boundaries.

4. **Do not write a terminal emulator early**
   - Use tmux and existing terminal/TUI libraries for the first working path.
   - Build session control and a better client before replacing the PTY backend.

5. **Conventional Commits with real bodies**
   - Use Conventional Commits for every commit.
   - Do not write title-only commits.
   - In the commit body, explain both what changed and why the change was made.

## Navigation

Use docs to understand constraints first. Keep this file coarse-grained and put
detailed design notes in docs when they become durable.

### Read these docs first

- `README.md` - project overview, commands, and local development.
- `docs/architecture.md` - current boundaries and backend model.
- `docs/roadmap.md` - staged implementation plan.

### Read these docs when the task matches

- Terminal UI behavior:
  - Read `docs/tui-debugging.md` before changing `crates/amux/src/tui.rs`.
- Backend or session model changes:
  - Read `docs/architecture.md` before editing `crates/amux/src/model.rs` or `crates/amux/src/tmux.rs`.

## Source Map

- `crates/amux/src/cli.rs` - command definitions and command dispatch.
- `crates/amux/src/main.rs` - binary entrypoint.
- `crates/amux/src/model.rs` - target/session/pane-facing data structures.
- `crates/amux/src/tmux.rs` - tmux backend integration.
- `crates/amux/src/tui.rs` - interactive dashboard.
- `docs/` - durable design, roadmap, and verification notes.

## Engineering Rules

- Rust everywhere.
- Keep command output scriptable unless the command is explicitly interactive.
- Validate external command output at backend boundaries.
- Keep UI state separate from backend command execution.
- Prefer direct, testable parsing helpers over ad hoc string handling in command dispatch.
- Update README and docs when commands, architecture, or verification expectations change.

## Verification Requirements

- Minimum bar for meaningful changes:
  - `cargo fmt`
  - `cargo test`
  - `cargo clippy --all-targets -- -D warnings`
- For TUI changes, run the built CLI under a real terminal or PTY and verify scroll, selection, refresh, attach, resize, and quit behavior.
- When using temporary tmux sessions for verification, close them before finishing.

## Collaboration Preferences

- Keep implementations small and legible.
- Optimize for code that future agents can read in one pass.
- Hold the requested quality bar; do not quietly retreat to a weaker, partial, or more convenient option.
- If the original goal appears infeasible or materially more complex than requested, discuss the constraint before lowering the target.
