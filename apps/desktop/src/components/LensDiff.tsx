/// A changed lens in an extension review: its text before and
/// after, side by side — what an agent would read from it, which is also
/// what it shows. No line diff yet; the two texts are short.
import type { CSSProperties } from "react";

export function LensDiff({ id, before, after }: { id: string; before: string; after: string }) {
  return (
    <div data-testid={`impact-lens-${id}`} style={{ marginTop: 6 }}>
      <div style={{ fontSize: "var(--text-xs)", color: "var(--text-secondary)" }}>{id}</div>
      <div style={{ display: "flex", gap: 8 }}>
        <div style={{ flex: 1, minWidth: 0 }}>
          <div style={labelStyle}>Before</div>
          <pre data-testid={`impact-lens-${id}-before`} style={textStyle}>
            {before}
          </pre>
        </div>
        <div style={{ flex: 1, minWidth: 0 }}>
          <div style={labelStyle}>After</div>
          <pre data-testid={`impact-lens-${id}-after`} style={textStyle}>
            {after}
          </pre>
        </div>
      </div>
    </div>
  );
}

const labelStyle: CSSProperties = { fontSize: "var(--text-xs)", color: "var(--text-secondary)", textTransform: "uppercase" };
const textStyle: CSSProperties = {
  margin: 0,
  padding: 6,
  fontSize: "var(--text-xs)",
  whiteSpace: "pre-wrap",
  wordBreak: "break-word",
  border: "1px solid var(--border-subtle)",
  borderRadius: 4,
  maxHeight: 200,
  overflow: "auto",
};
