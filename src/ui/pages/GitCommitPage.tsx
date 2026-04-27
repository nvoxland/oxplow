import { useEffect, useState } from "react";
import type { CommitDetail, Stream, ThreadWorkState } from "../api.js";
import { getCommitDetail } from "../api.js";
import { logUi } from "../logger.js";
import type { DiffRequest } from "../components/Diff/diff-request.js";
import { CommitDetailBody, buildCommitSlideoverTitle } from "../components/History/CommitDetailSlideover.js";
import { Page } from "../tabs/Page.js";
import { useBacklinks } from "../tabs/useBacklinks.js";
import { gitCommitRef } from "../tabs/pageRefs.js";
import { BacklinksList } from "../tabs/BacklinksList.js";
import type { TabRef } from "../tabs/tabState.js";

export interface GitCommitPageProps {
  stream: Stream | null;
  sha: string;
  /** Subject the caller already knows (for instant header rendering). */
  subject?: string;
  threadWork: ThreadWorkState | null;
  onOpenDiff?(request: DiffRequest): void;
  onOpenPage(ref: TabRef): void;
}

/**
 * Single-commit page. Bookmark- and history-friendly equivalent of the
 * legacy `CommitDetailSlideover`. Routed via `gitCommitRef(sha)`.
 */
export function GitCommitPage({ stream, sha, subject = "", threadWork, onOpenDiff, onOpenPage }: GitCommitPageProps) {
  const [detail, setDetail] = useState<CommitDetail | null>(null);
  const [loading, setLoading] = useState(false);
  const backlinkEntries = useBacklinks(gitCommitRef(sha), stream, threadWork);
  const backlinks = {
    count: backlinkEntries.length,
    body: <BacklinksList entries={backlinkEntries} onOpenPage={onOpenPage} />,
  };

  useEffect(() => {
    if (!sha || !stream) {
      setDetail(null);
      return;
    }
    let cancelled = false;
    setLoading(true);
    void getCommitDetail(stream.id, sha)
      .then((result) => {
        if (cancelled) return;
        setDetail(result);
        setLoading(false);
      })
      .catch((err) => {
        if (cancelled) return;
        logUi("warn", "commit detail failed", { error: String(err) });
        setLoading(false);
      });
    return () => { cancelled = true; };
  }, [sha, stream?.id]);

  const headerTitle = buildCommitSlideoverTitle({ sha, subject: detail?.subject ?? subject });

  return (
    <Page testId="page-git-commit" title={headerTitle} kind="commit" backlinks={backlinks}>
      <div style={{ padding: "12px 16px" }}>
        {!sha ? (
          <div style={{ color: "var(--text-secondary)", fontSize: 12 }}>No commit selected.</div>
        ) : loading && !detail ? (
          <div style={{ color: "var(--text-secondary)", fontSize: 12 }}>Loading…</div>
        ) : !detail ? (
          <div style={{ color: "var(--text-secondary)", fontSize: 12 }}>Commit not found.</div>
        ) : (
          <CommitDetailBody detail={detail} onOpenDiff={onOpenDiff} />
        )}
      </div>
    </Page>
  );
}
