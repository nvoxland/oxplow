import type { BacklinkEntry } from "./backlinksIndex.js";
import type { TabRef } from "./tabState.js";
import { setContextRefDrag } from "../agent-context-dnd.js";
import type { ContextRef } from "../agent-context-ref.js";

export interface BacklinksListProps {
  entries: BacklinkEntry[];
  onOpenPage(ref: TabRef): void;
}

/**
 * Translate a `TabRef` into a `ContextRef` so a backlink entry can be
 * dragged into the agent terminal as an @-mention. Files / notes /
 * work-items have direct mappings; finding refs land as a file ref to
 * the finding's path because the agent doesn't have its own
 * `@finding:<id>` syntax. Returns null for ref kinds the agent can't
 * use as context (index pages, settings, etc).
 *
 * Pure — exported for tests.
 */
export function tabRefToContextRef(ref: TabRef): ContextRef | null {
  if (ref.kind === "file") {
    const payload = ref.payload as { path?: unknown } | null;
    if (payload && typeof payload.path === "string") {
      return { kind: "file", path: payload.path };
    }
    return null;
  }
  if (ref.kind === "note") {
    const payload = ref.payload as { slug?: unknown } | null;
    if (payload && typeof payload.slug === "string") {
      return { kind: "note", slug: payload.slug };
    }
    return null;
  }
  if (ref.kind === "work-item") {
    const payload = ref.payload as { itemId?: unknown } | null;
    if (payload && typeof payload.itemId === "string") {
      // Backlinks don't carry the title/status of the target — use a
      // placeholder that the agent can resolve via the work-item id.
      // formatContextMention reads `title` / `status`; passing the id
      // as the title keeps the inserted snippet readable.
      return { kind: "work-item", itemId: payload.itemId, title: payload.itemId, status: "" };
    }
    return null;
  }
  return null;
}

/**
 * Default renderer for a Page's `backlinks` slot. Lists each entry as a
 * button that opens the referenced page. Empty state shows a hint so
 * the panel isn't ambiguous when the list is empty.
 */
export function BacklinksList({ entries, onOpenPage }: BacklinksListProps) {
  if (entries.length === 0) {
    return (
      <div data-testid="backlinks-list-empty" style={{ color: "var(--text-secondary)", fontStyle: "italic" }}>
        No backlinks yet.
      </div>
    );
  }
  return (
    <div data-testid="backlinks-list" style={{ display: "flex", flexDirection: "column", gap: 4 }}>
      {entries.map((entry) => {
        const ctxRef = tabRefToContextRef(entry.ref);
        return (
        <button
          key={entry.ref.id}
          type="button"
          data-testid={`backlinks-entry-${entry.ref.id}`}
          onClick={() => onOpenPage(entry.ref)}
          draggable={ctxRef !== null}
          onDragStart={ctxRef !== null ? (e) => setContextRefDrag(e, ctxRef) : undefined}
          style={{
            display: "flex",
            alignItems: "center",
            gap: 8,
            padding: "4px 8px",
            background: "transparent",
            color: "var(--text-primary)",
            border: "1px solid transparent",
            borderRadius: 4,
            cursor: "pointer",
            textAlign: "left",
            fontSize: 12,
          }}
        >
          <span style={{ flex: 1, minWidth: 0, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
            {entry.label}
          </span>
          {entry.subtitle ? (
            <span style={{ color: "var(--text-secondary)", fontSize: 11 }}>{entry.subtitle}</span>
          ) : null}
        </button>
        );
      })}
    </div>
  );
}
