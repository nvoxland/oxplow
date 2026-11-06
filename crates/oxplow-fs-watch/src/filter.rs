//! The single source of truth for "is this workspace path ignored?".

use std::path::{Path, PathBuf};

/// Whether `path` is a file git reads ignore rules from: a consumer holding
/// its own filter reads it again on a change to one.
pub fn is_ignore_file(path: &Path) -> bool {
    path.file_name().is_some_and(|n| n == ".gitignore")
        || path.ends_with(Path::new(".git/info/exclude"))
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
#[derive(Clone, Default)]
pub struct WorkspaceFilter {
    exclude: Vec<FilterEntry>,
    include: Vec<FilterEntry>,
    /// Repo root (absolute). `ignore()` joins the repo-relative query
    /// path onto this so it can be matched against each per-directory
    /// matcher, which are rooted at absolute directories.
    root: PathBuf,
    /// One matcher per directory that has a `.gitignore` (the root entry
    /// also folds in `.git/info/exclude`), in shallowest-first order.
    /// Each is rooted at its own directory so nested patterns anchor
    /// correctly; deeper matchers override shallower ones.
    gitignores: std::sync::Arc<Vec<ignore::gitignore::Gitignore>>,
    /// Git's global excludes file, read again by [`Self::refreshed`].
    global: Option<PathBuf>,
}

impl std::fmt::Debug for WorkspaceFilter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkspaceFilter")
            .field("exclude", &self.exclude)
            .field("include", &self.include)
            .field("gitignores", &self.gitignores.len())
            .finish()
    }
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
        Self {
            exclude: parse_filter_entries(entries),
            include: Vec::new(),
            root: PathBuf::new(),
            gitignores: std::sync::Arc::new(Vec::new()),
            global: None,
        }
    }

    /// Build the project filter: `.gitignore`-aware (root + nested
    /// `.gitignore` files + `.git/info/exclude`), plus the project's
    /// `generated.exclude` and `generated.include` lists. `root` is the
    /// worktree directory (absolute).
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

    /// [`Self::for_project`] for a listing of `folder` (workspace-relative;
    /// empty: the root) alone: the rules that judge its entries are those
    /// of the root, of each folder on the way to it and of the folder
    /// itself, so only those `.gitignore`s are read, and the tree is not
    /// walked for the rest. It judges the folder's entries as
    /// [`Self::for_project`] does; a path elsewhere in the tree it may
    /// judge differently.
    pub fn for_folder<E, S, I, T>(root: &Path, exclude: E, include: I, folder: &str) -> Self
    where
        E: IntoIterator<Item = S>,
        S: AsRef<str>,
        I: IntoIterator<Item = T>,
        T: AsRef<str>,
    {
        let global = ignore::gitignore::gitconfig_excludes_path();
        let mut gitignores = vec![root_gitignore(root, global.as_deref())];
        let mut dir = root.to_path_buf();
        for segment in folder.split('/').filter(|s| !s.is_empty()) {
            dir.push(segment);
            // An ignored folder is never entered: its rules are no one's.
            if segment == ".git" || gitignore_decision(&gitignores, &dir, true).is_ignore() {
                break;
            }
            if let Some(gi) = folder_gitignore(&dir) {
                gitignores.push(gi);
            }
        }
        Self {
            exclude: parse_filter_entries(exclude),
            include: parse_filter_entries(include),
            root: root.to_path_buf(),
            gitignores: std::sync::Arc::new(gitignores),
            global,
        }
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
            exclude: parse_filter_entries(exclude),
            include: parse_filter_entries(include),
            root: root.to_path_buf(),
            gitignores: std::sync::Arc::new(collect_gitignores(root, global.as_deref())),
            global,
        }
    }

    /// This filter with its ignore files (`.gitignore`s, `.git/info/exclude`,
    /// the global excludes file) read again: the same user entries, git's
    /// current rules.
    pub fn refreshed(&self) -> Self {
        if self.root.as_os_str().is_empty() {
            return self.clone();
        }
        Self {
            gitignores: std::sync::Arc::new(collect_gitignores(&self.root, self.global.as_deref())),
            ..self.clone()
        }
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
                            let abs = self.root.join(path);
                            gitignore_decision(&self.gitignores, &abs, is_dir).is_ignore()
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
                        Some(_) => {
                            let abs = self.root.join(path);
                            !gitignore_decision(&self.gitignores, &abs, is_dir).is_whitelist()
                        }
                    };
                }
                if DEFAULT_IGNORED_SEGMENTS.contains(&s) {
                    return true;
                }
            }
        }

        // 2. `include` forces a path back in (overrides exclude + gitignore).
        if matches_filter_entries(&self.include, path) {
            return false;
        }
        // 3. Explicit excludes.
        if matches_filter_entries(&self.exclude, path) {
            return true;
        }
        // 4. `.gitignore` — root + nested, full hierarchical semantics.
        if !self.gitignores.is_empty() && !path.as_os_str().is_empty() {
            let abs = self.root.join(path);
            if gitignore_decision(&self.gitignores, &abs, is_dir).is_ignore() {
                return true;
            }
        }
        false
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

/// Build the per-directory `.gitignore` matcher set for `root`,
/// shallowest-first: the root matcher (`.git/info/exclude` + root
/// `.gitignore`) followed by every nested `.gitignore` reachable without
/// descending into an already-ignored directory. Each matcher is rooted
/// at its own directory so nested patterns anchor correctly.
fn collect_gitignores(root: &Path, global: Option<&Path>) -> Vec<ignore::gitignore::Gitignore> {
    let mut out = vec![root_gitignore(root, global)];
    collect_nested_gitignores(root, &mut out);
    out
}

/// The matcher of `root`: lowest precedence first, as git reads them, the
/// global excludes file (`core.excludesFile`), `.git/info/exclude`,
/// then the root `.gitignore`.
fn root_gitignore(root: &Path, global: Option<&Path>) -> ignore::gitignore::Gitignore {
    use ignore::gitignore::GitignoreBuilder;
    let mut builder = GitignoreBuilder::new(root);
    if let Some(global) = global {
        let _ = builder.add(global);
    }
    let _ = builder.add(root.join(".git").join("info").join("exclude"));
    let _ = builder.add(root.join(".gitignore"));
    builder
        .build()
        .unwrap_or_else(|_| ignore::gitignore::Gitignore::empty())
}

/// The matcher of the `.gitignore` in `dir`, when it has one.
fn folder_gitignore(dir: &Path) -> Option<ignore::gitignore::Gitignore> {
    use ignore::gitignore::GitignoreBuilder;
    let path = dir.join(".gitignore");
    if !path.is_file() {
        return None;
    }
    let mut builder = GitignoreBuilder::new(dir);
    let _ = builder.add(&path);
    builder.build().ok()
}

/// Recurse `dir`'s subdirectories, pruning any the matchers-so-far
/// already ignore (so we never descend into `node_modules`/`target`/…),
/// and append each surviving subdirectory's `.gitignore` matcher.
fn collect_nested_gitignores(dir: &Path, out: &mut Vec<ignore::gitignore::Gitignore>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_dir() {
            continue;
        }
        let abs = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        // `.git` is never interesting. `.oxplow` IS descended into, purely to
        // pick up its own `.gitignore` — that file's `!` negations are what
        // make a shareable `project.yaml` visible (tsk176). Its `*` rule then
        // prunes the rest of the directory on the next recursion, so this
        // costs one extra `read_dir`, not a walk of the snapshot store.
        if name == ".git" {
            continue;
        }
        // Don't descend into (or load `.gitignore`s from) an ignored dir.
        if gitignore_decision(out, &abs, true).is_ignore() {
            continue;
        }
        if let Some(gi) = folder_gitignore(&abs) {
            out.push(gi);
        }
        collect_nested_gitignores(&abs, out);
    }
}

/// Hierarchical match of the absolute path `abs` against the directory
/// matcher set. A matcher applies only when `abs` is under its directory;
/// deeper matchers (later in the list) override shallower ones, so a
/// nested `!negation` re-includes a path a parent `.gitignore` dropped.
fn gitignore_decision(
    gitignores: &[ignore::gitignore::Gitignore],
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

    /// A listing of one folder needs the rules above it and in it, not
    /// those of every folder in the tree: `for_folder` reads only those
    /// `.gitignore`s (no walk of the tree), and judges the folder's entries
    /// as `for_project` does.
    #[test]
    fn for_folder_reads_only_the_gitignores_on_the_way_to_the_folder() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join(".gitignore"), "*.log\n").unwrap();
        for d in ["a/b/c", "a/other", "z/deep"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        std::fs::write(root.join("a/.gitignore"), "from-a\n").unwrap();
        std::fs::write(root.join("a/b/.gitignore"), "from-b\n!keep.log\n").unwrap();
        // Off the way: another folder's rules.
        std::fs::write(root.join("a/other/.gitignore"), "x\n").unwrap();
        std::fs::write(root.join("z/.gitignore"), "y\n").unwrap();
        std::fs::write(root.join("z/deep/.gitignore"), "w\n").unwrap();

        let all = WorkspaceFilter::for_project(root, empty(), empty());
        let one = WorkspaceFilter::for_folder(root, empty(), empty(), "a/b");
        // Root, `a`, `a/b`: three matchers, against six for the walk.
        assert_eq!(one.gitignores.len(), 3);
        assert_eq!(all.gitignores.len(), 6);
        for (path, is_dir) in [
            ("a/b/x.log", false),
            ("a/b/keep.log", false),
            ("a/b/from-a", false),
            ("a/b/from-b", true),
            ("a/b/c", true),
            ("a/b/plain.rs", false),
        ] {
            assert_eq!(
                one.ignore(Path::new(path), is_dir),
                all.ignore(Path::new(path), is_dir),
                "{path}"
            );
        }
        // The root of the tree is the root matcher alone.
        assert_eq!(
            WorkspaceFilter::for_folder(root, empty(), empty(), "")
                .gitignores
                .len(),
            1
        );
        // An ignored folder stops the way: its own rules aren't read.
        std::fs::write(root.join(".gitignore"), "*.log\nz\n").unwrap();
        let blocked = WorkspaceFilter::for_folder(root, empty(), empty(), "z/deep");
        assert!(blocked.ignore(Path::new("z"), true));
        assert_eq!(blocked.gitignores.len(), 1);
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
