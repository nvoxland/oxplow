import { useMemo, useState } from "react";
import type { ChangeAnalysisScope, ChangeAnalysisTarget } from "../../tabs/pageRefs.js";
import type { ChangeAnalysisState } from "./useChangeAnalysis.js";
import type { DiffSpec } from "../Diff/DiffPane.js";
import { DISK, refVersion, type FileVersion } from "../../file-version.js";
import { diffRef } from "../../diff-id.js";
import type { NavSiblingEntry, NavSiblings } from "../../tabs/PageNavigationContext.js";
import { DuplicationCard } from "./DuplicationCard.js";
import { LookHereFirstCard } from "./LookHereFirstCard.js";
import { CodeSmellsCard } from "./CodeSmellsCard.js";
import { CoChangeSurpriseCard } from "./CoChangeSurpriseCard.js";
import { ChangeTreemapCard } from "./ChangeTreemapCard.js";
// ZoneBarCard removed (tsk350): the Change Treemap subsumes it, with a
// zone-color legend below the map.
import {
  ALL_STATUSES,
  FilesPanel,
  statusPasses,
  type StatusKey,
} from "./FilesPanel.js";
import {
  buildFilePivots,
  type FunctionChurnRow,
  type FunctionsBuckets,
} from "./analysisHelpers.js";
import { usePageSnapshot } from "../../tabs/usePageSnapshot.js";

export interface ChangeAnalysisDrilldownProps {
  /** Optional drilldown filter; when absent, every panel still
   *  renders against the full file set. */
  scope?: ChangeAnalysisScope;
  target: ChangeAnalysisTarget;
  analysis: ChangeAnalysisState;
  /** When false, the inline Files/Functions panel is omitted — the host
   *  (the diff view) renders its own. Default true. */
  showFilesPanel?: boolean;
  /** When false, the Change Treemap is omitted (the diff view hoists it
   *  to the top). Default true. */
  showTreemap?: boolean;
  /** When false, the "Look here first" card is omitted (the diff view
   *  hoists it to the top). Default true. */
  showLookHere?: boolean;
  /** Render the Duplication card boxless (h2 header, no border) to match
   *  the diff view's other sections. Default false. */
  duplicationBoxless?: boolean;
  onOpenFile(path: string, opts?: { newTab?: boolean }): void;
  onOpenDiff?(spec: DiffSpec): void;
  onOpenDiffInTab?(spec: DiffSpec, siblings?: import("../../tabs/PageNavigationContext.js").NavSiblings): void;
}

export function ChangeAnalysisDrilldown({
  scope,
  target,
  analysis,
  showFilesPanel = true,
  showTreemap = true,
  showLookHere = true,
  duplicationBoxless = false,
  onOpenFile,
  onOpenDiff,
  onOpenDiffInTab,
}: ChangeAnalysisDrilldownProps) {
  // Initial status filter:
  //  - if the page was opened with a status scope, restrict to it.
  //  - otherwise default to "all three checked".
  const initialStatus: Set<StatusKey> =
    scope?.kind === "status" && (scope.value === "added" || scope.value === "modified" || scope.value === "deleted")
      ? new Set([scope.value as StatusKey])
      : new Set(ALL_STATUSES);
  const [statusFilter, setStatusFilter] = useState<Set<StatusKey>>(initialStatus);

  // Persist the filter selection across reloads.
  usePageSnapshot<{ statusFilter: StatusKey[] }>({
    serialize: () => ({ statusFilter: [...statusFilter] }),
    restore: (snap) => {
      if (Array.isArray(snap.statusFilter)) {
        const valid = snap.statusFilter.filter((k): k is StatusKey =>
          k === "added" || k === "modified" || k === "deleted",
        );
        setStatusFilter(new Set(valid.length > 0 ? valid : ALL_STATUSES));
      }
    },
    deps: [statusFilter],
  });

  const filesAfterStatus = useMemo(
    () => analysis.files.filter((f) => statusPasses(f.status, statusFilter)),
    [analysis.files, statusFilter],
  );

  const pivotsAfterStatus = useMemo(() => buildFilePivots(filesAfterStatus), [filesAfterStatus]);

  const filteredPathSet = useMemo(
    () => new Set(filesAfterStatus.map((f) => f.path)),
    [filesAfterStatus],
  );
  const functionsAfterStatus = useMemo<FunctionsBuckets>(() => ({
    added: analysis.functions.added.filter((fn) => filteredPathSet.has(fn.path)),
    deleted: analysis.functions.deleted.filter((fn) => filteredPathSet.has(fn.path)),
    modifiedSignature: analysis.functions.modifiedSignature.filter((fn) => filteredPathSet.has(fn.path)),
    modifiedBody: analysis.functions.modifiedBody.filter((fn) => filteredPathSet.has(fn.path)),
  }), [analysis.functions, filteredPathSet]);

  const dupAfterStatus = useMemo(() => ({
    ...analysis.duplication,
    findings: analysis.duplication.findings.filter((f) => filteredPathSet.has(f.path)),
  }), [analysis.duplication, filteredPathSet]);

  const churnAfterStatus = useMemo<FunctionChurnRow[]>(
    () => analysis.functionChurn.filter((c) => filteredPathSet.has(c.path)),
    [analysis.functionChurn, filteredPathSet],
  );

  /**
   * Per-status-filter sibling list. Each entry is the diff-tab the
   * Files panel would open when its row is clicked. Pre-computing
   * the full list here means the destination diff page can render
   * up/down arrows that step through every visible file without the
   * Files panel having to plumb sibling indices itself.
   */
  const fileDiffSiblings = useMemo<NavSiblingEntry[] | null>(() => {
    if (!analysis.refs) return null;
    const { baseRef, headRef } = analysis.refs;
    const rightVersion: FileVersion = headRef ? refVersion(headRef) : DISK;
    const baseLabel =
      target === "working" ? "working tree" : `parent of ${target.toString().slice(0, 7)}`;
    const ordered = [...filesAfterStatus].sort((a, b) => compareTreePathOrder(a.path, b.path));
    return ordered.map((f) => {
      const spec: DiffSpec = {
        path: f.path,
        leftVersion: refVersion(baseRef),
        rightVersion,
        baseLabel,
        revealLine: 1,
      };
      return {
        ref: diffRef(spec),
        label: f.path,
        diffSpec: spec,
      };
    });
  }, [analysis.refs, filesAfterStatus, target]);

  /**
   * Open the diff for `path` at `line` *in the current tab*. Falls
   * through to plain file-open if no diff handler is wired or refs
   * haven't resolved yet. When the path is in `fileDiffSiblings`,
   * threads the sibling list so the destination page can render
   * prev/next arrows that step through the rest of the visible files.
   */
  const openDiffAt = (path: string, line: number) => {
    if (!analysis.refs) {
      onOpenFile(path);
      return;
    }
    const { baseRef, headRef } = analysis.refs;
    const rightVersion: FileVersion = headRef ? refVersion(headRef) : DISK;
    const baseLabel =
      target === "working" ? "working tree" : `parent of ${target.toString().slice(0, 7)}`;
    const spec: DiffSpec = {
      path,
      leftVersion: refVersion(baseRef),
      rightVersion,
      baseLabel,
      revealLine: line,
    };
    let siblings: NavSiblings | undefined;
    if (fileDiffSiblings) {
      const idx = fileDiffSiblings.findIndex((e) => e.diffSpec?.path === path);
      if (idx >= 0) {
        siblings = {
          entries: fileDiffSiblings,
          index: idx,
          title:
            target === "working"
              ? "Uncommitted file changes"
              : `Files in ${target.toString().slice(0, 7)}`,
        };
      }
    }
    if (onOpenDiffInTab) onOpenDiffInTab(spec, siblings);
    else if (onOpenDiff) onOpenDiff(spec);
    else onOpenFile(path);
  };

  return (
    <>
      {showTreemap ? (
        <ChangeTreemapCard
          files={filesAfterStatus}
          functionChurn={churnAfterStatus}
          onOpenFile={onOpenFile}
          onOpenFileDiff={(path, line) => openDiffAt(path, line ?? 1)}
        />
      ) : null}
      <CoChangeSurpriseCard
        surprise={analysis.coChangeSurprise.filter((s) => filteredPathSet.has(s.path))}
        onOpenFile={onOpenFile}
      />
      {showFilesPanel ? (
        <FilesPanel
          files={filesAfterStatus}
          functions={functionsAfterStatus}
          functionChurn={churnAfterStatus}
          pivots={pivotsAfterStatus}
          statusFilter={statusFilter}
          onStatusChange={setStatusFilter}
          target={target}
          onOpenFile={onOpenFile}
          onOpenFunctionDiff={openDiffAt}
          onOpenFileDiff={(path) => openDiffAt(path, 1)}
        />
      ) : null}
      {showLookHere ? (
        <LookHereFirstCard
          files={filesAfterStatus}
          fileScores={analysis.fileScores}
          onOpenFile={onOpenFile}
          onOpenFileDiff={(path) => openDiffAt(path, 1)}
        />
      ) : null}
      <CodeSmellsCard
        functions={functionsAfterStatus}
        onOpenFile={onOpenFile}
        onOpenFileDiff={(path, line) => openDiffAt(path, line ?? 1)}
      />
      <DuplicationCard duplication={dupAfterStatus} onOpenFile={onOpenFile} boxless={duplicationBoxless} />
    </>
  );
}

/** Order two paths the way FileTreeView renders them: at every shared
 *  directory level, subdirectories (and their entire subtree) come
 *  before files at that level; siblings sort alphabetically. */
function compareTreePathOrder(a: string, b: string): number {
  const sa = a.split("/");
  const sb = b.split("/");
  let i = 0;
  while (i < sa.length && i < sb.length && sa[i] === sb[i]) i++;
  if (i === sa.length && i === sb.length) return 0;
  // One path is a strict prefix of the other — shouldn't happen for a
  // flat list of file paths, but order parent-first to be deterministic.
  if (i === sa.length) return -1;
  if (i === sb.length) return 1;
  // Diverging segment: a directory entry (path has more segments after
  // this one) outranks a file entry at the same level.
  const aIsDir = sa.length > i + 1;
  const bIsDir = sb.length > i + 1;
  if (aIsDir && !bIsDir) return -1;
  if (!aIsDir && bIsDir) return 1;
  return sa[i]!.localeCompare(sb[i]!);
}
