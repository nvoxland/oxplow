import { useCallback, useEffect, useState, type CSSProperties } from "react";
import { Page } from "../tabs/Page.js";
import type { TabRef } from "../tabs/tabState.js";
import { lensRef } from "../tabs/pageRefs.js";
import type { Stream } from "../tauri-bridge/index.js";
import { keepLensSpec, querySql, type LensRun, type LensViz } from "../api.js";
import { LensResultView } from "../lens/LensResultView.js";
import { ModelLineage } from "./ModelLineage.js";
import { PinToDashboard } from "../components/Dashboard/PinToDashboard.js";
import { adHocLens, NEW_LENS_PROMPT, slugify } from "../lens/lensModel.js";
import { NO_READS, useRerunOnChange } from "../lens/lensRerun.js";
import { insertIntoAgent } from "../agent-input-bus.js";
import { recordOpError } from "../components/opErrorsStore.js";
import { useRequestGuard } from "../request-guard.js";
import type { LensChart, Reads, SqlQueryResult } from "../tauri-bridge/generated/bindings.js";
import {
  chartDefaults,
  DIMENSIONS_SQL,
  metricTemplate,
  sliceTemplate,
  type MetricTemplate,
  keepBlockedReason,
  lineage,
  LINEAGE_SQL,
  MODEL_COLUMNS_SQL,
  MODELS_SQL,
  modelColumns,
  models,
  type Lineage,
  type ModelColumn,
  type ModelRow,
} from "./exploreData.js";

export interface ExploreDataPageProps {
  stream: Stream | null;
  onOpenPage(ref: TabRef): void;
}

const SAMPLE_LIMIT = 50;
const VIZ_OPTIONS: LensViz[] = ["table", "list", "number", "markdown", "bar", "line", "treemap"];
const METRICS_SQL = "SELECT key, title FROM v_metric_catalog ORDER BY title";

/** The columns each chart viz names, and which are optional. */
const CHART_ROLES: Partial<Record<LensViz, { key: keyof LensChart; label: string; optional: boolean }[]>> = {
  bar: [
    { key: "x", label: "X", optional: false },
    { key: "y", label: "Y", optional: false },
  ],
  line: [
    { key: "x", label: "X", optional: false },
    { key: "y", label: "Y", optional: false },
    { key: "series", label: "Slice By", optional: true },
  ],
  treemap: [
    { key: "label", label: "Label", optional: false },
    { key: "size", label: "Size", optional: false },
    { key: "group", label: "Group", optional: true },
  ],
};

/**
 * Explore Data: the semantic layer's catalog (every model in `v_model`,
 * with its documented columns from `v_model_column` and its lineage from
 * `v_model_lineage`), an editable SQL box over it — metrics too, through
 * `metric_grid()` — and "Save as Lens" / "Pin to Dashboard" to keep a
 * query. A person debugging can turn on raw tables (physical tables
 * too, IPC only); nothing read that way can be kept. The core,
 * deliberately simple starting point for people who want to see what data
 * exists before asking an agent for a lens. See
 * `.context/semantic-layer.md` and `.context/extensions.md`.
 */
export function ExploreDataPage({ stream, onOpenPage }: ExploreDataPageProps) {
  const [catalog, setCatalog] = useState<ModelRow[]>([]);
  const [catalogReads, setCatalogReads] = useState<Reads>(NO_READS);
  const [selected, setSelected] = useState<string | null>(null);
  const [columns, setColumns] = useState<ModelColumn[]>([]);
  const [lineageOf, setLineageOf] = useState<Lineage | null>(null);
  const [raw, setRaw] = useState(false);
  // Whether the result on screen was a raw read.
  const [ranRaw, setRanRaw] = useState(false);
  const [sql, setSql] = useState("");
  const [viz, setViz] = useState<LensViz>("table");
  const [chart, setChart] = useState<LensChart | null>(null);
  // The explorer's own metric query (P6.F1): Slice By regenerates it with a
  // dimension. Once the SQL is edited by hand it's free SQL, never rewritten.
  const [template, setTemplate] = useState<MetricTemplate | null>(null);
  const [metricKeys, setMetricKeys] = useState<{ key: string; title: string }[]>([]);
  const [dimensions, setDimensions] = useState<{ key: string; label: string }[]>([]);
  const [run, setRun] = useState<LensRun | null>(null);
  const [error, setError] = useState<string | null>(null);
  const guard = useRequestGuard();

  // The catalog is SQL too, and live: an extension's models appear when
  // they compile.
  const loadCatalog = useCallback(() => {
    querySql(MODELS_SQL, [], null)
      .then((result) => {
        setCatalog(models(result));
        setCatalogReads(result.reads);
      })
      .catch((e) => setError(String(e)));
  }, []);
  useEffect(loadCatalog, [loadCatalog]);
  useEffect(() => {
    Promise.all([querySql(METRICS_SQL, [], null), querySql(DIMENSIONS_SQL, [], null)])
      .then(([m, d]) => {
        setMetricKeys(m.rows.map(([key, title]) => ({ key: String(key), title: String(title ?? key) })));
        setDimensions(d.rows.map(([key, label]) => ({ key: String(key), label: String(label ?? key) })));
      })
      .catch(() => {
        // No metrics or dimensions: the pickers stay empty.
      });
  }, []);
  useRerunOnChange(catalogReads, loadCatalog);

  useEffect(() => {
    if (!selected) return;
    let cancelled = false;
    Promise.all([querySql(MODEL_COLUMNS_SQL, [selected], null), querySql(LINEAGE_SQL, [selected], null)])
      .then(([cols, lin]) => {
        if (cancelled) return;
        setColumns(modelColumns(cols));
        setLineageOf(lineage(selected, lin));
      })
      .catch((e) => setError(String(e)));
    return () => {
      cancelled = true;
    };
  }, [selected]);

  async function execute(
    query: string,
    as: LensViz = viz,
    rawRead: boolean = raw,
    withChart: LensChart | null | undefined = undefined,
  ) {
    // A slower earlier query mustn't land over this one.
    const current = guard.begin();
    setError(null);
    try {
      const result = await querySql(query, [], null, rawRead);
      if (!current()) return;
      // A chart keeps the columns it names while the result still has them.
      const kept = withChart !== undefined ? withChart : chart;
      const next = kept && chartFits(kept, result) ? kept : chartDefaults(as, result);
      setChart(next);
      setRun({ lens: adHocLens(query, as, next), params: {}, result, alert: null, warnings: [], inactive: null });
      setRanRaw(rawRead);
    } catch (e) {
      if (!current()) return;
      setRun(null);
      setError(e instanceof Error ? e.message : String(e));
    }
  }

  // Live like a lens: the query re-runs when what it read changes.
  useRerunOnChange(run?.result.reads ?? NO_READS, () => {
    if (run) void execute(run.lens.query, run.lens.viz, ranRaw, run.lens.chart);
  });
  const blocked = keepBlockedReason(ranRaw);

  function pick(name: string) {
    const q = `SELECT * FROM ${name} LIMIT ${SAMPLE_LIMIT}`;
    setSelected(name);
    setSql(q);
    setViz("table");
    setTemplate(null);
    void execute(q, "table", raw, null);
  }

  function applyTemplate(next: MetricTemplate) {
    setTemplate(next);
    setSql(next.sql);
    setViz(next.viz);
    void execute(next.sql, next.viz, false, next.chart);
  }
  const templateActive = template !== null && sql === template.sql;

  function showAs(next: LensViz) {
    setViz(next);
    if (!run) return;
    const nextChart = chartDefaults(next, run.result);
    setChart(nextChart);
    setRun({ ...run, lens: adHocLens(run.lens.query, next, nextChart) });
  }

  function setRole(key: keyof LensChart, column: string | null) {
    if (!run || !chart) return;
    const nextChart = { ...chart, [key]: column };
    setChart(nextChart);
    setRun({ ...run, lens: adHocLens(run.lens.query, viz, nextChart) });
  }

  const model = catalog.find((m) => m.view === selected) ?? null;

  const catalogList = (
    <ul data-testid="explore-entities" style={{ listStyle: "none", margin: 0, padding: 0 }}>
      {catalog.map((m) => (
        <li key={m.view}>
          <button
            type="button"
            data-testid={`explore-entity-${m.view}`}
            title={m.owner === "core" ? m.description : `${m.description} (${m.owner})`}
            onClick={() => pick(m.view)}
            style={{ ...entityButtonStyle, fontWeight: m.view === selected ? 600 : 400 }}
          >
            {m.view}
          </button>
        </li>
      ))}
    </ul>
  );

  return (
    <Page testId="page-explore-data" title="Explore Data" layout="details" rightRail={catalogList} rightRailTitle="Data">
      <p style={{ color: "var(--text-secondary)", marginTop: 0 }}>
        Everything oxplow knows, as read-only SQL views. Pick one on the right, tweak the query, and save it as a
        lens to keep it as a page. Metrics read as columns of a grid:{" "}
        <code>SELECT bucket, MEASURE('oxplow.coverage.abs_pct') FROM metric_grid('week')</code>. Or{" "}
        <button type="button" data-testid="explore-new-lens" onClick={() => insertIntoAgent(NEW_LENS_PROMPT)}>
          ask your agent to build one…
        </button>
      </p>
      {model ? (
        <details data-testid="explore-columns" style={{ marginBottom: 12 }}>
          <summary style={{ cursor: "pointer" }}>
            <strong>{model.view}</strong> — {model.description}
          </summary>
          <table style={docsTableStyle}>
            <tbody>
              {columns.map((c) => (
                <tr key={c.name}>
                  <td style={docsCellStyle}>
                    <code>{c.name}</code>
                  </td>
                  <td style={{ ...docsCellStyle, color: "var(--text-muted)" }}>{c.sqlType}</td>
                  <td style={docsCellStyle}>{c.doc}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </details>
      ) : null}
      {model && lineageOf ? <ModelLineage lineage={lineageOf} onPick={pick} /> : null}
      <textarea
        data-testid="explore-sql"
        value={sql}
        placeholder="SELECT … FROM v_task …  or  SELECT bucket, MEASURE('…') FROM metric_grid('day')   (Cmd/Ctrl+Enter runs)"
        onChange={(e) => setSql(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
            e.preventDefault();
            void execute(sql);
          }
        }}
        rows={5}
        style={sqlStyle}
      />
      <div style={{ display: "flex", gap: 8, alignItems: "center", margin: "8px 0 16px" }}>
        <button type="button" data-testid="explore-run" disabled={!sql.trim()} onClick={() => void execute(sql)}>
          Run
        </button>
        <label style={{ fontSize: "var(--text-sm)" }}>
          Show as{" "}
          <select
            data-testid="explore-viz"
            value={viz}
            onChange={(e) => showAs(e.target.value as LensViz)}
          >
            {VIZ_OPTIONS.map((v) => (
              <option key={v} value={v}>
                {v}
              </option>
            ))}
          </select>
        </label>
        {metricKeys.length > 0 ? (
          <label style={{ fontSize: "var(--text-sm)" }}>
            Chart a metric{" "}
            <select
              data-testid="explore-metric"
              value={templateActive ? template!.metric : ""}
              onChange={(e) => {
                if (e.target.value) applyTemplate(metricTemplate(e.target.value, null));
              }}
            >
              <option value="">—</option>
              {metricKeys.map((m) => (
                <option key={m.key} value={m.key}>
                  {m.title}
                </option>
              ))}
            </select>
          </label>
        ) : null}
        {templateActive ? (
          <label style={{ fontSize: "var(--text-sm)" }}>
            Slice By{" "}
            <select
              data-testid="explore-slice"
              value={template!.dimension ?? ""}
              onChange={(e) => applyTemplate(sliceTemplate(template!, e.target.value || null))}
            >
              <option value="">(none)</option>
              {dimensions.map((d) => (
                <option key={d.key} value={d.key}>
                  {d.label}
                </option>
              ))}
            </select>
          </label>
        ) : null}
        <label
          data-testid="explore-raw"
          style={rawToggleStyle}
          title="Read physical tables too, for debugging. Nothing read this way can be saved or pinned."
        >
          <input
            type="checkbox"
            data-testid="explore-raw-toggle"
            checked={raw}
            onChange={(e) => setRaw(e.target.checked)}
          />{" "}
          Raw tables
        </label>
        <span style={{ flex: 1 }} />
        {run ? (
          <PinToDashboard
            tile={{
              kind: "query",
              sql: run.lens.query,
              display: viz,
              optionsJson: JSON.stringify(chart ? { size: "wide", chart } : { size: "wide" }),
            }}
            testId="explore-pin"
            onOpenPage={onOpenPage}
            disabledReason={blocked}
          />
        ) : null}
        {run ? (
          <SaveAsLens
            query={run.lens.query}
            viz={viz}
            chart={chart}
            stream={stream}
            onOpenPage={onOpenPage}
            disabledReason={blocked}
          />
        ) : null}
      </div>
      {run && ranRaw ? (
        <div data-testid="explore-raw-banner" style={rawBannerStyle}>
          Raw tables: this read bypasses the models, so its columns can change without notice. It can't be saved as
          a lens or pinned to a dashboard.
        </div>
      ) : null}
      {error ? (
        <div data-testid="explore-error" style={errorStyle}>
          {error}
        </div>
      ) : null}
      {run && chart && CHART_ROLES[viz] ? (
        <div data-testid="explore-chart" style={{ display: "flex", gap: 10, flexWrap: "wrap", fontSize: "var(--text-sm)", marginBottom: 8 }}>
          {CHART_ROLES[viz]!.map((role) => (
            <label key={role.key}>
              {role.label}{" "}
              <select
                data-testid={`explore-chart-${role.key}`}
                value={chart[role.key] ?? ""}
                disabled={role.key === "series" && templateActive}
                title={role.key === "series" && templateActive ? "Slice the metric by a dimension above" : undefined}
                onChange={(e) => setRole(role.key, e.target.value || null)}
              >
                {role.optional ? <option value="">(none)</option> : null}
                {run.result.columns.map((c) => (
                  <option key={c} value={c}>
                    {c}
                  </option>
                ))}
              </select>
            </label>
          ))}
        </div>
      ) : null}
      {run ? <LensResultView run={run} onOpenPage={onOpenPage} /> : null}
    </Page>
  );
}

/** Inline "Save as Lens" strip: extension + title → a lens kept in this
 *  stream's worktree (`lens.keep` with a spec), then opens it. Enter saves, Escape cancels. */
export function SaveAsLens({
  query,
  viz,
  chart = null,
  stream,
  onOpenPage,
  disabledReason,
}: {
  query: string;
  viz: LensViz;
  /** A chart's columns, kept in the lens's `chart:`. */
  chart?: LensChart | null;
  stream: Stream | null;
  onOpenPage(ref: TabRef): void;
  /** Why it can't be saved now (a raw read). */
  disabledReason: string | null;
}) {
  const [open, setOpen] = useState(false);
  const [extension, setExtension] = useState("mine");
  const [title, setTitle] = useState("");

  async function save() {
    if (!title.trim() || !extension.trim()) return;
    try {
      const lens = await keepLensSpec(
        { title: title.trim(), description: "", query, viz, ...(chart ? { chart } : {}) },
        extension.trim(),
        slugify(title),
        stream?.id ?? null,
      );
      setOpen(false);
      setTitle("");
      onOpenPage(lensRef(lens));
    } catch (e) {
      recordOpError({ label: "Save lens", message: e instanceof Error ? e.message : String(e) });
    }
  }

  if (!open || disabledReason) {
    return (
      <button
        type="button"
        data-testid="explore-save-open"
        disabled={!!disabledReason}
        title={disabledReason ?? undefined}
        onClick={() => setOpen(true)}
      >
        Save as Lens
      </button>
    );
  }
  const onKey = (e: React.KeyboardEvent) => {
    if (e.key === "Enter") void save();
    if (e.key === "Escape") setOpen(false);
  };
  return (
    <span style={{ display: "flex", gap: 6, alignItems: "center" }}>
      <input
        data-testid="explore-save-extension"
        value={extension}
        onChange={(e) => setExtension(e.target.value)}
        onKeyDown={onKey}
        title="Extension (folder under oxplow/extensions/)"
        style={{ width: 90 }}
      />
      <input
        data-testid="explore-save-title"
        autoFocus
        value={title}
        placeholder="Lens title"
        onChange={(e) => setTitle(e.target.value)}
        onKeyDown={onKey}
      />
      <button type="button" data-testid="explore-save" disabled={!title.trim()} onClick={() => void save()}>
        Save
      </button>
    </span>
  );
}

/** Every column a chart names is in the result. */
function chartFits(chart: LensChart, result: SqlQueryResult): boolean {
  return Object.values(chart).every((c) => c === null || result.columns.includes(c));
}

const entityButtonStyle: CSSProperties = {
  background: "none",
  border: "none",
  padding: "4px 0",
  font: "inherit",
  fontFamily: "var(--font-mono, monospace)",
  fontSize: "var(--text-sm)",
  color: "var(--text-primary)",
  cursor: "pointer",
  textAlign: "left",
};
const sqlStyle: CSSProperties = {
  width: "100%",
  boxSizing: "border-box",
  fontFamily: "var(--font-mono, monospace)",
  fontSize: "var(--text-sm)",
};
const docsTableStyle: CSSProperties = { borderCollapse: "collapse", fontSize: "var(--text-xs)", marginTop: 6 };
const docsCellStyle: CSSProperties = { padding: "2px 8px 2px 0", verticalAlign: "top" };
const rawToggleStyle: CSSProperties = {
  fontSize: "var(--text-sm)",
  color: "var(--severity-medium)",
};
const rawBannerStyle: CSSProperties = {
  fontSize: "var(--text-xs)",
  padding: "6px 8px",
  marginBottom: 12,
  border: "1px solid var(--severity-medium)",
  borderRadius: 4,
  color: "var(--text-secondary)",
};
const errorStyle: CSSProperties = {
  fontFamily: "var(--font-mono, monospace)",
  fontSize: "var(--text-xs)",
  color: "var(--severity-critical)",
  whiteSpace: "pre-wrap",
  marginBottom: 12,
};
