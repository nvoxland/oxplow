import { useEffect, useRef, useState, type CSSProperties } from "react";
import { querySql, runCommand } from "../api.js";
import { refFromTabId } from "../tabs/pageRefs.js";
import { pageUri } from "../uri/uri.js";
import type { TabRef } from "../tabs/tabState.js";
import type { SqlCell } from "../tauri-bridge/generated/bindings.js";
import { useAppPreference } from "../prefs/remembered.js";
import { showToast } from "../components/toastStore.js";
import { useDashboardAuthoring } from "./dashboardAuthoring.js";
import { addTile } from "./dashboardEdits.js";
import { useActionAnchor, useRunner } from "../commands/RunnerContext.js";
import { InlineError } from "../kit/InlineError.js";

export interface PagePin { kind: "page"; page: string; params: Record<string, SqlCell>; title: string }
interface Dashboard { id: string; title: string; uri: string }

/** Pinning saves an embed into the dashboard's page file. Inline views
 * are first saved as pages; this menu always has a stable page identity. */
export function PagePinMenu({ tile, testId, onOpenPage, onDone, style }: {
  tile: PagePin; testId: string; onOpenPage(ref: TabRef): void; onDone(): void; style?: CSSProperties;
}) {
  const authoring = useDashboardAuthoring();
  const [extension] = useAppPreference("pages.extension", "my-pages");
  const [dashboards, setDashboards] = useState<Dashboard[] | null>(null);
  const [title, setTitle] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const saving = useRef(false);
  // What couldn't be read; a failed pin is the runner's, shown here too.
  const [error, setError] = useState<string | null>(null);
  const runner = useRunner();
  const anchor = `pin/${testId}`;
  const failure = useActionAnchor(anchor);
  useEffect(() => {
    let live = true;
    querySql("SELECT id, title, uri FROM v_page WHERE category = 'Dashboards' ORDER BY title, id", [], 200)
      .then(async (result) => {
        const candidates = result.rows.map((r) => ({ id: String(r[0]), title: String(r[1]), uri: String(r[2]) }));
        const list = await Promise.all(candidates.map(async (d) => {
          try { return (await authoring.read(d.id)).writable && d.id !== tile.page ? d : null; } catch { return null; }
        }));
        if (live) setDashboards(list.filter((d): d is Dashboard => d !== null));
      }).catch((e: unknown) => { if (live) setError(String(e)); });
    return () => { live = false; };
  }, [authoring, tile.page]);
  async function pin(dashboard: Dashboard) {
    const spec = await authoring.read(dashboard.id);
    if (!spec.writable) throw new Error("This dashboard is read-only");
    const yaml = addTile(spec.yaml, { id: `tile-${crypto.randomUUID()}`, page: tile.page, title: tile.title, params: tile.params });
    await authoring.write({ extension: spec.extension, page: spec.slug, spec: yaml, deploy: true });
    showToast({ message: `Pinned to ${dashboard.title}.` });
    const ref = refFromTabId(dashboard.uri);
    if (ref) onOpenPage(ref);
    onDone();
  }
  async function act(dashboard?: Dashboard) {
    if (saving.current) return;
    saving.current = true; setBusy(true);
    try {
      await runner.run("Pin to dashboard", async () => {
        if (dashboard) await pin(dashboard);
        else {
          const name = title?.trim();
          if (!name) return;
          const out = await runCommand("oxplow.extension.new_dashboard", { extension, title: name });
          const id = (out.result as { page: string }).page;
          const spec = await authoring.read(id);
          // Its id is its namespace's (`spec.page`), not its extension's name.
          await pin({ id, title: name, uri: pageUri(spec.page) });
        }
      }, { anchor });
    } finally { saving.current = false; setBusy(false); }
  }
  return <div data-testid={`${testId}-menu`} style={{ display: "flex", flexDirection: "column", gap: 4, ...style }} onKeyDown={(e) => { if (e.key === "Escape") { e.stopPropagation(); onDone(); } }}>
    {error ? <p role="alert" className="ox-error">{error}</p> : null}
    {failure ? <InlineError failure={failure} testId={`${testId}-error`} /> : null}
    {dashboards === null && !error ? <p className="ox-muted">Loading…</p> : null}
    {dashboards?.map((d) => <button key={d.id} type="button" data-testid={`${testId}-to-${d.id}`} disabled={busy} onClick={() => void act(d)}>{d.title}</button>)}
    {title === null ? <button type="button" data-testid={`${testId}-new`} disabled={busy} onClick={() => setTitle("")}>New dashboard…</button> :
      <form onSubmit={(e) => { e.preventDefault(); void act(); }}>
        <input autoFocus data-testid={`${testId}-new-name`} value={title} placeholder="Dashboard name" disabled={busy} onChange={(e) => setTitle(e.target.value)} />
        <button type="submit" data-testid={`${testId}-new-create`} disabled={busy || !title.trim()}>Create and pin</button>
      </form>}
  </div>;
}
