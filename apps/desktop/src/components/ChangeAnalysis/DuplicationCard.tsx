import type { CodeQualityFindingRow } from "../../api-types.js";
import { DISK, type FileVersion } from "../../file-version.js";
import { duplicateBlockRef } from "../../tabs/pageRefs.js";
import { useRouteDispatch } from "../../tabs/RouteLink.js";

interface DuplicationCardProps {
  duplication: {
    findings: CodeQualityFindingRow[];
    scanAgeMs: number | null;
    scanning: boolean;
    refresh(): Promise<void>;
    /** True iff a `done` scan exists for this exact version+filter
     *  combination. When false the card hides any stale findings
     *  list and renders the "Scan now" CTA — never substitutes a
     *  scan from a different version. */
    hasScan: boolean;
    /** Last scan error, shown inline in this card (not the panel banner). */
    error: string | null;
    /** Tree version the scan ran against — stamped onto every
     *  duplicate-block ref so the side-by-side page reads files at the
     *  same version, never silently substituting the working tree. */
    scanVersion: FileVersion;
  };
  onOpenFile(path: string, opts?: { newTab?: boolean }): void;
  /** Drop the bordered card and render the title as a section `<h2>`,
   *  matching the diff view's other boxless sections. Default false. */
  boxless?: boolean;
}

export function DuplicationCard({ duplication, onOpenFile, boxless = false }: DuplicationCardProps) {
  const scanVersion = duplication.scanVersion;
  const dupes = duplication.findings.filter((f) => f.kind === "duplicate-block");
  const versionLabel =
    scanVersion.kind === "disk"
      ? "the working tree"
      : scanVersion.kind === "ref"
        ? scanVersion.ref.length > 12
          ? scanVersion.ref.slice(0, 7)
          : scanVersion.ref
        : `snapshot ${scanVersion.id.slice(0, 7)}`;
  return (
    <section data-testid="change-analysis-duplication" style={boxless ? undefined : card}>
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: 8 }}>
        {boxless ? <h2 style={boxlessHeader}>Duplication</h2> : <div style={header}>Duplication</div>}
        {!duplication.hasScan && !duplication.scanning ? (
          <button
            type="button"
            data-testid="change-analysis-duplication-refresh"
            onClick={() => void duplication.refresh()}
            style={smallButton}
          >
            Scan for duplicates
          </button>
        ) : null}
      </div>
      {duplication.error ? (
        <div data-testid="change-analysis-duplication-error" style={errorLine}>
          {duplication.error}
        </div>
      ) : null}
      {!duplication.hasScan ? (
        <div style={muted}>
          {duplication.scanning
            ? `Scanning ${versionLabel} for duplicates… progress is shown in the status bar at the bottom.`
            : `No duplication scan has run against ${versionLabel} for these files. Click “Scan for duplicates” above — duplicate-block findings only show when the scan's tree version matches what you're analyzing.`}
        </div>
      ) : dupes.length === 0 ? (
        <div style={muted}>
          No duplicate-block findings touch the changed files in this scan.
        </div>
      ) : (
        <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
          {dupes.slice(0, 25).map((f) => (
            <DuplicateRow
              key={f.id}
              finding={f}
              scanVersion={scanVersion}
              onOpenFile={onOpenFile}
            />
          ))}
          {dupes.length > 25 ? (
            <div style={muted}>…and {dupes.length - 25} more</div>
          ) : null}
        </div>
      )}
    </section>
  );
}

interface DuplicateRowProps {
  finding: CodeQualityFindingRow;
  scanVersion: FileVersion;
  onOpenFile(path: string, opts?: { newTab?: boolean }): void;
}

function DuplicateRow({ finding, scanVersion, onOpenFile }: DuplicateRowProps) {
  const peerPath = (finding.extra?.peerPath as string | undefined) ?? null;
  const peerStart = (finding.extra?.peerStartLine as number | undefined) ?? null;
  const peerEnd = (finding.extra?.peerEndLine as number | undefined) ?? null;
  const hasPeer = peerPath != null && peerStart != null && peerEnd != null;
  const ref = hasPeer
    ? duplicateBlockRef({
        leftPath: finding.path,
        leftStart: finding.startLine,
        leftEnd: finding.endLine,
        leftVersion: scanVersion,
        rightPath: peerPath!,
        rightStart: peerStart!,
        rightEnd: peerEnd!,
        rightVersion: scanVersion,
      })
    : null;
  void DISK;
  const { handlers } = useRouteDispatch(
    ref ?? { id: "noop", kind: "file", payload: { path: finding.path } },
    { onNavigate: (r, opts) => onOpenFile((r.payload as { path: string }).path, opts) },
  );
  return (
    <div
      data-testid="change-analysis-duplicate-row"
      style={{ display: "flex", alignItems: "center", gap: 8, fontSize: "var(--text-xs)" }}
    >
      {hasPeer ? (
        <button
          type="button"
          {...handlers}
          style={pathButton}
          title="Open side-by-side duplicate view"
        >
          {finding.path}:{finding.startLine}-{finding.endLine}
        </button>
      ) : (
        <button
          type="button"
          onClick={(e) => onOpenFile(finding.path, { newTab: e.metaKey || e.ctrlKey })}
          style={pathButton}
        >
          {finding.path}:{finding.startLine}-{finding.endLine}
        </button>
      )}
      <span style={{ color: "var(--text-muted)" }}>↔</span>
      {hasPeer ? (
        <button
          type="button"
          {...handlers}
          style={pathButton}
          title="Open side-by-side duplicate view"
        >
          {peerPath}:{peerStart}-{peerEnd}
        </button>
      ) : (
        <span style={muted}>(unknown peer)</span>
      )}
      <span style={{ marginLeft: "auto", color: "var(--text-muted)" }}>
        {finding.metricValue} lines
      </span>
    </div>
  );
}

const card: React.CSSProperties = {
  background: "var(--surface-card)",
  border: "1px solid var(--border-subtle)",
  borderRadius: 6,
  padding: 12,
};
const header: React.CSSProperties = { fontWeight: 600 };
const boxlessHeader: React.CSSProperties = {
  margin: 0,
  fontSize: "var(--text-lg)",
  fontWeight: 600,
  color: "var(--text-primary)",
};
const errorLine: React.CSSProperties = {
  color: "var(--severity-critical, #f87171)",
  fontSize: "var(--text-xs)",
  marginBottom: 8,
};
const muted: React.CSSProperties = { color: "var(--text-muted)", fontSize: "var(--text-xs)" };
const smallButton: React.CSSProperties = {
  padding: "4px 10px",
  background: "var(--surface-card)",
  color: "var(--text-primary)",
  border: "1px solid var(--border-subtle)",
  borderRadius: 4,
  cursor: "pointer",
  fontSize: "var(--text-xs)",
};
const pathButton: React.CSSProperties = {
  background: "transparent",
  border: "none",
  padding: 0,
  color: "var(--text-link, #2563eb)",
  cursor: "pointer",
  fontSize: "var(--text-xs)",
};
