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
  /** Set by mergeTestRuns when this case had status "failed" in any earlier run. */
  everFailed?: boolean;
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

interface TreeNode {
  label: string;
  path: string;
  children: TreeNode[];
  leaves: { name: string; status: TestStatus; timeMs?: number; everFailed?: boolean }[];
  counts: { passed: number; failed: number; skipped: number };
}

/** Build a tree per suite by splitting each case's `classname` on `::`/`.`
 *  — the Rust module path / pytest file·class / jest describe path. */
function buildTestTree(suites: JUnitSuite[]): TreeNode[] {
  interface Mut {
    label: string;
    path: string;
    childMap: Map<string, Mut>;
    leaves: { name: string; status: TestStatus; timeMs?: number; everFailed?: boolean }[];
  }
  const mut = (label: string, path: string): Mut => ({ label, path, childMap: new Map(), leaves: [] });
  const finalize = (m: Mut): TreeNode => {
    const children = [...m.childMap.values()].map(finalize);
    const counts = { passed: 0, failed: 0, skipped: 0 };
    for (const leaf of m.leaves) counts[leaf.status]++;
    for (const c of children) {
      counts.passed += c.counts.passed;
      counts.failed += c.counts.failed;
      counts.skipped += c.counts.skipped;
    }
    return { label: m.label, path: m.path, children, leaves: m.leaves, counts };
  };
  return suites.map((suite) => {
    const suiteName = suite.name || "(tests)";
    const root = mut(suiteName, suiteName);
    for (const c of suite.cases) {
      // The grouping path differs by tech: nextest puts the module path in
      // `name` (classname = crate); pytest/jest put it in `classname`. Use
      // both, split on `::`/`.`, drop a leading segment that just repeats
      // the suite, and collapse consecutive dupes — the last segment is the
      // test, the rest is the natural module / describe tree.
      let segs = [...c.classname.split(/::|\./), ...c.name.split(/::|\./)]
        .map((s) => s.trim())
        .filter(Boolean)
        .filter((s, i, a) => i === 0 || s !== a[i - 1]);
      if (segs.length > 1 && segs[0] === suiteName) segs = segs.slice(1);
      const leafName = segs.pop() ?? c.name;
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
      node.leaves.push({ name: leafName, status: c.status, timeMs: c.timeMs, everFailed: c.everFailed });
    }
    return finalize(root);
  });
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

/** Compact "12✓ 1✗ 2⊘" rollup, omitting zeros. */
function CountsSummary({ counts }: { counts: TreeNode["counts"] }) {
  const parts: Array<[number, TestStatus]> = [
    [counts.passed, "passed"],
    [counts.failed, "failed"],
    [counts.skipped, "skipped"],
  ];
  return (
    <span style={{ display: "inline-flex", gap: 6, fontSize: "var(--text-xs)" }}>
      {parts
        .filter(([n]) => n > 0)
        .map(([n, s]) => (
          <span key={s} style={{ color: statusColor(s) }}>
            {n}
            {statusGlyph(s)}
          </span>
        ))}
    </span>
  );
}

function TestTreeNode({ node, depth }: { node: TreeNode; depth: number }) {
  // Auto-expand branches that contain a failure so failures are visible;
  // all-passing branches start collapsed.
  const [open, setOpen] = useState(node.counts.failed > 0);
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
        <CountsSummary counts={node.counts} />
      </button>
      {open ? (
        <div>
          {node.children.map((c) => (
            <TestTreeNode key={c.path} node={c} depth={depth + 1} />
          ))}
          {node.leaves.map((leaf) => {
            const onceFailed = leaf.everFailed && leaf.status !== "failed";
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
                <span style={{ color: statusColor(leaf.status) }}>{statusGlyph(leaf.status)}</span>
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

/** Merge all test-run observations into one suite list, last-write-wins per
 *  test case (keyed by `classname::name`). Observations are processed in
 *  storage order (oldest first), so later runs update the status of a case
 *  that ran earlier. Suites that never appeared together are unioned.
 *  Cases that had status "failed" in any run carry everFailed=true even if
 *  the final status is passing. */
function mergeTestRuns(runs: EffortObservation[]): JUnitSuite[] {
  const suiteOrder: string[] = [];
  const suiteMap = new Map<string, Map<string, JUnitCase>>();
  const everFailedKeys = new Set<string>();

  for (const obs of runs) {
    const run = parsePayload<TestRunPayload>(obs.payload_json);
    if (!run?.suites) continue;
    for (const suite of run.suites) {
      const sname = suite.name || "(tests)";
      if (!suiteMap.has(sname)) {
        suiteMap.set(sname, new Map());
        suiteOrder.push(sname);
      }
      const cases = suiteMap.get(sname)!;
      for (const c of suite.cases) {
        const key = `${sname}::${c.classname}::${c.name}`;
        if (c.status === "failed") everFailedKeys.add(key);
        cases.set(`${c.classname}::${c.name}`, c);
      }
    }
  }

  return suiteOrder.map((sname) => ({
    name: sname,
    cases: [...suiteMap.get(sname)!.values()].map((c) => ({
      ...c,
      everFailed: everFailedKeys.has(`${sname}::${c.classname}::${c.name}`),
    })),
  }));
}

/** Aggregate totals across a merged tree. */
function sumCounts(tree: TreeNode[]): { passed: number; failed: number; skipped: number } {
  return tree.reduce(
    (acc, n) => ({
      passed: acc.passed + n.counts.passed,
      failed: acc.failed + n.counts.failed,
      skipped: acc.skipped + n.counts.skipped,
    }),
    { passed: 0, failed: 0, skipped: 0 },
  );
}

/** Compact summary: pass/fail totals + top-5 groups + Details link. */
/** One logical test pass within an effort: summed pass/fail at a point in time. */
export interface TestIteration {
  at: string;
  passed: number;
  failed: number;
}

/** Cluster the effort's test-run observations into iterations by TIME. Test
 *  samples carry no snapshot/git stamp, so runs within ~90s are treated as one
 *  logical pass (e.g. the Rust + frontend stacks of a single `test:collect`); a
 *  larger gap starts a new iteration. Oldest-first, so a TDD red→green
 *  progression reads left→right. */
export function clusterTestRuns(runs: EffortObservation[]): TestIteration[] {
  const GAP_MS = 90_000;
  const sorted = [...runs].sort((a, b) =>
    a.created_at < b.created_at ? -1 : a.created_at > b.created_at ? 1 : 0,
  );
  const iters: TestIteration[] = [];
  let lastT = Number.NEGATIVE_INFINITY;
  for (const obs of sorted) {
    const p = parsePayload<TestRunPayload>(obs.payload_json);
    const passed = p?.passed ?? 0;
    const failed = p?.failed ?? 0;
    const t = Date.parse(String(obs.created_at));
    if (iters.length === 0 || (Number.isFinite(t) && t - lastT > GAP_MS)) {
      iters.push({ at: obs.created_at, passed, failed });
    } else {
      const cur = iters[iters.length - 1];
      cur.passed += passed;
      cur.failed += failed;
    }
    if (Number.isFinite(t)) lastT = t;
  }
  return iters;
}

function TestsRun({ effortId, runs }: { effortId: string; runs: EffortObservation[] }) {
  if (runs.length === 0) return null;
  const ctxNav = useOptionalPageNavigation();

  const merged = mergeTestRuns(runs);
  const tree = merged.some((s) => s.cases.length > 0) ? buildTestTree(merged) : null;
  const totals = tree ? sumCounts(tree) : null;
  // Per-iteration timeline (TDD visibility) — only meaningful with ≥2 passes.
  const iterations = clusterTestRuns(runs);

  // Fall back to raw counts from the last run when no suite data exists.
  const lastRunPayload = parsePayload<TestRunPayload>(runs[runs.length - 1].payload_json);
  const fallbackPassed = !totals && lastRunPayload?.total !== undefined ? (lastRunPayload.passed ?? 0) : null;
  const fallbackFailed = !totals && lastRunPayload?.failed !== undefined ? lastRunPayload.failed : null;

  // Top 5 suites by total case count, descending.
  const sorted = tree
    ? [...tree].sort((a, b) => {
        const ta = a.counts.passed + a.counts.failed + a.counts.skipped;
        const tb = b.counts.passed + b.counts.failed + b.counts.skipped;
        return tb - ta;
      })
    : [];
  const top5 = sorted.slice(0, 5);
  const overflow = sorted.length - top5.length;

  const navToDetail = ctxNav ? () => ctxNav.navigate(effortCoverageRef(effortId)) : undefined;
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
      {/* Header row: label + pass/fail + details link */}
      <div style={{ display: "flex", alignItems: "center", gap: 8, flexWrap: "wrap" }}>
        <span
          style={{
            fontSize: "var(--text-xs)",
            color: "var(--text-muted)",
            textTransform: "uppercase",
            letterSpacing: "0.04em",
          }}
        >
          Tests run
        </span>
        {totals ? (
          <>
            <span style={{ fontSize: "var(--text-sm)", fontWeight: "var(--weight-medium)", color: "var(--freshness-fresh)" }}>
              {totals.passed} passed
            </span>
            {totals.failed > 0 ? (
              <span style={{ fontSize: "var(--text-sm)", fontWeight: "var(--weight-medium)", color: "var(--freshness-very-stale)" }}>
                {totals.failed} failed
              </span>
            ) : null}
          </>
        ) : fallbackPassed !== null ? (
          <>
            <span style={{ fontSize: "var(--text-sm)", fontWeight: "var(--weight-medium)", color: "var(--freshness-fresh)" }}>
              {fallbackPassed} passed
            </span>
            {fallbackFailed ? (
              <span style={{ fontSize: "var(--text-sm)", fontWeight: "var(--weight-medium)", color: "var(--freshness-very-stale)" }}>
                {fallbackFailed} failed
              </span>
            ) : null}
          </>
        ) : null}
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
      {/* Per-iteration timeline: each test pass over time, so a TDD red→green
          progression is visible. Hidden for a single pass (redundant header). */}
      {iterations.length >= 2 ? (
        <div
          data-testid="tests-iterations"
          style={{
            display: "flex",
            alignItems: "baseline",
            gap: 6,
            flexWrap: "wrap",
            fontSize: "var(--text-xs)",
            paddingLeft: 4,
          }}
        >
          <span
            style={{
              color: "var(--text-muted)",
              textTransform: "uppercase",
              letterSpacing: "0.04em",
            }}
          >
            {iterations.length} runs
          </span>
          {iterations.map((it, i) => (
            <Fragment key={`${it.at}-${i}`}>
              {i > 0 ? <span style={{ color: "var(--text-muted)" }}>→</span> : null}
              <span
                title={new Date(it.at).toLocaleString()}
                style={{ display: "inline-flex", gap: 4, alignItems: "baseline" }}
              >
                <span style={{ color: "var(--freshness-fresh)" }}>{it.passed}✓</span>
                {it.failed > 0 ? (
                  <span style={{ color: "var(--freshness-very-stale)" }}>{it.failed}✗</span>
                ) : null}
              </span>
            </Fragment>
          ))}
        </div>
      ) : null}
      {/* Top 5 suites — each row navigates to the detail page */}
      {top5.length > 0 ? (
        <div style={{ display: "flex", flexDirection: "column", gap: 2, paddingLeft: 4 }}>
          {top5.map((n) => (
            <button key={n.path} type="button" onClick={navToDetail} style={suiteRowStyle}>
              <span style={{ fontFamily: "var(--font-mono)", color: "var(--text-secondary)", flex: "1 1 auto", minWidth: 0, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                {n.label}
              </span>
              <CountsSummary counts={n.counts} />
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

  const merged = mergeTestRuns(runs);
  const tree = merged.some((s) => s.cases.length > 0) ? buildTestTree(merged) : null;
  const totals = tree ? sumCounts(tree) : null;
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
        <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
          <h4 style={{ margin: 0 }}>Tests</h4>
          {totals ? (
            <>
              <span style={{ fontSize: "var(--text-sm)", fontWeight: "var(--weight-medium)", color: "var(--freshness-fresh)" }}>
                {totals.passed} passed
              </span>
              {totals.failed > 0 ? (
                <span style={{ fontSize: "var(--text-sm)", fontWeight: "var(--weight-medium)", color: "var(--freshness-very-stale)" }}>
                  {totals.failed} failed
                </span>
              ) : null}
            </>
          ) : null}
        </div>
        {tree ? (
          <div style={{ display: "flex", flexDirection: "column" }}>
            {tree.map((n) => (
              <TestTreeNode key={n.path} node={n} depth={0} />
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
 * (independently — a commit-hygiene nudge can fire with no observation),
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
          <h4>Coverage &amp; tests</h4>
          {coverage ? (
            <CoverageSummary obs={coverage} onOpenFile={onOpenFile} />
          ) : (
            <span style={mutedStyle}>
              No coverage recorded for this effort — run the configured coverage command.
            </span>
          )}
          {runs.length > 0 ? <TestsRun effortId={effortId} runs={runs} /> : <span style={mutedStyle}>No tests run.</span>}
          {/* Static analysis renders only when an analyzer ran for this effort,
              keeping untracked efforts uncluttered. */}
          {analysis ? <StaticAnalysisSummary obs={analysis} onOpenFile={onOpenFile} maxFiles={8} /> : null}
        </div>
      ) : null}
      <EffortTokenUsageBlock effortId={effortId} />
      <AgentNudgesBlock effortId={effortId} />
    </>
  );
}

/** Human label for a nudge kind. Open-ended — unknown kinds fall back to
 *  the raw kind string. */
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
 * Collapsed debug sub-view of the agent nudges oxplow fired for this effort
 * — the human/reviewer-facing record of "what oxplow told the agent." Low-key
 * by design (a `<details>` collapsed by default, native disclosure keyboard
 * behaviour), and self-hiding when no nudge fired. Live-updates on
 * `agentNudgesChanged` for this effort. See `.context/agent-model.md`.
 */
function AgentNudgesBlock({ effortId }: { effortId: string }) {
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

  const mutedStyle: React.CSSProperties = {
    fontSize: "var(--text-xs)",
    color: "var(--text-muted)",
  };

  return (
    <details
      data-testid={`effort-nudges-${effortId}`}
      style={{ marginTop: 4, fontSize: "var(--text-xs)" }}
    >
      <summary
        style={{ cursor: "pointer", color: "var(--text-muted)", userSelect: "none" }}
      >
        Agent nudges ({nudges.length})
      </summary>
      <div style={{ display: "flex", flexDirection: "column", gap: 6, marginTop: 6 }}>
        {nudges.map((n) => (
          <div
            key={n.id}
            data-testid={`nudge-row-${n.id}`}
            style={{ display: "flex", flexDirection: "column", gap: 2 }}
          >
            <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
              <span
                style={{
                  fontFamily: "var(--font-mono)",
                  fontSize: "var(--text-xxs, 10px)",
                  padding: "0 4px",
                  borderRadius: 3,
                  background: "var(--bg-tier-2, rgba(127,127,127,0.15))",
                  color: "var(--text-secondary, var(--text-muted))",
                }}
              >
                {NUDGE_KIND_LABEL[n.kind] ?? n.kind}
              </span>
              <span style={mutedStyle}>{nudgeRelative(n.created_at)}</span>
            </div>
            <span style={{ color: "var(--text-secondary, var(--text-primary))" }}>
              {n.message}
            </span>
            {n.trigger ? (
              <span style={{ ...mutedStyle, fontFamily: "var(--font-mono)" }}>
                {n.trigger}
              </span>
            ) : null}
          </div>
        ))}
      </div>
    </details>
  );
}
