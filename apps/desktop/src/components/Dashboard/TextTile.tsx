import type { DashboardItem } from "../../api.js";
import type { TileOptions } from "../../pages/customDashboardData.js";
import type { MenuItem } from "../../menu.js";
import { InlineEdit } from "../InlineEdit.js";
import { MarkdownView } from "../Wiki/MarkdownView.js";
import { useContextMenu } from "../useRowContextMenu.js";

/**
 * A `text` dashboard tile (tsk142) — a heading or markdown note used to group
 * and annotate a grid of metric tiles. The body is edited in place
 * (`InlineEdit`, multiline) and rendered with the shared `MarkdownView`, so the
 * same wikilink/markdown vocabulary as notes applies. Right-click carries the
 * size + remove actions, matching {@link MetricTile}.
 */
export function TextTile({
  item,
  opts,
  onRemove,
  onConfigure,
}: {
  item: DashboardItem;
  opts: TileOptions;
  onRemove?: () => void;
  onConfigure?: (next: Partial<TileOptions>) => void;
}) {
  const ctxMenu = useContextMenu();
  const text = opts.text ?? "";

  const menuItems: MenuItem[] = [
    {
      id: "size",
      label: "Size",
      enabled: !!onConfigure,
      submenu: (["small", "wide", "tall"] as const).map((s) => ({
        id: `size:${s}`,
        label: s[0]!.toUpperCase() + s.slice(1),
        enabled: true,
        checked: (opts.size ?? "small") === s,
        run: () => onConfigure?.({ size: s }),
      })),
    },
    { id: "sep", label: "", enabled: false, separator: true },
    { id: "remove", label: "Remove from dashboard", enabled: !!onRemove, run: () => onRemove?.() },
  ];

  return (
    <section
      data-testid={`text-tile-${item.id}`}
      onContextMenu={(e) => ctxMenu.open(e, menuItems)}
      style={{
        background: "var(--surface-card)",
        border: "1px solid var(--border-subtle)",
        borderRadius: 6,
        padding: 12,
        minWidth: 0,
        height: "100%",
        overflow: "auto",
      }}
    >
      <InlineEdit
        value={text}
        onCommit={(next) => onConfigure?.({ text: next })}
        multiline
        allowEmpty
        placeholder="Click to add a heading or note…"
        ariaLabel="Tile text"
        testId={`text-tile-edit-${item.id}`}
        renderDisplay={(value, beginEdit) =>
          value ? (
            <div onDoubleClick={beginEdit} style={{ cursor: "text" }}>
              <MarkdownView body={value} />
            </div>
          ) : (
            <button
              type="button"
              onClick={beginEdit}
              style={{
                all: "unset",
                cursor: "text",
                opacity: 0.5,
                fontSize: 13,
              }}
            >
              Click to add a heading or note…
            </button>
          )
        }
      />
      {ctxMenu.menu}
    </section>
  );
}
