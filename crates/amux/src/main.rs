mod cli;
mod model;
mod session_view;
mod tmux;
mod tui;

fn main() -> anyhow::Result<()> {
    cli::run()
}
