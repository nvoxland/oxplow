import { useMemo } from "react";
import type { BranchChangeEntry, GitFileStatus } from "../../api.js";
import { classifyZone, ZONE_LABELS } from "./zones.js";
import {
  HierarchyView,
  type HierarchyMetrics,
  type HierarchyNode,
  type HierarchyStatus,
} from "../HierarchyView/HierarchyView.js";
import { useOptionalPageNavigation } from "../../tabs/PageNavigationContext.js";
import { changeAnalysisRef, fileRef, type ChangeAnalysisTarget } from "../../tabs/pageRefs.js";

interface Props {
  files: BranchChangeEntry[];
  /** Analysis target ("working" or commit sha) — needed so a
   *  directory click can drill into the same host page scoped to
   *  that directory. */
  target: ChangeAnalysisTarget;
  onOpenFile(path: string, opts?: { newTab?: boolean }): void;
  /** When supplied, plain click opens the file's diff in the current
   *  tab (browser-tab semantic, back returns to this list).
   *  Otherwise falls through to in-tab file open. */
  onOpenFileDiff?(path: string): void;
  /** Show the "N files" total in the toolbar's far-right slot. Default
   *  `true`; the diff page's Files Changed control hides it (the dir
   *  rows already carry per-folder counts). */
  showFileCount?: boolean;
}

/**
 * Tree-style file list for the Change Analysis drilldown. Builds a
 * directory > file hierarchy and renders it through `HierarchyView`
 * so the toolbar (filter + Expand all / Collapse all), the chevron
 * toggle, and the status badges all match the Semantic view exactly.
 */
export function ChangeAnalysisFileTree({ files, target, onOpenFile, onOpenFileDiff, showFileCount = true }: Props) {
  const ctxNav = useOptionalPageNavigation();
  // Default click semantic: navigate **in-tab** to the file's diff
  // when an `onOpenFileDiff` is supplied; otherwise fall through to
  // an in-tab file open via the page-context chokepoint. Cmd/Ctrl-
  // click escapes to a new file tab.
  const openFile = (path: string, opts: { newTab: boolean }) => {
    if (opts.newTab) {
      onOpenFile(path, { newTab: true });
      return;
    }
    if (onOpenFileDiff) {
      onOpenFileDiff(path);
      return;
    }
    if (ctxNav) ctxNav.navigate(fileRef(path), { newTab: false });
    else onOpenFile(path, { newTab: false });
  };
  // Directory drill: navigate to the host page scoped to that
  // directory. In-tab by default; cmd/ctrl-click opens a new tab.
  const openDir = (dirPath: string, opts: { newTab: boolean }) => {
    const ref = changeAnalysisRef(target, { kind: "dir", value: dirPath });
    if (ctxNav) ctxNav.navigate(ref, { newTab: opts.newTab });
  };
  const tree = useMemo(() => buildTree(files, openFile, openDir), [files, openFile, openDir]);
  const total = files.length;
  return (
    <HierarchyView
      nodes={tree}
      testIdPrefix="change-analysis-file"
      searchPlaceholder="Filter by path…"
      emptyLabel="No files match the filter."
      toolbarExtra={
        showFileCount ? (
          <span style={{ color: "var(--text-muted)", fontSize: 11 }}>
            {total} file{total === 1 ? "" : "s"}
          </span>
        ) : undefined
      }
    />
  );
}

interface RawDirNode {
  name: string;
  path: string;
  files: Array<{ name: string; entry: BranchChangeEntry }>;
  dirs: Map<string, RawDirNode>;
}

function buildTree(
  files: BranchChangeEntry[],
  openFile: (path: string, opts: { newTab: boolean }) => void,
  openDir: (path: string, opts: { newTab: boolean }) => void,
): HierarchyNode[] {
  const root: RawDirNode = { name: "", path: "", files: [], dirs: new Map() };
  for (const file of files) {
    const segments = file.path.split("/");
    const fileName = segments[segments.length - 1] ?? file.path;
    let cursor = root;
    for (let i = 0; i < segments.length - 1; i++) {
      const seg = segments[i]!;
      let next = cursor.dirs.get(seg);
      if (!next) {
        const dirPath = cursor.path ? `${cursor.path}/${seg}` : seg;
        next = { name: seg, path: dirPath, files: [], dirs: new Map() };
        cursor.dirs.set(seg, next);
      }
      cursor = next;
    }
    cursor.files.push({ name: fileName, entry: file });
  }
  return materialize(root, "", openFile, openDir);
}

function materialize(
  node: RawDirNode,
  idPrefix: string,
  openFile: (path: string, opts: { newTab: boolean }) => void,
  openDir: (path: string, opts: { newTab: boolean }) => void,
): HierarchyNode[] {
  const out: HierarchyNode[] = [];
  // Directories first, alphabetical.
  const dirsSorted = [...node.dirs.values()].sort((a, b) => a.name.localeCompare(b.name));
  for (const d of dirsSorted) {
    const id = `${idPrefix}/dir:${d.path}`;
    const children = materialize(d, id, openFile, openDir);
    const summary = summarize(d);
    out.push({
      id,
      label: `${d.name}/`,
      icon: <FolderIcon />,
      statuses: summary.statuses,
      count: summary.count,
      metrics: summary.metrics,
      children,
      onDrill: (e) => openDir(d.path, { newTab: e.metaKey || e.ctrlKey }),
      drillTitle: `Drill into ${d.path}/`,
    });
  }
  // Then files.
  const filesSorted = [...node.files].sort((a, b) => a.name.localeCompare(b.name));
  for (const f of filesSorted) {
    const id = `${idPrefix}/file:${f.entry.path}`;
    const bucket = statusBucket(f.entry.status);
    const zone = classifyZone(f.entry.path);
    // Zone badge rendered as muted detail text. `other` is the
    // catch-all so suppressing it keeps the column quiet on
    // unclassified files.
    const detail = zone === "other" ? undefined : `[${ZONE_LABELS[zone]}]`;
    out.push({
      id,
      label: f.name,
      icon: <FileIcon />,
      statuses: gitStatusToHierarchy(f.entry.status),
      detail,
      metrics: {
        added: bucket === "added" ? 1 : 0,
        modified: bucket === "modified" ? 1 : 0,
        deleted: bucket === "deleted" ? 1 : 0,
        additions: f.entry.additions ?? 0,
        deletions: f.entry.deletions ?? 0,
      },
      onDrill: (e) => {
        // Plain click → in-tab navigate (handled by openFile via the
        // PageNavigationContext chokepoint when present). Cmd/Ctrl-
        // click → new-tab escape. Lists never default-open new tabs.
        openFile(f.entry.path, { newTab: e.metaKey || e.ctrlKey });
      },
      drillTitle: `Open ${f.entry.path}`,
      children: [],
    });
  }
  return out;
}

function statusBucket(status: GitFileStatus): "added" | "modified" | "deleted" {
  if (status === "added" || status === "untracked") return "added";
  if (status === "deleted") return "deleted";
  return "modified";
}

interface DirSummary {
  count: number;
  statuses: Set<HierarchyStatus>;
  metrics: HierarchyMetrics;
}

function summarize(node: RawDirNode): DirSummary {
  const statuses = new Set<HierarchyStatus>();
  let count = 0;
  const metrics: HierarchyMetrics = {
    added: 0,
    modified: 0,
    deleted: 0,
    additions: 0,
    deletions: 0,
  };
  for (const file of node.files) {
    const fileStatuses = gitStatusToHierarchy(file.entry.status);
    for (const s of fileStatuses) statuses.add(s);
    count += 1;
    metrics[statusBucket(file.entry.status)] += 1;
    metrics.additions += file.entry.additions ?? 0;
    metrics.deletions += file.entry.deletions ?? 0;
  }
  for (const child of node.dirs.values()) {
    const inner = summarize(child);
    for (const s of inner.statuses) statuses.add(s);
    count += inner.count;
    metrics.added += inner.metrics.added;
    metrics.modified += inner.metrics.modified;
    metrics.deleted += inner.metrics.deleted;
    metrics.additions += inner.metrics.additions;
    metrics.deletions += inner.metrics.deletions;
  }
  return { count, statuses, metrics };
}

function gitStatusToHierarchy(status: GitFileStatus): Set<HierarchyStatus> {
  if (status === "added" || status === "untracked") return new Set(["added"]);
  if (status === "deleted") return new Set(["deleted"]);
  // modified, renamed → "modified"
  return new Set(["modified"]);
}

function FolderIcon() {
  return (
    <svg viewBox="0 0 16 16" width="1em" height="1em" aria-hidden style={{ display: "block" }}>
      <path
        d="M1.5 3.5 a1 1 0 0 1 1 -1 h3.5 l1.5 1.5 h6.5 a1 1 0 0 1 1 1 v8 a1 1 0 0 1 -1 1 h-11.5 a1 1 0 0 1 -1 -1 z"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.2"
      />
    </svg>
  );
}

function FileIcon() {
  return (
    <svg viewBox="0 0 16 16" width="1em" height="1em" aria-hidden style={{ display: "block" }}>
      <path
        d="M3.5 1.5 h6 l3 3 v10 a1 1 0 0 1 -1 1 h-8 a1 1 0 0 1 -1 -1 v-12 a1 1 0 0 1 1 -1 z M9.5 1.5 v3 h3"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.2"
        strokeLinejoin="round"
      />
    </svg>
  );
}
