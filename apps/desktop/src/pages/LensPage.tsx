import { useCallback, useEffect, useRef, useState, type CSSProperties, type ReactNode } from "react";
import { Page, pageH1Style } from "../tabs/Page.js";
import { RouteLink } from "../tabs/RouteLink.js";
import { usePageTitle } from "../tabs/PageNavigationContext.js";
import type { TabRef } from "../tabs/tabState.js";
import type { Stream } from "../tauri-bridge/index.js";
import { runLens, subscribeOxplowEvents, type LensRun, type SqlCell } from "../api.js";
import { insertIntoAgent } from "../agent-input-bus.js";
import { formatContextMention } from "../agent-context-ref.js";
import { formatMetricValue, formatMetricValueExact } from "../components/format.js";
import { MarkdownView } from "../components/Wiki/MarkdownView.js";
import {
  cellLinkRef,
  changedParams,
  displayColumns,
  formatCell,
  parseParamInput,
  shouldRerunLens,
} from "../lens/lensModel.js";

export interface LensPageProps {
  /** `<extension>/<slug>`. */
  lensId: string;
  stream: Stream | null;
  onOpenPage(ref: TabRef): void;
}

const RERUN_DEBOUNCE_MS = 750;

/**
 * A lens: a user/agent-built query over the semantic layer, read from
 * `oxplow/extensions/<ext>/lenses/<slug>.yaml` in this stream's worktree.
 * Re-runs when oxplow data changes, so it stays live. Params are edited
 * in the right rail (Enter applies). See `.context/extensions.md`.
 */
export function LensPage({ lensId, stream, onOpenPage }: LensPageProps) {
  const streamId = stream?.id ?? null;
  const [run, setRun] = useState<LensRun | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [overrides, setOverrides] = useState<Record<string, SqlCell>>({});
  const overridesRef = useRef(overrides);
  overridesRef.current = overrides;

  const refresh = useCallback(async () => {
    try {
      const next = await runLens(lensId, overridesRef.current, streamId);
      setRun(next);
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, [lensId, streamId]);

  useEffect(() => {
    void refresh();
  }, [refresh, overrides]);

  useEffect(() => {
    let timer: ReturnType<typeof setTimeout> | null = null;
    const off = subscribeOxplowEvents((event) => {
      if (!shouldRerunLens({ kind: event.kind, path: (event as { path?: unknown }).path })) return;
      if (timer) clearTimeout(timer);
      timer = setTimeout(() => void refresh(), RERUN_DEBOUNCE_MS);
    });
    return () => {
      if (timer) clearTimeout(timer);
      off();
    };
  }, [refresh]);

  const lens = run?.lens ?? null;
  const title = lens?.title ?? lensId;
  usePageTitle(title);

  const improveWithAgent = () => {
    const params = lens ? changedParams(lens, run?.params ?? {}) : {};
    insertIntoAgent(formatContextMention({ kind: "lens", lensId, params }));
  };

  const actions = (
    <div style={{ display: "flex", gap: 8 }}>
      <button type="button" data-testid="lens-refresh" onClick={() => void refresh()}>
        Refresh
      </button>
      <button
        type="button"
        data-testid="lens-improve-with-agent"
        title="Add this lens to the agent's context so you can ask it to change the lens"
        onClick={improveWithAgent}
      >
        Improve with Agent
      </button>
    </div>
  );

  const rightRail =
    lens && lens.params.length > 0 ? (
      <ParamsForm
        lens={lens}
        values={run?.params ?? {}}
        onApply={(name, value) => setOverrides((o) => ({ ...o, [name]: value }))}
      />
    ) : undefined;

  return (
    <Page
      testId="page-lens"
      kind="lens"
      titleInBody
      layout="details"
      actions={actions}
      rightRail={rightRail}
      rightRailTitle="Parameters"
    >
      <h1 style={pageH1Style}>{title}</h1>
      {lens?.description ? (
        <p style={{ color: "var(--text-secondary)", marginTop: 0 }}>{lens.description}</p>
      ) : null}
      {error ? (
        <div data-testid="lens-error" style={errorStyle}>
          <div style={{ fontWeight: 600, marginBottom: 4 }}>This lens couldn't run.</div>
          <div style={{ fontFamily: "var(--font-mono, monospace)", whiteSpace: "pre-wrap" }}>{error}</div>
          <div style={{ marginTop: 8, color: "var(--text-secondary)" }}>
            Use “Improve with Agent” and ask the agent to fix it; it can check its work with
            <code> validate_extension</code>.
          </div>
        </div>
      ) : null}
      {run ? <LensBody run={run} onOpenPage={onOpenPage} /> : error ? null : <p>Loading…</p>}
      {lens ? (
        <p style={{ color: "var(--text-muted)", fontSize: "var(--text-xs)", marginTop: 24 }}>
          {lens.path}
        </p>
      ) : null}
    </Page>
  );
}

function LensBody({ run, onOpenPage }: { run: LensRun; onOpenPage(ref: TabRef): void }) {
  const { lens, result } = run;
  if (result.rows.length === 0) {
    return (
      <p data-testid="lens-empty" style={{ color: "var(--text-secondary)" }}>
        {lens.empty ?? "No rows."}
      </p>
    );
  }
  const cols = displayColumns(lens, result.columns);
  const cell = (row: SqlCell[], c: (typeof cols)[number]): ReactNode => {
    const text = formatCell(row[c.index] ?? null);
    const ref = c.link ? cellLinkRef(c.link, c.key, row, result.columns) : null;
    return ref ? (
      <RouteLink to={ref} onNavigate={() => onOpenPage(ref)} style={linkStyle}>
        {text}
      </RouteLink>
    ) : (
      text
    );
  };
  const truncated = result.truncated ? (
    <p style={{ color: "var(--text-muted)", fontSize: "var(--text-xs)" }}>
      Showing the first {result.rows.length} rows.
    </p>
  ) : null;

  switch (lens.viz) {
    case "number": {
      const v = result.rows[0]?.[0] ?? null;
      return (
        <div
          data-testid="lens-number"
          title={typeof v === "number" ? formatMetricValueExact(v) : undefined}
          style={{ fontSize: 48, fontWeight: 600, margin: "16px 0" }}
        >
          {typeof v === "number" ? formatMetricValue(v) : formatCell(v)}
        </div>
      );
    }
    case "markdown": {
      const v = result.rows[0]?.[0] ?? null;
      return <MarkdownView body={v === null ? "" : String(v)} />;
    }
    case "list":
      return (
        <>
          <ul data-testid="lens-list" style={{ listStyle: "none", padding: 0, margin: 0 }}>
            {result.rows.map((row, i) => (
              <li key={i} data-testid={`lens-row-${i}`} style={listRowStyle}>
                <div>{cols[0] ? cell(row, cols[0]) : null}</div>
                {cols.length > 1 ? (
                  <div style={{ color: "var(--text-secondary)", fontSize: "var(--text-sm)" }}>
                    {cols.slice(1).map((c, j) => (
                      <span key={c.key}>
                        {j > 0 ? " · " : null}
                        {cell(row, c)}
                      </span>
                    ))}
                  </div>
                ) : null}
              </li>
            ))}
          </ul>
          {truncated}
        </>
      );
    case "table":
    default:
      return (
        <>
          <table data-testid="lens-table" style={tableStyle}>
            <thead>
              <tr>
                {cols.map((c) => (
                  <th key={c.key} style={thStyle}>
                    {c.label}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {result.rows.map((row, i) => (
                <tr key={i} data-testid={`lens-row-${i}`}>
                  {cols.map((c) => (
                    <td key={c.key} style={tdStyle}>
                      {cell(row, c)}
                    </td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
          {truncated}
        </>
      );
  }
}

function ParamsForm({
  lens,
  values,
  onApply,
}: {
  lens: LensRun["lens"];
  values: Record<string, SqlCell>;
  onApply(name: string, value: SqlCell): void;
}) {
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
      {lens.params.map((p) => (
        <ParamInput key={p.name} name={p.name} label={p.label ?? p.name} value={values[p.name] ?? null} onApply={onApply} />
      ))}
      <div style={{ color: "var(--text-muted)", fontSize: "var(--text-xs)" }}>Enter applies; Escape reverts.</div>
    </div>
  );
}

function ParamInput({
  name,
  label,
  value,
  onApply,
}: {
  name: string;
  label: string;
  value: SqlCell;
  onApply(name: string, value: SqlCell): void;
}) {
  const shown = value === null ? "" : String(value);
  const [text, setText] = useState(shown);
  useEffect(() => setText(shown), [shown]);
  return (
    <label style={{ display: "flex", flexDirection: "column", gap: 4, fontSize: "var(--text-sm)" }}>
      {label}
      <input
        data-testid={`lens-param-${name}`}
        value={text}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") onApply(name, parseParamInput(text));
          if (e.key === "Escape") setText(shown);
        }}
        onBlur={() => {
          if (text !== shown) onApply(name, parseParamInput(text));
        }}
      />
    </label>
  );
}

const tableStyle: CSSProperties = { width: "100%", borderCollapse: "collapse", fontSize: "var(--text-sm)" };
const thStyle: CSSProperties = {
  textAlign: "left",
  padding: "6px 8px",
  borderBottom: "1px solid var(--border-subtle)",
  color: "var(--text-secondary)",
  fontWeight: 600,
};
const tdStyle: CSSProperties = { padding: "6px 8px", borderBottom: "1px solid var(--border-subtle)", verticalAlign: "top" };
const listRowStyle: CSSProperties = { padding: "8px 0", borderBottom: "1px solid var(--border-subtle)" };
const linkStyle: CSSProperties = {
  background: "none",
  border: "none",
  padding: 0,
  color: "var(--text-link, var(--accent))",
  cursor: "pointer",
  textAlign: "left",
  font: "inherit",
};
const errorStyle: CSSProperties = {
  border: "1px solid var(--border-subtle)",
  borderRadius: 6,
  padding: 12,
  margin: "12px 0",
  background: "var(--surface-card)",
};
