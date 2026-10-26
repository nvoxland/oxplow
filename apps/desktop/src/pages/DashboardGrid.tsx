/** A writable dashboard's grid. Every edit starts from the primary YAML
 * source and goes through write_page; tile order previews immediately. */
import { useEffect, useRef, useState, type ReactNode } from "react";
import { useContextMenu } from "../components/useRowContextMenu.js";
import { TileGrid, TileSlot, type TileSize } from "../kit/Tiles.js";
import { moveTile, removeTile } from "./dashboardEdits.js";
import { DashboardTitle } from "./DashboardTitle.js";
import { useDashboardAuthoring } from "./dashboardAuthoring.js";

export interface DashboardChild { id: string; size: TileSize; body: ReactNode }
const TILE_MIME = "application/x-oxplow-dashboard-tile";

export function DashboardGrid({ page, title, tiles }: { page: string; title: string; tiles: DashboardChild[] }) {
  const authoring = useDashboardAuthoring();
  const [writable, setWritable] = useState(false);
  const [order, setOrder] = useState<string[] | null>(null);
  const [removing, setRemoving] = useState<string | null>(null);
  const [over, setOver] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const saving = useRef(false);
  const menu = useContextMenu();
  const signature = JSON.stringify(tiles.map((t) => t.id));
  useEffect(() => { setOrder(null); }, [page, signature]);
  useEffect(() => {
    let live = true;
    setWritable(false);
    authoring.read(page).then((spec) => { if (live) setWritable(spec.writable); }).catch(() => {});
    return () => { live = false; };
  }, [page, authoring]);
  async function edit(change: (source: string) => string, optimistic?: string[]) {
    if (saving.current) return;
    saving.current = true;
    setBusy(true);
    if (optimistic) setOrder(optimistic);
    try {
      const spec = await authoring.read(page);
      if (!spec.writable) throw new Error("This dashboard is read-only");
      await authoring.write({
        extension: spec.extension, page: spec.slug, spec: change(spec.yaml), deploy: true,
      });
    } catch (e) {
      setOrder(null);
      authoring.report({ label: "Edit dashboard", message: String(e) });
    } finally {
      saving.current = false;
      setBusy(false);
    }
  }
  const shown = order ? order.flatMap((id) => tiles.filter((t) => t.id === id)) : tiles;
  return <>
    {writable ? <DashboardTitle page={page} title={title} /> : <h1 className="ox-title">{title}</h1>}
    <TileGrid testId="dashboard-grid" aria-busy={busy}>
      {shown.map((tile, index) => {
        const items = [{ id: "remove", label: "Remove tile…", enabled: !busy, run: () => setRemoving(tile.id) }];
        return <TileSlot key={tile.id} size={tile.size} tabIndex={writable ? 0 : undefined}
          data-testid={`dashboard-tile-${tile.id}`} draggable={writable && !busy}
          style={over === tile.id ? { outline: "2px solid var(--accent)" } : undefined}
          onContextMenu={writable ? (e) => menu.open(e, items) : undefined}
          onKeyDown={writable ? (e) => menu.openForKey(e, items) : undefined}
          onDragStart={(e) => { e.stopPropagation(); e.dataTransfer.setData(TILE_MIME, JSON.stringify({ page, id: tile.id })); e.dataTransfer.effectAllowed = "move"; }}
          onDragEnd={() => setOver(null)}
          onDragOver={(e) => { if (writable && !busy && e.dataTransfer.types.includes(TILE_MIME)) { e.preventDefault(); e.stopPropagation(); e.dataTransfer.dropEffect = "move"; setOver(tile.id); } }}
          onDragLeave={() => setOver(null)}
          onDrop={(e) => {
            if (!writable || busy || !e.dataTransfer.types.includes(TILE_MIME)) return;
            e.preventDefault(); e.stopPropagation(); setOver(null);
            try {
              const drag = JSON.parse(e.dataTransfer.getData(TILE_MIME)) as { page: string; id: string };
              if (drag.page !== page || !shown.some((t) => t.id === drag.id) || drag.id === tile.id) return;
              const next = shown.map((t) => t.id).filter((id) => id !== drag.id);
              next.splice(index, 0, drag.id);
              void edit((yaml) => moveTile(yaml, drag.id, index), next);
            } catch { /* An unrelated drag carries no dashboard edit. */ }
          }}>
          {removing === tile.id ? <form onSubmit={(e) => { e.preventDefault(); setRemoving(null); void edit((yaml) => removeTile(yaml, tile.id), shown.filter((t) => t.id !== tile.id).map((t) => t.id)); }}
            onKeyDown={(e) => { if (e.key === "Escape") { e.stopPropagation(); setRemoving(null); } }}>
            <span>Remove this tile? </span><button autoFocus type="submit">Confirm</button><button type="button" onClick={() => setRemoving(null)}>Cancel</button>
          </form> : null}
          {tile.body}
        </TileSlot>;
      })}
    </TileGrid>
    {menu.menu}
  </>;
}
