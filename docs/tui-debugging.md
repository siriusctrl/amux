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
- `r` refreshes the session list
- arrow keys and `j`/`k` move selection
- mouse wheel moves selection
- mouse click selects a visible session row
- terminal resize redraws cleanly
- `Enter` exits the dashboard and attaches the selected session

## Temporary Sessions

Create a disposable tmux session when the dashboard needs real data:

```bash
tmux new-session -d -s amux-smoke 'sleep 600'
cargo run -p amux -- tui
tmux kill-session -t amux-smoke
```

Always clean up temporary tmux sessions before finishing.
