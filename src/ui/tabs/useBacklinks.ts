import { useEffect, useState } from "react";
import type { CommitDetail, GitLogCommit, Stream, ThreadWorkState } from "../api.js";
import { getBranchChanges, getCommitDetail, getGitLog, listCodeQualityFindings, listWikiNotes, readWikiNoteBody, listWorkItemEfforts } from "../api.js";
import type { BacklinkContext, BacklinkEntry, BacklinkFindingEntry, BacklinkNoteEntry, BacklinkWorkItemEntry } from "./backlinksIndex.js";
import { computeBacklinks } from "./backlinksIndex.js";
import { APP_PAGE_BACKLINKS } from "./appPageBacklinks.js";
import type { TabRef } from "./tabState.js";

/**
 * Hook that materializes a `BacklinkContext` from live data and computes
 * backlinks for the given target. Pages call this to render their
 * footer panel without reaching into App.tsx's state slices.
 *
 * The context is best-effort: each data source is fetched once on mount
 * and then cached for the lifetime of the page. Notes are loaded
 * lazily (titles + bodies); findings are pulled in bulk from the
 * `code_quality_finding` store; work items come from the already-loaded
 * thread work state passed in by the caller.
 */
export function useBacklinks(
  target: TabRef,
  stream: Stream | null,
  threadWork: ThreadWorkState | null,
): BacklinkEntry[] {
  const [ctx, setCtx] = useState<BacklinkContext>({ notes: [], workItems: [], findings: [] });
  const [recentLog, setRecentLog] = useState<GitLogCommit[] | undefined>(undefined);
  const [uncommittedPaths, setUncommittedPaths] = useState<string[] | undefined>(undefined);
  const [currentBranch, setCurrentBranch] = useState<string | undefined>(undefined);
  const [commitDetail, setCommitDetail] = useState<CommitDetail | undefined>(undefined);

  useEffect(() => {
    if (!stream) {
      setCtx({ notes: [], workItems: [], findings: [] });
      return;
    }
    let cancelled = false;

    const items: BacklinkWorkItemEntry[] = (threadWork?.items ?? []).map((wi) => ({
      id: wi.id,
      title: wi.title,
      description: wi.description,
      acceptance_criteria: wi.acceptance_criteria,
      touched_files: [],
    }));

    // Fetch efforts per work item so we know which files each one
    // touched. Bounded to 100 items in the worst case (Phase-4-acceptable).
    void Promise.all(
      items.map(async (item) => {
        try {
          const efforts = await listWorkItemEfforts(item.id);
          const touched = new Set<string>();
          for (const detail of efforts) {
            for (const path of detail.changed_paths) touched.add(path);
          }
          item.touched_files = [...touched];
        } catch {
          // ignore — missing touched_files just means weaker backlinks.
        }
      }),
    ).then(() => {
      if (!cancelled) setCtx((prev) => ({ ...prev, workItems: items }));
    });

    void listCodeQualityFindings({ streamId: stream.id }).then((rows) => {
      if (cancelled) return;
      const findings: BacklinkFindingEntry[] = rows.map((r) => ({
        id: String(r.id),
        path: r.path,
        startLine: r.startLine,
        endLine: r.endLine,
        kind: r.kind,
        metricValue: r.metricValue,
      }));
      setCtx((prev) => ({ ...prev, findings }));
    });

    void listWikiNotes(stream.id).then(async (summaries) => {
      const notes: BacklinkNoteEntry[] = [];
      for (const summary of summaries) {
        try {
          const body = await readWikiNoteBody(stream.id, summary.slug);
          notes.push({ slug: summary.slug, title: summary.title, body });
        } catch {
          // skip
        }
      }
      if (!cancelled) setCtx((prev) => ({ ...prev, notes }));
    });

    return () => {
      cancelled = true;
    };
  }, [stream?.id, threadWork?.threadId]);

  // Fetch git data slices when the target is an app-page kind that
  // needs them. Cheap to skip when not relevant.
  const isAppPage = target.kind in APP_PAGE_BACKLINKS;
  useEffect(() => {
    if (!isAppPage || !stream) return;
    let cancelled = false;
    void getGitLog(stream.id, { all: false, limit: 30 }).then((res) => {
      if (cancelled) return;
      setRecentLog(res.commits);
      setCurrentBranch(res.currentBranch ?? undefined);
    }).catch(() => { /* ignore */ });
    if (target.kind === "uncommitted-changes") {
      void getBranchChanges(stream.id, "HEAD").then((res) => {
        if (cancelled) return;
        setUncommittedPaths(res.files.map((c) => c.path));
      }).catch(() => { /* ignore */ });
    }
    return () => { cancelled = true; };
  }, [isAppPage, target.kind, stream?.id]);

  // Per-commit detail (touched files) for git-commit pages — bounded
  // single fetch per (stream, sha) pair.
  const commitSha = target.kind === "git-commit"
    ? (target.payload as { sha?: string } | null)?.sha ?? ""
    : "";
  useEffect(() => {
    if (!stream || !commitSha) {
      setCommitDetail(undefined);
      return;
    }
    let cancelled = false;
    void getCommitDetail(stream.id, commitSha).then((res) => {
      if (cancelled) return;
      setCommitDetail(res ?? undefined);
    }).catch(() => { /* ignore */ });
    return () => { cancelled = true; };
  }, [stream?.id, commitSha]);

  const provider = APP_PAGE_BACKLINKS[target.kind];
  if (provider) {
    return provider(target.payload, { ...ctx, recentLog, uncommittedPaths, currentBranch, commitDetail });
  }
  return computeBacklinks(target, ctx);
}
