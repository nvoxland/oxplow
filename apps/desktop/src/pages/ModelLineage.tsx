import type { CSSProperties } from "react";

import type { Lineage } from "./exploreData.js";

/** A model's lineage (`v_model_lineage`) on Explore Data: the models and
 *  tables it reads, and the models that read it. A model is a link that
 *  opens it; a table is plain text — only a raw read reaches one. */
export function ModelLineage({ lineage, onPick }: { lineage: Lineage; onPick(view: string): void }) {
  return (
    <details data-testid="explore-lineage" style={{ marginBottom: 12 }}>
      <summary style={{ cursor: "pointer" }}>Lineage</summary>
      <div style={gridStyle}>
        <span style={labelStyle}>Reads</span>
        <span>
          {lineage.reads.length === 0 ? (
            <span style={mutedStyle}>Nothing: it reads no other model or table.</span>
          ) : (
            lineage.reads.map((r) =>
              r.kind === "ref" ? (
                <button
                  key={r.name}
                  type="button"
                  data-testid={`explore-lineage-reads-${r.name}`}
                  style={linkStyle}
                  onClick={() => onPick(r.name)}
                >
                  {r.name}
                </button>
              ) : (
                <code key={r.name} data-testid={`explore-lineage-table-${r.name}`} title="A table" style={{ marginRight: 8 }}>
                  {r.name}
                </code>
              ),
            )
          )}
        </span>
        <span style={labelStyle}>Read by</span>
        <span>
          {lineage.readBy.length === 0 ? (
            <span style={mutedStyle}>No other model.</span>
          ) : (
            lineage.readBy.map((v) => (
              <button
                key={v}
                type="button"
                data-testid={`explore-lineage-read-by-${v}`}
                style={linkStyle}
                onClick={() => onPick(v)}
              >
                {v}
              </button>
            ))
          )}
        </span>
      </div>
    </details>
  );
}

const gridStyle: CSSProperties = {
  display: "grid",
  gridTemplateColumns: "auto 1fr",
  gap: "4px 12px",
  fontSize: "var(--text-xs)",
  marginTop: 6,
};
const labelStyle: CSSProperties = { color: "var(--text-muted)" };
const mutedStyle: CSSProperties = { color: "var(--text-muted)" };
const linkStyle: CSSProperties = {
  background: "none",
  border: "none",
  padding: 0,
  marginRight: 8,
  font: "inherit",
  fontFamily: "var(--font-mono, monospace)",
  fontSize: "var(--text-xs)",
  color: "var(--accent)",
  cursor: "pointer",
};
