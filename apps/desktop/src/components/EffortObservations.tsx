import { useEffect, useState } from "react";

import {
  type EffortObservation,
  listEffortObservations,
  subscribeOxplowEvents,
} from "../api.js";

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


function parsePayload<T>(json: string | null): T | null {
  if (!json) return null;
  try {
    return JSON.parse(json) as T;
  } catch {
    return null;
  }
}

/** Emerald ≥80%, amber ≥50%, rose below — reusing the freshness ramp. */
function coverageColor(pct: number): string {
  if (pct >= 80) return "var(--freshness-fresh)";
  if (pct >= 50) return "var(--freshness-stale)";
  return "var(--freshness-very-stale)";
}

function ProvenanceTag({ provenance }: { provenance: string }) {
  // Provenance is the spine: an agent-`asserted` number must never read
  // as a measured one. `observed` is the trusted, quiet default.
  const asserted = provenance === "asserted";
  return (
    <span
      title={
        asserted
          ? "Reported by the agent — not independently measured"
          : "Measured directly by oxplow"
      }
      style={{
        fontSize: "var(--text-xs)",
        padding: "1px 6px",
        borderRadius: 4,
        whiteSpace: "nowrap",
        color: asserted ? "var(--freshness-stale)" : "var(--text-muted)",
        border: `1px solid ${asserted ? "var(--freshness-stale)" : "var(--border-subtle)"}`,
      }}
    >
      {asserted ? "agent-asserted" : "measured"}
    </span>
  );
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
        <ProvenanceTag provenance={obs.provenance} />
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

interface TreeNode {
  label: string;
  path: string;
  children: TreeNode[];
  leaves: { name: string; status: TestStatus; timeMs?: number }[];
  counts: { passed: number; failed: number; skipped: number };
}

/** Build a tree per suite by splitting each case's `classname` on `::`/`.`
 *  — the Rust module path / pytest file·class / jest describe path. */
function buildTestTree(suites: JUnitSuite[]): TreeNode[] {
  interface Mut {
    label: string;
    path: string;
    childMap: Map<string, Mut>;
    leaves: { name: string; status: TestStatus; timeMs?: number }[];
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
      node.leaves.push({ name: leafName, status: c.status, timeMs: c.timeMs });
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
        <span style={{ width: 10, color: "var(--text-muted)" }}>{open ? "▾" : "▸"}</span>
        <span style={{ fontFamily: "var(--font-mono)", flex: "0 1 auto" }}>{node.label}</span>
        <CountsSummary counts={node.counts} />
      </button>
      {open ? (
        <div>
          {node.children.map((c) => (
            <TestTreeNode key={c.path} node={c} depth={depth + 1} />
          ))}
          {node.leaves.map((leaf) => (
            <div
              key={leaf.name}
              data-testid={`test-case-${leaf.name}`}
              style={{
                display: "flex",
                alignItems: "center",
                gap: 6,
                paddingLeft: (depth + 1) * 12 + 16,
                fontSize: "var(--text-xs)",
              }}
            >
              <span style={{ color: statusColor(leaf.status) }}>{statusGlyph(leaf.status)}</span>
              <span style={{ fontFamily: "var(--font-mono)", color: "var(--text-primary)" }}>{leaf.name}</span>
              {leaf.timeMs !== undefined && leaf.timeMs > 0 ? (
                <span className="oxplow-tabular" style={{ color: "var(--text-muted)" }}>
                  {(leaf.timeMs / 1000).toFixed(2)}s
                </span>
              ) : null}
            </div>
          ))}
        </div>
      ) : null}
    </div>
  );
}

/** Merge all test-run observations into one suite list, last-write-wins per
 *  test case (keyed by `classname::name`). Observations are processed in
 *  storage order (oldest first), so later runs update the status of a case
 *  that ran earlier. Suites that never appeared together are unioned. */
function mergeTestRuns(runs: EffortObservation[]): JUnitSuite[] {
  const suiteOrder: string[] = [];
  const suiteMap = new Map<string, Map<string, JUnitCase>>();

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
        cases.set(`${c.classname}::${c.name}`, c);
      }
    }
  }

  return suiteOrder.map((sname) => ({
    name: sname,
    cases: [...suiteMap.get(sname)!.values()],
  }));
}

function TestsRun({ runs }: { runs: EffortObservation[] }) {
  if (runs.length === 0) return null;

  const merged = mergeTestRuns(runs);
  const tree = merged.some((s) => s.cases.length > 0) ? buildTestTree(merged) : null;

  // Aggregate counts from the merged (deduplicated) tree.
  const totals = tree
    ? tree.reduce(
        (acc, n) => ({
          passed: acc.passed + n.counts.passed,
          failed: acc.failed + n.counts.failed,
          skipped: acc.skipped + n.counts.skipped,
        }),
        { passed: 0, failed: 0, skipped: 0 },
      )
    : null;
  const total = totals ? totals.passed + totals.failed + totals.skipped : null;

  // Fall back to the last run's raw counts when no parsed suite data exists.
  const lastRun = parsePayload<TestRunPayload>(runs[runs.length - 1].payload_json);
  const fallbackCounts =
    !totals && lastRun?.total !== undefined
      ? `${lastRun.passed ?? 0}/${lastRun.total} passed`
      : null;

  const lastObs = runs[runs.length - 1];

  return (
    <div data-testid="tests-run" style={{ display: "flex", flexDirection: "column", gap: 4 }}>
      <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
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
        {totals && total ? (
          <span style={{ fontSize: "var(--text-sm)", color: "var(--text-secondary)" }}>
            {totals.passed}/{total} passed
          </span>
        ) : fallbackCounts ? (
          <span style={{ fontSize: "var(--text-sm)", color: "var(--text-secondary)" }}>
            {fallbackCounts}
          </span>
        ) : null}
        <ProvenanceTag provenance={lastObs.provenance} />
      </div>
      {tree ? (
        <div style={{ display: "flex", flexDirection: "column", paddingLeft: 8 }}>
          {tree.map((n) => (
            <TestTreeNode key={n.path} node={n} depth={0} />
          ))}
        </div>
      ) : null}
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

  if (obs.length === 0) return null;

  const coverage = obs.find((o) => o.kind === "diff-coverage");
  const runs = obs.filter((o) => o.kind === "test-run");
  const mutedStyle: React.CSSProperties = {
    fontSize: "var(--text-xs)",
    color: "var(--text-muted)",
  };

  return (
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
      {runs.length > 0 ? <TestsRun runs={runs} /> : <span style={mutedStyle}>No tests run.</span>}
    </div>
  );
}
