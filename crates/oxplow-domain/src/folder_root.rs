//! Where a declared folder of an extension's `files:` lives
//! (`.context/extensions-permissions.md` "Declared folders"): the project,
//! the person's home folder, or an agent harness's own data folder for this
//! project. Its `path` stays relative under every root, so a manifest never
//! names an absolute path or a `..`.

use std::fmt;
use std::path::PathBuf;
use std::str::FromStr;

/// What a folder's `path` is relative to.
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash, specta::Type)]
#[specta(type = String)]
pub enum FolderRoot {
    /// The project root (the primary checkout).
    #[default]
    Project,
    /// The person's home folder (a notes vault, `~/Notes`).
    Home,
    /// The data folder the harness with this id keeps for this project
    /// (`AgentHarness::project_data_dir`), such as Claude Code's
    /// `~/.claude/projects/<project>`.
    Agent(String),
}

impl FolderRoot {
    pub fn is_project(&self) -> bool {
        matches!(self, FolderRoot::Project)
    }
}

impl fmt::Display for FolderRoot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FolderRoot::Project => f.write_str("project"),
            FolderRoot::Home => f.write_str("home"),
            FolderRoot::Agent(id) => write!(f, "agent:{id}"),
        }
    }
}

impl FromStr for FolderRoot {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        let valid_id = |id: &str| {
            !id.is_empty()
                && id
                    .bytes()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_' || c == b'-')
        };
        match s {
            "project" => Ok(FolderRoot::Project),
            "home" => Ok(FolderRoot::Home),
            _ => match s.strip_prefix("agent:") {
                Some(id) if valid_id(id) => Ok(FolderRoot::Agent(id.to_string())),
                _ => Err(format!(
                    "`{s}` isn't a folder root: `project`, `home` or `agent:<harness id>`"
                )),
            },
        }
    }
}

impl serde::Serialize for FolderRoot {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> serde::Deserialize<'de> for FolderRoot {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

impl schemars::JsonSchema for FolderRoot {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "FolderRoot".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "description": "What the folder's `path` is relative to: `project` (the default), `home` (the person's home folder) or `agent:<harness id>` (that agent harness's own data folder for this project, such as Claude Code's). A folder outside the project is read only.",
            "type": "string",
            "pattern": "^(project|home|agent:[a-z0-9_-]+)$",
        })
    }
}

/// Where each root is on this machine: what the files service, the file
/// triggers and reads of rows join a declared folder's `path` to. `None`
/// when the root has no folder here (no home, a harness that keeps none):
/// its folders are absent, never an error.
pub trait FolderBases: Send + Sync {
    fn base(&self, root: &FolderRoot) -> Option<PathBuf>;
}

/// A bare project path: the project root, and no folder outside it.
impl FolderBases for std::path::Path {
    fn base(&self, root: &FolderRoot) -> Option<PathBuf> {
        root.is_project().then(|| self.to_path_buf())
    }
}

impl FolderBases for PathBuf {
    fn base(&self, root: &FolderRoot) -> Option<PathBuf> {
        self.as_path().base(root)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_root_reads_and_writes_as_its_name() {
        for (text, root) in [
            ("project", FolderRoot::Project),
            ("home", FolderRoot::Home),
            ("agent:claude", FolderRoot::Agent("claude".into())),
        ] {
            assert_eq!(text.parse::<FolderRoot>(), Ok(root.clone()));
            assert_eq!(root.to_string(), text);
            let json = serde_json::to_value(&root).unwrap();
            assert_eq!(json, serde_json::json!(text));
            assert_eq!(serde_json::from_value::<FolderRoot>(json).unwrap(), root);
        }
        for bad in [
            "",
            "agent:",
            "agent:Claude",
            "agent:a/b",
            "/abs",
            "~",
            "Home",
        ] {
            assert!(bad.parse::<FolderRoot>().is_err(), "{bad}");
        }
    }
}
