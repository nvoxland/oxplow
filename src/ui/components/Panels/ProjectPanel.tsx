import type { CSSProperties } from "react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  getBranchChanges,
  getChangeScopes,
  gitAddPath,
  gitAppendToGitignore,
  gitCommitAll,
  gitPull,
  gitPush,
  gitRestorePath,
  listAllRefs,
  listFileCommits,
  listWorkspaceEntries,
  listWorkspaceFiles,
  readWorkspaceFile,
  searchWorkspaceText,
  subscribeGitRefsEvents,
  subscribeWorkspaceEvents,
  type AgentTurn,
  type BatchFileChange,
  type ChangeScopes,
  type GitLogCommit,
  type GitOpResult,
  type RefOption,
  type Stream,
  type TextSearchHit,
  type WorkspaceEntry,
  type WorkspaceIndexedFile,
  type WorkspaceStatusSummary,
} from "../../api.js";
import type { DiffRequest } from "../Diff/diff-request.js";
import type { MenuItem } from "../../menu.js";
import { ContextMenu } from "../ContextMenu.js";
import { TreeEntries } from "../LeftPanel/FileTree.js";
import { GitSummary } from "../LeftPanel/GitSummary.js";
import { copyText, dirname, joinChildPath, type ContextMenuTarget } from "../LeftPanel/shared.js";

interface Props {
  stream: Stream | null;
  gitEnabled: boolean;
  selectedFilePath: string | null;
  currentBatchTurns: AgentTurn[] | null;
  currentBatchFileChanges: BatchFileChange[] | null;
  generatedDirs: string[];
  onOpenFile(path: string): void;
  onOpenDiff?(request: DiffRequest): void;
  onCreateFile(path: string): Promise<void>;
  onCreateDirectory(path: string): Promise<void>;
  onRenamePath(fromPath: string, toPath: string): Promise<void>;
  onDeletePath(path: string): Promise<void>;
  onToggleGeneratedDir(name: string, mark: boolean): Promise<void>;
}

export function ProjectPanel({
  stream,
  gitEnabled,
  selectedFilePath,
  currentBatchTurns,
  currentBatchFileChanges,
  generatedDirs,
  onOpenFile,
  onOpenDiff,
  onCreateFile,
  onCreateDirectory,
  onRenamePath,
  onDeletePath,
  onToggleGeneratedDir,
}: Props) {
  const [expandedDirs, setExpandedDirs] = useState<Record<string, boolean>>({ "": true });
  const [entriesByDir, setEntriesByDir] = useState<Record<string, WorkspaceEntry[]>>({});
  const [loadingDirs, setLoadingDirs] = useState<Record<string, boolean>>({});
  const [indexedFiles, setIndexedFiles] = useState<WorkspaceIndexedFile[]>([]);
  const [statusSummary, setStatusSummary] = useState<WorkspaceStatusSummary | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [contextMenu, setContextMenu] = useState<ContextMenuTarget | null>(null);
  const loadingDirsRef = useRef<Record<string, boolean>>({});

  const loadDir = useCallback(async (path: string) => {
    if (!stream || loadingDirsRef.current[path]) return;
    loadingDirsRef.current = { ...loadingDirsRef.current, [path]: true };
    setLoadingDirs((prev) => ({ ...prev, [path]: true }));
    try {
      const entries = await listWorkspaceEntries(stream.id, path);
      setEntriesByDir((prev) => ({ ...prev, [path]: sortFileTreeEntries(entries) }));
      setError(null);
    } catch (e) {
      setError(String(e));
    } finally {
      loadingDirsRef.current = { ...loadingDirsRef.current, [path]: false };
      setLoadingDirs((prev) => ({ ...prev, [path]: false }));
    }
  }, [stream]);

  const loadWorkspaceIndex = useCallback(async () => {
    if (!stream) return;
    try {
      const result = await listWorkspaceFiles(stream.id);
      setIndexedFiles(result.files);
      setStatusSummary(result.summary);
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }, [stream]);

  useEffect(() => {
    setExpandedDirs({ "": true });
    setEntriesByDir({});
    loadingDirsRef.current = {};
    setLoadingDirs({});
    setIndexedFiles([]);
    setStatusSummary(null);
    setError(null);
    setContextMenu(null);
  }, [stream?.id]);

  useEffect(() => {
    if (!stream) return;
    void loadDir("");
    void loadWorkspaceIndex();
  }, [stream?.id, loadDir, loadWorkspaceIndex]);

  useEffect(() => {
    if (!stream) return;
    let refreshTimer: ReturnType<typeof setTimeout> | null = null;
    const refreshForPath = (path: string) => {
      const parentDir = dirname(path);
      void loadWorkspaceIndex();
      void loadDir(parentDir);
    };
    const prunePath = (path: string) => {
      setEntriesByDir((prev) => Object.fromEntries(
        Object.entries(prev).filter(([candidate]) => candidate !== path && !candidate.startsWith(path + "/")),
      ));
      setExpandedDirs((prev) => Object.fromEntries(
        Object.entries(prev).filter(([candidate]) => candidate !== path && !candidate.startsWith(path + "/")),
      ));
    };
    const unsubscribe = subscribeWorkspaceEvents(stream.id, (event) => {
      if (event.kind === "updated") return;
      if (event.kind === "deleted") {
        prunePath(event.path);
      }
      if (refreshTimer) clearTimeout(refreshTimer);
      refreshTimer = setTimeout(() => refreshForPath(event.path), 75);
    });
    // Git ops (commit / reset / pull / push) change status markers without
    // touching the worktree entries we already have; refresh the index so the
    // colored status badges stay in sync.
    const unsubscribeRefs = subscribeGitRefsEvents(stream.id, () => {
      if (refreshTimer) clearTimeout(refreshTimer);
      refreshTimer = setTimeout(() => { void loadWorkspaceIndex(); }, 150);
    });
    return () => {
      unsubscribe();
      unsubscribeRefs();
      if (refreshTimer) clearTimeout(refreshTimer);
    };
  }, [loadDir, loadWorkspaceIndex, stream]);

  type FilterMode = "all" | "uncommitted" | "branch" | "unpushed" | "turn";
  const [filterMode, setFilterMode] = useState<FilterMode>("all");
  const [scopes, setScopes] = useState<ChangeScopes | null>(null);
  // Paths that the filter includes (always the superset — deleted files end
  // up here too so they pass the tree filter below).
  const [scopedPaths, setScopedPaths] = useState<string[] | null>(null);
  // Paths known to be deleted in the current scope. These are phantom entries
  // — they don't exist on disk, so we inject them into the tree manually.
  const [scopedDeletions, setScopedDeletions] = useState<Set<string>>(() => new Set());
  const [selectedTurnId, setSelectedTurnId] = useState<string | null>(null);
  const rootEntries = useMemo(() => entriesByDir[""] ?? [], [entriesByDir]);
  const generatedSet = useMemo(() => new Set(generatedDirs), [generatedDirs]);
  const uncommittedPaths = useMemo(
    () => indexedFiles.filter((f) => f.gitStatus !== null).map((f) => f.path),
    [indexedFiles],
  );
  const uncommittedDeletions = useMemo(
    () => new Set(indexedFiles.filter((f) => f.gitStatus === "deleted").map((f) => f.path)),
    [indexedFiles],
  );

  // The last 10 turns from the current batch, newest first. Used to populate
  // the "Turn" filter's sub-dropdown.
  const recentTurns = useMemo(() => {
    const turns = currentBatchTurns ?? [];
    return [...turns]
      .sort((a, b) => (a.started_at < b.started_at ? 1 : -1))
      .slice(0, 10);
  }, [currentBatchTurns]);

  // Default the selected turn when the user picks the Turn mode.
  useEffect(() => {
    if (filterMode !== "turn") return;
    if (selectedTurnId && recentTurns.some((t) => t.id === selectedTurnId)) return;
    setSelectedTurnId(recentTurns[0]?.id ?? null);
  }, [filterMode, recentTurns, selectedTurnId]);

  useEffect(() => {
    if (!stream || !gitEnabled) { setScopes(null); return; }
    let cancelled = false;
    void getChangeScopes(stream.id)
      .then((result) => { if (!cancelled) setScopes(result); })
      .catch(() => { if (!cancelled) setScopes(null); });
    return () => { cancelled = true; };
  }, [stream?.id, gitEnabled]);

  // If the user had a scope selected that became unavailable (e.g., switched
  // to the default branch), fall back to a sensible default.
  useEffect(() => {
    if (!scopes) return;
    if (filterMode === "branch" && (scopes.onDefaultBranch || !scopes.branchBase)) setFilterMode("uncommitted");
    if (filterMode === "unpushed" && !scopes.upstream) setFilterMode("uncommitted");
  }, [scopes, filterMode]);

  // Load paths+deletions for the currently-selected scope.
  // - Uncommitted: read directly from the workspace index (already subscribed).
  // - Branch/Unpushed: `getBranchChanges` against the appropriate ref.
  // - Turn: filter the batch's file-change log by `turn_id`.
  // Deletions are tracked separately so we can inject phantom rows into the
  // tree (the filesystem no longer has them).
  useEffect(() => {
    if (!stream) {
      setScopedPaths(null);
      setScopedDeletions(new Set());
      return;
    }
    if (filterMode === "all") {
      setScopedPaths(null);
      setScopedDeletions(new Set());
      return;
    }
    if (filterMode === "uncommitted") {
      setScopedPaths(uncommittedPaths);
      setScopedDeletions(uncommittedDeletions);
      return;
    }
    if (filterMode === "turn") {
      const changes = (currentBatchFileChanges ?? []).filter((c) => c.turn_id === selectedTurnId);
      // A single turn can write a file multiple times — collapse to the last
      // recorded kind per path (we assume the change log is ordered by time).
      const kindByPath = new Map<string, BatchFileChange["change_kind"]>();
      for (const change of changes) kindByPath.set(change.path, change.change_kind);
      setScopedPaths([...kindByPath.keys()]);
      setScopedDeletions(new Set(
        [...kindByPath.entries()].filter(([, kind]) => kind === "deleted").map(([path]) => path),
      ));
      return;
    }
    if (!gitEnabled) { setScopedPaths(null); setScopedDeletions(new Set()); return; }
    const ref = filterMode === "branch" ? scopes?.branchBase : scopes?.upstream;
    if (!ref) { setScopedPaths([]); setScopedDeletions(new Set()); return; }
    let cancelled = false;
    void getBranchChanges(stream.id, ref)
      .then((result) => {
        if (cancelled) return;
        setScopedPaths(result.files.map((f) => f.path));
        setScopedDeletions(new Set(result.files.filter((f) => f.status === "deleted").map((f) => f.path)));
      })
      .catch(() => {
        if (cancelled) return;
        setScopedPaths([]);
        setScopedDeletions(new Set());
      });
    return () => { cancelled = true; };
  }, [stream?.id, gitEnabled, filterMode, scopes?.branchBase, scopes?.upstream, uncommittedPaths, uncommittedDeletions, indexedFiles, selectedTurnId, currentBatchFileChanges]);

  const changedPathSet = useMemo(() => {
    const paths = scopedPaths ?? [];
    const set = new Set<string>();
    for (const path of paths) {
      set.add(path);
      let dir = dirname(path);
      while (dir) {
        if (set.has(dir)) break;
        set.add(dir);
        dir = dirname(dir);
      }
    }
    return set;
  }, [scopedPaths]);
  // "Turn" doesn't need a git repo, so `effectiveChangedOnly` must not gate on
  // gitEnabled alone. Any non-"all" mode filters the tree.
  const effectiveChangedOnly = filterMode !== "all";

  // When the user turns on "Changed only", auto-expand every ancestor directory
  // of a changed file and load any that haven't been fetched yet. Otherwise
  // the filter can leave the user staring at a collapsed root with nothing
  // visible because the matches are nested.
  useEffect(() => {
    if (!effectiveChangedOnly || !stream || !scopedPaths) return;
    const dirsToOpen = new Set<string>([""]);
    for (const path of scopedPaths) {
      let dir = dirname(path);
      while (dir) {
        dirsToOpen.add(dir);
        dir = dirname(dir);
      }
    }
    setExpandedDirs((prev) => {
      const next = { ...prev };
      for (const dir of dirsToOpen) next[dir] = true;
      return next;
    });
    for (const dir of dirsToOpen) {
      if (!entriesByDir[dir] && !loadingDirsRef.current[dir]) void loadDir(dir);
    }
  }, [effectiveChangedOnly, scopedPaths, entriesByDir, loadDir, stream]);
  const visibleEntriesByDir = useMemo(() => {
    if (!effectiveChangedOnly) return entriesByDir;
    const out: Record<string, WorkspaceEntry[]> = {};
    for (const [dir, entries] of Object.entries(entriesByDir)) {
      out[dir] = entries.filter((entry) => changedPathSet.has(entry.path));
    }
    // Inject phantom entries for deleted files — they don't exist on disk so
    // listWorkspaceEntries doesn't return them. We need a row in the tree so
    // the user can see that the file was touched in this scope.
    for (const path of scopedDeletions) {
      const parent = dirname(path);
      const name = path.slice(parent.length > 0 ? parent.length + 1 : 0);
      const entry: WorkspaceEntry = {
        name,
        path,
        kind: "file",
        gitStatus: "deleted",
        hasChanges: true,
      };
      const list = out[parent] ?? [];
      if (!list.some((e) => e.path === path)) {
        list.push(entry);
        // Sort to keep the tree's alphabetical ordering stable.
        list.sort((a, b) => a.name.localeCompare(b.name));
        out[parent] = list;
      }
      // Also make sure every ancestor dir is represented, otherwise a deleted
      // file under a dir that has no surviving children wouldn't render.
      let dir = parent;
      while (dir) {
        const grand = dirname(dir);
        const dirName = dir.slice(grand.length > 0 ? grand.length + 1 : 0);
        const siblingList = out[grand] ?? [];
        if (!siblingList.some((e) => e.path === dir)) {
          siblingList.push({
            name: dirName,
            path: dir,
            kind: "directory",
            gitStatus: null,
            hasChanges: true,
          });
          siblingList.sort((a, b) => a.name.localeCompare(b.name));
          out[grand] = siblingList;
        }
        dir = grand;
      }
    }
    return out;
  }, [effectiveChangedOnly, entriesByDir, changedPathSet, scopedDeletions]);
  const visibleRootEntries = visibleEntriesByDir[""] ?? [];

  const [findUsagesState, setFindUsagesState] = useState<{
    query: string;
    path: string;
    results: TextSearchHit[] | null;
    loading: boolean;
  } | null>(null);
  const [fileHistoryState, setFileHistoryState] = useState<{
    path: string;
    commits: GitLogCommit[] | null;
    loading: boolean;
  } | null>(null);
  const [compareState, setCompareState] = useState<{
    path: string;
    refs: RefOption[] | null;
    loading: boolean;
  } | null>(null);
  const [agentHistoryState, setAgentHistoryState] = useState<{ path: string } | null>(null);
  const [opResult, setOpResult] = useState<{ title: string; result: GitOpResult } | null>(null);
  const [pushPullDialog, setPushPullDialog] = useState<"push" | "pull" | null>(null);
  const [commitDialogOpen, setCommitDialogOpen] = useState(false);

  if (!stream) {
    return <div style={{ padding: 12, color: "var(--muted)", fontSize: 12 }}>loading stream…</div>;
  }

  async function toggleDirectory(path: string) {
    const nextExpanded = !expandedDirs[path];
    setExpandedDirs((prev) => ({ ...prev, [path]: nextExpanded }));
    if (nextExpanded && !entriesByDir[path]) {
      await loadDir(path);
    }
  }

  async function expandAll() {
    // Expand everything we currently know about, and fetch the children of
    // any expanded dir we haven't loaded yet. Clicking again after children
    // arrive will expand the next level.
    const known = Object.keys(entriesByDir);
    const dirsToExpand = new Set<string>(["", ...known]);
    for (const entries of Object.values(entriesByDir)) {
      for (const entry of entries) {
        if (entry.kind === "directory") dirsToExpand.add(entry.path);
      }
    }
    setExpandedDirs((prev) => {
      const next = { ...prev };
      for (const dir of dirsToExpand) next[dir] = true;
      return next;
    });
    await Promise.all(
      [...dirsToExpand]
        .filter((dir) => !entriesByDir[dir] && !loadingDirsRef.current[dir])
        .map((dir) => loadDir(dir)),
    );
  }

  function collapseAll() {
    setExpandedDirs({ "": true });
  }

  function openUncommittedDiff(path: string) {
    onOpenDiff?.({ path, leftRef: "HEAD", rightKind: "working", baseLabel: "HEAD" });
  }
  function openBranchDiff(path: string) {
    if (!scopes?.branchBase) return;
    onOpenDiff?.({ path, leftRef: scopes.branchBase, rightKind: "working", baseLabel: scopes.branchBase });
  }
  function openOriginDiff(path: string) {
    if (!scopes?.upstream) return;
    onOpenDiff?.({ path, leftRef: scopes.upstream, rightKind: "working", baseLabel: scopes.upstream });
  }

  // The file tree's click/double-click opens something contextual to the
  // current filter — file in "all", matching diff in scope-filtered views.
  function openForCurrentFilter(path: string) {
    if (filterMode === "uncommitted") { openUncommittedDiff(path); return; }
    if (filterMode === "branch" && scopes?.branchBase) { openBranchDiff(path); return; }
    if (filterMode === "unpushed" && scopes?.upstream) { openOriginDiff(path); return; }
    onOpenFile(path);
  }

  async function handleContextAction(
    action:
      | "open" | "new-file" | "new-folder" | "rename" | "delete"
      | "copy" | "copy-reference" | "find-usages" | "agent-history"
      | "git-show-history" | "git-rollback" | "git-compare" | "git-gitignore" | "git-add"
      | "mark-generated" | "unmark-generated"
      | "diff-uncommitted" | "diff-branch" | "diff-origin",
  ) {
    if (!contextMenu) return;
    try {
      switch (action) {
        case "open":
          onOpenFile(contextMenu.path);
          break;
        case "diff-uncommitted":
          openUncommittedDiff(contextMenu.path);
          break;
        case "diff-branch":
          openBranchDiff(contextMenu.path);
          break;
        case "diff-origin":
          openOriginDiff(contextMenu.path);
          break;
        case "new-file": {
          const suggested = contextMenu.kind === "directory"
            ? joinChildPath(contextMenu.path, "new-file.txt")
            : joinChildPath(dirname(contextMenu.path), "new-file.txt");
          const nextPath = window.prompt("New file path", suggested)?.trim();
          if (!nextPath) return;
          await onCreateFile(nextPath);
          break;
        }
        case "new-folder": {
          const suggested = contextMenu.kind === "directory"
            ? joinChildPath(contextMenu.path, "new-folder")
            : joinChildPath(dirname(contextMenu.path), "new-folder");
          const nextPath = window.prompt("New folder path", suggested)?.trim();
          if (!nextPath) return;
          await onCreateDirectory(nextPath);
          break;
        }
        case "rename": {
          const nextPath = window.prompt("Rename path", contextMenu.path)?.trim();
          if (!nextPath || nextPath === contextMenu.path) return;
          await onRenamePath(contextMenu.path, nextPath);
          break;
        }
        case "delete":
          if (!window.confirm(`Delete ${contextMenu.path}?`)) return;
          await onDeletePath(contextMenu.path);
          break;
        case "copy": {
          if (contextMenu.kind === "directory") {
            await copyText(contextMenu.path);
          } else if (stream) {
            // Copy file contents. For large files this is still useful.
            const file = await readWorkspaceFile(stream.id, contextMenu.path);
            await copyText(file.content);
          }
          break;
        }
        case "copy-reference":
          await copyText(contextMenu.path);
          break;
        case "find-usages": {
          if (!stream) return;
          // "Usages" here = text search for the basename (without extension).
          // A file's path is usually what shows up in imports/require() calls
          // across the codebase, so this catches the common cases without
          // trying to be a real language-aware symbol search.
          const basename = contextMenu.path.split("/").pop() ?? contextMenu.path;
          const withoutExt = basename.replace(/\.[^.]+$/, "");
          const query = withoutExt.length >= 2 ? withoutExt : basename;
          setFindUsagesState({ query, path: contextMenu.path, results: null, loading: true });
          setContextMenu(null);
          const hits = await searchWorkspaceText(stream.id, query, { limit: 200 });
          setFindUsagesState({ query, path: contextMenu.path, results: hits, loading: false });
          return;
        }
        case "agent-history":
          setAgentHistoryState({ path: contextMenu.path });
          setContextMenu(null);
          return;
        case "git-show-history": {
          if (!stream) return;
          setFileHistoryState({ path: contextMenu.path, commits: null, loading: true });
          setContextMenu(null);
          const commits = await listFileCommits(stream.id, contextMenu.path, 100);
          setFileHistoryState({ path: contextMenu.path, commits, loading: false });
          return;
        }
        case "git-compare": {
          if (!stream) return;
          setCompareState({ path: contextMenu.path, refs: null, loading: true });
          setContextMenu(null);
          const refs = await listAllRefs(stream.id);
          setCompareState({ path: contextMenu.path, refs, loading: false });
          return;
        }
        case "git-rollback": {
          if (!stream) return;
          if (!window.confirm(`Rollback ${contextMenu.path} to HEAD? Uncommitted changes will be lost.`)) return;
          const result = await gitRestorePath(stream.id, contextMenu.path);
          setOpResult({ title: `Rollback ${contextMenu.path}`, result });
          break;
        }
        case "git-add": {
          if (!stream) return;
          const result = await gitAddPath(stream.id, contextMenu.path);
          setOpResult({ title: `git add ${contextMenu.path}`, result });
          break;
        }
        case "git-gitignore": {
          if (!stream) return;
          const result = await gitAppendToGitignore(stream.id, contextMenu.path);
          setOpResult({ title: `Add ${contextMenu.path} to .gitignore`, result });
          break;
        }
        case "mark-generated":
          await onToggleGeneratedDir(contextMenu.name, true);
          break;
        case "unmark-generated":
          await onToggleGeneratedDir(contextMenu.name, false);
          break;
      }
      setError(null);
    } catch (e) {
      setError(String(e));
    } finally {
      setContextMenu(null);
    }
  }

  const isUntracked = contextMenu?.kind === "file"
    && indexedFiles.some((f) => f.path === contextMenu.path && f.gitStatus === "untracked");
  const isDirMarkedGenerated = contextMenu?.kind === "directory"
    && generatedDirs.includes(contextMenu.name);
  const contextMenuItems: MenuItem[] = contextMenu
    ? [
      ...(contextMenu.kind === "file"
        ? [
            { id: "files.open", label: "Open", enabled: true, run: () => handleContextAction("open") },
            ...(gitEnabled && !!onOpenDiff
              ? [
                  {
                    id: "files.diff-uncommitted",
                    label: "Show uncommitted",
                    enabled: uncommittedPaths.includes(contextMenu.path) || uncommittedDeletions.has(contextMenu.path),
                    run: () => handleContextAction("diff-uncommitted"),
                  },
                  {
                    id: "files.diff-branch",
                    label: scopes?.branchBase ? `Show branch changes (vs ${scopes.branchBase})` : "Show branch changes",
                    enabled: !!scopes?.branchBase && !scopes.onDefaultBranch,
                    run: () => handleContextAction("diff-branch"),
                  },
                  {
                    id: "files.diff-origin",
                    label: scopes?.upstream ? `Show difference from origin (vs ${scopes.upstream})` : "Show difference from origin",
                    enabled: !!scopes?.upstream,
                    run: () => handleContextAction("diff-origin"),
                  },
                ] as MenuItem[]
              : []),
          ]
        : []),
      { id: "files.new-file", label: "New File…", enabled: true, run: () => handleContextAction("new-file") },
      { id: "files.new-folder", label: "New Directory…", enabled: true, run: () => handleContextAction("new-folder") },
      { id: "files.copy", label: contextMenu.kind === "directory" ? "Copy Path" : "Copy", enabled: true, run: () => handleContextAction("copy") },
      { id: "files.copy-reference", label: "Copy Reference", enabled: true, run: () => handleContextAction("copy-reference") },
      ...(contextMenu.kind === "file"
        ? [{ id: "files.find-usages", label: "Find Usages", enabled: true, run: () => handleContextAction("find-usages") }]
        : []),
      { id: "files.agent-history", label: "Agent History", enabled: !!currentBatchFileChanges, run: () => handleContextAction("agent-history") },
      ...(gitEnabled ? [{
        id: "files.git",
        label: "Git",
        enabled: true,
        submenu: [
          { id: "files.git.show-history", label: "Show History", enabled: contextMenu.kind === "file", run: () => handleContextAction("git-show-history") },
          { id: "files.git.rollback", label: "Rollback", enabled: true, run: () => handleContextAction("git-rollback") },
          { id: "files.git.compare", label: "Compare With…", enabled: contextMenu.kind === "file", run: () => handleContextAction("git-compare") },
          { id: "files.git.gitignore", label: "Add to .gitignore", enabled: true, run: () => handleContextAction("git-gitignore") },
          { id: "files.git.add", label: "Add", enabled: isUntracked, run: () => handleContextAction("git-add") },
        ] as MenuItem[],
      }] : []),
      ...(contextMenu.kind === "directory"
        ? [
          isDirMarkedGenerated
            ? {
                id: "files.unmark-generated",
                label: `Unmark "${contextMenu.name}" as Generated`,
                enabled: true,
                run: () => handleContextAction("unmark-generated"),
              }
            : {
                id: "files.mark-generated",
                label: `Mark "${contextMenu.name}" as Generated`,
                enabled: true,
                run: () => handleContextAction("mark-generated"),
              },
        ]
        : []),
      { id: "files.rename", label: "Rename…", enabled: true, run: () => handleContextAction("rename") },
      { id: "files.delete", label: "Delete…", enabled: true, run: () => handleContextAction("delete") },
    ]
    : [];

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%", fontSize: 12, overflow: "hidden" }}>
      <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", padding: "6px 12px", borderBottom: "1px solid var(--border)", gap: 6 }}>
        <div style={{ color: "var(--muted)", fontSize: 11, textTransform: "uppercase", letterSpacing: 0.6, flex: 1, minWidth: 0, overflow: "hidden", textOverflow: "ellipsis" }}>{stream.branch}</div>
        <button type="button"
          onClick={() => void expandAll()}
          aria-label="Expand all"
          title="Expand all"
          style={iconButtonStyle}
        >⤡</button>
        <button type="button"
          onClick={collapseAll}
          aria-label="Collapse all"
          title="Collapse all"
          style={iconButtonStyle}
        >⤢</button>
        {gitEnabled ? (
          <>
            {uncommittedPaths.length > 0 ? (
              <button
                type="button"
                data-testid="files-commit"
                onClick={() => setCommitDialogOpen(true)}
                aria-label={`Commit ${uncommittedPaths.length} uncommitted change${uncommittedPaths.length === 1 ? "" : "s"}`}
                title={`Commit ${uncommittedPaths.length} uncommitted change${uncommittedPaths.length === 1 ? "" : "s"}`}
                style={commitButtonStyle}
              >
                Commit ({uncommittedPaths.length})
              </button>
            ) : null}
            <button type="button" onClick={() => setPushPullDialog("pull")} aria-label="Pull…" title="Pull…" style={iconButtonStyle}>↓</button>
            <button type="button" onClick={() => setPushPullDialog("push")} aria-label="Push…" title="Push…" style={iconButtonStyle}>↑</button>
          </>
        ) : null}
        <FilterMenuButton
          filterMode={filterMode}
          setFilterMode={setFilterMode}
          gitEnabled={gitEnabled}
          scopes={scopes}
          recentTurns={recentTurns}
        />
        {filterMode === "turn" && recentTurns.length > 0 ? (
          <select
            value={selectedTurnId ?? ""}
            onChange={(e) => setSelectedTurnId(e.target.value || null)}
            style={filterSelectStyle}
            title="Show files touched in this agent turn"
          >
            {recentTurns.map((turn, index) => (
              <option key={turn.id} value={turn.id}>
                #{recentTurns.length - index} · {truncate(turn.prompt, 40)}
              </option>
            ))}
          </select>
        ) : null}
      </div>
      <div style={{ flex: 1, overflowX: "auto", overflowY: "auto", padding: 12, display: "flex", flexDirection: "column", gap: 8, minWidth: "100%", width: "max-content" }}>
        {gitEnabled && statusSummary ? <GitSummary summary={statusSummary} /> : null}
        {error ? <div style={{ color: "#ff6b6b" }}>{error}</div> : null}
        {rootEntries.length === 0 && !loadingDirs[""] ? (
          <div style={{ color: "var(--muted)" }}>No files loaded yet.</div>
        ) : (
          <>
            {effectiveChangedOnly && scopedPaths !== null && scopedPaths.length === 0 ? (
              <div style={{ color: "var(--muted)" }}>No changes in this scope.</div>
            ) : null}
            <TreeEntries
              parentPath=""
              entries={visibleRootEntries}
              entriesByDir={visibleEntriesByDir}
              expandedDirs={expandedDirs}
              loadingDirs={loadingDirs}
              selectedFilePath={selectedFilePath}
              generatedSet={generatedSet}
              onToggleDirectory={toggleDirectory}
              onOpenFile={openForCurrentFilter}
              onContextMenu={setContextMenu}
            />
          </>
        )}
      </div>
      <div style={filterStatusBarStyle}>
        <span>Showing: {filterModeLabel(filterMode, scopes, selectedTurnId, recentTurns)}</span>
      </div>
      {contextMenu ? (
        <ContextMenu
          items={contextMenuItems}
          position={{ x: contextMenu.x, y: contextMenu.y }}
          onClose={() => setContextMenu(null)}
          minWidth={180}
        />
      ) : null}
      {findUsagesState ? (
        <FindUsagesModal
          state={findUsagesState}
          onClose={() => setFindUsagesState(null)}
          onOpen={(path) => {
            onOpenFile(path);
            setFindUsagesState(null);
          }}
        />
      ) : null}
      {fileHistoryState ? (
        <FileHistoryModal
          state={fileHistoryState}
          onClose={() => setFileHistoryState(null)}
          onOpenDiff={(sha, parent) => {
            if (!onOpenDiff) return;
            const left = parent ?? "4b825dc642cb6eb9a060e54bf8d69288fbee4904";
            onOpenDiff({
              path: fileHistoryState.path,
              leftRef: left,
              rightKind: { ref: sha },
              baseLabel: parent ? parent.slice(0, 7) : "(root)",
            });
            setFileHistoryState(null);
          }}
        />
      ) : null}
      {compareState ? (
        <CompareWithModal
          state={compareState}
          onClose={() => setCompareState(null)}
          onPick={(ref) => {
            if (onOpenDiff) {
              onOpenDiff({
                path: compareState.path,
                leftRef: ref,
                rightKind: "working",
                baseLabel: ref,
              });
            }
            setCompareState(null);
          }}
        />
      ) : null}
      {agentHistoryState ? (
        <AgentHistoryModal
          path={agentHistoryState.path}
          turns={currentBatchTurns ?? []}
          fileChanges={currentBatchFileChanges ?? []}
          onClose={() => setAgentHistoryState(null)}
        />
      ) : null}
      {commitDialogOpen && stream ? (
        <CommitDialog
          streamId={stream.id}
          pathCount={uncommittedPaths.length}
          onClose={() => setCommitDialogOpen(false)}
          onComplete={(result) => {
            setCommitDialogOpen(false);
            setOpResult({ title: "git commit", result });
          }}
        />
      ) : null}
      {pushPullDialog && stream ? (
        <PushPullDialog
          kind={pushPullDialog}
          streamId={stream.id}
          onClose={() => setPushPullDialog(null)}
          onComplete={(result) => {
            setPushPullDialog(null);
            setOpResult({ title: pushPullDialog === "push" ? "git push" : "git pull", result });
          }}
        />
      ) : null}
      {opResult ? (
        <GitOpResultModal
          title={opResult.title}
          result={opResult.result}
          onClose={() => setOpResult(null)}
        />
      ) : null}
    </div>
  );
}

// Dev-noise directory names that a first-time user almost never wants to
// scroll past to get to their code. At the project root, these sort to
// the bottom (after src, app, lib, etc.) so the tree leads with source.
// Inside the list we still keep directories-before-files and alphabetical
// ordering within each bucket.
const DEV_NOISE_DIRS = new Set([
  ".claude", ".newde", ".git", ".github", ".vscode", ".idea",
  "bin", "dist", "build", "out", "node_modules", "public", "coverage",
  "target", "vendor", "tmp", ".tmp", ".cache",
]);

function sortFileTreeEntries(entries: WorkspaceEntry[]): WorkspaceEntry[] {
  const copy = entries.slice();
  copy.sort((a, b) => {
    const aDir = a.kind === "directory";
    const bDir = b.kind === "directory";
    if (aDir !== bDir) return aDir ? -1 : 1;
    const aNoise = aDir && DEV_NOISE_DIRS.has(a.name);
    const bNoise = bDir && DEV_NOISE_DIRS.has(b.name);
    if (aNoise !== bNoise) return aNoise ? 1 : -1;
    return a.name.localeCompare(b.name);
  });
  return copy;
}

function FindUsagesModal({
  state,
  onClose,
  onOpen,
}: {
  state: { query: string; path: string; results: TextSearchHit[] | null; loading: boolean };
  onClose(): void;
  onOpen(path: string): void;
}) {
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") { event.preventDefault(); onClose(); }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  // Group hits by file — one expanded list per file is closer to an IDE's
  // Find Usages output than a flat alternating list, and makes large result
  // sets a lot easier to scan.
  const grouped = new Map<string, TextSearchHit[]>();
  for (const hit of state.results ?? []) {
    const list = grouped.get(hit.path) ?? [];
    list.push(hit);
    grouped.set(hit.path, list);
  }

  return (
    <div
      onClick={onClose}
      style={{
        position: "fixed",
        inset: 0,
        background: "rgba(0,0,0,0.5)",
        zIndex: 2000,
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
      }}
    >
      <div
        onClick={(e) => e.stopPropagation()}
        style={{
          background: "var(--bg)",
          border: "1px solid var(--border)",
          borderRadius: 8,
          width: "min(720px, 92vw)",
          maxHeight: "80vh",
          display: "flex",
          flexDirection: "column",
          boxShadow: "0 8px 24px rgba(0,0,0,0.4)",
          overflow: "hidden",
        }}
      >
        <div style={{ padding: "10px 14px", borderBottom: "1px solid var(--border)", display: "flex", alignItems: "center", justifyContent: "space-between", gap: 8 }}>
          <div style={{ minWidth: 0, overflow: "hidden" }}>
            <div style={{ fontWeight: 600, fontSize: 13 }}>Find usages</div>
            <div style={{ color: "var(--muted)", fontSize: 11, whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>
              “{state.query}” &middot; from {state.path}
            </div>
          </div>
          <button type="button" onClick={onClose} style={{ border: "none", background: "transparent", color: "var(--muted)", cursor: "pointer", fontSize: 14 }}>✕</button>
        </div>
        <div style={{ flex: 1, minHeight: 0, overflow: "auto", padding: "6px 0" }}>
          {state.loading ? (
            <div style={{ padding: 14, color: "var(--muted)", fontSize: 12 }}>Searching…</div>
          ) : !state.results || state.results.length === 0 ? (
            <div style={{ padding: 14, color: "var(--muted)", fontSize: 12 }}>No matches.</div>
          ) : (
            [...grouped.entries()].map(([path, hits]) => (
              <div key={path} style={{ borderTop: "1px solid var(--border)" }}>
                <div style={{ padding: "4px 14px", background: "var(--bg-2)", fontSize: 11, color: "var(--muted)", display: "flex", justifyContent: "space-between" }}>
                  <span style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{path}</span>
                  <span>{hits.length}</span>
                </div>
                {hits.map((hit) => (
                  <button type="button"
                    key={`${hit.path}:${hit.line}`}
                    onClick={() => onOpen(hit.path)}
                    style={{
                      display: "flex",
                      width: "100%",
                      border: "none",
                      background: "transparent",
                      color: "inherit",
                      font: "inherit",
                      padding: "3px 14px",
                      cursor: "pointer",
                      textAlign: "left",
                      gap: 10,
                      fontSize: 12,
                      alignItems: "baseline",
                    }}
                    onMouseEnter={(e) => { e.currentTarget.style.background = "var(--bg-2)"; }}
                    onMouseLeave={(e) => { e.currentTarget.style.background = "transparent"; }}
                  >
                    <span style={{ color: "var(--muted)", fontFamily: "var(--mono, monospace)", fontSize: 11, minWidth: 36, textAlign: "right" }}>{hit.line}</span>
                    <span style={{ fontFamily: "var(--mono, monospace)", whiteSpace: "pre", overflow: "hidden", textOverflow: "ellipsis" }}>{hit.snippet}</span>
                  </button>
                ))}
              </div>
            ))
          )}
        </div>
      </div>
    </div>
  );
}

function FileHistoryModal({
  state,
  onClose,
  onOpenDiff,
}: {
  state: { path: string; commits: GitLogCommit[] | null; loading: boolean };
  onClose(): void;
  onOpenDiff(sha: string, parent: string | null): void;
}) {
  useEscape(onClose);
  return (
    <ModalShell onClose={onClose} title={`History · ${state.path}`}>
      {state.loading && !state.commits ? (
        <div style={modalEmptyStyle}>Loading…</div>
      ) : !state.commits || state.commits.length === 0 ? (
        <div style={modalEmptyStyle}>No commits touched this file.</div>
      ) : (
        state.commits.map((commit) => (
          <button type="button"
            key={commit.sha}
            onDoubleClick={() => onOpenDiff(commit.sha, commit.parents[0]?.sha ?? null)}
            title="Double-click to open diff"
            style={modalRowStyle}
          >
            <span style={{ fontFamily: "var(--mono, monospace)", color: "var(--muted)", fontSize: 11, minWidth: 56 }}>
              {commit.sha.slice(0, 7)}
            </span>
            <span style={{ flex: 1, minWidth: 0, overflow: "hidden", textOverflow: "ellipsis" }}>
              {commit.commit.message}
            </span>
            <span style={{ color: "var(--muted)", fontSize: 11 }}>{commit.commit.author.name}</span>
          </button>
        ))
      )}
    </ModalShell>
  );
}

function CompareWithModal({
  state,
  onClose,
  onPick,
}: {
  state: { path: string; refs: RefOption[] | null; loading: boolean };
  onClose(): void;
  onPick(ref: string): void;
}) {
  const [filter, setFilter] = useState("");
  useEscape(onClose);
  const filtered = (state.refs ?? []).filter((ref) => ref.name.toLowerCase().includes(filter.toLowerCase()));
  return (
    <ModalShell onClose={onClose} title={`Compare With… · ${state.path}`}>
      <input
        autoFocus
        value={filter}
        onChange={(e) => setFilter(e.target.value)}
        placeholder="Filter branches/tags…"
        style={{ margin: "6px 12px", ...modalInputStyle }}
      />
      {state.loading && !state.refs ? (
        <div style={modalEmptyStyle}>Loading refs…</div>
      ) : filtered.length === 0 ? (
        <div style={modalEmptyStyle}>No matches.</div>
      ) : (
        filtered.map((ref) => (
          <button type="button" key={`${ref.kind}-${ref.name}`} onClick={() => onPick(ref.ref)} style={modalRowStyle}>
            <span style={{ color: ref.kind === "tag" ? "#fcd34d" : "#4a9eff", fontSize: 10, textTransform: "uppercase", minWidth: 48 }}>
              {ref.kind}
            </span>
            <span style={{ flex: 1, minWidth: 0, overflow: "hidden", textOverflow: "ellipsis" }}>{ref.name}</span>
          </button>
        ))
      )}
    </ModalShell>
  );
}

function AgentHistoryModal({
  path,
  turns,
  fileChanges,
  onClose,
}: {
  path: string;
  turns: AgentTurn[];
  fileChanges: BatchFileChange[];
  onClose(): void;
}) {
  useEscape(onClose);
  const turnsById = new Map(turns.map((t) => [t.id, t]));
  // Keep only changes for this path, and collapse to (turn, latest-kind).
  const rows = fileChanges
    .filter((c) => c.path === path && c.turn_id)
    .map((c) => ({ turnId: c.turn_id!, kind: c.change_kind, at: c.created_at }))
    .sort((a, b) => (a.at < b.at ? 1 : -1));
  const seen = new Set<string>();
  const collapsed = rows.filter((r) => {
    if (seen.has(r.turnId)) return false;
    seen.add(r.turnId);
    return true;
  });
  return (
    <ModalShell onClose={onClose} title={`Agent history · ${path}`}>
      {collapsed.length === 0 ? (
        <div style={modalEmptyStyle}>No agent turns touched this path.</div>
      ) : (
        collapsed.map((row) => {
          const turn = turnsById.get(row.turnId);
          return (
            <div key={row.turnId} style={{ ...modalRowStyle, cursor: "default" }}>
              <span style={{ ...modalStatusPillStyle, color: statusColor(row.kind) }}>{statusLabel(row.kind)}</span>
              <span style={{ flex: 1, minWidth: 0, overflow: "hidden", textOverflow: "ellipsis" }}>
                {truncate(turn?.prompt ?? "(no prompt)", 90)}
              </span>
              <span style={{ color: "var(--muted)", fontSize: 11, whiteSpace: "nowrap" }}>
                {new Date(row.at).toLocaleString()}
              </span>
            </div>
          );
        })
      )}
    </ModalShell>
  );
}

function PushPullDialog({
  kind,
  streamId,
  onClose,
  onComplete,
}: {
  kind: "push" | "pull";
  streamId: string;
  onClose(): void;
  onComplete(result: GitOpResult): void;
}) {
  const [force, setForce] = useState(false);
  const [setUpstream, setSetUpstream] = useState(false);
  const [rebase, setRebase] = useState(false);
  const [running, setRunning] = useState(false);
  useEscape(onClose);

  const run = async () => {
    setRunning(true);
    const result = kind === "push"
      ? await gitPush(streamId, { force, setUpstream })
      : await gitPull(streamId, { rebase });
    setRunning(false);
    onComplete(result);
  };

  return (
    <ModalShell onClose={onClose} title={kind === "push" ? "Push" : "Pull"}>
      <form
        onSubmit={(e) => { e.preventDefault(); void run(); }}
        style={{ display: "flex", flexDirection: "column", gap: 8, padding: "10px 14px", fontSize: 12 }}
      >
        {kind === "push" ? (
          <>
            <label style={modalCheckboxStyle}>
              <input type="checkbox" checked={force} onChange={(e) => setForce(e.target.checked)} />
              Force push (with lease)
            </label>
            <label style={modalCheckboxStyle}>
              <input type="checkbox" checked={setUpstream} onChange={(e) => setSetUpstream(e.target.checked)} />
              Set upstream to current remote/branch
            </label>
          </>
        ) : (
          <label style={modalCheckboxStyle}>
            <input type="checkbox" checked={rebase} onChange={(e) => setRebase(e.target.checked)} />
            Rebase instead of merge
          </label>
        )}
        <div style={{ display: "flex", justifyContent: "flex-end", gap: 6, marginTop: 4 }}>
          <button type="button" onClick={onClose} style={modalBtnStyle}>Cancel</button>
          <button type="submit" disabled={running} style={{ ...modalBtnStyle, background: "var(--accent)", color: "#fff" }}>
            {running ? "Running…" : kind === "push" ? "Push" : "Pull"}
          </button>
        </div>
      </form>
    </ModalShell>
  );
}

function CommitDialog({
  streamId,
  pathCount,
  onClose,
  onComplete,
}: {
  streamId: string;
  pathCount: number;
  onClose(): void;
  onComplete(result: GitOpResult): void;
}) {
  const [message, setMessage] = useState("");
  const [running, setRunning] = useState(false);
  useEscape(onClose);

  const trimmed = message.trim();
  const canSubmit = trimmed.length > 0 && !running;

  const run = async () => {
    if (!canSubmit) return;
    setRunning(true);
    const result = await gitCommitAll(streamId, trimmed);
    setRunning(false);
    onComplete(result);
  };

  return (
    <ModalShell onClose={onClose} title={`Commit ${pathCount} change${pathCount === 1 ? "" : "s"}`}>
      <form
        onSubmit={(e) => { e.preventDefault(); void run(); }}
        style={{ display: "flex", flexDirection: "column", gap: 8, padding: "10px 14px", fontSize: 12, minWidth: 380 }}
      >
        <label htmlFor="files-commit-message" style={{ color: "var(--muted)", fontSize: 11, textTransform: "uppercase", letterSpacing: 0.6 }}>
          Commit message
        </label>
        <textarea
          id="files-commit-message"
          data-testid="files-commit-message"
          autoFocus
          value={message}
          onChange={(e) => setMessage(e.target.value)}
          placeholder="Summary line&#10;&#10;Optional body"
          style={{
            minHeight: 96,
            resize: "vertical",
            background: "var(--bg)",
            color: "var(--fg)",
            border: "1px solid var(--border)",
            borderRadius: 4,
            padding: "6px 8px",
            font: "inherit",
          }}
          onKeyDown={(event) => {
            if ((event.metaKey || event.ctrlKey) && event.key === "Enter" && canSubmit) {
              event.preventDefault();
              void run();
            }
          }}
        />
        <div style={{ color: "var(--muted)", fontSize: 11 }}>
          Runs <code>git add -A &amp;&amp; git commit -m …</code> in the stream's worktree.
          Cmd/Ctrl+Enter to commit.
        </div>
        <div style={{ display: "flex", justifyContent: "flex-end", gap: 6, marginTop: 4 }}>
          <button type="button" onClick={onClose} style={modalBtnStyle}>Cancel</button>
          <button
            type="submit"
            data-testid="files-commit-submit"
            disabled={!canSubmit}
            style={{ ...modalBtnStyle, background: "var(--accent)", color: "#fff", opacity: canSubmit ? 1 : 0.5 }}
          >
            {running ? "Committing…" : "Commit"}
          </button>
        </div>
      </form>
    </ModalShell>
  );
}

function GitOpResultModal({ title, result, onClose }: { title: string; result: GitOpResult; onClose(): void }) {
  useEscape(onClose);
  const colour = result.ok ? "#86efac" : "#f87171";
  return (
    <ModalShell onClose={onClose} title={title}>
      <div style={{ padding: "8px 14px 12px", display: "flex", flexDirection: "column", gap: 6 }}>
        <div style={{ color: colour, fontSize: 12, fontWeight: 600 }}>
          {result.ok ? "Success" : `Failed (exit ${result.exitCode ?? "?"})`}
        </div>
        {result.stdout ? (
          <pre style={modalPreStyle}>{result.stdout}</pre>
        ) : null}
        {result.stderr ? (
          <pre style={{ ...modalPreStyle, color: "#f87171" }}>{result.stderr}</pre>
        ) : null}
      </div>
    </ModalShell>
  );
}

function ModalShell({ title, onClose, children }: { title: string; onClose(): void; children: React.ReactNode }) {
  return (
    <div
      onClick={onClose}
      style={{
        position: "fixed",
        inset: 0,
        background: "rgba(0,0,0,0.5)",
        zIndex: 2000,
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
      }}
    >
      <div
        onClick={(e) => e.stopPropagation()}
        style={{
          background: "var(--bg)",
          border: "1px solid var(--border)",
          borderRadius: 8,
          width: "min(640px, 92vw)",
          maxHeight: "80vh",
          display: "flex",
          flexDirection: "column",
          overflow: "hidden",
          boxShadow: "0 8px 24px rgba(0,0,0,0.4)",
        }}
      >
        <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", padding: "10px 14px", borderBottom: "1px solid var(--border)" }}>
          <div style={{ fontWeight: 600, fontSize: 13, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{title}</div>
          <button type="button" onClick={onClose} style={{ border: "none", background: "transparent", color: "var(--muted)", cursor: "pointer", fontSize: 14 }}>✕</button>
        </div>
        <div style={{ flex: 1, minHeight: 0, overflow: "auto" }}>
          {children}
        </div>
      </div>
    </div>
  );
}

function useEscape(handler: () => void) {
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") { event.preventDefault(); handler(); }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [handler]);
}

function statusLabel(kind: string): string {
  switch (kind) {
    case "created": return "A";
    case "updated": return "M";
    case "deleted": return "D";
    default: return "·";
  }
}

function statusColor(kind: string): string {
  switch (kind) {
    case "created": return "#86efac";
    case "updated": return "#e5a06a";
    case "deleted": return "#f87171";
    default: return "var(--muted)";
  }
}

const modalEmptyStyle = { padding: 14, color: "var(--muted)", fontSize: 12 } as const;
const modalRowStyle = {
  display: "flex",
  width: "100%",
  border: "none",
  background: "transparent",
  color: "inherit",
  font: "inherit",
  padding: "6px 14px",
  cursor: "pointer",
  textAlign: "left" as const,
  gap: 10,
  fontSize: 12,
  alignItems: "center" as const,
  borderTop: "1px solid var(--border)",
};
const modalInputStyle = {
  borderRadius: 6,
  border: "1px solid var(--border)",
  background: "var(--bg-2)",
  color: "inherit",
  font: "inherit",
  padding: "4px 6px",
  fontSize: 12,
} as const;
const modalStatusPillStyle = {
  display: "inline-flex",
  alignItems: "center",
  justifyContent: "center",
  width: 16,
  fontFamily: "var(--mono, monospace)",
  fontSize: 10,
  fontWeight: 600,
  flexShrink: 0,
} as const;
const modalCheckboxStyle = { display: "inline-flex", alignItems: "center", gap: 6, cursor: "pointer" } as const;
const modalBtnStyle = {
  border: "1px solid var(--border)",
  background: "var(--bg-2)",
  color: "inherit",
  borderRadius: 6,
  padding: "4px 10px",
  cursor: "pointer",
  fontFamily: "inherit",
  fontSize: 12,
} as const;
const modalPreStyle = {
  whiteSpace: "pre-wrap" as const,
  wordBreak: "break-word" as const,
  fontSize: 11,
  fontFamily: "var(--mono, monospace)",
  background: "var(--bg-2)",
  padding: 8,
  borderRadius: 4,
  margin: 0,
  maxHeight: 320,
  overflow: "auto",
};

const iconButtonStyle = {
  border: "1px solid var(--border)",
  background: "var(--bg-2)",
  color: "inherit",
  borderRadius: 6,
  padding: "2px 6px",
  cursor: "pointer",
  fontFamily: "inherit",
  fontSize: 12,
  lineHeight: 1,
  flexShrink: 0,
} as const;

const commitButtonStyle = {
  ...iconButtonStyle,
  background: "var(--accent)",
  color: "#fff",
  padding: "2px 10px",
  fontWeight: 600,
} as const;

const filterSelectStyle = {
  borderRadius: 6,
  border: "1px solid var(--border)",
  background: "var(--bg)",
  color: "inherit",
  font: "inherit",
  padding: "3px 6px",
  fontSize: 11,
  maxWidth: 220,
} as const;

function truncate(input: string, max: number): string {
  const oneLine = input.replace(/\s+/g, " ").trim();
  if (oneLine.length <= max) return oneLine || "(empty prompt)";
  return oneLine.slice(0, max - 1) + "…";
}

type FilterMode = "all" | "uncommitted" | "branch" | "unpushed" | "turn";

function filterModeLabel(
  mode: FilterMode,
  scopes: { branchBase?: string | null; upstream?: string | null; onDefaultBranch?: boolean } | null,
  selectedTurnId: string | null,
  recentTurns: { id: string; prompt: string }[],
): string {
  if (mode === "all") return "all files";
  if (mode === "uncommitted") return "uncommitted changes";
  if (mode === "branch") return `branch changes${scopes?.branchBase ? ` (vs ${scopes.branchBase})` : ""}`;
  if (mode === "unpushed") return `unpushed changes${scopes?.upstream ? ` (vs ${scopes.upstream})` : ""}`;
  if (mode === "turn") {
    const turn = recentTurns.find((t) => t.id === selectedTurnId);
    return `files from turn${turn ? ` · ${truncate(turn.prompt, 40)}` : ""}`;
  }
  return mode;
}

function FilterMenuButton({
  filterMode,
  setFilterMode,
  gitEnabled,
  scopes,
  recentTurns,
}: {
  filterMode: FilterMode;
  setFilterMode: (mode: FilterMode) => void;
  gitEnabled: boolean;
  scopes: { branchBase?: string | null; upstream?: string | null; onDefaultBranch?: boolean } | null;
  recentTurns: { id: string; prompt: string }[];
}) {
  const [open, setOpen] = useState(false);
  const containerRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    function handler(event: MouseEvent) {
      if (!containerRef.current) return;
      if (containerRef.current.contains(event.target as Node)) return;
      setOpen(false);
    }
    window.addEventListener("mousedown", handler);
    return () => window.removeEventListener("mousedown", handler);
  }, [open]);

  const options: Array<{ value: FilterMode; label: string; disabled?: boolean }> = [
    { value: "all", label: "All files" },
  ];
  if (gitEnabled) options.push({ value: "uncommitted", label: "Uncommitted changes" });
  if (gitEnabled) options.push({
    value: "branch",
    label: `Branch changes${scopes?.branchBase && !scopes?.onDefaultBranch ? ` (vs ${scopes.branchBase})` : ""}`,
    disabled: !scopes?.branchBase || !!scopes?.onDefaultBranch,
  });
  if (gitEnabled) options.push({
    value: "unpushed",
    label: `Unpushed changes${scopes?.upstream ? ` (vs ${scopes.upstream})` : " (no upstream)"}`,
    disabled: !scopes?.upstream,
  });
  options.push({ value: "turn", label: `Turn${recentTurns.length === 0 ? " (no turns yet)" : ""}`, disabled: recentTurns.length === 0 });

  return (
    <div ref={containerRef} style={{ position: "relative" }}>
      <button type="button"
        onClick={() => setOpen((v) => !v)}
        title={`Filter: ${filterModeLabel(filterMode, scopes, null, recentTurns)}`}
        style={{
          ...iconButtonStyle,
          background: filterMode !== "all" ? "var(--accent)" : "var(--bg-2)",
          color: filterMode !== "all" ? "#fff" : "inherit",
        }}
      >
        {/* eye icon (SVG for crisp rendering) */}
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" style={{ display: "block" }}>
          <path d="M1 12s4-8 11-8 11 8 11 8-4 8-11 8-11-8-11-8z" />
          <circle cx="12" cy="12" r="3" />
        </svg>
      </button>
      {open ? (
        <div style={filterMenuStyle}>
          {options.map((opt) => (
            <button type="button"
              key={opt.value}
              disabled={opt.disabled}
              onClick={() => {
                if (opt.disabled) return;
                setFilterMode(opt.value);
                setOpen(false);
              }}
              style={{
                ...filterMenuItemStyle,
                background: opt.value === filterMode ? "rgba(74,158,255,0.18)" : "transparent",
                color: opt.disabled ? "var(--muted)" : "var(--fg)",
                cursor: opt.disabled ? "not-allowed" : "pointer",
              }}
            >
              {opt.label}
            </button>
          ))}
        </div>
      ) : null}
    </div>
  );
}

const filterMenuStyle: CSSProperties = {
  position: "absolute",
  top: "calc(100% + 4px)",
  right: 0,
  zIndex: 30,
  minWidth: 220,
  background: "var(--bg-2)",
  border: "1px solid var(--border)",
  borderRadius: 6,
  boxShadow: "0 6px 24px rgba(0,0,0,0.4)",
  display: "flex",
  flexDirection: "column",
  padding: 4,
};

const filterMenuItemStyle: CSSProperties = {
  textAlign: "left",
  border: "none",
  borderRadius: 4,
  padding: "6px 10px",
  fontFamily: "inherit",
  fontSize: 12,
};

const filterStatusBarStyle: CSSProperties = {
  borderTop: "1px solid var(--border)",
  padding: "4px 10px",
  background: "var(--bg-2)",
  color: "var(--muted)",
  fontSize: 11,
  display: "flex",
  alignItems: "center",
  gap: 8,
  flexShrink: 0,
};
