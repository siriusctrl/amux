use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub id: String,
    pub label: String,
    pub kind: TargetKind,
}

impl Target {
    pub fn local() -> Self {
        Self {
            id: "local".to_owned(),
            label: "Local machine".to_owned(),
            kind: TargetKind::Local,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetKind {
    Local,
}

impl fmt::Display for TargetKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TargetKind::Local => formatter.write_str("local"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub id: String,
    pub name: String,
    pub windows: usize,
    pub attached: bool,
}

impl Session {
    pub fn display_status(&self) -> &'static str {
        if self.attached {
            "attached"
        } else {
            "detached"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_target_uses_stable_id() {
        let target = Target::local();
        assert_eq!(target.id, "local");
        assert_eq!(target.kind.to_string(), "local");
    }

    #[test]
    fn session_status_reflects_attachment() {
        let detached = Session {
            id: "$0".to_owned(),
            name: "work".to_owned(),
            windows: 1,
            attached: false,
        };
        assert_eq!(detached.display_status(), "detached");
    }
}
