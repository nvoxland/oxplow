//! The repository's remotes, as `git remote -v` lists them: what a forge
//! link (Open In ▸ GitHub, Copy Revision Link) is built from.

use std::path::Path;
use std::process::Command;

/// One remote: its name and where it fetches from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitRemote {
    pub name: String,
    pub url: String,
}

/// The remotes of the repository at `repo_root`, by name, each once (its
/// fetch URL). None when it isn't a repository or git can't say.
pub fn list_remotes(repo_root: impl AsRef<Path>) -> Vec<GitRemote> {
    let repo_root = repo_root.as_ref();
    if !crate::repo::is_git_repo(repo_root) {
        return vec![];
    }
    let output = match Command::new("git")
        .args(["remote", "-v"])
        .current_dir(repo_root)
        .output()
    {
        Ok(o) if o.status.success() => o,
        _ => return vec![],
    };
    parse_remotes(&String::from_utf8_lossy(&output.stdout))
}

/// `git remote -v`'s lines (`origin\tgit@host:a/b.git (fetch)`): the fetch
/// URL of each remote, in name order.
pub(crate) fn parse_remotes(raw: &str) -> Vec<GitRemote> {
    let mut out: Vec<GitRemote> = raw
        .lines()
        .filter_map(|line| {
            let (name, rest) = line.split_once('\t')?;
            let url = rest.strip_suffix(" (fetch)")?;
            Some(GitRemote {
                name: name.to_string(),
                url: url.trim().to_string(),
            })
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out.dedup_by(|a, b| a.name == b.name);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_remote_once_by_its_fetch_url() {
        let raw = "origin\tgit@github.com:acme/app.git (fetch)\n\
                   origin\tgit@github.com:acme/app.git (push)\n\
                   backup\thttps://gitlab.com/acme/app (fetch)\n\
                   backup\thttps://gitlab.com/acme/app (push)\n";
        assert_eq!(
            parse_remotes(raw),
            vec![
                GitRemote {
                    name: "backup".into(),
                    url: "https://gitlab.com/acme/app".into()
                },
                GitRemote {
                    name: "origin".into(),
                    url: "git@github.com:acme/app.git".into()
                },
            ]
        );
    }

    #[test]
    fn a_repository_lists_its_remotes() {
        let dir = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| {
            assert!(Command::new("git")
                .args(args)
                .current_dir(dir.path())
                .status()
                .unwrap()
                .success());
        };
        git(&["init", "-q"]);
        git(&["remote", "add", "origin", "https://github.com/acme/app.git"]);
        assert_eq!(
            list_remotes(dir.path()),
            vec![GitRemote {
                name: "origin".into(),
                url: "https://github.com/acme/app.git".into()
            }]
        );
        assert!(list_remotes(dir.path().join("nope")).is_empty());
    }
}
