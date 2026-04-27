use std::{path::Path, process::Command};

use anyhow::{Context, Result, anyhow, bail};

use crate::model::{Pane, Session, SplitDirection};

const NO_SERVER_MARKER: &str = "no server running";

pub fn version() -> Result<String> {
    let output = Command::new("tmux")
        .arg("-V")
        .output()
        .context("failed to run tmux -V")?;
    ensure_success("tmux -V", &output)?;
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

pub fn list_sessions() -> Result<Vec<Session>> {
    let output = Command::new("tmux")
        .args([
            "list-sessions",
            "-F",
            "#{session_id}\t#{session_name}\t#{session_windows}\t#{session_attached}",
        ])
        .output()
        .context("failed to run tmux list-sessions")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains(NO_SERVER_MARKER) {
            return Ok(Vec::new());
        }
        ensure_success("tmux list-sessions", &output)?;
    }

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(parse_session_line)
        .collect()
}

pub fn list_panes(session: &str) -> Result<Vec<Pane>> {
    validate_session_name(session)?;

    let output = Command::new("tmux")
        .args([
            "list-panes",
            "-t",
            session,
            "-F",
            "#{pane_id}\t#{pane_index}\t#{pane_active}\t#{pane_current_command}\t#{pane_current_path}\t#{pane_width}\t#{pane_height}\t#{pane_left}\t#{pane_top}\t#{cursor_x}\t#{cursor_y}\t#{cursor_flag}",
        ])
        .output()
        .with_context(|| format!("failed to list panes for tmux session {session}"))?;
    ensure_success("tmux list-panes", &output)?;

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(parse_pane_line)
        .collect()
}

pub fn create_session(name: &str, cwd: Option<&Path>, command: &[String]) -> Result<()> {
    validate_session_name(name)?;

    let mut tmux = Command::new("tmux");
    tmux.args(["new-session", "-d", "-s", name]);
    if let Some(cwd) = cwd {
        tmux.arg("-c").arg(cwd);
    }
    if !command.is_empty() {
        tmux.arg(join_shell_command(command));
    }

    let output = tmux
        .output()
        .with_context(|| format!("failed to create tmux session {name}"))?;
    ensure_success("tmux new-session", &output)
}

pub fn select_pane(pane_id: &str) -> Result<()> {
    validate_pane_id(pane_id)?;
    let output = Command::new("tmux")
        .args(["select-pane", "-t", pane_id])
        .output()
        .with_context(|| format!("failed to select tmux pane {pane_id}"))?;
    ensure_success("tmux select-pane", &output)
}

pub fn split_pane(pane_id: &str, direction: SplitDirection) -> Result<()> {
    validate_pane_id(pane_id)?;
    let flag = match direction {
        SplitDirection::Right => "-h",
        SplitDirection::Down => "-v",
    };
    let output = Command::new("tmux")
        .args([
            "split-window",
            "-t",
            pane_id,
            flag,
            "-c",
            "#{pane_current_path}",
        ])
        .output()
        .with_context(|| format!("failed to split tmux pane {pane_id}"))?;
    ensure_success("tmux split-window", &output)
}

pub fn kill_pane(pane_id: &str) -> Result<()> {
    validate_pane_id(pane_id)?;
    let output = Command::new("tmux")
        .args(["kill-pane", "-t", pane_id])
        .output()
        .with_context(|| format!("failed to close tmux pane {pane_id}"))?;
    ensure_success("tmux kill-pane", &output)
}

pub fn resize_window(session: &str, width: u16, height: u16) -> Result<()> {
    validate_session_name(session)?;
    let width = width.max(1).to_string();
    let height = height.max(1).to_string();
    let output = Command::new("tmux")
        .args(["resize-window", "-t", session, "-x", &width, "-y", &height])
        .output()
        .with_context(|| format!("failed to resize tmux session {session}"))?;
    ensure_success("tmux resize-window", &output)
}

pub fn capture_pane(pane_id: &str, height: usize, scroll_offset: usize) -> Result<String> {
    validate_pane_id(pane_id)?;

    let mut tmux = Command::new("tmux");
    tmux.args(["capture-pane", "-e", "-p", "-N", "-t", pane_id]);

    if height > 0 && scroll_offset > 0 {
        let start = format!("-{scroll_offset}");
        let end = (height.saturating_sub(1) as isize) - (scroll_offset as isize);
        tmux.args(["-S", &start, "-E", &end.to_string()]);
    }

    let output = tmux
        .output()
        .with_context(|| format!("failed to capture tmux pane {pane_id}"))?;
    ensure_success("tmux capture-pane", &output)?;
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

pub fn send_literal(pane_id: &str, text: &str) -> Result<()> {
    validate_pane_id(pane_id)?;
    if text.is_empty() {
        return Ok(());
    }

    let output = Command::new("tmux")
        .args(["send-keys", "-t", pane_id, "-l", text])
        .output()
        .with_context(|| format!("failed to send text to tmux pane {pane_id}"))?;
    ensure_success("tmux send-keys -l", &output)
}

pub fn send_key(pane_id: &str, key: &str) -> Result<()> {
    validate_pane_id(pane_id)?;
    if key.trim().is_empty() {
        bail!("key name cannot be empty");
    }

    let output = Command::new("tmux")
        .args(["send-keys", "-t", pane_id, key])
        .output()
        .with_context(|| format!("failed to send key {key} to tmux pane {pane_id}"))?;
    ensure_success("tmux send-keys", &output)
}

fn parse_session_line(line: &str) -> Result<Session> {
    let parts = line.split('\t').collect::<Vec<_>>();
    if parts.len() != 4 {
        bail!("unexpected tmux session row: {line}");
    }

    let windows = parts[2]
        .parse::<usize>()
        .with_context(|| format!("invalid tmux window count: {}", parts[2]))?;
    let attached = match parts[3] {
        "0" => false,
        "1" => true,
        value => bail!("invalid tmux attached flag: {value}"),
    };

    Ok(Session {
        id: parts[0].to_owned(),
        name: parts[1].to_owned(),
        windows,
        attached,
    })
}

fn parse_pane_line(line: &str) -> Result<Pane> {
    let parts = line.split('\t').collect::<Vec<_>>();
    if parts.len() != 12 {
        bail!("unexpected tmux pane row: {line}");
    }

    let active = match parts[2] {
        "0" => false,
        "1" => true,
        value => bail!("invalid tmux pane active flag: {value}"),
    };

    Ok(Pane {
        id: parts[0].to_owned(),
        index: parse_usize_field("pane index", parts[1])?,
        active,
        current_command: parts[3].to_owned(),
        current_path: parts[4].to_owned(),
        width: parse_usize_field("pane width", parts[5])?,
        height: parse_usize_field("pane height", parts[6])?,
        left: parse_usize_field("pane left", parts[7])?,
        top: parse_usize_field("pane top", parts[8])?,
        cursor_x: parse_usize_field("cursor x", parts[9])?,
        cursor_y: parse_usize_field("cursor y", parts[10])?,
        cursor_visible: parse_bool_field("cursor flag", parts[11])?,
    })
}

fn parse_usize_field(label: &str, value: &str) -> Result<usize> {
    value
        .parse::<usize>()
        .with_context(|| format!("invalid tmux {label}: {value}"))
}

fn parse_bool_field(label: &str, value: &str) -> Result<bool> {
    match value {
        "0" => Ok(false),
        "1" => Ok(true),
        value => bail!("invalid tmux {label}: {value}"),
    }
}

fn validate_session_name(name: &str) -> Result<()> {
    if name.trim().is_empty() {
        bail!("session name cannot be empty");
    }
    if name.contains(':') {
        bail!("session name cannot contain ':' because tmux treats it as a target separator");
    }
    Ok(())
}

fn validate_pane_id(pane_id: &str) -> Result<()> {
    if pane_id.strip_prefix('%').is_none() {
        bail!("pane id must start with '%': {pane_id}");
    }
    if pane_id[1..].parse::<usize>().is_err() {
        bail!("pane id must be a tmux numeric pane id: {pane_id}");
    }
    Ok(())
}

fn ensure_success(label: &str, output: &std::process::Output) -> Result<()> {
    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    Err(anyhow!(
        "{label} failed with status {}: {}{}{}",
        output.status,
        stderr.trim(),
        if stderr.is_empty() || stdout.is_empty() {
            ""
        } else {
            "\n"
        },
        stdout.trim()
    ))
}

fn join_shell_command(parts: &[String]) -> String {
    parts
        .iter()
        .map(|part| quote_shell_arg(part))
        .collect::<Vec<_>>()
        .join(" ")
}

fn quote_shell_arg(value: &str) -> String {
    if value.is_empty() {
        return "''".to_owned();
    }

    if value
        .bytes()
        .all(|byte| matches!(byte, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'-' | b'.' | b'/' | b':' | b'=' | b'+' | b','))
    {
        return value.to_owned();
    }

    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tmux_session_rows() {
        let session = parse_session_line("$1\twork\t2\t0").unwrap();
        assert_eq!(session.id, "$1");
        assert_eq!(session.name, "work");
        assert_eq!(session.windows, 2);
        assert!(!session.attached);
    }

    #[test]
    fn parses_tmux_pane_rows() {
        let pane = parse_pane_line("%1\t1\t1\tbash\t/root/amux\t80\t24\t0\t0\t12\t3\t1").unwrap();
        assert_eq!(pane.id, "%1");
        assert_eq!(pane.index, 1);
        assert!(pane.active);
        assert_eq!(pane.current_command, "bash");
        assert_eq!(pane.current_path, "/root/amux");
        assert_eq!(pane.width, 80);
        assert_eq!(pane.height, 24);
        assert_eq!(pane.left, 0);
        assert_eq!(pane.top, 0);
        assert_eq!(pane.cursor_x, 12);
        assert_eq!(pane.cursor_y, 3);
        assert!(pane.cursor_visible);
    }

    #[test]
    fn rejects_malformed_session_rows() {
        assert!(parse_session_line("work\t1").is_err());
        assert!(parse_session_line("$1\twork\tmany\t0").is_err());
        assert!(parse_session_line("$1\twork\t1\tmaybe").is_err());
    }

    #[test]
    fn rejects_malformed_pane_rows() {
        assert!(parse_pane_line("%1\t1").is_err());
        assert!(parse_pane_line("%1\tone\t1\tbash\t/tmp\t80\t24\t0\t0\t0\t0\t1").is_err());
        assert!(parse_pane_line("%1\t1\tmaybe\tbash\t/tmp\t80\t24\t0\t0\t0\t0\t1").is_err());
        assert!(parse_pane_line("%1\t1\t1\tbash\t/tmp\t80\t24\t0\t0\t0\t0\tmaybe").is_err());
    }

    #[test]
    fn shell_command_join_quotes_only_when_needed() {
        assert_eq!(
            join_shell_command(&["printf".to_owned(), "hello world".to_owned()]),
            "printf 'hello world'"
        );
        assert_eq!(
            join_shell_command(&["printf".to_owned(), "it's".to_owned()]),
            "printf 'it'\"'\"'s'"
        );
    }

    #[test]
    fn rejects_tmux_target_separator_in_session_names() {
        assert!(validate_session_name("repo:1").is_err());
        assert!(validate_session_name("repo").is_ok());
    }

    #[test]
    fn validates_tmux_pane_ids() {
        assert!(validate_pane_id("%1").is_ok());
        assert!(validate_pane_id("1").is_err());
        assert!(validate_pane_id("%abc").is_err());
    }
}
