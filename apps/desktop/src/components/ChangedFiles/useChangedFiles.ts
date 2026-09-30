import { useCallback, useEffect, useRef, useState } from "react";
import {
  diffRevisions,
  vcsHead,
  subscribeGitRefsEvents,
  subscribeSnapshotEvents,
  subscribeWorkspaceEvents,
  type DiffEntry,
} from "../../api.js";
import { WORKING, type Revision } from "../../revision.js";

/** What changed: the working tree against HEAD, or any two revisions
 *  (snapshots, commits, working tree). The commit page reads its files
 *  from the commit detail it already loads. */
export type ChangedFilesSource =
  | { kind: "working"; streamId: string }
  | { kind: "endpoints"; streamId: string; start: Revision | null; end: Revision };

export interface ChangedFiles {
  loading: boolean;
  error: string | null;
  files: DiffEntry[];
  /** The two sides, for opening a file's diff. `base` is null when the
   *  change has no older side (endpoints with no start). */
  base: Revision | null;
  head: Revision | null;
  refresh(): Promise<void>;
}

/** What a commit is compared against: its first parent, or `<sha>^`
 *  for a root commit. */
export function commitBase(sha: string, parents: string[]): string {
  return parents[0] ?? `${sha}^`;
}

async function load(
  source: ChangedFilesSource,
): Promise<{ files: DiffEntry[]; base: Revision | null; head: Revision }> {
  switch (source.kind) {
    case "working": {
      // The working tree against its head (from nothing before the first
      // commit).
      const head = (await vcsHead(source.streamId)).revision;
      const files = await diffRevisions(source.streamId, head, WORKING);
      return { files, base: head, head: WORKING };
    }
    case "endpoints": {
      return {
        files: await diffRevisions(source.streamId, source.start, source.end),
        base: source.start,
        head: source.end,
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
    const live = src.kind === "working" || src.end === WORKING;
    if (live) offs.push(subscribeWorkspaceEvents(src.streamId, () => void refresh()));
    if (src.kind === "endpoints") offs.push(subscribeSnapshotEvents(src.streamId, () => void refresh()));
    return () => offs.forEach((off) => off());
  }, [refresh]);

  return { ...state, refresh };
}
