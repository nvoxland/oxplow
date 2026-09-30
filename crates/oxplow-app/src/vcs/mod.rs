//! The VCS capability's providers (`oxplow_domain::vcs::Vcs`,
//! `.context/vcs.md`). Git is the one built in.

mod git;
pub mod reads;

pub use git::GitProvider;

#[cfg(test)]
mod tests {
    /// Files still calling `oxplow_git` directly: each moves onto the
    /// VCS capability in P5 (B4–B7) and leaves this list, which ends
    /// empty. A new caller fails the test.
    const NOT_YET_ON_VCS: &[&str] = &[
        "oxplow-session/src/lib.rs",
        "oxplow-app/src/task_service.rs",
        "oxplow-app/src/workspace_watch.rs",
        "oxplow-app/src/commit_indexer.rs",
        "oxplow-app/src/change_analysis.rs",
        "oxplow-app/src/metrics_service.rs",
        "oxplow-app/src/collection.rs",
        "oxplow-app/src/git_service.rs",
        "oxplow-mcp/src/lib.rs",
        "oxplow-rpc/src/lib.rs",
        "oxplow-rpc/src/commands/branch.rs",
        "oxplow-rpc/src/commands/config.rs",
        "oxplow-rpc/src/commands/log.rs",
        "oxplow-rpc/src/commands/git.rs",
    ];

    /// P5.B3 (tsk522): only the git provider (`vcs/git.rs`) calls
    /// `oxplow_git`; core reads git through the `Vcs` trait. Test code is
    /// exempt.
    #[test]
    fn only_the_git_provider_calls_oxplow_git() {
        let crates = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let mut callers = Vec::new();
        let mut todo = vec![crates.clone()];
        while let Some(dir) = todo.pop() {
            for entry in std::fs::read_dir(&dir).unwrap().flatten() {
                let path = entry.path();
                let name = path.file_name().unwrap().to_string_lossy().to_string();
                if path.is_dir() {
                    if name != "target" && name != "oxplow-git" && !name.starts_with('.') {
                        todo.push(path);
                    }
                } else if path.extension().is_some_and(|e| e == "rs") {
                    let text = std::fs::read_to_string(&path).unwrap();
                    let prod = text.split("#[cfg(test)]").next().unwrap_or_default();
                    if prod.contains("oxplow_git::") {
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
        let unexpected: Vec<_> = callers
            .iter()
            .filter(|c| *c != "oxplow-app/src/vcs/git.rs" && !NOT_YET_ON_VCS.contains(&c.as_str()))
            .collect();
        assert!(
            unexpected.is_empty(),
            "oxplow_git called outside the git provider: {unexpected:?}"
        );
        let moved: Vec<_> = NOT_YET_ON_VCS
            .iter()
            .filter(|f| !callers.iter().any(|c| c == *f))
            .collect();
        assert!(
            moved.is_empty(),
            "no longer calls oxplow_git — take it off NOT_YET_ON_VCS: {moved:?}"
        );
    }
}
