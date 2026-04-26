import type { BacklinkEntry } from "./backlinksIndex.js";
import type { TabRef } from "./tabState.js";

export interface BacklinksListProps {
  entries: BacklinkEntry[];
  onOpenPage(ref: TabRef): void;
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
      {entries.map((entry) => (
        <button
          key={entry.ref.id}
          type="button"
          data-testid={`backlinks-entry-${entry.ref.id}`}
          onClick={() => onOpenPage(entry.ref)}
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
      ))}
    </div>
  );
}
