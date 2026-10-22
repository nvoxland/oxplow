/// A failed operation's details (shown inline on the Alerts page): the
/// command, when, its signal, and its captured stderr / stdout, read from
/// the failure's event body when it opens (`readOpErrorOutput`) —
/// read-only; resolving it happens where it was started.

import { useEffect, useState } from "react";

import { readOpErrorOutput, type OpError } from "../../opErrors.js";

type Output = { stderr: string | null; stdout: string | null } | null;

export function OpErrorDetail({ entry }: { entry: OpError }) {
  const when = new Date(entry.at);
  // undefined: still reading.
  const [output, setOutput] = useState<Output | undefined>(entry.outputSize === null ? null : undefined);
  useEffect(() => {
    if (entry.outputSize === null) return;
    let live = true;
    readOpErrorOutput(entry.id)
      .then((o) => {
        if (live) setOutput(o);
      })
      .catch(() => {
        if (live) setOutput(null);
      });
    return () => {
      live = false;
    };
  }, [entry.id, entry.outputSize]);
  const stderr = output?.stderr ?? null;
  const stdout = output?.stdout ?? null;
  return (
    <div data-testid={`op-error-detail-${entry.id}`} style={{ display: "flex", flexDirection: "column", gap: 12, padding: "8px 0" }}>
      {entry.command ? <Field label="Command" value={entry.command} mono /> : null}
      <Field label="When" value={`${when.toLocaleString()}${formatTiming(entry)}`} />
      {entry.signal ? <Field label="Signal" value={entry.signal} mono /> : null}
      {output === undefined ? <div style={{ color: "var(--text-muted)" }}>Loading the output…</div> : null}
      {stderr ? <Block label="stderr" body={stderr} tone="error" /> : null}
      {stdout ? <Block label="stdout" body={stdout} /> : null}
      {entry.message && !stderr ? <Block label="Message" body={entry.message} tone="error" /> : null}
      {output !== undefined && !stderr && !stdout && !entry.message ? (
        <div style={{ color: "var(--text-muted)", fontStyle: "italic" }}>
          {entry.outputSize !== null ? "The output has expired." : "No output was captured."}
        </div>
      ) : null}
    </div>
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
      <div style={{ fontFamily: mono ? "var(--mono, monospace)" : undefined, fontSize: "var(--text-sm)", color: "var(--text-primary)" }}>
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
          fontSize: "var(--text-xs)",
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
