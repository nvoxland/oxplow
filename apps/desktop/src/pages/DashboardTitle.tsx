import { useRef, useState } from "react";

import { InlineEdit } from "../components/InlineEdit.js";
import { rename } from "./dashboardEdits.js";
import { useDashboardAuthoring } from "./dashboardAuthoring.js";
import { useActionAnchor, useRunner } from "../commands/RunnerContext.js";
import { InlineError } from "../kit/InlineError.js";

/** The writable dashboard's title, in the nav bar. Enter/blur saves; Escape cancels.
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
  const runner = useRunner();
  const anchor = `dashboard-title/${page}`;
  const failure = useActionAnchor(anchor);

  const save = async (next: string) => {
    // Enter may unmount the input and trigger blur before React commits
    // its state update. The synchronous latch prevents a second write.
    if (saving.current || !next.trim() || next.trim() === title) return;
    saving.current = true;
    setBusy(true);
    try {
      const ran = await runner.run(
        "Rename dashboard",
        async () => {
          const spec = await authoring.read(page);
          if (!spec.writable) throw new Error("This dashboard's source cannot be edited here.");
          await authoring.write({ extension: spec.extension, page: spec.slug, spec: rename(spec.yaml, next), deploy: true });
        },
        { anchor, command: "oxplow.extension.write_page" },
      );
      if (ran.ok) onSaved?.(next.trim());
    } finally {
      saving.current = false;
      setBusy(false);
    }
  };

  // Drawn in the nav bar as the page's title, so it is inline, not a heading.
  return <span data-testid="dashboard-title" aria-busy={busy}>
    <span>
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
          onClick={() => beginEdit()}
          data-testid="dashboard-title-edit"
          aria-label="Rename dashboard"
          style={{ border: 0, padding: 0, background: "transparent", color: "inherit", font: "inherit", textAlign: "left", cursor: busy ? "wait" : "text" }}
        >{value}</button>}
      />
    </span>
    {busy ? <span role="status" className="ox-muted">Saving…</span> : null}
    {failure ? <InlineError failure={failure} testId="dashboard-title-error" /> : null}
  </span>;
}
