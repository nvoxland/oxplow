/// The Problems page (`page:problems`, P6.E3): what the stream's language
/// servers report now (`v_diagnostic`), by file — files with errors
/// first — with a severity filter and counts. A problem opens its file
/// at its line. Live: it re-reads when diagnostics change.
import type { CSSProperties } from "react";
import { useCallback, useEffect, useState } from "react";

import { EmptyState } from "../components/Prompts/EmptyState.js";
import { problemsByFile, readDiagnostics, type Diagnostic, type Severity } from "../codeIntel.js";
import { NO_READS, useRerunOnChange } from "../lens/lensRerun.js";
import { streamRowId } from "../modelIds.js";
import { Page } from "../tabs/Page.js";
import { fileRef } from "../tabs/pageRefs.js";
import type { TabRef } from "../tabs/tabState.js";
import type { Reads } from "../tauri-bridge/generated/bindings.js";

const SEVERITIES: Severity[] = ["error", "warning", "information", "hint"];

export function ProblemsPage({ streamId, onOpenPage }: { streamId: string; onOpenPage(ref: TabRef): void }) {
  const [diagnostics, setDiagnostics] = useState<Diagnostic[]>([]);
  const [reads, setReads] = useState<Reads>(NO_READS);
  const [severity, setSeverity] = useState<Severity | null>(null);
  const refresh = useCallback(async () => {
    const out = await readDiagnostics(streamRowId(streamId)).catch(() => null);
    if (!out) return;
    setDiagnostics(out.diagnostics);
    setReads(out.reads);
  }, [streamId]);
  useEffect(() => void refresh(), [refresh]);
  useRerunOnChange(reads, () => void refresh());

  const files = problemsByFile(diagnostics, severity);
  const count = (s: Severity) => diagnostics.filter((d) => d.severity === s).length;
  const open = (d: Diagnostic) =>
    onOpenPage({ ...fileRef(d.path), payload: { path: d.path, version: "working", line: d.line } });

  return (
    <Page testId="page-problems" title="Problems" kind="problems">
      <div style={{ padding: 12, display: "flex", flexDirection: "column", gap: 10 }}>
        <div style={{ display: "flex", gap: 6, flexWrap: "wrap" }} role="group" aria-label="Severity">
          <button type="button" data-testid="problems-filter-all" aria-pressed={severity === null} onClick={() => setSeverity(null)}>
            All {diagnostics.length}
          </button>
          {SEVERITIES.map((s) => (
            <button
              key={s}
              type="button"
              data-testid={`problems-filter-${s}`}
              aria-pressed={severity === s}
              onClick={() => setSeverity(severity === s ? null : s)}
            >
              {s[0]!.toUpperCase() + s.slice(1)} {count(s)}
            </button>
          ))}
        </div>
        {files.length === 0 ? (
          <EmptyState
            testId="problems-empty"
            title={diagnostics.length === 0 ? "No problems" : "Nothing at this severity"}
            text="Language servers report errors and warnings for the files they have open or indexed."
          />
        ) : (
          files.map((f) => (
            <section key={f.path} data-testid="problems-file">
              <h3 style={fileStyle}>
                {f.path} <span style={countStyle}>{f.problems.length}</span>
              </h3>
              {f.problems.map((d, i) => (
                <button key={i} type="button" style={rowStyle} onClick={() => open(d)} title="Open at this line">
                  <span style={{ color: SEVERITY_COLOR[d.severity], minWidth: 64 }}>{d.severity}</span>
                  <span style={{ flex: 1 }}>{d.message}</span>
                  <span style={countStyle}>
                    {d.source ?? ""}
                    {d.code ? ` ${d.code}` : ""} {d.line}:{d.col}
                  </span>
                </button>
              ))}
            </section>
          ))
        )}
      </div>
    </Page>
  );
}

const SEVERITY_COLOR: Record<Severity, string> = {
  error: "var(--severity-critical)",
  warning: "var(--status-waiting)",
  information: "var(--text-secondary)",
  hint: "var(--text-muted)",
};
const fileStyle: CSSProperties = { fontSize: "var(--text-sm)", margin: "4px 0", fontFamily: "var(--font-mono)" };
const countStyle: CSSProperties = { color: "var(--text-secondary)", fontSize: "var(--text-xs)", fontWeight: 400 };
const rowStyle: CSSProperties = {
  display: "flex",
  gap: 8,
  width: "100%",
  textAlign: "left",
  background: "none",
  border: "none",
  padding: "3px 8px",
  fontSize: "var(--text-sm)",
  color: "var(--text-primary)",
  cursor: "pointer",
};
