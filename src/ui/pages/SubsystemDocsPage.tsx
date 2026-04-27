import { useEffect, useState } from "react";
import type { Stream } from "../api.js";
import { listWorkspaceEntries } from "../api.js";
import { Page } from "../tabs/Page.js";
import type { TabRef } from "../tabs/tabState.js";
import { fileRef } from "../tabs/pageRefs.js";

export interface SubsystemDocsPageProps {
  stream: Stream | null;
  /** Open the requested page (a file ref for the chosen doc). */
  onOpenPage(ref: TabRef): void;
}

interface DocEntry {
  name: string;
  path: string;
}

/**
 * Filters a `.context/` directory listing down to the markdown docs we
 * actually want to surface, sorted alphabetically. Pulled out so it can
 * be unit-tested without a workspace.
 */
export function filterSubsystemDocs(rows: ReadonlyArray<{ name: string; path: string; kind: "file" | "directory" }>): DocEntry[] {
  return rows
    .filter((r) => r.kind === "file" && r.name.endsWith(".md"))
    .map((r) => ({ name: r.name, path: r.path }))
    .sort((a, b) => a.name.localeCompare(b.name));
}

/**
 * Lists `.context/*.md` subsystem docs. Clicking an entry opens the doc
 * as a regular file tab. Pages are read-only-ish here — actual editing
 * happens in the Monaco file tab.
 */
export function SubsystemDocsPage({ stream, onOpenPage }: SubsystemDocsPageProps) {
  const [entries, setEntries] = useState<DocEntry[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!stream) {
      setEntries([]);
      return;
    }
    let cancelled = false;
    setLoading(true);
    setError(null);
    listWorkspaceEntries(stream.id, ".context")
      .then((rows) => {
        if (cancelled) return;
        setEntries(filterSubsystemDocs(rows));
      })
      .catch((e) => {
        if (cancelled) return;
        setError(String(e instanceof Error ? e.message : e));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [stream]);

  return (
    <Page testId="page-subsystem-docs" title="Subsystem docs">
      <div style={{ padding: "16px 20px", maxWidth: 720 }}>
        <p
          style={{
            color: "var(--text-secondary)",
            margin: "0 0 16px",
            fontSize: 13,
          }}
        >
          Durable knowledge base under <code>.context/</code>. Read these before touching the matching subsystem.
        </p>
        {loading ? (
          <div style={{ color: "var(--text-secondary)", fontSize: 13 }}>Loading…</div>
        ) : null}
        {error ? (
          <div
            data-testid="page-subsystem-docs-error"
            style={{ color: "var(--severity-critical)", fontSize: 12 }}
          >
            {error}
          </div>
        ) : null}
        {!loading && !error && entries.length === 0 ? (
          <div style={{ color: "var(--text-secondary)", fontSize: 13 }}>
            No <code>.context/*.md</code> docs found in this workspace.
          </div>
        ) : null}
        <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
          {entries.map((entry) => (
            <button
              key={entry.path}
              type="button"
              data-testid={`page-subsystem-docs-entry-${entry.name}`}
              onClick={() => onOpenPage(fileRef(entry.path))}
              style={{
                display: "flex",
                alignItems: "center",
                gap: 8,
                padding: "8px 10px",
                background: "var(--surface-tab-inactive)",
                color: "var(--text-primary)",
                border: "1px solid var(--border-subtle)",
                borderRadius: 6,
                cursor: "pointer",
                fontSize: 13,
                textAlign: "left",
              }}
            >
              <span aria-hidden style={{ fontSize: 14 }}>📄</span>
              <span>{entry.name}</span>
            </button>
          ))}
        </div>
      </div>
    </Page>
  );
}
