import { useRef, useState } from "react";

import { InlineEdit } from "../components/InlineEdit.js";
import { rename } from "./dashboardEdits.js";
import { useDashboardAuthoring } from "./dashboardAuthoring.js";

/** The writable dashboard's heading. Enter/blur saves; Escape cancels.
 * Read the current source before editing it, preserving edits that have
 * not been deployed. write_page currently has no concurrent-write CAS. */
export function DashboardTitle({ page, title, onSaved }: {
  page: string;
  title: string;
  onSaved?(title: string): void;
}) {
  const authoring = useDashboardAuthoring();
  const saving = useRef(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const save = async (next: string) => {
    // Enter may unmount the input and trigger blur before React commits
    // its state update. The synchronous latch prevents a second write.
    if (saving.current || !next.trim() || next.trim() === title) return;
    saving.current = true;
    setBusy(true);
    setError(null);
    try {
      const spec = await authoring.read(page);
      if (!spec.writable) throw new Error("This dashboard's source cannot be edited here.");
      const yaml = rename(spec.yaml, next);
      await authoring.write({
        extension: spec.extension,
        page: spec.slug,
        spec: yaml,
        deploy: true,
      });
      onSaved?.(next.trim());
    } catch (cause) {
      const message = cause instanceof Error ? cause.message : String(cause);
      setError(message);
      authoring.report({ label: "Rename dashboard", command: "oxplow.extension.write_page", message });
    } finally {
      saving.current = false;
      setBusy(false);
    }
  };

  return <div data-testid="dashboard-title" aria-busy={busy}>
    <h1 style={{ margin: 0, fontSize: "var(--text-xl)", color: "var(--text-primary)" }}>
      <InlineEdit
        key={page}
        value={title}
        onCommit={(next) => { void save(next); }}
        testId="dashboard-title-input"
        ariaLabel="Dashboard title"
        inputStyle={{ fontSize: "inherit" }}
        renderDisplay={(value, beginEdit) => <button
          type="button"
          disabled={busy}
          onClick={() => { setError(null); beginEdit(); }}
          data-testid="dashboard-title-edit"
          aria-label="Rename dashboard"
          style={{ border: 0, padding: 0, background: "transparent", color: "inherit", font: "inherit", textAlign: "left", cursor: busy ? "wait" : "text" }}
        >{value}</button>}
      />
    </h1>
    {busy ? <span role="status" className="ox-muted">Saving…</span> : null}
    {error ? <div role="alert" className="ox-error">{error}</div> : null}
  </div>;
}
