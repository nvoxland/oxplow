/// A small line above a lens result when it shows only the first rows or a
/// collector feeding it hasn't kept up (`completeness`). Not a modal: the
/// rows are still useful, so this only says what they aren't.
import type { SqlQueryResult } from "../tauri-bridge/generated/bindings.js";
import { completeness } from "./completeness.js";

export function ResultCompleteness({ result }: { result: SqlQueryResult }) {
  const { rows, coverage } = completeness(result);
  if (rows === null && coverage.length === 0) return null;
  return (
    <div data-testid="lens-completeness" style={{ fontSize: "var(--text-xs)", margin: "0 0 6px" }}>
      {rows ? (
        <div data-testid="lens-truncated" style={{ color: "var(--text-muted)" }}>
          {rows}
        </div>
      ) : null}
      {coverage.map((line) => (
        <div key={line} data-testid="lens-coverage" style={{ color: "var(--status-waiting)" }}>
          {line}
        </div>
      ))}
    </div>
  );
}
