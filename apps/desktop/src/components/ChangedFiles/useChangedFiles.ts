import { useCallback, useEffect, useRef, useState } from "react";
import {
  diffEndpoints,
  getBranchChanges,
  subscribeGitRefsEvents,
  subscribeSnapshotEvents,
  subscribeWorkspaceEvents,
  type BranchChangeEntry,
} from "../../api.js";
import type { DiffEndpoint } from "../../tauri-bridge/generated/bindings.js";
import { DISK, refVersion, snapshotVersion, type FileVersion } from "../../file-version.js";

/** What changed: the working tree against HEAD, or any two diff
 *  endpoints (snapshots, commits, working tree). The commit page reads
 *  its files from the commit detail it already loads. */
export type ChangedFilesSource =
  | { kind: "working"; streamId: string }
  | { kind: "endpoints"; streamId: string; start: DiffEndpoint | null; end: DiffEndpoint };

export interface ChangedFiles {
  loading: boolean;
  error: string | null;
  files: BranchChangeEntry[];
  /** The two sides, for opening a file's diff. `base` is null when the
   *  change has no older side (endpoints with no start). */
  base: FileVersion | null;
  head: FileVersion | null;
  refresh(): Promise<void>;
}

/** The file version the diff pane reads for a diff endpoint. */
export function endpointVersion(ep: DiffEndpoint): FileVersion {
  switch (ep.kind) {
    case "snapshot":
      return snapshotVersion(String(ep.snapshot_id));
    case "commit":
      return refVersion(ep.sha);
    case "working":
      return DISK;
  }
}

/** What a commit is compared against: its first parent, or `<sha>^`
 *  for a root commit. */
export function commitBase(sha: string, parents: string[]): string {
  return parents[0] ?? `${sha}^`;
}

async function load(
  source: ChangedFilesSource,
): Promise<{ files: BranchChangeEntry[]; base: FileVersion | null; head: FileVersion }> {
  switch (source.kind) {
    case "working": {
      const changes = await getBranchChanges(source.streamId, "HEAD");
      return { files: changes.files, base: refVersion("HEAD"), head: DISK };
    }
    case "endpoints": {
      const entries = await diffEndpoints(source.start, source.end);
      return {
        files: entries.map((e) => ({
          path: e.path,
          status: e.status as BranchChangeEntry["status"],
          additions: e.additions,
          deletions: e.deletions,
        })),
        base: source.start ? endpointVersion(source.start) : null,
        head: endpointVersion(source.end),
      };
    }
  }
}

/**
 * The files a change touched, with their status and line counts — the
 * core file list on the uncommitted and diff pages. Analysis of
 * the change (review priority, functions, co-change, duplication) is
 * the change-analysis producer's job, read through `v_change*` by
 * extension lenses. Live for the working tree and snapshot endpoints.
 */
export function useChangedFiles(source: ChangedFilesSource | null): ChangedFiles {
  const key = source ? JSON.stringify(source) : "";
  const sourceRef = useRef(source);
  sourceRef.current = source;
  const [state, setState] = useState<Omit<ChangedFiles, "refresh">>({
    loading: true,
    error: null,
    files: [],
    base: null,
    head: null,
  });
  const reqId = useRef(0);

  const refresh = useCallback(async () => {
    const src = sourceRef.current;
    const id = ++reqId.current;
    if (!src) {
      setState({ loading: false, error: null, files: [], base: null, head: null });
      return;
    }
    setState((s) => ({ ...s, loading: true }));
    try {
      const next = await load(src);
      if (id === reqId.current) setState({ loading: false, error: null, ...next });
    } catch (e) {
      if (id === reqId.current) {
        setState({ loading: false, error: e instanceof Error ? e.message : String(e), files: [], base: null, head: null });
      }
    }
    // `key` stands in for `source` (a fresh object each render).
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key]);

  useEffect(() => {
    void refresh();
    const src = sourceRef.current;
    if (!src) return;
    const offs = [subscribeGitRefsEvents(src.streamId, () => void refresh())];
    const live = src.kind === "working" || src.end.kind === "working";
    if (live) offs.push(subscribeWorkspaceEvents(src.streamId, () => void refresh()));
    if (src.kind === "endpoints") offs.push(subscribeSnapshotEvents(src.streamId, () => void refresh()));
    return () => offs.forEach((off) => off());
  }, [refresh]);

  return { ...state, refresh };
}
