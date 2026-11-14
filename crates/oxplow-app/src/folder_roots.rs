//! Where each folder root (`oxplow_domain::folder_root`) is on this
//! machine: the project root, the person's home folder, and each agent
//! harness's own data folder for the project (`AgentHarness::project_data_dir`).
//! What the files service, the file triggers and reads of rows join a
//! declared folder's `path` to. A test's oxplow puts every outside root
//! under a folder of its own ([`Outside::Under`]), so it never reads the
//! person's home.

use std::path::{Path, PathBuf};

use oxplow_domain::agent::registry::HarnessRegistry;
use oxplow_domain::folder_root::{FolderBases, FolderRoot};

/// Where the roots outside the project are.
#[derive(Debug, Clone)]
pub enum Outside {
    /// This machine's: the person's home folder (`None` when `HOME` is
    /// unset), and what each registered harness says.
    Machine { home: Option<PathBuf> },
    /// All under one folder: `home` is `<dir>/home`, `agent:<id>` is
    /// `<dir>/agent/<id>`. A test's and a fixture's oxplow.
    Under(PathBuf),
}

impl Outside {
    /// This machine's, from `HOME`.
    pub fn machine() -> Self {
        Outside::Machine {
            home: std::env::var_os("HOME")
                .filter(|h| !h.is_empty())
                .map(PathBuf::from),
        }
    }
}

#[derive(Clone)]
pub struct FolderRoots {
    project: PathBuf,
    outside: Outside,
    harnesses: HarnessRegistry,
}

impl FolderRoots {
    pub fn new(project: PathBuf, outside: Outside, harnesses: HarnessRegistry) -> Self {
        Self {
            project,
            outside,
            harnesses,
        }
    }

    pub fn project(&self) -> &Path {
        &self.project
    }
}

impl FolderBases for FolderRoots {
    fn base(&self, root: &FolderRoot) -> Option<PathBuf> {
        match (root, &self.outside) {
            (FolderRoot::Project, _) => Some(self.project.clone()),
            (FolderRoot::Home, Outside::Machine { home }) => home.clone(),
            (FolderRoot::Agent(id), Outside::Machine { home }) => self
                .harnesses
                .get(id)
                .ok()?
                .project_data_dir(home.as_deref()?, &self.project),
            (FolderRoot::Home, Outside::Under(dir)) => Some(dir.join("home")),
            (FolderRoot::Agent(id), Outside::Under(dir)) => Some(dir.join("agent").join(id)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry() -> HarnessRegistry {
        let r = HarnessRegistry::new(std::sync::Arc::new(String::new));
        r.set_declared(vec![oxplow_harnesses::built_in(
            "oxplow:claude-code",
            "claude",
            "Claude",
        )
        .unwrap()]);
        r
    }

    #[test]
    fn each_root_resolves_on_the_machine_and_under_a_tests_folder() {
        let project = PathBuf::from("/src/app");
        let machine = FolderRoots::new(
            project.clone(),
            Outside::Machine {
                home: Some("/home/me".into()),
            },
            registry(),
        );
        assert_eq!(machine.base(&FolderRoot::Project), Some(project.clone()));
        assert_eq!(machine.base(&FolderRoot::Home), Some("/home/me".into()));
        if std::env::var_os("CLAUDE_CONFIG_DIR").is_none_or(|v| v.is_empty()) {
            assert_eq!(
                machine.base(&FolderRoot::Agent("claude".into())),
                Some("/home/me/.claude/projects/-src-app".into())
            );
        }
        // A harness that isn't registered, or no home: no folder.
        assert_eq!(machine.base(&FolderRoot::Agent("nope".into())), None);
        let homeless =
            FolderRoots::new(project.clone(), Outside::Machine { home: None }, registry());
        assert_eq!(homeless.base(&FolderRoot::Home), None);
        assert_eq!(homeless.base(&FolderRoot::Agent("claude".into())), None);

        let under = FolderRoots::new(project, Outside::Under("/t".into()), registry());
        assert_eq!(under.base(&FolderRoot::Home), Some("/t/home".into()));
        assert_eq!(
            under.base(&FolderRoot::Agent("claude".into())),
            Some("/t/agent/claude".into())
        );
    }
}
