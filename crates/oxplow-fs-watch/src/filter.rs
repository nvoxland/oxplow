//! The single source of truth for "is this workspace path ignored?".

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use ignore::gitignore::{Gitignore, GitignoreBuilder};

const EXCLUDE_IN_GIT_DIR: &str = "info/exclude";
const EXCLUDE_FILE: &str = ".git/info/exclude";
/// How many directories a filter remembers decisions for before starting over.
const MAX_REMEMBERED_DIRS: usize = 200_000;

/// Whether `path` is a file git reads ignore rules from: a consumer holding
/// its own filter reads it again on a change to one.
pub fn is_ignore_file(path: &Path) -> bool {
    path.file_name().is_some_and(|n| n == ".gitignore") || path.ends_with(Path::new(EXCLUDE_FILE))
}

/// Path segments that ALWAYS trigger a watch-ignore, regardless of
/// user config. Limited to `.git` — git's internal write spool
/// churns faster than we can capture and produces ephemeral
/// lock/tmp files that always race the watcher to a NotFound, so
/// watching it is never useful. Everything else (build outputs,
/// language caches, IDE state) is the user's call via the
/// `generated` config; we don't pretend to know which dirs each
/// project actually treats as generated. `.oxplow` is handled
/// separately above — watched for the wiki and for anything
/// `.oxplow/.gitignore` un-ignores with a `!` — not via this segment
/// list.
const DEFAULT_IGNORED_SEGMENTS: &[&str] = &[".git"];

/// The single source of truth for "is this workspace path ignored?".
/// Built once per project ([`WorkspaceFilter::for_project`]) and shared
/// by cheap clone across snapshot capture, code-quality scans, fs-watch
/// consumers, and the indexer — no consumer keeps its own skip-list.
///
/// A path is ignored when, in precedence order:
/// 1. it is under `.git` or `.oxplow` (except `.oxplow/wiki/`) —
///    absolute, never overridable;
/// 2. otherwise, if it matches an **`include`** entry it is **kept**
///    (force-track something `.gitignore` would otherwise drop);
/// 3. otherwise, if it matches an **`exclude`** entry it is ignored;
/// 4. otherwise, if `.gitignore` (the repo root `.gitignore` +
///    `.git/info/exclude`) ignores it, it is ignored;
/// 5. otherwise it is kept.
///
/// Root-level `.gitignore` patterns are matched at every depth (git
/// semantics), so `node_modules` / `target` / build caches are pruned
/// throughout the tree, and per-subdirectory `.gitignore` files are
/// loaded too — each matched relative to its own directory, so nested
/// patterns (and `!negations` that re-include) work with full git
/// semantics.
///
/// `exclude`/`include` entries are either a single segment (matches any
/// path component — `target` matches every `target/`) or a repo-relative
/// path (matches that path exactly or as a prefix — `apps/desktop/dist`).
///
/// **A handle, cheap to clone: every clone is the same filter**, so a watch and
/// the code that sweeps the same tree cannot disagree. Nothing is read when it
/// is built and it never walks the tree: a directory's `.gitignore` is read the
/// first time a path in it is judged, and kept until
/// [`Self::note_ignore_file`] says it changed. So what a filter costs follows
/// the directories judged, not the size of the tree or what it ignores.
#[derive(Clone, Default)]
pub struct WorkspaceFilter {
    shared: Arc<Shared>,
}

#[derive(Default)]
struct Shared {
    /// Repo root (absolute). `ignore()` joins the repo-relative query
    /// path onto this so it can be matched against each per-directory
    /// matcher, which are rooted at absolute directories. Empty: a filter
    /// with no project, which reads no ignore files.
    root: PathBuf,
    /// Git's global excludes file.
    global: Option<PathBuf>,
    entries: RwLock<UserEntries>,
    /// Each directory's own rules, by its path from the root (the root's
    /// also folds in the exclude files), as read so far.
    rules: RwLock<HashMap<PathBuf, Option<Arc<Gitignore>>>>,
    /// The rules that judge a path directly in each directory, shallowest
    /// first. Deeper matchers override shallower ones.
    chains: RwLock<HashMap<PathBuf, Chain>>,
    /// Whether each directory judged by [`WorkspaceFilter::ignore_change`]
    /// is ignored, so a burst under an ignored folder costs a lookup each.
    dirs: RwLock<HashMap<PathBuf, bool>>,
}

type Chain = Arc<Vec<Arc<Gitignore>>>;

#[derive(Default)]
struct UserEntries {
    exclude: Vec<FilterEntry>,
    include: Vec<FilterEntry>,
}

impl std::fmt::Debug for WorkspaceFilter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let entries = read(&self.shared.entries);
        f.debug_struct("WorkspaceFilter")
            .field("root", &self.shared.root)
            .field("exclude", &entries.exclude)
            .field("include", &entries.include)
            .finish()
    }
}

fn read<T>(lock: &RwLock<T>) -> std::sync::RwLockReadGuard<'_, T> {
    lock.read().unwrap_or_else(|e| e.into_inner())
}

fn write<T>(lock: &RwLock<T>) -> std::sync::RwLockWriteGuard<'_, T> {
    lock.write().unwrap_or_else(|e| e.into_inner())
}

#[derive(Debug, Clone)]
enum FilterEntry {
    Segment(String),
    Path(PathBuf),
}

impl WorkspaceFilter {
    /// Exclude-only filter with no `.gitignore` awareness — for tests
    /// and callers that have no project root. Equivalent to
    /// `for_project` with an empty include list and no repo.
    pub fn with_user_entries<I, S>(entries: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        Self::build(Path::new(""), entries, Vec::<String>::new(), None)
    }

    /// The project filter of the worktree at `root` (absolute):
    /// `.gitignore`-aware (root + nested `.gitignore` files + the exclude
    /// files), plus the project's `generated.exclude` and `generated.include`
    /// lists.
    pub fn for_project<E, S, I, T>(root: &Path, exclude: E, include: I) -> Self
    where
        E: IntoIterator<Item = S>,
        S: AsRef<str>,
        I: IntoIterator<Item = T>,
        T: AsRef<str>,
    {
        Self::build(
            root,
            exclude,
            include,
            ignore::gitignore::gitconfig_excludes_path(),
        )
    }

    /// [`Self::for_project`] with git's global excludes file given
    /// (`core.excludesFile`, found as git finds it), so a test needn't
    /// touch the person's.
    pub(crate) fn build<E, S, I, T>(
        root: &Path,
        exclude: E,
        include: I,
        global: Option<PathBuf>,
    ) -> Self
    where
        E: IntoIterator<Item = S>,
        S: AsRef<str>,
        I: IntoIterator<Item = T>,
        T: AsRef<str>,
    {
        Self {
            shared: Arc::new(Shared {
                root: root.to_path_buf(),
                global,
                entries: RwLock::new(UserEntries {
                    exclude: parse_filter_entries(exclude),
                    include: parse_filter_entries(include),
                }),
                ..Shared::default()
            }),
        }
    }

    /// The project's `generated` lists changed: every clone judges by the new
    /// ones from now on, and reads the ignore files again.
    pub fn set_user_entries<E, S, I, T>(&self, exclude: E, include: I)
    where
        E: IntoIterator<Item = S>,
        S: AsRef<str>,
        I: IntoIterator<Item = T>,
        T: AsRef<str>,
    {
        *write(&self.shared.entries) = UserEntries {
            exclude: parse_filter_entries(exclude),
            include: parse_filter_entries(include),
        };
        write(&self.shared.rules).clear();
        self.forget_decisions();
    }

    /// An ignore file changed (`path` from the root, or an exclude file
    /// outside it): its directory's rules are read again when next needed.
    /// Nothing is walked.
    pub fn note_ignore_file(&self, path: &Path) {
        let dir = if path.is_absolute() || path.ends_with(EXCLUDE_FILE) {
            Path::new("")
        } else {
            path.parent().unwrap_or(Path::new(""))
        };
        write(&self.shared.rules).remove(dir);
        self.forget_decisions();
    }

    fn forget_decisions(&self) {
        write(&self.shared.chains).clear();
        write(&self.shared.dirs).clear();
    }

    /// The rules of the directory at `dir`, from the root.
    fn rules_of(&self, dir: &Path) -> Option<Arc<Gitignore>> {
        if let Some(rules) = read(&self.shared.rules).get(dir) {
            return rules.clone();
        }
        let shared = &self.shared;
        let rules = if dir.as_os_str().is_empty() {
            Some(root_gitignore(&shared.root, shared.global.as_deref()))
        } else {
            folder_gitignore(&shared.root.join(dir))
        }
        .map(Arc::new);
        write(&shared.rules).insert(dir.to_path_buf(), rules.clone());
        rules
    }

    /// The rules that judge a path directly in `dir`: those of the root and
    /// each directory on the way to it, stopping at an ignored directory (its
    /// rules are no one's) and at `.git`.
    fn chain_of(&self, dir: &Path) -> Chain {
        if let Some(chain) = read(&self.shared.chains).get(dir) {
            return chain.clone();
        }
        let chain = if self.shared.root.as_os_str().is_empty() {
            Chain::default()
        } else if dir.as_os_str().is_empty() {
            Arc::new(self.rules_of(dir).into_iter().collect())
        } else {
            let above = self.chain_of(dir.parent().unwrap_or(Path::new("")));
            let abs = self.shared.root.join(dir);
            if dir.ends_with(".git") || gitignore_decision(&above, &abs, true).is_ignore() {
                above
            } else if let Some(rules) = self.rules_of(dir) {
                let mut chain = (*above).clone();
                chain.push(rules);
                Arc::new(chain)
            } else {
                above
            }
        };
        let mut chains = write(&self.shared.chains);
        // Bounded: a walk of a vast tree must not hold a list per directory
        // forever. They are rebuilt from `rules` when next asked for.
        if chains.len() >= MAX_REMEMBERED_DIRS {
            chains.clear();
        }
        chains.insert(dir.to_path_buf(), chain.clone());
        chain
    }

    /// What git's rules say of `path`. `path` may be absolute: under the root
    /// it is judged as from the root; elsewhere no rules apply to it.
    fn git_decision(&self, path: &Path, is_dir: bool) -> ignore::Match<()> {
        let rel = if path.is_absolute() {
            match path.strip_prefix(&self.shared.root) {
                Ok(rel) => rel,
                Err(_) => return ignore::Match::None,
            }
        } else {
            path
        };
        let chain = self.chain_of(rel.parent().unwrap_or(Path::new("")));
        gitignore_decision(&chain, &self.shared.root.join(rel), is_dir)
    }

    /// [`Self::ignore`] for a path a watch just reported: a path under an
    /// ignored directory is answered from a remembered lookup, with no `stat`
    /// (`is_dir` is asked only of a path still in question). A build writing
    /// thousands of files under `target/` must cost next to nothing.
    pub fn ignore_change(&self, path: &Path, is_dir: impl FnOnce() -> bool) -> bool {
        // An `include` can keep a path under an ignored directory.
        if read(&self.shared.entries).include.is_empty() {
            let mut dirs: Vec<&Path> = path.ancestors().skip(1).collect();
            dirs.pop(); // The root itself.
            if dirs.iter().rev().any(|dir| self.dir_ignored(dir)) {
                return true;
            }
        }
        self.ignore(path, is_dir())
    }

    fn dir_ignored(&self, dir: &Path) -> bool {
        if let Some(&ignored) = read(&self.shared.dirs).get(dir) {
            return ignored;
        }
        let ignored = self.ignore(dir, true);
        let mut dirs = write(&self.shared.dirs);
        if dirs.len() >= MAX_REMEMBERED_DIRS {
            dirs.clear();
        }
        dirs.insert(dir.to_path_buf(), ignored);
        ignored
    }

    /// True if `path` (workspace-relative) should be ignored. `is_dir`
    /// lets `.gitignore` directory-only patterns (`build/`) prune a
    /// whole subtree without descending into it — pass the real value
    /// at walk sites; `false` is a safe default for file events.
    pub fn ignore(&self, path: &Path, is_dir: bool) -> bool {
        self.decide(path, is_dir, true)
    }

    /// True if `path` is out of a tree (`oxplow-app`'s `Trees`: a diff,
    /// the working tree, a snapshot's files). The same rules as
    /// [`Self::ignore`] but git's view of `.oxplow/wiki/`, which oxplow
    /// watches though git ignores it: a tree answers "what would git
    /// see?", so the wiki is no one's uncommitted work (tsk1083).
    pub fn ignore_in_trees(&self, path: &Path, is_dir: bool) -> bool {
        self.decide(path, is_dir, false)
    }

    fn decide(&self, path: &Path, is_dir: bool, watching: bool) -> bool {
        use std::path::Component;

        // 1. Absolute defaults: `.git` anywhere, and oxplow's own `.oxplow`
        //    state dir. Not overridable by the `generated` include/exclude
        //    lists — but `.oxplow` DOES honour its own `.gitignore`
        //    negations, so a file the project declares shareable (notably
        //    `project.yaml`) is watched.
        let mut comps = path.components().peekable();
        while let Some(c) = comps.next() {
            if let Component::Normal(seg) = c {
                let s = match seg.to_str() {
                    Some(s) => s,
                    None => continue,
                };
                if s == ".oxplow" {
                    let next = comps.peek().and_then(|c| match c {
                        Component::Normal(n) => n.to_str(),
                        _ => None,
                    });
                    return match next {
                        // `.oxplow` ITSELF must not be reported as ignored:
                        // walkers prune on directories (`WalkDir::filter_entry`,
                        // the capture sweep), so returning true here stops the
                        // walk before it can ever see the files below — which is
                        // what made `project.yaml` invisible even once the
                        // per-file rule was right (tsk176).
                        None => false,
                        // The wiki is watched. This can't be expressed in
                        // `.gitignore`: it is deliberately NOT committed, but it
                        // IS snapshotted (Local History's wiki badges read
                        // `list_wiki_slugs_for_snapshots`). `.gitignore` answers
                        // "should this be committed?"; this filter answers
                        // "should oxplow watch this?" — and for the wiki those
                        // differ, so the one distinction gitignore can't carry
                        // lives here.
                        Some("wiki") if watching => false,
                        // Extension sources are watched for the same reason:
                        // an agent's edits to one are its work (snapshotted,
                        // given to its effort), committed or not. Watching
                        // never runs them; only a deploy does. In git's view
                        // too: a shared one is committed code (git decides
                        // which are there): in a tree, `.oxplow/.gitignore`
                        // decides, so a personal source is no one's
                        // uncommitted work.
                        Some("extensions") if !watching => {
                            self.git_decision(path, is_dir).is_ignore()
                        }
                        Some("extensions") => false,
                        // Every other SUBDIRECTORY is pruned wholesale, so a
                        // walk never descends into `snapshots/` (thousands of
                        // blobs), `runtime/`, or the LSP caches. Negations
                        // therefore reach files sitting directly in `.oxplow/`,
                        // which is exactly what `.oxplow/.gitignore` negates.
                        Some(_) if is_dir => true,
                        // A file in oxplow's own state directory: ignored by
                        // default — watching the live SQLite would feed
                        // oxplow's own writes back through its watcher — UNLESS
                        // `.oxplow/.gitignore` un-ignores it with a `!`. oxplow
                        // writes `!project.yaml` there so the config is
                        // shareable, and a project may negate more. Honouring
                        // the `!` keeps this general: the visible set is data,
                        // not a per-file carve-out in code.
                        Some(_) => !self.git_decision(path, is_dir).is_whitelist(),
                    };
                }
                if DEFAULT_IGNORED_SEGMENTS.contains(&s) {
                    return true;
                }
            }
        }

        {
            let entries = read(&self.shared.entries);
            // 2. `include` forces a path back in (overrides exclude + gitignore).
            if matches_filter_entries(&entries.include, path) {
                return false;
            }
            // 3. Explicit excludes.
            if matches_filter_entries(&entries.exclude, path) {
                return true;
            }
        }
        // 4. `.gitignore` — root + nested, full hierarchical semantics.
        !path.as_os_str().is_empty() && self.git_decision(path, is_dir).is_ignore()
    }
}

fn parse_filter_entries<I, S>(entries: I) -> Vec<FilterEntry>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    entries
        .into_iter()
        .filter_map(|raw| {
            let trimmed = raw.as_ref().trim().trim_matches('/');
            if trimmed.is_empty() {
                None
            } else if trimmed.contains('/') {
                Some(FilterEntry::Path(PathBuf::from(trimmed)))
            } else {
                Some(FilterEntry::Segment(trimmed.to_string()))
            }
        })
        .collect()
}

fn matches_filter_entries(entries: &[FilterEntry], path: &Path) -> bool {
    use std::path::Component;
    for entry in entries {
        match entry {
            FilterEntry::Segment(seg) => {
                if path
                    .components()
                    .any(|c| matches!(c, Component::Normal(n) if n.to_str() == Some(seg.as_str())))
                {
                    return true;
                }
            }
            FilterEntry::Path(p) => {
                if path == p.as_path() || path.starts_with(p) {
                    return true;
                }
            }
        }
    }
    false
}

/// The matcher of `root`: lowest precedence first, as git reads them, the
/// global excludes file (`core.excludesFile`), `.git/info/exclude`,
/// then the root `.gitignore`.
fn root_gitignore(root: &Path, global: Option<&Path>) -> Gitignore {
    let mut builder = GitignoreBuilder::new(root);
    if let Some(global) = global {
        let _ = builder.add(global);
    }
    if let Some(exclude) = exclude_file(root) {
        let _ = builder.add(exclude);
    }
    let _ = builder.add(root.join(".gitignore"));
    builder.build().unwrap_or_else(|_| Gitignore::empty())
}

/// The matcher of the `.gitignore` in `dir`, when it has one.
fn folder_gitignore(dir: &Path) -> Option<Gitignore> {
    let path = dir.join(".gitignore");
    if !path.is_file() {
        return None;
    }
    let mut builder = GitignoreBuilder::new(dir);
    let _ = builder.add(&path);
    builder.build().ok()
}

/// The exclude file git reads for the worktree at `root`: a linked worktree
/// shares its main checkout's (`.git` is a file naming its git dir, whose
/// `commondir` names the shared one).
pub fn exclude_file(root: &Path) -> Option<PathBuf> {
    let git = root.join(".git");
    if git.is_dir() {
        return Some(git.join(EXCLUDE_IN_GIT_DIR));
    }
    let pointer = std::fs::read_to_string(&git).ok()?;
    let git_dir = root.join(pointer.strip_prefix("gitdir:")?.trim());
    let common = std::fs::read_to_string(git_dir.join("commondir"))
        .map(|c| git_dir.join(c.trim()))
        .unwrap_or(git_dir);
    Some(common.join(EXCLUDE_IN_GIT_DIR))
}

/// Hierarchical match of the absolute path `abs` against the directory
/// matcher set. A matcher applies only when `abs` is under its directory;
/// deeper matchers (later in the list) override shallower ones, so a
/// nested `!negation` re-includes a path a parent `.gitignore` dropped.
fn gitignore_decision(
    gitignores: &[Arc<Gitignore>],
    abs: &Path,
    is_dir: bool,
) -> ignore::Match<()> {
    let mut decision = ignore::Match::None;
    for gi in gitignores {
        if abs.starts_with(gi.path()) {
            match gi.matched_path_or_any_parents(abs, is_dir) {
                ignore::Match::None => {}
                ignore::Match::Ignore(_) => decision = ignore::Match::Ignore(()),
                ignore::Match::Whitelist(_) => decision = ignore::Match::Whitelist(()),
            }
        }
    }
    decision
}

/// Default-only shorthand for callers that don't have a configured
/// filter handy (e.g. the snapshot-sweep example binary). Applies the
/// always-on `.git` / `.oxplow` defaults only — no gitignore, no user
/// entries.
pub fn should_ignore_workspace_watch_path(path: &Path) -> bool {
    WorkspaceFilter::default().ignore(path, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    /// tsk1083: git's global excludes file (`core.excludesFile`) counts
    /// like any `.gitignore`; and the wiki, watched though git ignores it,
    /// is out of a tree (`ignore_in_trees`): trees are git's view.
    #[test]
    fn global_excludes_count_and_trees_follow_git_for_the_wiki() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(root.join(".oxplow/wiki")).unwrap();
        std::fs::write(
            root.join(".oxplow/.gitignore"),
            "*\n!.gitignore\n!project.yaml\n",
        )
        .unwrap();
        let global = dir.path().join("global-ignore");
        std::fs::write(&global, "**/.claude/settings.local.json\n").unwrap();
        let filter = WorkspaceFilter::build(
            &root,
            Vec::<String>::new(),
            Vec::<String>::new(),
            Some(global),
        );
        let local = Path::new(".claude/settings.local.json");
        assert!(filter.ignore(local, false));
        assert!(filter.ignore_in_trees(local, false));
        let wiki = Path::new(".oxplow/wiki/notes.md");
        assert!(!filter.ignore(wiki, false), "the wiki is watched");
        assert!(filter.ignore_in_trees(wiki, false), "git ignores it");
        let config = Path::new(".oxplow/project.yaml");
        assert!(!filter.ignore(config, false) && !filter.ignore_in_trees(config, false));
    }

    /// A harness's worktree inside the checkout (Claude's
    /// `.claude/worktrees/<name>`, a checkout of its own with a `.git`
    /// file) is out of the stream's scans when git's exclude names it, and
    /// in them when nothing does: the same rule as any other path.
    #[test]
    fn a_native_worktree_follows_git_s_exclude() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("repo");
        let tree = root.join(".claude/worktrees/x");
        std::fs::create_dir_all(root.join(".git/info")).unwrap();
        std::fs::create_dir_all(tree.join("src")).unwrap();
        std::fs::write(tree.join(".git"), "gitdir: ../../../.git/worktrees/x\n").unwrap();
        let none = Vec::<String>::new;
        let inside = Path::new(".claude/worktrees/x/src/a.rs");
        let filter = WorkspaceFilter::build(&root, none(), none(), None);
        assert!(!filter.ignore(inside, false), "nothing excludes it");
        assert!(!filter.ignore(Path::new(".claude/worktrees/x"), true));

        std::fs::write(root.join(".git/info/exclude"), ".claude/worktrees/\n").unwrap();
        let filter = WorkspaceFilter::build(&root, none(), none(), None);
        assert!(filter.ignore(inside, false));
        assert!(filter.ignore_in_trees(inside, false));
        assert!(filter.ignore(Path::new(".claude/worktrees/x"), true));
        assert!(!filter.ignore(Path::new("src/a.rs"), false));
    }

    /// A personal extension source (`.oxplow/.gitignore` ignores it) is
    /// watched but is no one's uncommitted work: out of a tree. A shared one
    /// and `project/` are in both.
    #[test]
    fn a_personal_extension_source_is_out_of_a_tree_but_watched() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(root.join(".oxplow/extensions")).unwrap();
        std::fs::write(
            root.join(".oxplow/.gitignore"),
            "*\n!.gitignore\n!project.yaml\n!extensions/\nextensions/*\n!extensions/project/\n!extensions/project/**\n!extensions/team/\n!extensions/team/**\n",
        )
        .unwrap();
        let f = WorkspaceFilter::build(&root, Vec::<String>::new(), Vec::<String>::new(), None);
        let mine = Path::new(".oxplow/extensions/my-pages/pages/a.yaml");
        assert!(!f.ignore(mine, false), "watched");
        assert!(f.ignore_in_trees(mine, false), "not uncommitted work");
        assert!(f.ignore_in_trees(Path::new(".oxplow/extensions/my-pages"), true));
        assert!(!f.ignore_in_trees(Path::new(".oxplow/extensions"), true));
        for kept in ["project", "team"] {
            let file = format!(".oxplow/extensions/{kept}/pages/a.yaml");
            assert!(!f.ignore_in_trees(Path::new(&file), false), "{kept}");
            let folder = format!(".oxplow/extensions/{kept}");
            assert!(!f.ignore_in_trees(Path::new(&folder), true), "{kept}");
        }
    }

    #[test]
    fn should_ignore_filters_oxplow_and_git() {
        // Always-on: anything under `.oxplow/` except `.oxplow/wiki/`,
        // and anything under `.git/`. Everything else is the user's
        // call via the `generated` config.
        assert!(should_ignore_workspace_watch_path(Path::new(
            ".oxplow/snapshots/aa/foo.tmp"
        )));
        assert!(!should_ignore_workspace_watch_path(Path::new(
            ".oxplow/wiki/local-snapshots.md"
        )));
        assert!(should_ignore_workspace_watch_path(Path::new(
            ".oxplow/local.sqlite"
        )));
        assert!(should_ignore_workspace_watch_path(Path::new(
            ".git/index.lock"
        )));
        // Build dirs are NOT default-ignored; users opt in via `generated`.
        assert!(!should_ignore_workspace_watch_path(Path::new(
            "target/debug/x.bin"
        )));
        assert!(!should_ignore_workspace_watch_path(Path::new(
            "node_modules/foo/index.js"
        )));
        assert!(!should_ignore_workspace_watch_path(Path::new(
            "src/main.rs"
        )));
        assert!(!should_ignore_workspace_watch_path(Path::new(
            "docs/README.md"
        )));
    }

    #[test]
    fn workspace_filter_user_segment_matches_anywhere() {
        let f = WorkspaceFilter::with_user_entries([".idea"]);
        assert!(f.ignore(Path::new(".idea/workspace.xml"), false));
        assert!(f.ignore(Path::new("nested/.idea/foo"), false));
        assert!(!f.ignore(Path::new("src/main.rs"), false));
    }

    #[test]
    fn workspace_filter_user_path_matches_prefix_only() {
        // Use a path entry whose segments don't collide with the
        // always-on defaults (no `dist`, `target`, etc.). That lets
        // us isolate the "matches prefix only" semantics.
        let f = WorkspaceFilter::with_user_entries(["apps/desktop/generated"]);
        assert!(f.ignore(Path::new("apps/desktop/generated"), false));
        assert!(f.ignore(Path::new("apps/desktop/generated/index.js"), false));
        assert!(!f.ignore(Path::new("apps/desktop"), false));
        // Not a free-floating match — only the exact prefix counts.
        assert!(!f.ignore(Path::new("crates/foo/apps/desktop/generated/x"), false));
    }

    #[test]
    fn workspace_filter_user_file_path_matches_exact() {
        let f = WorkspaceFilter::with_user_entries(["docs/generated/output.txt"]);
        assert!(f.ignore(Path::new("docs/generated/output.txt"), false));
        assert!(!f.ignore(Path::new("docs/generated/other.txt"), false));
    }

    #[test]
    fn workspace_filter_defaults_apply_even_with_empty_user_list() {
        // Defaults are `.git` (segment) and `.oxplow/*` (with a
        // `.oxplow/wiki/` carve-out). Build dirs are NOT defaults —
        // they require the user to add them to `generated`.
        let f = WorkspaceFilter::default();
        assert!(f.ignore(Path::new(".git/HEAD"), false));
        assert!(f.ignore(Path::new("crates/foo/.git/HEAD"), false));
        assert!(!f.ignore(Path::new("node_modules/x"), false));
        assert!(!f.ignore(Path::new("target/debug"), false));
        assert!(!f.ignore(Path::new("src/main.rs"), false));
    }

    #[test]
    fn workspace_filter_oxplow_wiki_passes_through() {
        let f = WorkspaceFilter::default();
        assert!(!f.ignore(Path::new(".oxplow/wiki/page.md"), false));
        assert!(f.ignore(Path::new(".oxplow/local.sqlite"), false));
    }

    #[test]
    fn workspace_filter_empty_entries_are_dropped() {
        let f = WorkspaceFilter::with_user_entries(["", "  ", "/"]);
        assert!(!f.ignore(Path::new("foo.txt"), false));
    }

    fn empty() -> Vec<String> {
        Vec::new()
    }

    #[test]
    fn for_project_respects_root_gitignore_at_every_depth() {
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join(".gitignore"),
            "node_modules/\n*.log\ndist/\n",
        )
        .unwrap();
        let f = WorkspaceFilter::for_project(dir.path(), empty(), empty());
        // Root-level pattern, matched at every depth (git semantics).
        assert!(f.ignore(Path::new("node_modules"), true));
        assert!(f.ignore(Path::new("node_modules/pkg/index.js"), false));
        assert!(f.ignore(Path::new("crates/foo/node_modules/x"), false));
        assert!(f.ignore(Path::new("build.log"), false));
        assert!(f.ignore(Path::new("dist/app.js"), false));
        // Tracked sources pass through.
        assert!(!f.ignore(Path::new("src/main.rs"), false));
        assert!(!f.ignore(Path::new("README.md"), false));
    }

    #[test]
    fn for_project_include_overrides_gitignore() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join(".gitignore"), "dist/\n").unwrap();
        let f = WorkspaceFilter::for_project(dir.path(), empty(), ["dist/keep.json"]);
        assert!(f.ignore(Path::new("dist/other.js"), false)); // still ignored
        assert!(!f.ignore(Path::new("dist/keep.json"), false)); // forced back in
    }

    #[test]
    fn for_project_exclude_adds_beyond_gitignore() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join(".gitignore"), "node_modules/\n").unwrap();
        // A noisy dir that isn't in .gitignore but we still don't want.
        let f = WorkspaceFilter::for_project(dir.path(), ["snapshots-out"], empty());
        assert!(f.ignore(Path::new("snapshots-out/big.json"), false));
        assert!(f.ignore(Path::new("node_modules/x"), false));
        assert!(!f.ignore(Path::new("src/main.rs"), false));
    }

    #[test]
    fn for_project_dir_only_pattern_uses_is_dir() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join(".gitignore"), "build/\n").unwrap();
        let f = WorkspaceFilter::for_project(dir.path(), empty(), empty());
        // `build/` is dir-only: a directory named `build` is pruned...
        assert!(f.ignore(Path::new("build"), true));
        // ...but a *file* literally named `build` is not matched by `build/`.
        assert!(!f.ignore(Path::new("build"), false));
    }

    #[test]
    fn for_project_without_gitignore_is_defaults_only() {
        let dir = tempdir().unwrap(); // no .gitignore written
        let f = WorkspaceFilter::for_project(dir.path(), empty(), empty());
        assert!(f.ignore(Path::new(".git/HEAD"), false));
        assert!(!f.ignore(Path::new("node_modules/x"), false));
        assert!(!f.ignore(Path::new("src/main.rs"), false));
    }

    #[test]
    fn for_project_loads_nested_gitignore_additions() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join(".gitignore"), "*.log\n").unwrap();
        std::fs::create_dir_all(dir.path().join("sub/local-out")).unwrap();
        // A nested .gitignore adds a pattern meaningful only under sub/.
        std::fs::write(dir.path().join("sub/.gitignore"), "local-out/\n").unwrap();
        let f = WorkspaceFilter::for_project(dir.path(), empty(), empty());
        // Root pattern still applies at every depth.
        assert!(f.ignore(Path::new("a.log"), false));
        assert!(f.ignore(Path::new("sub/b.log"), false));
        // Nested pattern applies under sub/ ...
        assert!(f.ignore(Path::new("sub/local-out"), true));
        assert!(f.ignore(Path::new("sub/local-out/x.txt"), false));
        // ... but NOT at the root — it's a nested addition, not global.
        assert!(!f.ignore(Path::new("local-out"), true));
        assert!(!f.ignore(Path::new("src/main.rs"), false));
    }

    /// Rules are read as they are needed: nothing walks the tree, so one
    /// written after the filter was built still counts, and folders nobody
    /// asks about are never read.
    #[test]
    fn a_filter_reads_only_the_gitignores_on_the_way_to_a_path() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join(".gitignore"), "*.log\n").unwrap();
        for d in ["a/b/c", "a/other"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        let filter = WorkspaceFilter::for_project(root, empty(), empty());
        // Written after it was built.
        std::fs::write(root.join("a/.gitignore"), "from-a\n").unwrap();
        std::fs::write(root.join("a/b/.gitignore"), "from-b\n!keep.log\n").unwrap();
        std::fs::write(root.join("a/other/.gitignore"), "x\n").unwrap();
        for (path, is_dir, ignored) in [
            ("a/b/x.log", false, true),
            ("a/b/keep.log", false, false),
            ("a/b/from-a", false, true),
            ("a/b/from-b", true, true),
            ("a/b/c", true, false),
            ("a/b/plain.rs", false, false),
        ] {
            assert_eq!(filter.ignore(Path::new(path), is_dir), ignored, "{path}");
        }
        let read = read(&filter.shared.rules);
        assert!(read.contains_key(Path::new("a/b")));
        assert!(!read.contains_key(Path::new("a/other")), "not on the way");
    }

    /// An ignored folder stops the way: its own rules aren't read.
    #[test]
    fn an_ignored_folders_rules_are_no_ones() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join(".gitignore"), "z\n").unwrap();
        std::fs::create_dir_all(root.join("z/deep")).unwrap();
        std::fs::write(root.join("z/.gitignore"), "!deep\n!deep/*\n").unwrap();
        let filter = WorkspaceFilter::for_project(root, empty(), empty());
        assert!(filter.ignore(Path::new("z/deep/f"), false));
        assert!(!read(&filter.shared.rules).contains_key(Path::new("z")));
    }

    /// One filter, however many hold it; an ignore-file edit is read again
    /// once it is noted, and a `generated` change reaches every clone.
    #[test]
    fn every_clone_is_the_same_filter_and_follows_a_change() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir(root.join("src")).unwrap();
        std::fs::write(root.join("src/.gitignore"), "old\n").unwrap();
        let filter = WorkspaceFilter::for_project(root, empty(), empty());
        let clone = filter.clone();
        assert!(clone.ignore(Path::new("src/old"), false));

        std::fs::write(root.join("src/.gitignore"), "new\n").unwrap();
        assert!(clone.ignore(Path::new("src/old"), false), "not noted yet");
        filter.note_ignore_file(Path::new("src/.gitignore"));
        assert!(!clone.ignore(Path::new("src/old"), false));
        assert!(clone.ignore(Path::new("src/new"), false));

        assert!(!clone.ignore(Path::new("build/x"), false));
        filter.set_user_entries(["build"], empty());
        assert!(clone.ignore(Path::new("build/x"), false));
    }

    /// A path a watch reports: under an ignored directory it is answered
    /// without asking what it is, and remembered so a burst costs a lookup
    /// each. A rule change forgets what was remembered.
    #[test]
    fn a_change_under_an_ignored_directory_needs_no_stat() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join(".gitignore"), "target/\n").unwrap();
        let filter = WorkspaceFilter::for_project(root, empty(), empty());
        let no_stat = || -> bool { panic!("asked what the path is") };
        assert!(filter.ignore_change(Path::new("a/target/deep/x.o"), no_stat));
        assert_eq!(
            read(&filter.shared.dirs).get(Path::new("a/target")),
            Some(&true)
        );
        assert!(!read(&filter.shared.dirs).contains_key(Path::new("a/target/deep")));
        assert!(!filter.ignore_change(Path::new("a/kept.rs"), || false));

        std::fs::write(root.join(".gitignore"), "").unwrap();
        filter.note_ignore_file(Path::new(".gitignore"));
        assert!(!filter.ignore_change(Path::new("a/target/deep/x.o"), || false));
    }

    /// A linked worktree reads its main checkout's exclude file, as git does.
    #[test]
    fn a_linked_worktree_follows_its_main_checkouts_exclude_file() {
        let dir = tempdir().unwrap();
        let main = dir.path().join("main");
        let tree = dir.path().join("tree");
        let git_dir = main.join(".git/worktrees/tree");
        std::fs::create_dir_all(&git_dir).unwrap();
        std::fs::create_dir_all(main.join(".git/info")).unwrap();
        std::fs::create_dir_all(&tree).unwrap();
        std::fs::write(main.join(".git/info/exclude"), "local-only\n").unwrap();
        std::fs::write(git_dir.join("commondir"), "../..\n").unwrap();
        std::fs::write(
            tree.join(".git"),
            format!("gitdir: {}\n", git_dir.display()),
        )
        .unwrap();
        let filter = WorkspaceFilter::build(&tree, empty(), empty(), None);
        assert!(filter.ignore(Path::new("local-only"), false));
        assert!(!filter.ignore(Path::new("other"), false));
    }

    #[test]
    fn for_project_nested_negation_reincludes() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join(".gitignore"), "*.secret\n").unwrap();
        std::fs::create_dir_all(dir.path().join("keep")).unwrap();
        // A deeper .gitignore re-includes via `!` — git's negation semantics.
        std::fs::write(dir.path().join("keep/.gitignore"), "!*.secret\n").unwrap();
        let f = WorkspaceFilter::for_project(dir.path(), empty(), empty());
        assert!(f.ignore(Path::new("a.secret"), false)); // ignored at root
        assert!(!f.ignore(Path::new("keep/a.secret"), false)); // re-included under keep/
    }

    #[test]
    fn oxplow_dir_honours_its_own_gitignore_negations() {
        // tsk176: `.oxplow` is oxplow's OWN state dir, so it's ignored by
        // default — watching the live SQLite would feed oxplow's writes back
        // through its own watcher. But oxplow writes `.oxplow/.gitignore` as
        // `*` / `!.gitignore` / `!project.yaml` precisely so the config is
        // shareable, and that `!` has to be honoured or the one file in there
        // meant to be committed is invisible to snapshots, Local History and
        // effort attribution.
        //
        // Generalised deliberately: this is NOT a `project.yaml` carve-out.
        // Whatever a project un-ignores in that file becomes visible.
        let dir = tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".oxplow/wiki")).unwrap();
        std::fs::create_dir_all(dir.path().join(".oxplow/snapshots")).unwrap();
        std::fs::write(
            dir.path().join(".oxplow/.gitignore"),
            "*\n!.gitignore\n!project.yaml\n!team-shared.toml\n",
        )
        .unwrap();
        let f = WorkspaceFilter::for_project(dir.path(), empty(), empty());

        // Negated → watched.
        assert!(
            !f.ignore(Path::new(".oxplow/project.yaml"), false),
            "project.yaml is un-ignored by `!` and must be watched"
        );
        assert!(
            !f.ignore(Path::new(".oxplow/team-shared.toml"), false),
            "ANY negated file, not just project.yaml — the rule is general"
        );

        // Everything else in oxplow's state dir stays ignored.
        assert!(f.ignore(Path::new(".oxplow/local.sqlite"), false));
        assert!(f.ignore(Path::new(".oxplow/snapshots/ab/cd"), false));
        assert!(f.ignore(Path::new(".oxplow/runtime/hooks.json"), false));
        // Subdirectories are pruned wholesale so a walk never descends into
        // the blob store; `.oxplow` itself must NOT be, or the walk stops
        // before it can see the negated files directly inside it.
        assert!(
            f.ignore(Path::new(".oxplow/snapshots"), true),
            "prune the blob store"
        );
        assert!(
            !f.ignore(Path::new(".oxplow"), true),
            "but descend into .oxplow itself"
        );
        assert!(
            !f.ignore(Path::new(".oxplow/wiki"), true),
            "and into the wiki"
        );
        // Extension sources are watched; their deployments are not.
        assert!(!f.ignore(Path::new(".oxplow/extensions"), true));
        assert!(!f.ignore(Path::new(".oxplow/extensions/mine/pages/a.yaml"), false));
        assert!(f.ignore(
            Path::new(".oxplow/runtime/extensions/mine/0123/a.yaml"),
            false
        ));

        // The wiki stays watched. This one CANNOT come from .gitignore:
        // it is deliberately not committed, but it IS snapshotted (Local
        // History's wiki badges), and gitignore can't say "don't commit but
        // do watch".
        assert!(!f.ignore(Path::new(".oxplow/wiki/page.md"), false));
    }

    #[test]
    fn oxplow_dir_stays_ignored_without_a_gitignore() {
        // No `.oxplow/.gitignore` at all (a project that predates it, or one
        // where the user deleted it) → the safe default still holds.
        let dir = tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".oxplow")).unwrap();
        let f = WorkspaceFilter::for_project(dir.path(), empty(), empty());
        assert!(f.ignore(Path::new(".oxplow/project.yaml"), false));
        assert!(f.ignore(Path::new(".oxplow/local.sqlite"), false));
        assert!(!f.ignore(Path::new(".oxplow/wiki/page.md"), false));
    }
}
