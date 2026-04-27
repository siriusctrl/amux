# Roadmap

This is the current implementation plan. Keep it short and update it when the
project direction changes.

## Phase 1: tmux-backed Local Client

- Provide a usable CLI for local sessions.
- Provide a small mouse-friendly TUI dashboard.
- Hide tmux ids and prefix-key behavior from the normal workflow.
- Keep command output scriptable.

## Phase 2: Remote Targets

- Add an SSH target model.
- Execute the same backend operations against remote tmux.
- Keep local and remote commands identical from the user's perspective.
- Add target selection in the TUI.

## Phase 3: Pane Control

- Add pane listing and split operations.
- Support mouse focus and split controls in the TUI.
- Preserve a simple keyboard fallback for all mouse operations.

## Phase 4: Agent Awareness

- Track whether a pane is running an agent, shell, or other command.
- Surface waiting/running/done/error states when they can be detected reliably.
- Integrate Codex and Claude Code hooks when available.
- Keep the agent model additive; do not hard-code one provider as the core.

## Phase 5: Direct Backend Evaluation

- Revisit whether tmux should remain the backend.
- Consider an `amuxd` PTY/session daemon only after the client model is proven.
- Preserve the public command and UI model if the backend changes.
