import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import type { CommitDetail, Stream, ThreadWorkState } from "../api.js";
import { getCommitDetail, gitCherryPick, gitRevert } from "../api.js";
import { awaitGitOp, gitOpErrorMessage, gitOpOutcomeMessage } from "../git-op.js";
import { logUi } from "../logger.js";
import type { DiffSpec } from "../components/Diff/DiffPane.js";
import { InlineConfirm } from "../components/InlineConfirm.js";
import { recordOpError } from "../components/opErrorsStore.js";
import { showToast } from "../components/toastStore.js";
import { Page } from "../tabs/Page.js";
import { useBacklinks, usePageOutbound } from "../tabs/useBacklinks.js";
import { gitCommitRef, opErrorRef, type ChangeAnalysisScope } from "../tabs/pageRefs.js";
import { BacklinksList } from "../tabs/BacklinksList.js";
import type { TabRef } from "../tabs/tabState.js";
import { ChangeAnalysisPanel } from "../components/ChangeAnalysis/ChangeAnalysisPanel.js";
import { ScopeFilterBanner } from "../components/ChangeAnalysis/ScopeFilterBanner.js";
import { SummaryCard } from "../components/ChangeAnalysis/SummaryCard.js";
import { useChangeAnalysis } from "../components/ChangeAnalysis/useChangeAnalysis.js";
import {
  summarizeTestFunctions,
  summarizeTestLineRatio,
} from "../components/ChangeAnalysis/analysisHelpers.js";

export interface GitCommitPageProps {
  stream: Stream | null;
  sha: string;
  /** Subject the caller already knows (for instant header rendering). */
  subject?: string;
  /** Optional drilldown scope. Pivot clicks from inside the embedded
   *  analysis panel set this on the page's own ref so the user
   *  stays on this commit while filtering by extension / directory
   *  / status. */
  scope?: ChangeAnalysisScope;
  threadWork: ThreadWorkState | null;
  /** DiffSpec opener forwarded to the embedded change-analysis panel. */
  onOpenDiff?(spec: DiffSpec): void;
  onOpenDiffInTab?(spec: DiffSpec, siblings?: import("../tabs/PageNavigationContext.js").NavSiblings): void;
  onOpenPage(ref: TabRef, opts?: { newTab?: boolean }): void;
  onOpenFile?(path: string, opts?: { newTab?: boolean }): void;
}

/**
 * Build the page title from the small commit identifier the caller
 * already has (sha + subject), so the header renders synchronously
 * without waiting on `getCommitDetail`. Exported for tests.
 */
export function buildCommitTitle(input: { sha: string; subject: string }): string {
  const shaPrefix = input.sha.slice(0, 7);
  const trimmed = input.subject.trim();
  const subject = trimmed.length > 0 ? trimmed : "(no message)";
  return `${shaPrefix} · ${subject}`;
}

/**
 * Single-commit page. Renders the standard `SummaryCard` at the top,
 * the inline commit metadata (subject, message body, SHA, author,
 * date, parents), then the embedded `ChangeAnalysisPanel` which
 * owns all file viewing for this surface — the page no longer
 * keeps a separate "files changed" tree.
 *
 * Linked from `gitCommitRef(sha)`. Backlink clicks anywhere in the
 * app open this page rather than a slideover.
 */
export function GitCommitPage({
  stream,
  sha,
  subject = "",
  scope,
  threadWork,
  onOpenDiff,
  onOpenDiffInTab,
  onOpenPage,
  onOpenFile,
}: GitCommitPageProps) {
  const [detail, setDetail] = useState<CommitDetail | null>(null);
  const [loading, setLoading] = useState(false);
  const analysis = useChangeAnalysis({ streamId: stream?.id ?? null, target: sha, scope });
  // Measure the SummaryCard so the CommitMeta on its left collapses
  // to the same height — the meta panel is content-driven and would
  // otherwise either be much shorter (1-line message) or much taller
  // (long body) than the summary it sits beside.
  const summaryRef = useRef<HTMLDivElement>(null);
  const [summaryHeight, setSummaryHeight] = useState<number | null>(null);
  useLayoutEffect(() => {
    const el = summaryRef.current;
    if (!el) {
      setSummaryHeight(null);
      return;
    }
    const measure = () => setSummaryHeight(el.getBoundingClientRect().height);
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, [analysis.files.length, sha, stream?.id]);
  const refForGraph = gitCommitRef(sha);
  const backlinkEntries = useBacklinks(refForGraph);
  const outboundEntries = usePageOutbound(refForGraph);
  const backlinks = {
    count: backlinkEntries.length,
    body: <BacklinksList entries={backlinkEntries} onOpenPage={onOpenPage} />,
  };
  const outbound =
    outboundEntries.length > 0
      ? {
          count: outboundEntries.length,
          body: <BacklinksList entries={outboundEntries} onOpenPage={onOpenPage} />,
        }
      : undefined;

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

  // Apply this commit to the current branch (cherry-pick) or undo it
  // (revert). Both fold the smart-merge auto-resolve count into the
  // success toast; failures record an op-error the user can open.
  const runCommitOp = useCallback(
    async (op: "cherry-pick" | "revert") => {
      if (!stream) return;
      const short = sha.slice(0, 7);
      const label = `${op === "cherry-pick" ? "Cherry-pick" : "Revert"} ${short}`;
      const command = `git ${op === "cherry-pick" ? "cherry-pick" : "revert --no-edit"} ${short}`;
      const kickoff =
        op === "cherry-pick" ? await gitCherryPick(stream.id, sha) : await gitRevert(stream.id, sha);
      const result = await awaitGitOp(kickoff);
      if (result.success) {
        showToast({ message: gitOpOutcomeMessage(label, result) });
        return;
      }
      const errorId = recordOpError({
        label,
        command,
        stderr: result.stderr,
        stdout: result.stdout,
        exitCode: result.status,
        message: gitOpErrorMessage(result, `${label} failed`),
      });
      showToast({
        message: `${label} failed`,
        actionLabel: "Show details",
        onUndo: () => onOpenPage(opErrorRef(errorId), { newTab: true }),
      });
    },
    [stream, sha, onOpenPage],
  );

  const headerTitle = buildCommitTitle({ sha, subject: detail?.subject ?? subject });

  return (
    <Page testId="page-git-commit" title={headerTitle} kind="git-commit" backlinks={backlinks} outbound={outbound}>
      <div style={{ display: "flex", flexDirection: "column", gap: 16, padding: "12px 16px" }}>
        {scope ? (
          <ScopeFilterBanner
            scope={scope}
            onClear={() => onOpenPage(gitCommitRef(sha))}
          />
        ) : null}
        <div
          style={{
            display: "grid",
            gridTemplateColumns: "minmax(0, 3fr) minmax(0, 2fr)",
            gap: 16,
            alignItems: "start",
          }}
        >
          <div style={{ minWidth: 0 }}>
            {!sha ? (
              <div style={muted}>No commit selected.</div>
            ) : loading && !detail ? (
              <div style={muted}>Loading…</div>
            ) : !detail ? (
              <div style={muted}>Commit not found.</div>
            ) : (
              <CommitMeta
                detail={detail}
                collapsedMaxHeight={summaryHeight}
                onRunOp={stream ? runCommitOp : undefined}
              />
            )}
          </div>
          <div style={{ minWidth: 0 }} ref={summaryRef}>
            {sha && stream && analysis.files.length > 0 ? (
              <SummaryCard
                fileCount={analysis.files.length}
                additions={analysis.totals.additions}
                deletions={analysis.totals.deletions}
                byStatus={analysis.pivots.byStatus}
                tests={analysis.tests}
                testFunctions={summarizeTestFunctions(analysis.functions)}
                testLineRatio={summarizeTestLineRatio(analysis.functionChurn)}
              />
            ) : null}
          </div>
        </div>

        {sha && stream && onOpenFile ? (
          <ChangeAnalysisPanel
            analysis={analysis}
            target={sha}
            scope={scope}
            showHeader={false}
            onOpenPage={onOpenPage}
            onOpenFile={onOpenFile}
            onOpenDiff={onOpenDiff}
            onOpenDiffInTab={onOpenDiffInTab}
          />
        ) : null}
      </div>
    </Page>
  );
}

interface CommitMetaProps {
  detail: CommitDetail;
  /** Pixel height the SummaryCard alongside this panel renders at —
   *  the panel collapses to roughly that height, with a "Show more"
   *  toggle when the message body would overflow. `null` means "let
   *  it grow" (e.g. before the summary mounts). */
  collapsedMaxHeight: number | null;
  /** Run cherry-pick / revert of this commit against the current
   *  stream's worktree. Absent when no stream is selected. */
  onRunOp?(op: "cherry-pick" | "revert"): void;
}

function CommitMeta({ detail, collapsedMaxHeight, onRunOp }: CommitMetaProps) {
  const date = formatAbsolute(new Date(detail.timestamp_secs * 1000).toISOString());
  const author = detail.email ? `${detail.author} <${detail.email}>` : detail.author;
  const sectionRef = useRef<HTMLElement>(null);
  const [expanded, setExpanded] = useState(false);
  const [overflowing, setOverflowing] = useState(false);
  useLayoutEffect(() => {
    const el = sectionRef.current;
    if (!el || collapsedMaxHeight == null) {
      setOverflowing(false);
      return;
    }
    // Compare the natural scrollHeight against the cap. A small
    // tolerance avoids a false positive from sub-pixel rounding.
    setOverflowing(el.scrollHeight > collapsedMaxHeight + 2);
  }, [collapsedMaxHeight, detail.body, detail.subject, detail.sha]);

  const collapsed = !expanded && overflowing && collapsedMaxHeight != null;
  return (
    <section
      ref={sectionRef}
      data-ref-kind="git-commit"
      data-ref-id={detail.sha}
      style={{
        ...card,
        position: "relative",
        maxHeight: collapsed ? collapsedMaxHeight! : undefined,
        overflow: collapsed ? "hidden" : undefined,
      }}
    >
      <div style={{ display: "flex", flexDirection: "column", gap: 10, fontSize: "var(--text-xs)" }}>
        <div style={{ color: "var(--text-secondary)", fontSize: 11 }}>
          {date} · {author}
        </div>
        <div style={{ userSelect: "text" }}>
          <div style={{ fontWeight: 600, marginBottom: 4 }}>{detail.subject}</div>
          {detail.body ? (
            <div style={{ whiteSpace: "pre-wrap", color: "var(--text-secondary)", fontSize: 11 }}>
              {detail.body}
            </div>
          ) : null}
        </div>
        <div
          style={{
            display: "grid",
            gridTemplateColumns: "auto 1fr",
            gap: "2px 10px",
            color: "var(--text-secondary)",
            fontSize: 11,
          }}
        >
          <span>Version</span>
          <span style={{ fontFamily: "var(--mono, monospace)", color: "var(--text-primary)" }}>
            {detail.sha}
          </span>
        </div>
        {onRunOp ? (
          <div
            data-testid="commit-actions"
            style={{ display: "flex", gap: 8, alignItems: "center", marginTop: 2 }}
          >
            <InlineConfirm
              triggerLabel="Cherry-pick"
              confirmLabel="Cherry-pick"
              title="Apply this commit's changes onto the current branch"
              testIdPrefix="commit-cherry-pick"
              triggerStyle={actionButton}
              onConfirm={() => onRunOp("cherry-pick")}
            />
            <InlineConfirm
              triggerLabel="Revert"
              confirmLabel="Revert"
              title="Create a new commit that undoes this commit's changes"
              testIdPrefix="commit-revert"
              triggerStyle={actionButton}
              onConfirm={() => onRunOp("revert")}
            />
          </div>
        ) : null}
      </div>
      {overflowing ? (
        <button
          type="button"
          onClick={() => setExpanded((v) => !v)}
          style={{
            position: "absolute",
            right: 8,
            bottom: 6,
            background: "var(--surface-card)",
            border: "1px solid var(--border-subtle)",
            borderRadius: 3,
            padding: "1px 6px",
            color: "var(--text-link, #2563eb)",
            cursor: "pointer",
            fontSize: 11,
          }}
        >
          {expanded ? "Show less" : "Show more"}
        </button>
      ) : null}
    </section>
  );
}

function formatAbsolute(input: string): string {
  if (!input) return "";
  const d = new Date(input);
  if (Number.isNaN(d.getTime())) return input;
  return d.toLocaleString();
}

const muted: React.CSSProperties = { color: "var(--text-secondary)", fontSize: "var(--text-xs)" };
const card: React.CSSProperties = {
  background: "var(--surface-card)",
  border: "1px solid var(--border-subtle)",
  borderRadius: 6,
  padding: 12,
};
const actionButton: React.CSSProperties = {
  background: "var(--surface-card)",
  border: "1px solid var(--border-subtle)",
  borderRadius: 4,
  padding: "3px 10px",
  color: "var(--text-primary)",
  cursor: "pointer",
  fontSize: 11,
};
