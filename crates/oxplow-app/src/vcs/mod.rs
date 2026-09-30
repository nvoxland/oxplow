//! The VCS capability's providers (`oxplow_domain::vcs::Vcs`,
//! `.context/vcs.md`). Git is the one built in.

mod git;
pub mod reads;

pub use git::{ChangeScopes, CommitRefLabel, GitProvider, RemoteBranchEntry};

#[cfg(test)]
mod tests {
    /// P5 (B3–B7): only the git provider (`vcs/git.rs`) touches git —
    /// `oxplow_git` or libgit2 — so core reads and changes version control
    /// through the `Vcs` trait alone and a second provider needs no core
    /// change. Test code (after `#[cfg(test)]`, a `#![cfg(test)]` module,
    /// and `tests/` / `benches/` targets) is exempt: it builds real
    /// repositories as fixtures.
    #[test]
    fn only_the_git_provider_touches_git() {
        let crates = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let mut callers = Vec::new();
        let mut todo = vec![crates.clone()];
        while let Some(dir) = todo.pop() {
            for entry in std::fs::read_dir(&dir).unwrap().flatten() {
                let path = entry.path();
                let name = path.file_name().unwrap().to_string_lossy().to_string();
                if path.is_dir() {
                    let skip = ["target", "oxplow-git", "tests", "benches"];
                    if !skip.contains(&name.as_str()) && !name.starts_with('.') {
                        todo.push(path);
                    }
                } else if path.extension().is_some_and(|e| e == "rs") {
                    let text = std::fs::read_to_string(&path).unwrap();
                    if text.contains("#![cfg(test)]") {
                        continue;
                    }
                    let prod = text.split("#[cfg(test)]").next().unwrap_or_default();
                    if prod.contains("oxplow_git::") || prod.contains("git2::") {
                        callers.push(
                            path.strip_prefix(&crates)
                                .unwrap()
                                .to_string_lossy()
                                .replace('\\', "/"),
                        );
                    }
                }
            }
        }
        callers.sort();
        assert_eq!(
            callers,
            vec!["oxplow-app/src/vcs/git.rs".to_string()],
            "git touched outside the git provider"
        );
    }
}
