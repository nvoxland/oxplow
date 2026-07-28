import { Fragment, useEffect, useState } from "react";

import {
  type AgentNudge,
  type EffortObservation,
  listEffortObservations,
  listNudgesForEffort,
  subscribeOxplowEvents,
} from "../api.js";
import { effortCoverageRef } from "../tabs/pageRefs.js";
import { useOptionalPageNavigation } from "../tabs/PageNavigationContext.js";
import { EffortTokenUsageBlock } from "./EffortTokenUsage.js";

/** Parsed `diff-coverage` payload (see collection.md / observation_store). */
interface DiffCoveragePayload {
  summaryPct: number;
  changedLines: number;
  coveredLines: number;
  files: { path: string; uncoveredChangedLines: number[] }[];
}

type TestStatus = "passed" | "failed" | "skipped";

interface JUnitCase {
  classname: string;
  name: string;
  status: TestStatus;
  timeMs?: number;
}
interface JUnitSuite {
  name: string;
  cases: JUnitCase[];
}

interface TestRunPayload {
  command: string;
  exitCode?: number;
  passed?: number;
  failed?: number;
  total?: number;
  skipped?: number;
  durationMs?: number;
  /** Parsed JUnit tree (present when oxplow parsed a test report). */
  suites?: JUnitSuite[];
}


/** One static-analysis finding (see collection.md / AnalysisReport). */
export type FindingSeverity = "error" | "warning" | "info" | "note";
export interface AnalysisFinding {
  path: string;
  line?: number;
  column?: number;
  severity: FindingSeverity;
  rule?: string;
  message: string;
}
/** Parsed `static-analysis` payload. */
export interface StaticAnalysisPayload {
  command?: string;
  analyzer?: string;
  errorCount?: number;
  warningCount?: number;
  infoCount?: number;
  noteCount?: number;
  findings?: AnalysisFinding[];
}

function parsePayload<T>(json: string | null): T | null {
  if (!json) return null;
  try {
    return JSON.parse(json) as T;
  } catch {
    return null;
  }
}

/** Per-severity counts: prefer the payload's explicit counts, else derive
 *  from the findings list (older rows or command-only ran-records). */
export function analysisCounts(p: StaticAnalysisPayload): {
  error: number;
  warning: number;
  info: number;
  note: number;
} {
  const has =
    p.errorCount !== undefined ||
    p.warningCount !== undefined ||
    p.infoCount !== undefined ||
    p.noteCount !== undefined;
  if (has) {
    return {
      error: p.errorCount ?? 0,
      warning: p.warningCount ?? 0,
      info: p.infoCount ?? 0,
      note: p.noteCount ?? 0,
    };
  }
  const counts = { error: 0, warning: 0, info: 0, note: 0 };
  for (const f of p.findings ?? []) counts[f.severity]++;
  return counts;
}

/** Group findings by file path, preserving first-seen order. */
export function groupFindingsByFile(findings: AnalysisFinding[]): {
  path: string;
  findings: AnalysisFinding[];
}[] {
  const order: string[] = [];
  const byPath = new Map<string, AnalysisFinding[]>();
  for (const f of findings) {
    if (!byPath.has(f.path)) {
      byPath.set(f.path, []);
      order.push(f.path);
    }
    byPath.get(f.path)!.push(f);
  }
  return order.map((path) => ({ path, findings: byPath.get(path)! }));
}

/** Headline result line, e.g. "0 errors, 3 warnings". Errors+warnings lead;
 *  info/note appended only when present. */
export function analysisHeadline(c: { error: number; warning: number; info: number; note: number }): string {
  const parts = [
    `${c.error} error${c.error === 1 ? "" : "s"}`,
    `${c.warning} warning${c.warning === 1 ? "" : "s"}`,
  ];
  if (c.info > 0) parts.push(`${c.info} info`);
  if (c.note > 0) parts.push(`${c.note} note${c.note === 1 ? "" : "s"}`);
  return parts.join(", ");
}

/** Green when clean, amber when only warnings/info/note, rose on any error. */
function analysisColor(c: { error: number; warning: number; info: number; note: number }): string {
  if (c.error > 0) return "var(--freshness-very-stale)";
  if (c.warning + c.info + c.note > 0) return "var(--freshness-stale)";
  return "var(--freshness-fresh)";
}

function severityColor(s: FindingSeverity): string {
  return s === "error"
    ? "var(--freshness-very-stale)"
    : s === "warning"
      ? "var(--freshness-stale)"
      : "var(--text-muted)";
}
function severityGlyph(s: FindingSeverity): string {
  return s === "error" ? "✗" : s === "warning" ? "⚠" : s === "info" ? "ℹ" : "·";
}

// Coverage coloring thresholds. These MIRROR the `oxplow.coverage.diff_pct`
// metric definition's `target` (80) and `fail_at` (50) — the canonical source
// of the ramp now lives in DATA on the definition (tsk220), and the substrate
// Metrics page colors straight from `def.target`/`def.fail_at`/`def.direction`
// via `statusColor`. This legacy effort-observation panel reads the raw pct (not
// the def), so it restates the same two numbers here until it's retired (tsk215).
const COVERAGE_TARGET_PCT = 80;
const COVERAGE_FAIL_PCT = 50;

/** Emerald ≥ target, amber ≥ fail floor, rose below — reusing the freshness ramp. */
function coverageColor(pct: number): string {
  if (pct >= COVERAGE_TARGET_PCT) return "var(--freshness-fresh)";
  if (pct >= COVERAGE_FAIL_PCT) return "var(--freshness-stale)";
  return "var(--freshness-very-stale)";
}

/** Segmented covered/uncovered bar for the changed lines of an effort. */
function CoverageBar({
  covered,
  total,
  testId,
}: {
  covered: number;
  total: number;
  testId?: string;
}) {
  const pct = total > 0 ? (covered / total) * 100 : 0;
  return (
    <div
      data-testid={testId}
      title={`${covered} of ${total} changed lines covered`}
      style={{
        display: "flex",
        height: 10,
        borderRadius: 5,
        overflow: "hidden",
        background: "var(--surface-app)",
        border: "1px solid var(--border-subtle)",
      }}
    >
      <div style={{ width: `${pct}%`, background: "var(--freshness-fresh)" }} />
      <div style={{ flex: 1, background: "var(--freshness-very-stale)" }} />
    </div>
  );
}

function fileBasename(path: string): { dir: string; name: string } {
  const idx = path.lastIndexOf("/");
  return idx < 0
    ? { dir: "", name: path }
    : { dir: path.slice(0, idx + 1), name: path.slice(idx + 1) };
}

/** The "dashboard": changed files ranked by how many changed lines are
 *  uncovered, with a proportional bar. Click opens the file. */
function MostUntested({
  cov,
  onOpenFile,
}: {
  cov: DiffCoveragePayload;
  onOpenFile?: (path: string) => void;
}) {
  const ranked = cov.files
    .map((f) => ({ path: f.path, count: f.uncoveredChangedLines.length, lines: f.uncoveredChangedLines }))
    .filter((f) => f.count > 0)
    .sort((a, b) => b.count - a.count);
  if (ranked.length === 0) {
    return (
      <div style={{ fontSize: "var(--text-xs)", color: "var(--freshness-fresh)" }}>
        Every changed line is covered.
      </div>
    );
  }
  const max = ranked[0].count;
  const shown = ranked.slice(0, 8);
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
      <span style={{ fontSize: "var(--text-xs)", color: "var(--text-muted)", textTransform: "uppercase", letterSpacing: "0.04em" }}>
        Most untested changes
      </span>
      {shown.map((f) => {
        const { dir, name } = fileBasename(f.path);
        return (
          <button
            key={f.path}
            type="button"
            data-testid={`untested-file-${f.path}`}
            onClick={() => onOpenFile?.(f.path)}
            title={`${f.path} — uncovered changed lines ${f.lines.join(", ")}`}
            style={{
              display: "grid",
              gridTemplateColumns: "1fr 90px 2ch",
              alignItems: "center",
              gap: 8,
              textAlign: "left",
              background: "transparent",
              border: "none",
              padding: "2px 0",
              cursor: onOpenFile ? "pointer" : "default",
            }}
          >
            <span
              style={{
                fontFamily: "var(--font-mono)",
                fontSize: "var(--text-xs)",
                overflow: "hidden",
                textOverflow: "ellipsis",
                whiteSpace: "nowrap",
                minWidth: 0,
              }}
            >
              <span style={{ color: "var(--text-muted)" }}>{dir}</span>
              <span style={{ color: "var(--text-secondary)" }}>{name}</span>
            </span>
            <span style={{ display: "flex", height: 6, borderRadius: 3, overflow: "hidden", background: "var(--surface-app)" }}>
              <span style={{ width: `${(f.count / max) * 100}%`, background: "var(--freshness-very-stale)" }} />
            </span>
            <span className="oxplow-tabular" style={{ color: "var(--text-secondary)", fontSize: "var(--text-xs)", textAlign: "right" }}>
              {f.count}
            </span>
          </button>
        );
      })}
      {ranked.length > shown.length ? (
        <span style={{ fontSize: "var(--text-xs)", color: "var(--text-muted)" }}>
          + {ranked.length - shown.length} more file{ranked.length - shown.length === 1 ? "" : "s"}
        </span>
      ) : null}
    </div>
  );
}

function CoverageSummary({
  obs,
  onOpenFile,
}: {
  obs: EffortObservation;
  onOpenFile?: (path: string) => void;
}) {
  const cov = parsePayload<DiffCoveragePayload>(obs.payload_json);
  if (!cov) return null;
  const pct = Math.round(cov.summaryPct);
  const pin = obs.git_version_exact
    ? obs.closest_git_version
      ? `@ ${obs.closest_git_version.slice(0, 7)}`
      : null
    : "uncommitted";
  return (
    <div data-testid={`coverage-badge-${obs.effort_id}`} style={{ display: "flex", flexDirection: "column", gap: 8 }}>
      <div style={{ display: "flex", alignItems: "baseline", gap: 8, flexWrap: "wrap" }}>
        <span style={{ fontSize: "var(--text-lg)", fontWeight: "var(--weight-bold)", color: coverageColor(cov.summaryPct) }}>
          {pct}%
        </span>
        <span style={{ color: "var(--text-secondary)", fontSize: "var(--text-sm)" }}>
          of changed lines covered · {cov.coveredLines}/{cov.changedLines}
        </span>
        {pin ? (
          <span className="oxplow-tabular" style={{ color: "var(--text-muted)" }}>
            {pin}
          </span>
        ) : null}
      </div>
      <CoverageBar covered={cov.coveredLines} total={cov.changedLines} testId={`coverage-bar-${obs.effort_id}`} />
      <MostUntested cov={cov} onOpenFile={onOpenFile} />
    </div>
  );
}

/** Static-analysis result: analyzer ran, the high-level error/warning count,
 *  and a drill-in findings list grouped by file. `maxFiles` caps the inline
 *  list (the compact effort block shows a few; the detail view shows all). */
function StaticAnalysisSummary({
  obs,
  onOpenFile,
  maxFiles,
}: {
  obs: EffortObservation;
  onOpenFile?: (path: string) => void;
  maxFiles: number;
}) {
  const payload = parsePayload<StaticAnalysisPayload>(obs.payload_json);
  if (!payload) return null;
  const counts = analysisCounts(payload);
  const label = payload.analyzer?.trim() || "Static analysis";
  const grouped = groupFindingsByFile(payload.findings ?? []);
  const shownGroups = grouped.slice(0, maxFiles);
  const hiddenFiles = grouped.length - shownGroups.length;

  return (
    <div data-testid={`analysis-badge-${obs.effort_id}`} style={{ display: "flex", flexDirection: "column", gap: 6 }}>
      <div style={{ display: "flex", alignItems: "baseline", gap: 8, flexWrap: "wrap" }}>
        <span style={{ fontFamily: "var(--font-mono)", color: "var(--text-secondary)", fontSize: "var(--text-sm)" }}>
          {label}
        </span>
        <span style={{ fontSize: "var(--text-sm)", fontWeight: "var(--weight-medium)", color: analysisColor(counts) }}>
          {analysisHeadline(counts)}
        </span>
      </div>
      {grouped.length > 0 ? (
        <div style={{ display: "flex", flexDirection: "column", gap: 4, paddingLeft: 4 }}>
          {shownGroups.map((g) => {
            const { dir, name } = fileBasename(g.path);
            return (
              <div key={g.path} style={{ display: "flex", flexDirection: "column", gap: 1 }}>
                <button
                  type="button"
                  data-testid={`analysis-file-${g.path}`}
                  onClick={() => onOpenFile?.(g.path)}
                  title={g.path}
                  style={{
                    display: "flex",
                    alignItems: "baseline",
                    gap: 6,
                    textAlign: "left",
                    background: "transparent",
                    border: "none",
                    padding: "1px 0",
                    cursor: onOpenFile ? "pointer" : "default",
                    fontFamily: "var(--font-mono)",
                    fontSize: "var(--text-xs)",
                    overflow: "hidden",
                    textOverflow: "ellipsis",
                    whiteSpace: "nowrap",
                    minWidth: 0,
                  }}
                >
                  <span style={{ color: "var(--text-muted)" }}>{dir}</span>
                  <span style={{ color: "var(--text-secondary)" }}>{name}</span>
                </button>
                {g.findings.map((f, i) => (
                  <div
                    key={`${f.line ?? "?"}-${f.rule ?? ""}-${i}`}
                    data-testid={`analysis-finding-${g.path}`}
                    style={{ display: "flex", alignItems: "baseline", gap: 6, paddingLeft: 14, fontSize: "var(--text-xs)" }}
                  >
                    <span style={{ color: severityColor(f.severity) }}>{severityGlyph(f.severity)}</span>
                    <span className="oxplow-tabular" style={{ color: "var(--text-muted)" }}>
                      {f.line !== undefined ? `:${f.line}` : ""}
                    </span>
                    {f.rule ? <span style={{ color: "var(--text-muted)", fontFamily: "var(--font-mono)" }}>{f.rule}</span> : null}
                    <span style={{ color: "var(--text-secondary)", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", minWidth: 0 }}>
                      {f.message}
                    </span>
                  </div>
                ))}
              </div>
            );
          })}
          {hiddenFiles > 0 ? (
            <span style={{ fontSize: "var(--text-xs)", color: "var(--text-muted)" }}>
              + {hiddenFiles} more file{hiddenFiles === 1 ? "" : "s"}
            </span>
          ) : null}
        </div>
      ) : (
        <span style={{ fontSize: "var(--text-xs)", color: "var(--freshness-fresh)" }}>No issues found.</span>
      )}
    </div>
  );
}

/** A node enriched with PER-RUN data: `counts[i]` is the aggregate for run i
 *  (oldest-first) and each leaf carries its status in every run. Lets both the
 *  overview and the detail tree show pass/fail for each run, not just a merge. */
export interface MultiRunLeaf {
  name: string;
  /** Status in each run, oldest-first; `null` for a run the case didn't run in. */
  statuses: (TestStatus | null)[];
  /** Most recent non-null status — the leaf's current state. */
  finalStatus: TestStatus;
  everFailed: boolean;
  timeMs?: number;
}
export interface MultiRunNode {
  label: string;
  path: string;
  children: MultiRunNode[];
  leaves: MultiRunLeaf[];
  /** Aggregate pass/fail/skip per run, oldest-first (one entry per run). */
  counts: { passed: number; failed: number; skipped: number }[];
}

/** The grouping path for one case. The path differs by tech: nextest puts the
 *  module path in `name` (classname = crate); pytest/jest put it in `classname`.
 *  Use both, split on `::`/`.`, drop a leading segment that just repeats the
 *  suite, collapse consecutive dupes — the last segment is the test, the rest
 *  is the natural module / describe tree. */
function caseSegments(suiteName: string, c: JUnitCase): { segs: string[]; leafName: string } {
  let segs = [...c.classname.split(/::|\./), ...c.name.split(/::|\./)]
    .map((s) => s.trim())
    .filter(Boolean)
    .filter((s, i, a) => i === 0 || s !== a[i - 1]);
  if (segs.length > 1 && segs[0] === suiteName) segs = segs.slice(1);
  const leafName = segs.pop() ?? c.name;
  return { segs, leafName };
}

/** Build a per-suite tree across ALL runs (union of nodes/leaves), recording
 *  each leaf's status in every run and each node's aggregate counts per run.
 *  Runs are ordered oldest-first so a red→green progression reads left→right. */
export function buildMultiRunTree(runs: EffortObservation[]): MultiRunNode[] {
  const ordered = [...runs].sort((a, b) =>
    a.created_at < b.created_at ? -1 : a.created_at > b.created_at ? 1 : 0,
  );
  const n = ordered.length;

  interface MutLeaf {
    name: string;
    statuses: (TestStatus | null)[];
    timeMs?: number;
  }
  interface Mut {
    label: string;
    path: string;
    childMap: Map<string, Mut>;
    leafMap: Map<string, MutLeaf>;
  }
  const mut = (label: string, path: string): Mut => ({
    label,
    path,
    childMap: new Map(),
    leafMap: new Map(),
  });

  const suiteRoots = new Map<string, Mut>();
  const suiteOrder: string[] = [];

  ordered.forEach((obs, runIdx) => {
    const payload = parsePayload<TestRunPayload>(obs.payload_json);
    for (const suite of payload?.suites ?? []) {
      const suiteName = suite.name || "(tests)";
      let root = suiteRoots.get(suiteName);
      if (!root) {
        root = mut(suiteName, suiteName);
        suiteRoots.set(suiteName, root);
        suiteOrder.push(suiteName);
      }
      for (const c of suite.cases) {
        const { segs, leafName } = caseSegments(suiteName, c);
        let node = root;
        let path = root.path;
        for (const seg of segs) {
          path += `/${seg}`;
          let child = node.childMap.get(seg);
          if (!child) {
            child = mut(seg, path);
            node.childMap.set(seg, child);
          }
          node = child;
        }
        let leaf = node.leafMap.get(leafName);
        if (!leaf) {
          leaf = { name: leafName, statuses: new Array<TestStatus | null>(n).fill(null) };
          node.leafMap.set(leafName, leaf);
        }
        leaf.statuses[runIdx] = c.status;
        if (c.timeMs !== undefined) leaf.timeMs = c.timeMs; // last run wins
      }
    }
  });

  const finalize = (m: Mut): MultiRunNode => {
    const children = [...m.childMap.values()].map(finalize);
    const leaves: MultiRunLeaf[] = [...m.leafMap.values()].map((l) => {
      const everFailed = l.statuses.some((s) => s === "failed");
      let finalStatus: TestStatus = "skipped";
      for (let i = l.statuses.length - 1; i >= 0; i--) {
        const s = l.statuses[i];
        if (s) {
          finalStatus = s;
          break;
        }
      }
      return { name: l.name, statuses: l.statuses, finalStatus, everFailed, timeMs: l.timeMs };
    });
    const counts = Array.from({ length: n }, (_, i) => {
      const c = { passed: 0, failed: 0, skipped: 0 };
      for (const leaf of leaves) {
        const s = leaf.statuses[i];
        if (s) c[s]++;
      }
      for (const child of children) {
        c.passed += child.counts[i].passed;
        c.failed += child.counts[i].failed;
        c.skipped += child.counts[i].skipped;
      }
      return c;
    });
    return { label: m.label, path: m.path, children, leaves, counts };
  };

  return suiteOrder.map((s) => finalize(suiteRoots.get(s)!));
}

function statusColor(status: TestStatus): string {
  return status === "passed"
    ? "var(--freshness-fresh)"
    : status === "failed"
      ? "var(--freshness-very-stale)"
      : "var(--text-muted)";
}
function statusGlyph(status: TestStatus): string {
  return status === "passed" ? "✓" : status === "failed" ? "✗" : "⊘";
}

/** One run's "12✓ 1✗ 2⊘" rollup, omitting zeros. `–` when the node ran nothing
 *  that round (so a suite that skipped a run reads distinctly from one at 0). */
function RunCounts({ counts }: { counts: MultiRunNode["counts"][number] }) {
  const parts: Array<[number, TestStatus]> = [
    [counts.passed, "passed"],
    [counts.failed, "failed"],
    [counts.skipped, "skipped"],
  ];
  const shown = parts.filter(([nn]) => nn > 0);
  if (shown.length === 0) return <span style={{ color: "var(--text-muted)" }}>–</span>;
  return (
    <span style={{ display: "inline-flex", gap: 4 }}>
      {shown.map(([nn, s]) => (
        <span key={s} style={{ color: statusColor(s) }}>
          {nn}
          {statusGlyph(s)}
        </span>
      ))}
    </span>
  );
}

/** Per-run pass/fail strip for a node: `300✓ → 305✓` reads oldest→newest, so a
 *  TDD red→green progression is visible on every row, not just one merged count. */
function RunCountsStrip({ counts }: { counts: MultiRunNode["counts"] }) {
  return (
    <span style={{ display: "inline-flex", gap: 4, alignItems: "baseline", fontSize: "var(--text-xs)" }}>
      {counts.map((c, i) => (
        <Fragment key={i}>
          {i > 0 ? <span style={{ color: "var(--text-muted)" }}>→</span> : null}
          <RunCounts counts={c} />
        </Fragment>
      ))}
    </span>
  );
}

/** Per-run status glyphs for a leaf case: `✗ ✓ ✓`; `·` for a run it didn't run in. */
function LeafStatusStrip({ statuses }: { statuses: (TestStatus | null)[] }) {
  return (
    <span style={{ display: "inline-flex", gap: 4, fontSize: "var(--text-xs)" }}>
      {statuses.map((s, i) => (
        <span
          key={i}
          title={s ? `Run ${i + 1}: ${s}` : `Run ${i + 1}: not run`}
          style={{ color: s ? statusColor(s) : "var(--text-muted)" }}
        >
          {s ? statusGlyph(s) : "·"}
        </span>
      ))}
    </span>
  );
}

/** Header strip of per-run totals — `714✓ 6✗ → 720✓` — the single source for
 *  "number of runs and the pass/fail in each". `iterations` is oldest-first and
 *  already windowed by the caller (overview: last few; detail: all). */
function RunTimeline({ iterations }: { iterations: TestIteration[] }) {
  return (
    <span style={{ display: "inline-flex", alignItems: "baseline", gap: 6, flexWrap: "wrap", fontSize: "var(--text-sm)" }}>
      {iterations.map((it, i) => (
        <Fragment key={`${it.at}-${i}`}>
          {i > 0 ? <span style={{ color: "var(--text-muted)" }}>→</span> : null}
          <span title={new Date(it.at).toLocaleString()} style={{ display: "inline-flex", gap: 4, alignItems: "baseline" }}>
            <span style={{ color: "var(--freshness-fresh)", fontWeight: "var(--weight-medium)" }}>{it.passed}✓</span>
            {it.failed > 0 ? (
              <span style={{ color: "var(--freshness-very-stale)", fontWeight: "var(--weight-medium)" }}>{it.failed}✗</span>
            ) : null}
          </span>
        </Fragment>
      ))}
    </span>
  );
}

function MultiRunTreeNode({ node, depth }: { node: MultiRunNode; depth: number }) {
  // Auto-expand branches that failed in any run so failures are visible;
  // always-passing branches start collapsed.
  const everFailed = node.counts.some((c) => c.failed > 0);
  const [open, setOpen] = useState(everFailed);
  return (
    <div>
      <button
        type="button"
        data-testid={`test-node-${node.path}`}
        onClick={() => setOpen((o) => !o)}
        style={{
          display: "flex",
          alignItems: "center",
          gap: 6,
          width: "100%",
          textAlign: "left",
          background: "transparent",
          border: "none",
          padding: "1px 0",
          paddingLeft: depth * 12,
          cursor: "pointer",
          color: "var(--text-secondary)",
          fontSize: "var(--text-xs)",
        }}
      >
        <span style={{ width: 16, fontSize: 16, lineHeight: 1, color: "var(--text-muted)" }}>{open ? "▾" : "▸"}</span>
        <span style={{ fontFamily: "var(--font-mono)", flex: "0 1 auto" }}>{node.label}</span>
        <RunCountsStrip counts={node.counts} />
      </button>
      {open ? (
        <div>
          {node.children.map((c) => (
            <MultiRunTreeNode key={c.path} node={c} depth={depth + 1} />
          ))}
          {node.leaves.map((leaf) => {
            const onceFailed = leaf.everFailed && leaf.finalStatus !== "failed";
            return (
              <div
                key={leaf.name}
                data-testid={`test-case-${leaf.name}`}
                title={onceFailed ? "Was failing during this effort" : undefined}
                style={{
                  display: "flex",
                  alignItems: "center",
                  gap: 6,
                  paddingLeft: (depth + 1) * 12 + 16,
                  fontSize: "var(--text-xs)",
                }}
              >
                <LeafStatusStrip statuses={leaf.statuses} />
                <span style={{ fontFamily: "var(--font-mono)", color: onceFailed ? "var(--freshness-stale)" : "var(--text-primary)" }}>
                  {leaf.name}
                </span>
                {onceFailed ? (
                  <span style={{ color: "var(--freshness-stale)", fontSize: "var(--text-xs)" }}>↩</span>
                ) : null}
                {leaf.timeMs !== undefined && leaf.timeMs > 0 ? (
                  <span className="oxplow-tabular" style={{ color: "var(--text-muted)" }}>
                    {(leaf.timeMs / 1000).toFixed(2)}s
                  </span>
                ) : null}
              </div>
            );
          })}
        </div>
      ) : null}
    </div>
  );
}

/** One logical test pass within an effort: summed pass/fail at a point in time. */
export interface TestIteration {
  at: string;
  passed: number;
  failed: number;
}

/** One iteration per test-run observation, oldest-first so a TDD red→green
 *  progression reads left→right. Each `record_test_run` the effort owns is its
 *  own iteration — snapshots are coarse (captured only at effort boundaries, not
 *  per-edit), so grouping by `local_snapshot_id` collapses every run in an effort
 *  into one bar and hides exactly the repeated-runs signal this view exists to
 *  show. Attribution is now exact per-effort (the run ledger), so the raw run
 *  sequence is the truthful timeline. */
export function clusterTestRuns(runs: EffortObservation[]): TestIteration[] {
  return [...runs]
    .sort((a, b) => (a.created_at < b.created_at ? -1 : a.created_at > b.created_at ? 1 : 0))
    .map((obs) => {
      const p = parsePayload<TestRunPayload>(obs.payload_json);
      return { at: obs.created_at, passed: p?.passed ?? 0, failed: p?.failed ?? 0 };
    });
}

/** Run-count label for the Tests-run header. Calls out the zero case explicitly
 *  so an effort with observations but no test run reads clearly, rather than the
 *  section silently omitting tests. */
export function runsLabel(count: number): string {
  if (count === 0) return "no runs";
  return `${count} ${count === 1 ? "run" : "runs"}`;
}

/** The overview windows the per-run breakdown to the most recent few runs; the
 *  full history lives on the Details page. */
const OVERVIEW_RUN_LIMIT = 3;

const kickerStyle: React.CSSProperties = {
  fontSize: "var(--text-xs)",
  color: "var(--text-muted)",
  textTransform: "uppercase",
  letterSpacing: "0.04em",
};

/** Largest case count this suite reached in any shown run — the sort key for
 *  "top suites" so a suite that ran big in some round still ranks. */
function suiteTotal(node: MultiRunNode): number {
  return Math.max(0, ...node.counts.map((c) => c.passed + c.failed + c.skipped));
}

/** Test-run timeline + top-suite breakdown for a set of runs. `effortId` is
 *  optional: when present (single-effort context) the header links to that
 *  effort's coverage detail page; the diff view passes runs unioned across
 *  several efforts and omits the link. */
export function TestsRun({ effortId, runs }: { effortId?: string; runs: EffortObservation[] }) {
  const ctxNav = useOptionalPageNavigation();

  // One source of truth for the header: per-run totals, oldest-first. The
  // overview shows only the last few; the full count is still spelled out.
  const iterations = clusterTestRuns(runs);
  const shown = iterations.slice(-OVERVIEW_RUN_LIMIT);
  const truncated = iterations.length - shown.length;

  // Suite breakdown over the same last-N window, so each row's per-run strip
  // lines up with the header timeline.
  const windowRuns = [...runs]
    .sort((a, b) => (a.created_at < b.created_at ? -1 : a.created_at > b.created_at ? 1 : 0))
    .slice(-OVERVIEW_RUN_LIMIT);
  const tree = buildMultiRunTree(windowRuns);
  const sorted = [...tree].sort((a, b) => suiteTotal(b) - suiteTotal(a));
  const top5 = sorted.slice(0, 5);
  const overflow = sorted.length - top5.length;

  const navToDetail = ctxNav && effortId ? () => ctxNav.navigate(effortCoverageRef(effortId)) : undefined;
  const suiteRowStyle: React.CSSProperties = {
    display: "flex",
    alignItems: "center",
    gap: 6,
    fontSize: "var(--text-xs)",
    width: "100%",
    textAlign: "left",
    background: "transparent",
    border: "none",
    padding: "1px 0",
    cursor: navToDetail ? "pointer" : "default",
  };

  return (
    <div data-testid="tests-run" style={{ display: "flex", flexDirection: "column", gap: 6 }}>
      {/* Single header: run count + each run's pass/fail (zero called out). */}
      <div data-testid="tests-runs-header" style={{ display: "flex", alignItems: "baseline", gap: 8, flexWrap: "wrap" }}>
        <span style={kickerStyle}>Tests run</span>
        <span style={{ fontSize: "var(--text-xs)", color: "var(--text-muted)" }}>{runsLabel(iterations.length)}</span>
        {truncated > 0 ? <span style={{ fontSize: "var(--text-sm)", color: "var(--text-muted)" }}>…→</span> : null}
        {shown.length > 0 ? <RunTimeline iterations={shown} /> : null}
        {navToDetail ? (
          <button
            type="button"
            onClick={navToDetail}
            style={{
              marginLeft: "auto",
              background: "transparent",
              border: "none",
              padding: 0,
              cursor: "pointer",
              fontSize: "var(--text-xs)",
              color: "var(--accent)",
            }}
          >
            Details →
          </button>
        ) : null}
      </div>
      {/* Top 5 suites — each row shows its per-run pass/fail and navigates to detail */}
      {top5.length > 0 ? (
        <div style={{ display: "flex", flexDirection: "column", gap: 2, paddingLeft: 4 }}>
          {top5.map((n) => (
            <button key={n.path} type="button" onClick={navToDetail} style={suiteRowStyle}>
              <span style={{ fontFamily: "var(--font-mono)", color: "var(--text-secondary)", flex: "1 1 auto", minWidth: 0, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                {n.label}
              </span>
              <RunCountsStrip counts={n.counts} />
            </button>
          ))}
          {overflow > 0 ? (
            <button type="button" onClick={navToDetail} style={{ ...suiteRowStyle, color: "var(--text-muted)", paddingLeft: 0 }}>
              {overflow} more…
            </button>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

/**
 * Full merged test tree + coverage — used by the effort-coverage detail page.
 * Expects already-loaded observations; the page handles the data-fetch lifecycle.
 */
export function FullCoverageView({
  effortId,
  obs,
  onOpenFile,
}: {
  effortId: string;
  obs: EffortObservation[];
  onOpenFile?: (path: string) => void;
}) {
  const coverage = obs.find((o) => o.kind === "diff-coverage");
  const runs = obs.filter((o) => o.kind === "test-run");
  const analysis = obs.find((o) => o.kind === "static-analysis");

  // Detail view shows ALL runs: the header timeline and every per-node strip.
  const runIterations = clusterTestRuns(runs);
  const tree = buildMultiRunTree(runs);
  const mutedStyle: React.CSSProperties = { fontSize: "var(--text-xs)", color: "var(--text-muted)" };

  return (
    <div
      data-testid={`effort-coverage-full-${effortId}`}
      style={{ display: "flex", flexDirection: "column", gap: 16 }}
    >
      <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
        <h4>Coverage</h4>
        {coverage ? (
          <CoverageSummary obs={coverage} onOpenFile={onOpenFile} />
        ) : (
          <span style={mutedStyle}>No coverage recorded for this effort.</span>
        )}
      </div>
      <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
        {/* All runs listed at the top — run count + each run's pass/fail. */}
        <div style={{ display: "flex", alignItems: "baseline", gap: 8, flexWrap: "wrap" }}>
          <h4 style={{ margin: 0 }}>Tests</h4>
          <span style={{ fontSize: "var(--text-xs)", color: "var(--text-muted)" }}>{runsLabel(runIterations.length)}</span>
          {runIterations.length > 0 ? <RunTimeline iterations={runIterations} /> : null}
        </div>
        {tree.length > 0 ? (
          <div style={{ display: "flex", flexDirection: "column" }}>
            {tree.map((n) => (
              <MultiRunTreeNode key={n.path} node={n} depth={0} />
            ))}
          </div>
        ) : runs.length > 0 ? (
          <span style={mutedStyle}>No parsed test report — exit-code only.</span>
        ) : (
          <span style={mutedStyle}>No tests run.</span>
        )}
      </div>
      <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
        <h4 style={{ margin: 0 }}>Static analysis</h4>
        {analysis ? (
          <StaticAnalysisSummary obs={analysis} onOpenFile={onOpenFile} maxFiles={1000} />
        ) : (
          <span style={mutedStyle}>No analyzer run.</span>
        )}
      </div>
    </div>
  );
}

/**
 * Coverage + tests for a SINGLE effort, rendered inside that effort's
 * Activity section: a red/green coverage bar over the changed lines, a
 * ranked "most untested changes" list, and the test runs that fed it.
 * Renders nothing until the effort has at least one observation
 * (collection is opt-in via `/oxplow:configure`), so untracked efforts
 * stay uncluttered. When it does render, the tests portion always says
 * something — the run tree, or an explicit "No tests run."
 *
 * The collapsed "Agent nudges" debug sub-view renders alongside it
 * (independently — a nudge can fire with no observation behind it),
 * self-hiding when the effort has no fired nudges.
 */
export function EffortObservationsBlock({
  effortId,
  onOpenFile,
}: {
  effortId: string;
  onOpenFile?: (path: string) => void;
}) {
  const [obs, setObs] = useState<EffortObservation[]>([]);

  useEffect(() => {
    let cancelled = false;
    const load = () => {
      void listEffortObservations(effortId).then((rows) => {
        if (!cancelled) setObs(rows);
      });
    };
    load();
    const unsub = subscribeOxplowEvents((event) => {
      if (event.kind !== "effortObservationsChanged") return;
      load();
    });
    return () => {
      cancelled = true;
      unsub();
    };
  }, [effortId]);

  const coverage = obs.find((o) => o.kind === "diff-coverage");
  const runs = obs.filter((o) => o.kind === "test-run");
  const analysis = obs.find((o) => o.kind === "static-analysis");
  const mutedStyle: React.CSSProperties = {
    fontSize: "var(--text-xs)",
    color: "var(--text-muted)",
  };

  return (
    <>
      {obs.length > 0 ? (
        <div
          data-testid={`effort-observations-${effortId}`}
          style={{ display: "flex", flexDirection: "column", gap: 8 }}
        >
          <h2 className="task-activity-heading">Coverage and Tests</h2>
          {coverage ? (
            <CoverageSummary obs={coverage} onOpenFile={onOpenFile} />
          ) : (
            <span style={mutedStyle}>
              No coverage recorded for this effort — run the configured coverage command.
            </span>
          )}
          {/* Always rendered — TestsRun calls out the zero-runs case itself. */}
          <TestsRun effortId={effortId} runs={runs} />
          {/* Static analysis renders only when an analyzer ran for this effort,
              keeping untracked efforts uncluttered. */}
          {analysis ? <StaticAnalysisSummary obs={analysis} onOpenFile={onOpenFile} maxFiles={8} /> : null}
        </div>
      ) : null}
      <EffortTokenUsageBlock effortId={effortId} />
    </>
  );
}

/** Human label for a nudge kind. Open-ended — unknown kinds fall back to
 *  the raw kind string. `commit-hygiene` no longer fires (tsk250) but keeps
 *  its label so rows already in a project's history still read properly. */
const NUDGE_KIND_LABEL: Record<string, string> = {
  "report-less-run": "Report-less run",
  "commit-hygiene": "Commit hygiene",
  configure: "Configure",
};

function nudgeRelative(iso: string): string {
  const ms = Date.now() - Date.parse(iso);
  if (!Number.isFinite(ms) || ms < 0) return iso;
  const m = Math.round(ms / 60_000);
  if (m < 1) return "just now";
  if (m < 60) return `${m}m ago`;
  const h = Math.round(m / 60);
  if (h < 24) return `${h}h ago`;
  const d = Math.round(h / 24);
  return `${d}d ago`;
}

/**
 * The agent nudges oxplow fired for this effort — the human/reviewer-facing
 * record of "what oxplow told the agent." Rendered as a full H3 section
 * (matching the other effort sub-sections), self-hiding when no nudge fired.
 * Live-updates on `agentNudgesChanged` for this effort. See
 * `.context/agent-model.md`.
 */
export function AgentNudgesBlock({ effortId }: { effortId: string }) {
  const [nudges, setNudges] = useState<AgentNudge[]>([]);

  useEffect(() => {
    let cancelled = false;
    const load = () => {
      void listNudgesForEffort(effortId).then((rows) => {
        if (!cancelled) setNudges(rows);
      });
    };
    load();
    const unsub = subscribeOxplowEvents((event) => {
      if (event.kind !== "agentNudgesChanged") return;
      if (event.effortId !== effortId) return;
      load();
    });
    return () => {
      cancelled = true;
      unsub();
    };
  }, [effortId]);

  if (nudges.length === 0) return null;

  return (
    <div
      data-testid={`effort-nudges-${effortId}`}
      style={{ display: "flex", flexDirection: "column", gap: 8 }}
    >
      <h2 className="task-activity-heading">Agent Nudges</h2>
      <div style={{ display: "flex", flexDirection: "column", gap: 16 }}>
        {nudges.map((n) => (
          <div
            key={n.id}
            data-testid={`nudge-row-${n.id}`}
            style={{ display: "flex", flexDirection: "column", gap: 6 }}
          >
            <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
              <span
                style={{
                  fontFamily: "var(--font-mono)",
                  fontSize: "var(--text-xs)",
                  padding: "1px 6px",
                  borderRadius: 4,
                  background: "var(--surface-elevated)",
                  color: "var(--text-secondary)",
                }}
              >
                {NUDGE_KIND_LABEL[n.kind] ?? n.kind}
              </span>
              <span style={{ fontSize: "var(--text-xs)", color: "var(--text-muted)" }}>
                {nudgeRelative(n.created_at)}
              </span>
            </div>
            <p
              style={{
                margin: 0,
                fontSize: "var(--text-base)",
                lineHeight: "var(--leading-prose)",
                color: "var(--text-primary)",
              }}
            >
              {n.message}
            </p>
            {n.trigger ? (
              <pre
                style={{
                  margin: 0,
                  fontFamily: "var(--font-mono)",
                  fontSize: "var(--text-sm)",
                  color: "var(--text-secondary)",
                  background: "var(--surface-app)",
                  border: "1px solid var(--border-subtle)",
                  borderRadius: 4,
                  padding: "6px 8px",
                  whiteSpace: "pre-wrap",
                  overflowWrap: "anywhere",
                }}
              >
                {n.trigger}
              </pre>
            ) : null}
          </div>
        ))}
      </div>
    </div>
  );
}
