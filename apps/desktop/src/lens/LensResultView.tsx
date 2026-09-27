import type { CSSProperties, ReactNode } from "react";
import type { LensRun, SqlCell } from "../api.js";
import { formatMetricValue, formatMetricValueExact } from "../components/format.js";
import { MarkdownView } from "../components/Wiki/MarkdownView.js";
import { RouteLink } from "../tabs/RouteLink.js";
import type { TabRef } from "../tabs/tabState.js";
import { cellLinkRef, displayColumns, formatCell, limitRows, type DisplayColumn } from "./lensModel.js";

type CellRenderer = (row: SqlCell[], col: DisplayColumn) => ReactNode;

/**
 * Renders a lens run's result in the lens's viz. Shared by the lens page
 * and dashboard lens tiles; `maxRows` caps rows for compact views.
 */
export function LensResultView({
  run,
  onOpenPage,
  maxRows,
}: {
  run: LensRun;
  onOpenPage(ref: TabRef): void;
  maxRows?: number;
}) {
  const lens = run.lens;
  const result = limitRows(run.result, maxRows);
  if (result.rows.length === 0) {
    return (
      <p data-testid="lens-empty" style={{ color: "var(--text-secondary)" }}>
        {lens.empty ?? "No rows."}
      </p>
    );
  }
  const cols = displayColumns(lens, result.columns);
  const cell: CellRenderer = (row, c) => {
    const text = formatCell(row[c.index] ?? null);
    const ref = c.link ? cellLinkRef(c.link, c.key, row, result.columns) : null;
    if (!ref) return text;
    return (
      <RouteLink to={ref} onNavigate={() => onOpenPage(ref)} style={linkStyle}>
        {text}
      </RouteLink>
    );
  };
  const first = result.rows[0]?.[0] ?? null;
  switch (lens.viz) {
    case "number":
      return <NumberViz value={first} />;
    case "markdown":
      return <MarkdownView body={first === null ? "" : String(first)} />;
    case "list":
      return <ListViz rows={result.rows} cols={cols} cell={cell} truncated={result.truncated} />;
    case "table":
    default:
      return <TableViz rows={result.rows} cols={cols} cell={cell} truncated={result.truncated} />;
  }
}

function NumberViz({ value }: { value: SqlCell }) {
  return (
    <div
      data-testid="lens-number"
      title={typeof value === "number" ? formatMetricValueExact(value) : undefined}
      style={{ fontSize: 48, fontWeight: 600, margin: "16px 0" }}
    >
      {typeof value === "number" ? formatMetricValue(value) : formatCell(value)}
    </div>
  );
}

interface RowsVizProps {
  rows: SqlCell[][];
  cols: DisplayColumn[];
  cell: CellRenderer;
  truncated: boolean;
}

function TruncatedNote({ rows, truncated }: { rows: number; truncated: boolean }) {
  if (!truncated) return null;
  return (
    <p style={{ color: "var(--text-muted)", fontSize: "var(--text-xs)" }}>Showing the first {rows} rows.</p>
  );
}

function ListViz({ rows, cols, cell, truncated }: RowsVizProps) {
  const [head, ...rest] = cols;
  return (
    <>
      <ul data-testid="lens-list" style={{ listStyle: "none", padding: 0, margin: 0 }}>
        {rows.map((row, i) => (
          <li key={i} data-testid={`lens-row-${i}`} style={listRowStyle}>
            <div>{head ? cell(row, head) : null}</div>
            {rest.length > 0 ? (
              <div style={{ color: "var(--text-secondary)", fontSize: "var(--text-sm)" }}>
                {rest.map((c, j) => (
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
      <TruncatedNote rows={rows.length} truncated={truncated} />
    </>
  );
}

function TableViz({ rows, cols, cell, truncated }: RowsVizProps) {
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
          {rows.map((row, i) => (
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
      <TruncatedNote rows={rows.length} truncated={truncated} />
    </>
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
