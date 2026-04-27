import type { CSSProperties } from "react";
import { useEffect, useMemo, useState } from "react";
import {
  getCommitDetail,
  type CommitDetail,
  type Stream,
} from "../../api.js";
import { logUi } from "../../logger.js";
import type { DiffRequest } from "../Diff/diff-request.js";
import { Slideover } from "../Slideover.js";

/**
 * Pure helper exported for unit tests: builds the Slideover header
 * label from the small commit identifier callers already have on hand
 * (sha + subject), so the header renders synchronously without waiting
 * on `getCommitDetail`.
 */
export function buildCommitSlideoverTitle(input: { sha: string; subject: string }): string {
  const shaPrefix = input.sha.slice(0, 7);
  const trimmed = input.subject.trim();
  const subject = trimmed.length > 0 ? trimmed : "(no message)";
  return `${shaPrefix} · ${subject}`;
}

export interface CommitDetailSlideoverProps {
  open: boolean;
  onClose(): void;
  stream: Stream | null;
  /** Commit sha to load. */
  sha: string | null;
  /** Pre-known subject for the header so it renders without a flash. */
  subject?: string;
  /** Forwarded to file rows. */
  onOpenDiff?(request: DiffRequest): void;
}

/**
 * Right-edge Slideover wrapper around the commit detail body. Used for
 * cross-page opens (e.g. a Backlinks entry pointing at a commit from
 * another page). The docked HistoryPanel keeps its inline detail
 * layout because that panel already has the horizontal real estate.
 */
export function CommitDetailSlideover({
  open,
  onClose,
  stream,
  sha,
  subject = "",
  onOpenDiff,
}: CommitDetailSlideoverProps) {
  const [detail, setDetail] = useState<CommitDetail | null>(null);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    if (!open || !sha || !stream) {
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
    return () => {
      cancelled = true;
    };
  }, [open, sha, stream?.id]);

  const title = buildCommitSlideoverTitle({ sha: sha ?? "", subject });

  return (
    <Slideover
      open={open}
      onClose={onClose}
      title={title}
      testId="commit-detail-slideover"
    >
      {!sha ? (
        <div style={{ color: "var(--text-secondary)", fontSize: 12 }}>No commit selected.</div>
      ) : loading && !detail ? (
        <div style={{ color: "var(--text-secondary)", fontSize: 12 }}>Loading…</div>
      ) : !detail ? (
        <div style={{ color: "var(--text-secondary)", fontSize: 12 }}>Commit not found.</div>
      ) : (
        <CommitDetailBody detail={detail} onOpenDiff={onOpenDiff} />
      )}
    </Slideover>
  );
}

export function CommitDetailBody({
  detail,
  onOpenDiff,
}: {
  detail: CommitDetail;
  onOpenDiff?(request: DiffRequest): void;
}) {
  const totalAdditions = useMemo(() => detail.files.reduce((sum, f) => sum + f.additions, 0), [detail.files]);
  const totalDeletions = useMemo(() => detail.files.reduce((sum, f) => sum + f.deletions, 0), [detail.files]);
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 10, fontSize: 12 }}>
      <div>
        <div style={{ fontWeight: 600, marginBottom: 4 }}>{detail.subject}</div>
        {detail.body ? (
          <div style={{ whiteSpace: "pre-wrap", color: "var(--text-secondary)", fontSize: 11 }}>{detail.body}</div>
        ) : null}
      </div>
      <div style={{ display: "grid", gridTemplateColumns: "auto 1fr", gap: "2px 10px", color: "var(--text-secondary)", fontSize: 11 }}>
        <span>SHA</span>
        <span style={{ fontFamily: "var(--mono, monospace)", color: "var(--text-primary)" }}>{detail.sha}</span>
        <span>Author</span>
        <span>
          {detail.author.name}
          {detail.author.email ? ` <${detail.author.email}>` : ""}
        </span>
        <span>Date</span>
        <span>{formatAbsolute(detail.author.date)}</span>
        {detail.parents.length > 0 ? (
          <>
            <span>Parents</span>
            <span style={{ fontFamily: "var(--mono, monospace)" }}>
              {detail.parents.map((p) => p.slice(0, 7)).join(", ")}
            </span>
          </>
        ) : null}
      </div>
      <div>
        <div style={{ textTransform: "uppercase", letterSpacing: 0.4, fontSize: 10, color: "var(--text-secondary)", marginBottom: 4 }}>
          {detail.files.length} file{detail.files.length === 1 ? "" : "s"} changed
          <span style={{ marginLeft: 6, color: "var(--severity-ok, #86efac)" }}>+{totalAdditions}</span>
          <span style={{ marginLeft: 4, color: "var(--severity-critical, #f87171)" }}>−{totalDeletions}</span>
        </div>
        <div style={{ display: "flex", flexDirection: "column", gap: 2 }}>
          {detail.files.map((file) => {
            const realPath = file.path.includes(" → ") ? file.path.split(" → ")[1]! : file.path;
            return (
              <div
                key={file.path}
                title={`${file.path}\nDouble-click to open diff`}
                onDoubleClick={() => {
                  if (!onOpenDiff) return;
                  const left = detail.parents[0] ?? "4b825dc642cb6eb9a060e54bf8d69288fbee4904";
                  onOpenDiff({
                    path: realPath,
                    leftRef: left,
                    rightKind: { ref: detail.sha },
                    baseLabel: detail.parents[0] ? detail.parents[0].slice(0, 7) : "(root)",
                  });
                }}
                data-testid={`commit-slideover-file-${realPath}`}
                style={{ display: "flex", alignItems: "center", gap: 6, fontSize: 11, cursor: "pointer" }}
              >
                <span style={{ ...statusBadgeStyle, color: statusColor(file.status) }}>{statusLabel(file.status)}</span>
                <span style={{ flex: 1, minWidth: 0, overflow: "hidden", textOverflow: "ellipsis" }}>{realPath}</span>
                {file.additions > 0 ? <span style={{ color: "var(--severity-ok, #86efac)" }}>+{file.additions}</span> : null}
                {file.deletions > 0 ? <span style={{ color: "var(--severity-critical, #f87171)" }}>−{file.deletions}</span> : null}
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}

function statusLabel(status: string): string {
  switch (status) {
    case "added":
      return "A";
    case "deleted":
      return "D";
    case "modified":
      return "M";
    case "renamed":
      return "R";
    case "untracked":
      return "?";
    default:
      return "·";
  }
}

function statusColor(status: string): string {
  switch (status) {
    case "added":
      return "var(--severity-ok, #86efac)";
    case "deleted":
      return "var(--severity-critical, #f87171)";
    case "modified":
      return "var(--severity-warn, #e5a06a)";
    case "renamed":
      return "var(--accent, #c4b5fd)";
    default:
      return "var(--text-secondary)";
  }
}

function formatAbsolute(input: string): string {
  if (!input) return "";
  const d = new Date(input);
  if (Number.isNaN(d.getTime())) return input;
  return d.toLocaleString();
}

const statusBadgeStyle: CSSProperties = {
  display: "inline-flex",
  alignItems: "center",
  justifyContent: "center",
  width: 16,
  fontFamily: "var(--mono, monospace)",
  fontSize: 10,
  fontWeight: 600,
  flexShrink: 0,
};
