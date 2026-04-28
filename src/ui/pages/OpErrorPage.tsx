import { useEffect, useSyncExternalStore } from "react";
import { Page } from "../tabs/Page.js";
import { getOpErrorsStore } from "../components/opErrorsStore.js";

export interface OpErrorPageProps {
  errorId: string;
}

/**
 * Detail view for a recorded async-op failure (see `opErrorsStore`).
 * Shows the captured command, stderr, stdout, exit code, and timestamp
 * — read-only; resolution actions live on the originating page.
 */
export function OpErrorPage({ errorId }: OpErrorPageProps) {
  const store = getOpErrorsStore();
  const entries = useSyncExternalStore(store.subscribe, store.getSnapshot);
  const entry = entries.find((e) => e.id === errorId) ?? null;

  useEffect(() => {
    if (entry && !entry.seen) store.markSeen(entry.id);
  }, [entry, store]);

  if (!entry) {
    return (
      <Page testId="page-op-error" title="Operation error">
        <div style={{ padding: 16, color: "var(--text-muted)" }}>
          This error has been dismissed or expired from the in-memory log.
        </div>
      </Page>
    );
  }

  const when = new Date(entry.at);

  return (
    <Page testId="page-op-error" title={`Error: ${entry.label}`}>
      <div style={{ padding: 16, display: "flex", flexDirection: "column", gap: 16, overflow: "auto" }}>
        <Field label="Operation" value={entry.label} />
        {entry.command ? <Field label="Command" value={entry.command} mono /> : null}
        {entry.args && entry.args.length > 0 ? (
          <Field label="Argv" value={JSON.stringify(entry.args)} mono />
        ) : null}
        <Field
          label="When"
          value={`${when.toLocaleString()}${formatTiming(entry)}`}
        />
        {entry.signal ? <Field label="Signal" value={entry.signal} mono /> : null}
        {entry.stderr ? <Block label="stderr" body={entry.stderr} tone="error" /> : null}
        {entry.stdout ? <Block label="stdout" body={entry.stdout} /> : null}
        {entry.message && !entry.stderr ? <Block label="Message" body={entry.message} tone="error" /> : null}
        {!entry.stderr && !entry.stdout && !entry.message ? (
          <div style={{ color: "var(--text-muted)", fontStyle: "italic" }}>
            No output was captured.
            {entry.blankFailure
              ? " (Runner reported a blank failure — no stderr, stdout, or exit code. Likely an awaitDone race or process kill; check the main-process log for the matching `git op` entry.)"
              : ""}
          </div>
        ) : null}
      </div>
    </Page>
  );
}

function formatTiming(entry: { exitCode: number | null; durationMs: number | null }): string {
  const parts: string[] = [];
  if (entry.exitCode != null) parts.push(`exit ${entry.exitCode}`);
  if (entry.durationMs != null) parts.push(`${entry.durationMs} ms`);
  return parts.length ? ` · ${parts.join(" · ")}` : "";
}

function Field({ label, value, mono }: { label: string; value: string; mono?: boolean }) {
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
      <div style={{ fontSize: 11, fontWeight: 600, color: "var(--text-secondary)", textTransform: "uppercase", letterSpacing: 0.4 }}>
        {label}
      </div>
      <div style={{ fontFamily: mono ? "var(--mono, monospace)" : undefined, fontSize: 13, color: "var(--text-primary)" }}>
        {value}
      </div>
    </div>
  );
}

function Block({ label, body, tone }: { label: string; body: string; tone?: "error" }) {
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
      <div style={{ fontSize: 11, fontWeight: 600, color: "var(--text-secondary)", textTransform: "uppercase", letterSpacing: 0.4 }}>
        {label}
      </div>
      <pre
        style={{
          margin: 0,
          padding: 12,
          background: "var(--surface-card)",
          border: `1px solid ${tone === "error" ? "var(--diff-del-fg, #f85149)" : "var(--border-subtle)"}`,
          borderRadius: 4,
          fontFamily: "var(--mono, monospace)",
          fontSize: 12,
          color: tone === "error" ? "var(--diff-del-fg, #f85149)" : "var(--text-primary)",
          whiteSpace: "pre-wrap",
          wordBreak: "break-word",
          overflow: "auto",
          maxHeight: 480,
        }}
      >
        {body}
      </pre>
    </div>
  );
}
