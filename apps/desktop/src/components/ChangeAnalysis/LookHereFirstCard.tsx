import { useState } from "react";
import type { BranchChangeEntry } from "../../api-types.js";
import type { InterestingnessResult } from "./interestingness.js";

interface LookHereFirstCardProps {
  files: BranchChangeEntry[];
  /** path -> { score, reasons } */
  fileScores: Map<string, InterestingnessResult>;
  onOpenFile?: (path: string, opts?: { newTab?: boolean }) => void;
  /** Plain click target: open the file's diff in-tab. Cmd/ctrl-
   *  click escapes to a new-tab file open via onOpenFile. */
  onOpenFileDiff?: (path: string, line?: number) => void;
  /** Drop the bordered card and render the title as a section `<h2>`,
   *  matching the diff view's other boxless sections. Default false
   *  (keeps the card on the commit/uncommitted analysis panels). */
  boxless?: boolean;
}

const COLLAPSED_CAP = 8;
const EXPANDED_CAP = 25;
const SCORE_FLOOR = 1.5;

interface Row {
  path: string;
  score: number;
  reasons: string[];
  additions: number;
  deletions: number;
}

/**
 * Headline panel for the Change Analysis dashboard.
 *
 * Ranks every changed file by `fileInterestingness` and surfaces the
 * top 8 (expandable to 25) above the score floor. Each row shows
 * the score badge, the file path, and the human-readable
 * `reasons` so the reviewer can decide where to drill in. When no
 * row clears the floor, renders a "diff looks routine" empty
 * state — no point cluttering the dashboard with low-signal
 * panels.
 */
export function LookHereFirstCard({ files, fileScores, onOpenFile, onOpenFileDiff, boxless = false }: LookHereFirstCardProps) {
  const [expanded, setExpanded] = useState(false);
  const ranked: Row[] = files
    .map((f) => {
      const result = fileScores.get(f.path);
      return {
        path: f.path,
        score: result?.score ?? 0,
        reasons: result?.reasons ?? [],
        additions: f.additions ?? 0,
        deletions: f.deletions ?? 0,
      };
    })
    .filter((r) => r.score >= SCORE_FLOOR)
    .sort((a, b) => b.score - a.score);

  if (files.length === 0) return null;

  const sectionStyle = boxless ? undefined : card;
  const headerEl = boxless ? (
    <h2 style={boxlessHeader}>Look here first</h2>
  ) : (
    <div style={header}>Look here first</div>
  );

  if (ranked.length === 0) {
    return (
      <section data-testid="change-analysis-look-here-first" style={sectionStyle}>
        {headerEl}
        <div style={{ color: "var(--text-muted)", fontSize: "var(--text-xs)" }}>
          Nothing stands out — diff looks routine.
        </div>
      </section>
    );
  }

  const cap = expanded ? EXPANDED_CAP : COLLAPSED_CAP;
  const visible = ranked.slice(0, cap);
  const moreCount = Math.min(ranked.length, EXPANDED_CAP) - visible.length;

  return (
    <section data-testid="change-analysis-look-here-first" style={sectionStyle}>
      {headerEl}
      <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
        {visible.map((row) => (
          <div
            key={row.path}
            style={{ display: "flex", alignItems: "center", gap: 10, fontSize: "var(--text-xs)" }}
            data-testid="change-analysis-look-here-first-row"
            title={row.reasons.join(" · ")}
          >
            <span style={badgeStyle(row.score)}>▲ {row.score.toFixed(1)}</span>
            <button
              type="button"
              onClick={(e) => {
                if (e.metaKey || e.ctrlKey) {
                  onOpenFile?.(row.path, { newTab: true });
                  return;
                }
                if (onOpenFileDiff) onOpenFileDiff(row.path);
                else onOpenFile?.(row.path);
              }}
              style={pathButton}
              title={row.path}
            >
              {row.path}
            </button>
            <div style={{ display: "flex", gap: 4, flexWrap: "wrap" }}>
              {row.reasons.slice(0, 4).map((r) => (
                <span key={r} style={reasonStyle}>
                  {r}
                </span>
              ))}
            </div>
            <span style={{ marginLeft: "auto", color: "var(--text-muted)", fontSize: 11 }}>
              +{row.additions} −{row.deletions}
            </span>
          </div>
        ))}
      </div>
      {moreCount > 0 && !expanded ? (
        <button type="button" onClick={() => setExpanded(true)} style={showMoreButton}>
          Show {moreCount} more
        </button>
      ) : null}
    </section>
  );
}

function badgeStyle(score: number): React.CSSProperties {
  // Tier the badge color by absolute score: warm for "you really
  // should look", neutral for "worth a glance".
  const color =
    score >= 12
      ? "var(--text-danger, #dc2626)"
      : score >= 5
        ? "var(--text-warning, #d97706)"
        : "var(--text-muted)";
  return {
    fontFamily: "ui-monospace, monospace",
    fontSize: 11,
    color,
    minWidth: 56,
    textAlign: "right",
  };
}

const card: React.CSSProperties = {
  background: "var(--surface-card)",
  border: "1px solid var(--border-subtle)",
  borderRadius: 6,
  padding: 12,
};
const header: React.CSSProperties = { fontWeight: 600, marginBottom: 8 };
const boxlessHeader: React.CSSProperties = {
  margin: "0 0 8px",
  fontSize: "var(--text-lg)",
  fontWeight: 600,
  color: "var(--text-primary)",
};
const pathButton: React.CSSProperties = {
  background: "transparent",
  border: "none",
  padding: 0,
  color: "var(--text-link, #2563eb)",
  cursor: "pointer",
  fontSize: "var(--text-xs)",
  textAlign: "left",
  flexShrink: 0,
  maxWidth: 320,
  overflow: "hidden",
  textOverflow: "ellipsis",
  whiteSpace: "nowrap",
};
const reasonStyle: React.CSSProperties = {
  fontSize: 11,
  color: "var(--text-muted)",
  background: "var(--surface-muted, var(--surface-card))",
  border: "1px solid var(--border-subtle)",
  borderRadius: 3,
  padding: "0 6px",
};
const showMoreButton: React.CSSProperties = {
  marginTop: 8,
  background: "transparent",
  border: "none",
  color: "var(--text-link, #2563eb)",
  cursor: "pointer",
  fontSize: "var(--text-xs)",
  padding: 0,
};
