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
cargo run -p amux --
```

Verify:

- the alternate screen is restored after `Esc`
- with no sessions, the launcher shows `Start Session`
- `Enter` on a launcher item creates a session in the current directory and opens the amux session view
- `Ctrl-A` enters command mode
- command mode `r` refreshes the session list
- command mode `q` quits the dashboard
- command mode `Esc` returns to normal mode
- `Tab` switches keyboard focus between session and pane lists
- arrow keys move selection in the focused list
- mouse wheel moves selection in the hovered list
- mouse click selects a visible session or pane row
- mouse click on `Right` creates a side-by-side pane
- mouse click on `Down` creates a stacked pane
- mouse click on `Close` closes the selected pane when it is not the last pane
- mouse click on `New` creates and opens a starter session
- terminal resize redraws cleanly
- `Enter` or the `Open` button exits the dashboard and opens the selected session view
- session view shows amux-rendered pane borders and content, not the native tmux client
- session view shows a cursor in the selected pane when the pane cursor is visible
- common ANSI foreground colors in pane output are preserved
- split panes show separator lines in the gap between pane content areas
- selected panes have highlighted adjacent borders/separators without shifting content or cursor geometry
- outer session borders use rounded corners rather than hard tmux-style joints
- typing in session view reaches the selected pane
- session view `Ctrl-A v` and `Ctrl-A h` split panes without showing tmux UI
- session view left click selects panes
- session view wheel scrolls the hovered pane history
- session view `Ctrl-A q` detaches and restores the terminal
- typing `exit` in the last shell exits the session view instead of leaving an error page

## Temporary Sessions

Create a disposable tmux session when the dashboard needs real data:

```bash
tmux new-session -d -s amux-smoke 'sleep 600'
cargo run -p amux --
tmux kill-session -t amux-smoke
```

Always clean up temporary tmux sessions before finishing.
