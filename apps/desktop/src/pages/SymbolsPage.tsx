/// The Symbols page (`page:symbols[?path=]`, P6.E3): a file's outline, or
/// every file's symbols matching a filter, from `v_symbol` — nested under
/// their container. A symbol opens its file at its name's line.
import type { CSSProperties } from "react";
import { useCallback, useEffect, useState } from "react";

import { EmptyState } from "../components/Prompts/EmptyState.js";
import { readSymbols, symbolTree, type SymbolNode, type SymbolRow } from "../codeIntel.js";
import { NO_READS, useRerunOnChange } from "../lens/lensRerun.js";
import { streamRowId } from "../modelIds.js";
import { Page } from "../tabs/Page.js";
import { fileRef } from "../tabs/pageRefs.js";
import type { TabRef } from "../tabs/tabState.js";
import type { Reads } from "../tauri-bridge/generated/bindings.js";

export function SymbolsPage({
  streamId,
  path,
  onOpenPage,
}: {
  streamId: string;
  /** One file's outline; null for the whole project (filtered). */
  path: string | null;
  onOpenPage(ref: TabRef): void;
}) {
  const [symbols, setSymbols] = useState<SymbolRow[]>([]);
  const [reads, setReads] = useState<Reads>(NO_READS);
  const [filter, setFilter] = useState("");
  // The query follows the filter once typing pauses, not per keystroke.
  const [query, setQuery] = useState("");
  useEffect(() => {
    const t = setTimeout(() => setQuery(filter), FILTER_DEBOUNCE_MS);
    return () => clearTimeout(t);
  }, [filter]);
  const refresh = useCallback(async () => {
    const out = await readSymbols(streamRowId(streamId), { path, filter: query }).catch(() => null);
    if (!out) return;
    setSymbols(out.symbols);
    setReads(out.reads);
  }, [streamId, path, query]);
  useEffect(() => void refresh(), [refresh]);
  useRerunOnChange(reads, () => void refresh());

  const open = (s: SymbolRow) =>
    onOpenPage({ ...fileRef(s.path), payload: { path: s.path, version: "working", line: s.line } });
  const tree = symbolTree(symbols);
  const byFile = new Map<string, SymbolNode[]>();
  for (const n of tree) byFile.set(n.symbol.path, [...(byFile.get(n.symbol.path) ?? []), n]);

  return (
    <Page testId="page-symbols" title={path ? `Symbols — ${path}` : "Symbols"} kind="symbols">
      <div style={{ padding: 12, display: "flex", flexDirection: "column", gap: 8 }}>
        <input
          data-testid="symbols-filter"
          placeholder={path ? "Filter this file's symbols" : "Find a symbol by name"}
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Escape") setFilter("");
          }}
          autoFocus
        />
        {tree.length === 0 ? (
          <EmptyState
            testId="symbols-empty"
            title="No symbols"
            text="Symbols come from the running language servers, collected at each snapshot; a language without a running server has none."
          />
        ) : (
          [...byFile.entries()].map(([file, nodes]) => (
            <section key={file}>
              {path ? null : <h3 style={fileStyle}>{file}</h3>}
              <ul style={listStyle}>
                {nodes.map((n) => (
                  <SymbolItem key={n.symbol.ref} node={n} onOpen={open} />
                ))}
              </ul>
            </section>
          ))
        )}
      </div>
    </Page>
  );
}

function SymbolItem({ node, onOpen }: { node: SymbolNode; onOpen(s: SymbolRow): void }) {
  const [open, setOpen] = useState(true);
  const s = node.symbol;
  return (
    <li>
      <div style={{ display: "flex", alignItems: "center", gap: 4 }}>
        {node.children.length > 0 ? (
          <button type="button" aria-label={open ? "Collapse" : "Expand"} style={toggleStyle} onClick={() => setOpen(!open)}>
            {open ? "▾" : "▸"}
          </button>
        ) : (
          <span style={{ width: 14 }} />
        )}
        <button type="button" data-testid="symbols-item" style={itemStyle} onClick={() => onOpen(s)} title={`${s.path}:${s.line}`}>
          <span style={kindStyle}>{s.kind}</span> {s.name}
        </button>
      </div>
      {open && node.children.length > 0 ? (
        <ul style={{ ...listStyle, paddingLeft: 16 }}>
          {node.children.map((c) => (
            <SymbolItem key={c.symbol.ref} node={c} onOpen={onOpen} />
          ))}
        </ul>
      ) : null}
    </li>
  );
}

const fileStyle: CSSProperties = { fontSize: "var(--text-sm)", margin: "6px 0 2px", fontFamily: "var(--font-mono)" };
const listStyle: CSSProperties = { listStyle: "none", margin: 0, padding: 0 };
const toggleStyle: CSSProperties = { background: "none", border: "none", padding: 0, width: 14, cursor: "pointer", color: "var(--text-secondary)" };
const itemStyle: CSSProperties = {
  background: "none",
  border: "none",
  padding: "2px 4px",
  fontSize: "var(--text-sm)",
  color: "var(--text-primary)",
  cursor: "pointer",
  textAlign: "left",
  fontFamily: "var(--font-mono)",
};
const kindStyle: CSSProperties = { color: "var(--text-secondary)", fontSize: "var(--text-xs)", fontFamily: "inherit" };

const FILTER_DEBOUNCE_MS = 200;
