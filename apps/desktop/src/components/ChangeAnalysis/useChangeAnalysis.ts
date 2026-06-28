import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  diffEndpoints,
  getBranchChanges,
  getCommitDetail,
  listCodeQualityFindings,
  readFileAtRef,
  readWorkspaceFile,
  subscribeCodeQualityEvents,
  subscribeGitRefsEvents,
  subscribeSnapshotEvents,
  subscribeWorkspaceEvents,
} from "../../api.js";
import type { DiffEndpoint } from "../../tauri-bridge/generated/bindings.js";
import type {
  BranchChangeEntry,
  CodeQualityFindingRow,
  CodeQualityScanRow,
} from "../../api-types.js";
import type { FileSurprise, ImportDelta } from "../../tauri-bridge/index.js";
import { commands } from "../../tauri-bridge/index.js";
import { DISK, refVersion, type FileVersion } from "../../file-version.js";
import {
  attachChurn,
  buildFilePivots,
  diffFunctions,
  fileExtension,
  indexSides,
  summarizeTests,
  topDirectory,
  type FilePivots,
  type FunctionChurnRow,
  type FunctionsBuckets,
  type TestSummary,
} from "./analysisHelpers.js";
import {
  fileInterestingness,
  type InterestingnessResult,
} from "./interestingness.js";
import type { ChangeAnalysisScope } from "../../tabs/pageRefs.js";

export interface UseChangeAnalysisInput {
  streamId: string | null;
  /** "working" or a commit SHA. Ignored for routing when `endpoints`
   *  is set, but still used as a stable cache key — pass a stable
   *  string (e.g. the tab id). */
  target: string;
  /** Optional drilldown filter applied before pivots / functions /
   *  duplication / tests are computed. When omitted the hook returns
   *  unfiltered dashboard-mode data. */
  scope?: ChangeAnalysisScope;
  /** When set, routes the file list + status through `diff_endpoints`
   *  (the unified snapshot/commit diff substrate) instead of git refs
   *  or the single-snapshot source. The function / duplication /
   *  co-change passes are skipped: the endpoint substrate doesn't yet
   *  expose per-file content ids or line counts (tracked in tsk339), so
   *  the diff view renders the file list + status breakdown only. */
  endpoints?: { start: DiffEndpoint | null; end: DiffEndpoint };
}

/** Predicate that matches a file against a drilldown scope. Pure;
 *  exported for tests. */
export function fileMatchesScope(
  file: BranchChangeEntry,
  scope: ChangeAnalysisScope | undefined,
): boolean {
  if (!scope) return true;
  if (scope.kind === "ext") {
    const ext = fileExtension(file.path) || "(none)";
    return ext === scope.value;
  }
  if (scope.kind === "dir") {
    // Match the literal directory path or any descendant. The
    // dashboard's pivot rows pass the first segment (so this still
    // matches "apps" → "apps/desktop/..."); the semantic tree can
    // pass deeper paths like "apps/desktop/src/components".
    if (file.path === scope.value) return true;
    return file.path.startsWith(`${scope.value}/`);
  }
  if (scope.kind === "status") {
    return file.status === scope.value;
  }
  return true;
}

export interface ChangeAnalysisState {
  loading: boolean;
  error: string | null;
  files: BranchChangeEntry[];
  totals: { additions: number; deletions: number };
  pivots: FilePivots;
  functions: FunctionsBuckets;
  duplication: {
    findings: CodeQualityFindingRow[];
    scanAgeMs: number | null;
    scanning: boolean;
    /** Run a duplication scan against the analyzed tree version,
     *  restricted to the changed files. Triggered by the user — the
     *  card never auto-scans on open because libgit2 + tree-sitter
     *  on a large repo isn't fast enough to make that feel
     *  background. */
    refresh: () => Promise<void>;
    /** True iff a `done` scan exists for this exact (treeVersion,
     *  filter) combination. When false, the duplication card should
     *  render a "Scan at <commit>" CTA instead of an empty findings
     *  list — the data the card had previously been surfacing was
     *  stale findings from a *different* scan. */
    hasScan: boolean;
  };
  tests: TestSummary;
  /** Per-function churn rows (added/deleted/modified lines). One
   *  row per function with non-zero churn. */
  functionChurn: FunctionChurnRow[];
  /** Per-file import edge deltas (added / removed / cross-zone-added).
   *  Computed but not currently surfaced in the UI (was the ZoneBarCard
   *  cross-zone-edge callout, removed in tsk350). Empty for files with
   *  stable imports. */
  importDeltas: ImportDelta[];
  /** Per-file co-change surprise classification. Files with reason
   *  `Normal` are still included so the UI can render counts. */
  coChangeSurprise: FileSurprise[];
  /** Per-file interestingness score (path → score + reasons). */
  fileScores: Map<string, InterestingnessResult>;
  refresh: () => Promise<void>;
  /** The (base, head) refs used to compute this analysis. `headRef`
   *  is null in working-tree mode (caller should diff against the
   *  workspace). Null until the first refresh resolves. */
  refs: { baseRef: string; headRef: string | null } | null;
}

const EMPTY_FILES: BranchChangeEntry[] = [];
const EMPTY_BUCKETS: FunctionsBuckets = {
  added: [],
  deleted: [],
  modifiedSignature: [],
  modifiedBody: [],
};
const EMPTY_PIVOTS: FilePivots = {
  byExtension: [],
  byTopDir: [],
  byStatus: { modified: 0, added: 0, deleted: 0, renamed: 0, untracked: 0 },
};

/**
 * Snapshot-mode targets are encoded as `"snapshot:<snapshotId>"`.
 * Targets that match this prefix route through the snapshot-source
 * path instead of git refs.
 */
const SNAPSHOT_TARGET_PREFIX = "snapshot:";

export function parseSnapshotTarget(target: string): number | null {
  if (!target.startsWith(SNAPSHOT_TARGET_PREFIX)) return null;
  const n = Number(target.slice(SNAPSHOT_TARGET_PREFIX.length));
  return Number.isFinite(n) ? n : null;
}

/**
 * Resolve the (baseRef, headRef) pair for the requested target. For
 * the working-tree variant `headRef` is null — the hook reads workspace
 * file content directly. For a commit SHA, base is the parent SHA and
 * head is the commit itself. Snapshot targets short-circuit with
 * placeholder refs (the content reader keys off snapshot file ids
 * rather than refs).
 */
async function resolveRefs(
  streamId: string,
  target: string,
): Promise<{ baseRef: string; headRef: string | null } | { error: string }> {
  if (target === "working") {
    return { baseRef: "HEAD", headRef: null };
  }
  if (parseSnapshotTarget(target) !== null) {
    // Snapshot-mode refs are sentinels — the snapshot source reads
    // file content by FileSnapshot row id, not by ref name. The
    // strings just need to be non-empty so downstream checks don't
    // misinterpret them as "no base".
    return { baseRef: "snapshot-prev", headRef: target };
  }
  try {
    const detail = await getCommitDetail(streamId, target);
    if (!detail) return { error: `Commit ${target.slice(0, 7)} not found.` };
    // Use the first parent (or the commit itself for the initial commit).
    const parents = (detail as { parents?: string[] | null }).parents ?? [];
    const baseRef = parents.length > 0 ? parents[0]! : `${target}^`;
    return { baseRef, headRef: target };
  } catch (err) {
    return { error: err instanceof Error ? err.message : String(err) };
  }
}

/** Snapshot-mode entries cached during the current fetch so the
 *  function-analyzer pass can look up the prior/current FileSnapshot
 *  ids per path without a second IPC round-trip. */
let snapshotEntriesCache: { snapshotId: number; entries: Map<string, { currentFileId: number; priorFileId: number | null }> } | null = null;

async function fetchFiles(
  streamId: string,
  baseRef: string,
  target: string,
): Promise<BranchChangeEntry[]> {
  const snapshotId = parseSnapshotTarget(target);
  if (snapshotId !== null) {
    const entries = await commands.listSnapshotChangeEntries(snapshotId);
    if (entries.status === "error") return [];
    const cache = new Map<string, { currentFileId: number; priorFileId: number | null }>();
    const out: BranchChangeEntry[] = [];
    for (const e of entries.data) {
      cache.set(e.path, {
        currentFileId: e.current_file_id,
        priorFileId: e.prior_file_id ?? null,
      });
      out.push({
        path: e.path,
        // Snapshot status uses the same set of strings as
        // BranchChangeEntry — added/modified/deleted — so the
        // downstream pivots / SummaryCard work unchanged.
        status: e.status as BranchChangeEntry["status"],
        // Line counts aren't computed; the SummaryCard renders 0s.
        // Function-level churn comes from the analyzer pass on raw
        // content, so the dashboard still gets useful data.
        additions: null,
        deletions: null,
      });
    }
    snapshotEntriesCache = { snapshotId, entries: cache };
    return out;
  }
  // For both working and commit targets, getBranchChanges over baseRef
  // yields the right diff: HEAD vs working tree, or parent vs commit.
  const branchChanges = await getBranchChanges(streamId, baseRef);
  if (target === "working") return branchChanges.files;
  // Commit-mode: getBranchChanges compares baseRef..HEAD which isn't
  // what we want — instead read commit detail's file list.
  const detail = await getCommitDetail(streamId, target);
  if (!detail) return [];
  const files = ((detail as { files?: BranchChangeEntry[] | null }).files ?? []) as BranchChangeEntry[];
  return files;
}

export function useChangeAnalysis(input: UseChangeAnalysisInput): ChangeAnalysisState {
  const { streamId, target, scope, endpoints } = input;
  // Stable key for the endpoints object so refresh's deps don't churn
  // on every render (the caller may pass a fresh object literal).
  const endpointsKey = endpoints ? JSON.stringify(endpoints) : "";
  const endpointsRef = useRef(endpoints);
  endpointsRef.current = endpoints;
  const [files, setFiles] = useState<BranchChangeEntry[]>(EMPTY_FILES);
  const [functions, setFunctions] = useState<FunctionsBuckets>(EMPTY_BUCKETS);
  const [functionChurn, setFunctionChurn] = useState<FunctionChurnRow[]>([]);
  const [importDeltas, setImportDeltas] = useState<ImportDelta[]>([]);
  const [coChangeSurprise, setCoChangeSurprise] = useState<FileSurprise[]>([]);
  const [duplication, setDuplication] = useState<{
    findings: CodeQualityFindingRow[];
    scanAgeMs: number | null;
    hasScan: boolean;
  }>({ findings: [], scanAgeMs: null, hasScan: false });
  const [scanning, setScanning] = useState(false);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [resolvedRefs, setResolvedRefs] = useState<{ baseRef: string; headRef: string | null } | null>(null);
  const reqIdRef = useRef(0);

  // Derive the tree version + file filter the duplication scan is
  // expected to run against. Working-tree → DISK; commit sha → that
  // ref. The filter is "all changed files in this analysis" — both
  // the lookup and the trigger use the same value so they line up.
  const treeVersion: FileVersion = useMemo(
    () => (target === "working" ? DISK : refVersion(target)),
    [target],
  );

  const refresh = useCallback(async () => {
    if (!streamId) {
      setFiles(EMPTY_FILES);
      setFunctions(EMPTY_BUCKETS);
      setImportDeltas([]);
      setCoChangeSurprise([]);
      setDuplication({ findings: [], scanAgeMs: null, hasScan: false });
      setLoading(false);
      return;
    }
    const reqId = ++reqIdRef.current;
    setLoading(true);
    setError(null);

    // Shared function-level analysis: hand it base+head content per
    // file and it runs analyze_functions_at_refs → function buckets,
    // churn, import deltas. Both the endpoint-diff branch and the
    // git/snapshot branch build `specs` their own way, then hand off
    // here so the two paths produce identical drilldown data.
    const runFunctionAnalysis = async (
      specs: Array<{ path: string; base_content: string | null; head_content: string | null }>,
    ): Promise<void> => {
      const analysis = await commands.analyzeFunctionsAtRefs(specs);
      if (reqId !== reqIdRef.current) return;
      if (analysis.status === "error") {
        setError(typeof analysis.error === "string" ? analysis.error : "Function analysis failed.");
        setFunctions(EMPTY_BUCKETS);
        return;
      }
      const result = analysis.data;
      const sides = result.sides.map((s) => ({
        path: s.path,
        side: s.side,
        functions: s.functions.map((fn) => ({
          name: fn.name,
          paramCount: fn.parameter_count,
          complexity: fn.complexity,
          length: fn.length,
          startLine: fn.start_line,
          containerPath: fn.container_path,
          visibility: (fn.visibility === "public" || fn.visibility === "private")
            ? (fn.visibility as "public" | "private")
            : ("unknown" as const),
        })),
      }));
      const buckets = diffFunctions(indexSides(sides));
      const churnRows: FunctionChurnRow[] = (result.churn ?? []).flatMap((file) =>
        file.functions.map((fn) => ({
          path: file.path,
          name: fn.name,
          containerPath: fn.container_path,
          startLineHead: fn.start_line_head,
          addedLines: fn.added_lines,
          deletedLines: fn.deleted_lines,
          modifiedLines: fn.modified_lines,
        })),
      );
      attachChurn(buckets, churnRows);
      setFunctions(buckets);
      setFunctionChurn(churnRows);
      setImportDeltas(result.import_deltas ?? []);
    };

    // Endpoint-diff mode: file list + status from the unified diff
    // substrate; base/head content per file via read_endpoint_files_content
    // feeds the same function analysis the git/snapshot branches run.
    // Duplication + co-change stay off — snapshot/commit endpoints have
    // no git-ref tree to scan and aren't commit history (matches the
    // snapshot-target path's behavior).
    const eps = endpointsRef.current;
    if (eps) {
      try {
        const entries = await diffEndpoints(eps.start, eps.end);
        if (reqId !== reqIdRef.current) return;
        const fileList: BranchChangeEntry[] = entries.map((e) => ({
          path: e.path,
          status: e.status as BranchChangeEntry["status"],
          additions: e.additions,
          deletions: e.deletions,
        }));
        setFiles(fileList);

        const analyzable = fileList.slice(0, 200);
        const paths = analyzable.map((e) => e.path);
        const baseRes = eps.start
          ? await commands.readEndpointFilesContent(eps.start, paths)
          : null;
        const headRes = await commands.readEndpointFilesContent(eps.end, paths);
        if (reqId !== reqIdRef.current) return;
        const baseContents =
          baseRes && baseRes.status === "ok" ? baseRes.data : paths.map(() => null);
        const headContents = headRes.status === "ok" ? headRes.data : paths.map(() => null);
        const specs = analyzable.map((e, i) => ({
          path: e.path,
          base_content: e.status === "added" ? null : (baseContents[i] ?? null),
          head_content: e.status === "deleted" ? null : (headContents[i] ?? null),
        }));
        await runFunctionAnalysis(specs);
        if (reqId !== reqIdRef.current) return;

        setCoChangeSurprise([]);
        setDuplication({ findings: [], scanAgeMs: null, hasScan: false });
        // Sentinel refs: snapshot/commit endpoints have no single git-ref
        // pair, but a non-null `refs` lets the drilldown render its
        // file-diff affordances (same posture as the snapshot target).
        setResolvedRefs({ baseRef: "endpoint-base", headRef: "endpoint-head" });
      } catch (err) {
        if (reqId !== reqIdRef.current) return;
        setError(err instanceof Error ? err.message : String(err));
        setFiles(EMPTY_FILES);
        setFunctions(EMPTY_BUCKETS);
      } finally {
        if (reqId === reqIdRef.current) setLoading(false);
      }
      return;
    }

    try {
      const refs = await resolveRefs(streamId, target);
      if ("error" in refs) {
        if (reqId !== reqIdRef.current) return;
        setError(refs.error);
        setFiles(EMPTY_FILES);
        setFunctions(EMPTY_BUCKETS);
        setResolvedRefs(null);
        setLoading(false);
        return;
      }
      if (reqId === reqIdRef.current) {
        setResolvedRefs({ baseRef: refs.baseRef, headRef: refs.headRef });
      }
      const fileList = await fetchFiles(streamId, refs.baseRef, target);
      if (reqId !== reqIdRef.current) return;
      setFiles(fileList);

      // Co-change surprise classification. Independent of the
      // function/import analysis; runs in parallel via spawn_blocking
      // on the Rust side. Snapshot-mode skips it — co-change is a
      // git-history concept and snapshots aren't commits.
      if (parseSnapshotTarget(target) === null && fileList.length > 0) {
        const paths = fileList.map((f) => f.path);
        const surprise = await commands.analyzeCoChangeSurprise(paths);
        if (reqId !== reqIdRef.current) return;
        if (surprise.status === "ok") {
          setCoChangeSurprise(surprise.data);
        } else {
          // Non-fatal — co-change is a nice-to-have signal.
          setCoChangeSurprise([]);
        }
      } else {
        setCoChangeSurprise([]);
      }

      // Function diff: read base + head content for each non-binary,
      // non-deleted, non-added-only file. (Added files have no base
      // content; deleted files have no head content; both still flow
      // through analyze_functions_at_refs so add/delete buckets work.)
      const limit = 200; // hard cap to keep large changesets tractable
      const analyzable = fileList.slice(0, limit);
      const snapshotId = parseSnapshotTarget(target);
      const specs = await Promise.all(
        analyzable.map(async (entry) => {
          if (snapshotId !== null) {
            const ids = snapshotEntriesCache?.entries.get(entry.path);
            const baseContent =
              entry.status === "added" || ids?.priorFileId == null
                ? null
                : await safeReadSnapshotContent(ids.priorFileId);
            const headContent =
              entry.status === "deleted" || ids?.currentFileId == null
                ? null
                : await safeReadSnapshotContent(ids.currentFileId);
            return {
              path: entry.path,
              base_content: baseContent,
              head_content: headContent,
            };
          }
          const baseContent = entry.status === "added" || entry.status === "untracked"
            ? null
            : await safeReadAtRef(streamId, refs.baseRef, entry.path);
          const headContent = await readHead(streamId, refs.headRef, entry);
          return {
            path: entry.path,
            base_content: baseContent,
            head_content: headContent,
          };
        }),
      );
      await runFunctionAnalysis(specs);
      if (reqId !== reqIdRef.current) return;

      // Duplication: snapshot-mode skips this entirely. The scan
      // store is keyed on git refs / DISK; snapshot pairs have no
      // matching TreeVersion, so the lookup would always miss and
      // the trigger button has no useful tree to scan against.
      if (snapshotId !== null) {
        if (reqId === reqIdRef.current) {
          setDuplication({ findings: [], scanAgeMs: null, hasScan: false });
        }
        return;
      }

      // Duplication: look up the latest `done` scan for THIS exact
      // (treeVersion, filter) combination. The legacy "show whatever
      // duplication scan was run last in this stream" path was the
      // root cause of the wrong-tree bug — a scan that ran against
      // the working tree would surface in a commit-target analysis
      // page with line ranges that didn't match the commit's
      // content. With the version+filter columns the lookup now
      // refuses to substitute.
      const filterPaths = fileList.map((f) => f.path);
      const filterSpec = filterPaths.length > 0
        ? { kind: "explicit" as const, paths: filterPaths }
        : { kind: "all" as const };
      const scanResult = await commands.findLatestCodeQualityScan(
        "duplication",
        treeVersion,
        filterSpec,
      );
      if (scanResult.status === "error") {
        if (reqId === reqIdRef.current) {
          setDuplication({ findings: [], scanAgeMs: null, hasScan: false });
        }
      } else if (scanResult.data) {
        const scan = scanResult.data as unknown as CodeQualityScanRow;
        const findings = await listCodeQualityFindings({
          streamId,
          scanId: scan.id,
        });
        const changed = new Set(fileList.map((f) => f.path));
        const filtered = findings.filter((f) => changed.has(f.path));
        const scanAgeMs = scanAgeFor(scan);
        if (reqId === reqIdRef.current) {
          setDuplication({ findings: filtered, scanAgeMs, hasScan: true });
        }
      } else if (reqId === reqIdRef.current) {
        setDuplication({ findings: [], scanAgeMs: null, hasScan: false });
      }
    } catch (err) {
      if (reqId !== reqIdRef.current) return;
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      if (reqId === reqIdRef.current) setLoading(false);
    }
  }, [streamId, target, treeVersion, endpointsKey]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  // Live refresh when the working tree or refs change (working-tree
  // variant only — a commit SHA is immutable, and a snapshot id
  // names an already-frozen capture).
  useEffect(() => {
    if (!streamId || target !== "working") return;
    const a = subscribeGitRefsEvents(streamId, () => void refresh());
    const b = subscribeWorkspaceEvents(streamId, () => void refresh());
    return () => {
      a();
      b();
    };
  }, [streamId, target, refresh]);

  // Snapshot-mode refresh: a new request_snapshot() may have written
  // a row that supersedes this one's prior-pointer, so re-run when
  // a fresh batch lands. Cheap because the rebuilt list is small.
  useEffect(() => {
    if (!streamId) return;
    if (parseSnapshotTarget(target) === null) return;
    return subscribeSnapshotEvents(streamId, () => void refresh());
  }, [streamId, target, refresh]);

  // Re-pull duplication when a code-quality scan completes.
  useEffect(() => {
    if (!streamId) return;
    return subscribeCodeQualityEvents(streamId, (event) => {
      if (event.tool !== "duplication" || event.status !== "done") return;
      void refresh();
    });
  }, [streamId, refresh]);

  const triggerScan = useCallback(async () => {
    if (!streamId) return;
    setScanning(true);
    try {
      // Scan against the analyzed tree version, restricted to the
      // currently-known changed files. `files` is derived state — by
      // the time the user clicks the button, refresh() has populated
      // it from the commit detail (or workspace).
      const filterPaths = files.map((f) => f.path);
      const filterSpec = filterPaths.length > 0
        ? { kind: "explicit" as const, paths: filterPaths }
        : { kind: "all" as const };
      const result = await commands.runDuplicationScanAt(
        treeVersion,
        filterSpec,
        "diff",
      );
      if (result.status === "error") {
        setError(typeof result.error === "string" ? result.error : "Scan failed.");
      }
      // The CodeQualityScanned event will fire `refresh()` which
      // re-pulls the new findings.
    } finally {
      setScanning(false);
    }
  }, [streamId, files, treeVersion]);

  // Apply the drilldown scope (if any) once and feed the filtered file
  // list into every downstream derivation. Pivots remain unfiltered so
  // dashboards can show the full breakdown even when nested inside a
  // drilldown caller — but the drilldown page isn't supposed to render
  // pivots anyway, and the dashboard never passes a scope.
  const filteredFiles = useMemo(
    () => (scope ? files.filter((f) => fileMatchesScope(f, scope)) : files),
    [files, scope],
  );
  const filteredPathSet = useMemo(() => new Set(filteredFiles.map((f) => f.path)), [filteredFiles]);
  const filteredFunctions = useMemo<FunctionsBuckets>(() => {
    if (!scope) return functions;
    return {
      added: functions.added.filter((fn) => filteredPathSet.has(fn.path)),
      deleted: functions.deleted.filter((fn) => filteredPathSet.has(fn.path)),
      modifiedSignature: functions.modifiedSignature.filter((fn) => filteredPathSet.has(fn.path)),
      modifiedBody: functions.modifiedBody.filter((fn) => filteredPathSet.has(fn.path)),
    };
  }, [functions, filteredPathSet, scope]);
  const filteredDupFindings = useMemo(
    () => (scope ? duplication.findings.filter((f) => filteredPathSet.has(f.path)) : duplication.findings),
    [duplication.findings, filteredPathSet, scope],
  );

  const totals = useMemo(() => {
    let add = 0;
    let del = 0;
    for (const f of filteredFiles) {
      add += f.additions ?? 0;
      del += f.deletions ?? 0;
    }
    return { additions: add, deletions: del };
  }, [filteredFiles]);

  const pivots = useMemo(() => buildFilePivots(filteredFiles), [filteredFiles]);
  const tests = useMemo(() => summarizeTests(filteredFiles), [filteredFiles]);
  const filteredChurn = useMemo<FunctionChurnRow[]>(
    () => (scope ? functionChurn.filter((c) => filteredPathSet.has(c.path)) : functionChurn),
    [functionChurn, filteredPathSet, scope],
  );

  // Per-file interestingness. Caller filters/scopes the input file
  // list, so the score map is consistent with whatever the
  // dashboard or drilldown will render. The "matching test" check
  // uses ALL changed test files (unscoped) — a test file in a
  // sibling directory still de-risks the file regardless of the
  // current drilldown.
  const fileScores = useMemo<Map<string, InterestingnessResult>>(() => {
    // No test-presence factor. The only honest test signal we
    // could derive at the file level (without per-file pair
    // matching or coverage data) is global, which doesn't
    // differentiate one file from another. The score now
    // reflects size + complexity / param spikes + new-function
    // length only.
    const result = new Map<string, InterestingnessResult>();
    const fnsByPath = new Map<string, {
      added: FunctionsBuckets["added"];
      deleted: FunctionsBuckets["deleted"];
      modifiedSignature: FunctionsBuckets["modifiedSignature"];
      modifiedBody: FunctionsBuckets["modifiedBody"];
    }>();
    const ensure = (path: string) => {
      let bucket = fnsByPath.get(path);
      if (!bucket) {
        bucket = { added: [], deleted: [], modifiedSignature: [], modifiedBody: [] };
        fnsByPath.set(path, bucket);
      }
      return bucket;
    };
    for (const fn of functions.added) ensure(fn.path).added.push(fn);
    for (const fn of functions.deleted) ensure(fn.path).deleted.push(fn);
    for (const fn of functions.modifiedSignature) ensure(fn.path).modifiedSignature.push(fn);
    for (const fn of functions.modifiedBody) ensure(fn.path).modifiedBody.push(fn);
    for (const file of filteredFiles) {
      const bucketed = fnsByPath.get(file.path) ?? {
        added: [],
        deleted: [],
        modifiedSignature: [],
        modifiedBody: [],
      };
      result.set(
        file.path,
        fileInterestingness({ file, bucketed }),
      );
    }
    return result;
  }, [filteredFiles, files, functions]);

  return {
    loading,
    error,
    files: filteredFiles,
    totals,
    pivots,
    functions: filteredFunctions,
    duplication: {
      findings: filteredDupFindings,
      scanAgeMs: duplication.scanAgeMs,
      scanning,
      refresh: triggerScan,
      hasScan: duplication.hasScan,
    },
    tests,
    functionChurn: filteredChurn,
    importDeltas: scope
      ? importDeltas.filter((d) => filteredPathSet.has(d.path))
      : importDeltas,
    coChangeSurprise: scope
      ? coChangeSurprise.filter((s) => filteredPathSet.has(s.path))
      : coChangeSurprise,
    fileScores,
    refresh,
    refs: resolvedRefs,
  };
}

async function safeReadAtRef(
  streamId: string,
  ref: string,
  path: string,
): Promise<string | null> {
  try {
    const result = await readFileAtRef(streamId, ref, path);
    return result.content ?? null;
  } catch {
    return null;
  }
}

async function safeReadSnapshotContent(fileSnapshotId: number): Promise<string | null> {
  try {
    const result = await commands.readSnapshotFileContent(fileSnapshotId);
    if (result.status === "error") return null;
    return result.data ?? null;
  } catch {
    return null;
  }
}

async function readHead(
  streamId: string,
  headRef: string | null,
  entry: BranchChangeEntry,
): Promise<string | null> {
  if (entry.status === "deleted") return null;
  if (headRef === null) {
    // Working-tree variant: read the on-disk file.
    try {
      const file = await readWorkspaceFile(streamId, entry.path);
      return file.content ?? null;
    } catch {
      return null;
    }
  }
  return safeReadAtRef(streamId, headRef, entry.path);
}

function scanAgeFor(scan: CodeQualityScanRow): number | null {
  const finishedRaw = (scan as { finished_at?: string | null }).finished_at;
  const startedRaw = scan.started_at;
  const stamp = finishedRaw ?? startedRaw;
  if (!stamp) return null;
  const ms = Date.parse(stamp);
  if (Number.isNaN(ms)) return null;
  return Date.now() - ms;
}
