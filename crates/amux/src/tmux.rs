use std::{
    path::Path,
    process::{Command, ExitStatus, Stdio},
};

use anyhow::{Context, Result, anyhow, bail};

use crate::model::Session;

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

pub fn attach_session(name: &str) -> Result<ExitStatus> {
    validate_session_name(name)?;
    Command::new("tmux")
        .args(["attach-session", "-t", name])
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .with_context(|| format!("failed to attach tmux session {name}"))
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

fn validate_session_name(name: &str) -> Result<()> {
    if name.trim().is_empty() {
        bail!("session name cannot be empty");
    }
    if name.contains(':') {
        bail!("session name cannot contain ':' because tmux treats it as a target separator");
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
    fn rejects_malformed_session_rows() {
        assert!(parse_session_line("work\t1").is_err());
        assert!(parse_session_line("$1\twork\tmany\t0").is_err());
        assert!(parse_session_line("$1\twork\t1\tmaybe").is_err());
    }

    #[test]
    fn shell_command_join_quotes_only_when_needed() {
        assert_eq!(
            join_shell_command(&["codex".to_owned(), "hello world".to_owned()]),
            "codex 'hello world'"
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
}
