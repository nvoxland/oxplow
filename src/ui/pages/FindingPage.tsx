import { useEffect, useState } from "react";
import type { CodeQualityFindingRow, Stream, ThreadWorkState } from "../api.js";
import { listCodeQualityFindings, readWorkspaceFile } from "../api.js";
import { Page } from "../tabs/Page.js";
import type { TabRef } from "../tabs/tabState.js";
import { fileRef, findingRef } from "../tabs/pageRefs.js";
import { BacklinksList } from "../tabs/BacklinksList.js";
import { useBacklinks } from "../tabs/useBacklinks.js";

export interface FindingPageProps {
  stream: Stream | null;
  findingId: string;
  threadWork: ThreadWorkState | null;
  onOpenPage(ref: TabRef): void;
  /** Optional opener used by "Jump to source" — falls back to onOpenPage(fileRef). */
  onOpenFileAtLine?(path: string, line: number): void;
}

/**
 * Single-record page for a code-quality finding. Loads the finding row,
 * shows kind/path/line range/metric, and pulls a small snippet from the
 * file for context. "Jump to source" opens the file (at the line if the
 * caller wires it).
 */
export function FindingPage({ stream, findingId, threadWork, onOpenPage, onOpenFileAtLine }: FindingPageProps) {
  const backlinkEntries = useBacklinks(findingRef(findingId), stream, threadWork);
  const backlinks = <BacklinksList entries={backlinkEntries} onOpenPage={onOpenPage} />;
  const [row, setRow] = useState<CodeQualityFindingRow | null>(null);
  const [snippet, setSnippet] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!stream) {
      setRow(null);
      return;
    }
    let cancelled = false;
    setError(null);
    listCodeQualityFindings({ streamId: stream.id })
      .then((rows) => {
        if (cancelled) return;
        const id = Number(findingId);
        const match = rows.find((r) => r.id === id) ?? null;
        setRow(match);
      })
      .catch((e) => {
        if (cancelled) return;
        setError(String(e instanceof Error ? e.message : e));
      });
    return () => {
      cancelled = true;
    };
  }, [stream, findingId]);

  useEffect(() => {
    if (!stream || !row) {
      setSnippet(null);
      return;
    }
    let cancelled = false;
    readWorkspaceFile(stream.id, row.path)
      .then((file) => {
        if (cancelled) return;
        const lines = file.content.split("\n");
        const start = Math.max(0, row.startLine - 1);
        const end = Math.min(lines.length, row.endLine);
        setSnippet(lines.slice(start, end).join("\n"));
      })
      .catch(() => {
        if (cancelled) return;
        setSnippet(null);
      });
    return () => {
      cancelled = true;
    };
  }, [stream, row]);

  const title = row ? `${row.kind} in ${row.path}` : `Finding ${findingId}`;
  const chips = row
    ? [
        { label: `lines ${row.startLine}–${row.endLine}` },
        { label: `metric ${row.metricValue}` },
      ]
    : [];

  const handleJump = () => {
    if (!row) return;
    if (onOpenFileAtLine) onOpenFileAtLine(row.path, row.startLine);
    else onOpenPage(fileRef(row.path));
  };

  return (
    <Page testId="page-finding" title={title} kind="finding" chips={chips} backlinks={backlinks}>
      <div style={{ padding: "16px 20px", maxWidth: 880 }}>
        {error ? (
          <div data-testid="page-finding-error" style={{ color: "var(--severity-critical)", fontSize: 12 }}>{error}</div>
        ) : null}
        {!row && !error ? (
          <div style={{ color: "var(--text-secondary)", fontSize: 13 }}>Loading…</div>
        ) : null}
        {row ? (
          <>
            <div style={{ display: "flex", gap: 8, marginBottom: 12 }}>
              <button
                type="button"
                data-testid="page-finding-jump"
                onClick={handleJump}
                style={{
                  padding: "6px 12px",
                  background: "var(--surface-tab-inactive)",
                  color: "var(--text-primary)",
                  border: "1px solid var(--border-subtle)",
                  borderRadius: 6,
                  cursor: "pointer",
                  fontSize: 12,
                }}
              >
                Jump to source
              </button>
            </div>
            {snippet !== null ? (
              <pre
                data-testid="page-finding-snippet"
                style={{
                  background: "var(--surface-app)",
                  border: "1px solid var(--border-subtle)",
                  borderRadius: 6,
                  padding: "10px 12px",
                  fontSize: 12,
                  lineHeight: 1.45,
                  overflow: "auto",
                  maxHeight: 400,
                  color: "var(--text-primary)",
                  margin: 0,
                }}
              >
                {snippet}
              </pre>
            ) : null}
          </>
        ) : null}
      </div>
    </Page>
  );
}
