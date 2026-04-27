mod cli;
mod model;
mod tmux;
mod tui;

fn main() -> anyhow::Result<()> {
    cli::run()
}
