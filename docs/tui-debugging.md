# TUI Debugging

The TUI should be verified as an actual terminal program, not only through unit
tests or raw ANSI output.

## Minimum Checks

Run:

```bash
cargo fmt
cargo test
cargo clippy --all-targets -- -D warnings
```

Then run the built binary in a real terminal or PTY:

```bash
cargo run -p amux -- tui
```

Verify:

- the alternate screen is restored after `q` or `Esc`
- with no sessions, the launcher shows `Start Codex` and `Start Shell`
- `Enter` on a launcher item creates a session in the current directory and attaches
- `r` refreshes the session list
- `Tab` switches keyboard focus between session and pane lists
- arrow keys and `j`/`k` move selection in the focused list
- mouse wheel moves selection in the hovered list
- mouse click selects a visible session or pane row
- mouse click on `Right` creates a side-by-side pane
- mouse click on `Down` creates a stacked pane
- mouse click on `Close` closes the selected pane when it is not the last pane
- mouse click on `Codex` or `Shell` creates and attaches a starter session
- terminal resize redraws cleanly
- `Enter` or the `Attach` button exits the dashboard and attaches the selected session
- attached tmux sessions have mouse support enabled for pane selection and resizing

## Temporary Sessions

Create a disposable tmux session when the dashboard needs real data:

```bash
tmux new-session -d -s amux-smoke 'sleep 600'
cargo run -p amux -- tui
tmux kill-session -t amux-smoke
```

Always clean up temporary tmux sessions before finishing.
