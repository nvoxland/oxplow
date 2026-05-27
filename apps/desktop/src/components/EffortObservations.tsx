import { useEffect, useState } from "react";

import {
  type EffortDetail,
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

interface TestRunPayload {
  command: string;
  exitCode?: number;
  passed?: number;
  failed?: number;
  total?: number;
  durationMs?: number;
}

const TESTS_VISIBLE = 6;

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

function TestRunRow({ obs }: { obs: EffortObservation }) {
  const run = parsePayload<TestRunPayload>(obs.payload_json);
  if (!run) return null;
  const ok = run.exitCode === undefined ? null : run.exitCode === 0;
  const counts =
    run.total !== undefined
      ? `${run.passed ?? 0}/${run.total} passed`
      : run.failed
        ? `${run.failed} failed`
        : null;
  // Commands can be long / multi-line (heredocs). Collapse whitespace and
  // ellipsize to one line; the full text is on the title.
  const oneLine = run.command.replace(/\s+/g, " ").trim();
  const duration = run.durationMs !== undefined ? `${(run.durationMs / 1000).toFixed(1)}s` : null;
  return (
    <div data-testid="test-run-row" style={{ display: "flex", alignItems: "center", gap: 8, fontSize: "var(--text-sm)" }}>
      <span
        title={ok === null ? "exit code unknown" : ok ? "passed" : "failed"}
        style={{
          flex: "0 0 auto",
          color: ok === null ? "var(--text-muted)" : ok ? "var(--freshness-fresh)" : "var(--freshness-very-stale)",
        }}
      >
        {ok === null ? "•" : ok ? "✓" : "✗"}
      </span>
      <code
        title={run.command}
        style={{
          flex: "1 1 auto",
          minWidth: 0,
          overflow: "hidden",
          textOverflow: "ellipsis",
          whiteSpace: "nowrap",
          fontFamily: "var(--font-mono)",
          fontSize: "var(--text-xs)",
          color: "var(--text-primary)",
        }}
      >
        {oneLine}
      </code>
      {counts ? <span style={{ flex: "0 0 auto", color: "var(--text-secondary)" }}>{counts}</span> : null}
      {duration ? (
        <span className="oxplow-tabular" style={{ flex: "0 0 auto", color: "var(--text-muted)" }}>
          {duration}
        </span>
      ) : null}
      <span style={{ flex: "0 0 auto" }}>
        <ProvenanceTag provenance={obs.provenance} />
      </span>
    </div>
  );
}

function TestsRun({ runs }: { runs: EffortObservation[] }) {
  const [expanded, setExpanded] = useState(false);
  if (runs.length === 0) return null;
  const passed = runs.filter((r) => parsePayload<TestRunPayload>(r.payload_json)?.exitCode === 0).length;
  const shown = expanded ? runs : runs.slice(0, TESTS_VISIBLE);
  const hidden = runs.length - shown.length;
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
      <span style={{ fontSize: "var(--text-xs)", color: "var(--text-muted)", textTransform: "uppercase", letterSpacing: "0.04em" }}>
        Tests run ({runs.length}
        {runs.some((r) => parsePayload<TestRunPayload>(r.payload_json)?.exitCode !== undefined)
          ? ` · ${passed} passing`
          : ""}
        )
      </span>
      {shown.map((r) => (
        <TestRunRow key={r.id} obs={r} />
      ))}
      {hidden > 0 ? (
        <button
          type="button"
          onClick={() => setExpanded(true)}
          style={{
            alignSelf: "flex-start",
            background: "transparent",
            border: "none",
            padding: 0,
            cursor: "pointer",
            fontSize: "var(--text-xs)",
            color: "var(--accent)",
          }}
        >
          + {hidden} earlier run{hidden === 1 ? "" : "s"}
        </button>
      ) : null}
    </div>
  );
}

/**
 * Effort-review surface for the collection subsystem: per effort, a
 * red/green coverage bar over the changed lines, a ranked "most untested
 * changes" list, and the test runs that fed it. Renders nothing until at
 * least one observation exists (collection is opt-in via
 * `/oxplow:configure`), so untracked tasks stay uncluttered.
 */
export function EffortObservations({
  efforts,
  onOpenFile,
}: {
  efforts: EffortDetail[];
  onOpenFile?: (path: string) => void;
}) {
  const [byEffort, setByEffort] = useState<Map<string, EffortObservation[]>>(new Map());
  const effortIds = efforts.map((e) => e.effort.id).join(",");

  useEffect(() => {
    let cancelled = false;
    const load = () => {
      void Promise.all(
        efforts.map(
          async (e) => [e.effort.id, await listEffortObservations(e.effort.id)] as const,
        ),
      ).then((pairs) => {
        if (!cancelled) setByEffort(new Map(pairs));
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
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [effortIds]);

  const rendered = efforts
    .map((e, i) => ({ effort: e, index: i, obs: byEffort.get(e.effort.id) ?? [] }))
    .filter((row) => row.obs.length > 0);

  if (rendered.length === 0) return null;

  return (
    <section data-testid="effort-observations">
      <h2 className="task-activity-heading">Coverage &amp; Tests</h2>
      <div style={{ display: "flex", flexDirection: "column", gap: 16 }}>
        {rendered.map(({ effort, index, obs }) => {
          const coverage = obs.find((o) => o.kind === "diff-coverage");
          const runs = obs.filter((o) => o.kind === "test-run");
          return (
            <div
              key={effort.effort.id}
              data-testid={`effort-observations-${effort.effort.id}`}
              style={{
                display: "flex",
                flexDirection: "column",
                gap: 12,
                padding: "12px 14px",
                background: "var(--surface-card)",
                border: "1px solid var(--border-subtle)",
                borderRadius: 6,
              }}
            >
              {rendered.length > 1 ? (
                <span style={{ fontSize: "var(--text-xs)", color: "var(--text-muted)" }}>
                  Effort {index + 1}
                </span>
              ) : null}
              {coverage ? (
                <CoverageSummary obs={coverage} onOpenFile={onOpenFile} />
              ) : (
                <span style={{ fontSize: "var(--text-xs)", color: "var(--text-muted)" }}>
                  No coverage recorded for this effort yet — run the configured coverage command.
                </span>
              )}
              <TestsRun runs={runs} />
            </div>
          );
        })}
      </div>
    </section>
  );
}
