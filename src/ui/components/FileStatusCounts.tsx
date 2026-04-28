import type { WorkspaceStatusSummary } from "../api.js";

/**
 * Shared `A## M## D## U##` cell — green/muted/red/yellow, mono-spaced
 * and 2-digit padded so columns line up across rows. A/M/D always
 * render; U is optional and only renders when `untracked` is provided
 * (commit-history rows don't have an untracked concept). Calling sites
 * decide whether to render the cell at all (hide when there's nothing
 * changed). Hover title explains what each letter rolls up.
 */
export function FileStatusCounts({
  filesAdded,
  filesModified,
  filesDeleted,
  filesUntracked,
  title,
  testId,
}: {
  filesAdded: number;
  filesModified: number;
  filesDeleted: number;
  filesUntracked?: number;
  title: string;
  testId?: string;
}) {
  return (
    <span
      data-testid={testId}
      title={title}
      style={{
        color: "var(--text-muted)",
        fontSize: 12,
        fontFamily: "var(--mono, monospace)",
        display: "inline-flex",
        gap: 6,
        whiteSpace: "pre",
        cursor: "help",
      }}
    >
      <span style={{ color: "var(--text-success, #16a34a)" }}>{`A${pad2(filesAdded)}`}</span>
      <span>{`M${pad2(filesModified)}`}</span>
      <span style={{ color: "var(--text-danger, #dc2626)" }}>{`D${pad2(filesDeleted)}`}</span>
      {filesUntracked != null ? (
        <span style={{ color: "var(--text-warning, #d97706)" }}>{`U${pad2(filesUntracked)}`}</span>
      ) : null}
    </span>
  );
}

/**
 * Render a `FileStatusCounts` from the workspace status summary.
 * Renames roll into M; untracked stays separate (its own U cell).
 * Returns `null` when nothing has changed so callers can render
 * "clean" instead.
 */
export function FileStatusCountsForSummary({
  summary,
  testId,
}: {
  summary: WorkspaceStatusSummary | null;
  testId?: string;
}) {
  if (!summary || summary.total === 0) return null;
  const filesAdded = summary.added;
  const filesModified = summary.modified + summary.renamed;
  const filesDeleted = summary.deleted;
  const filesUntracked = summary.untracked;
  const title =
    `${summary.total} uncommitted file${summary.total === 1 ? "" : "s"}:\n` +
    `  A — ${summary.added} added (staged new file)\n` +
    `  M — ${summary.modified} modified + ${summary.renamed} renamed\n` +
    `  D — ${summary.deleted} deleted\n` +
    `  U — ${summary.untracked} untracked (not in git, not gitignored)`;
  return (
    <FileStatusCounts
      filesAdded={filesAdded}
      filesModified={filesModified}
      filesDeleted={filesDeleted}
      filesUntracked={filesUntracked}
      title={title}
      testId={testId}
    />
  );
}

function pad2(n: number): string {
  return String(n).padStart(2, " ");
}
