import { useMemo, useState } from "react";
import { useCollapsedSet } from "../../tabs/useViewParam.js";
import type { ReactNode } from "react";

export type HierarchyStatus = "added" | "modified" | "deleted";

/** Optional row-right metrics rendered as a stacked status bar
 *  plus +/− line columns. When supplied, HierarchyView replaces
 *  the status-badge cluster on that row with a colored bar; when
 *  any row in the tree carries metrics the bar widths scale
 *  against the largest visible row. */
export interface HierarchyMetrics {
  /** File / function counts per status. Drives the stacked bar
   *  segments (green = added, yellow = modified, red = deleted). */
  added: number;
  modified: number;
  deleted: number;
  /** Line counts for the +/− columns. */
  additions: number;
  deletions: number;
}

export interface HierarchyNode {
  /** Stable id for keying + collapse-state lookups. Must be unique
   *  across the entire tree (callers usually prefix with the parent
   *  id to ensure uniqueness). */
  id: string;
  /** Visible label. Search matches against this. */
  label: string;
  /** Optional left-side icon (folder, file, fn, etc.). Should render
   *  at the same line-height as the label so it doesn't force the
   *  row taller — the wrapper sets `width: 1em; height: 1em`. */
  icon?: ReactNode;
  /** Status badges shown after the icon. Branch nodes pass the union
   *  of their descendants' statuses. */
  statuses?: Set<HierarchyStatus>;
  /** Muted text rendered after the label. */
  detail?: string;
  /** Numeric count rendered in muted parens after the detail. Branch
   *  nodes typically pass their descendant count. */
  count?: number;
  /** When supplied, the label area becomes a clickable button. */
  onDrill?(e: React.MouseEvent): void;
  /** Hover-title for the drill button. */
  drillTitle?: string;
  /** Test-id for the row. */
  testId?: string;
  /** Optional override for the label color. Used by callers that
   *  want to color-code rows (e.g. by function visibility). */
  labelColor?: string;
  /** Per-row tree-table metrics: status counts (drives stacked
   *  bar) + line-level +/− totals. When a tree has any metrics-
   *  bearing rows, every row reserves space for the bar so the
   *  columns line up. */
  metrics?: HierarchyMetrics;
  children: HierarchyNode[];
}

export interface HierarchyViewProps {
  nodes: HierarchyNode[];
  /** Toolbar visibility. Defaults to true. */
  showToolbar?: boolean;
  searchPlaceholder?: string;
  /** Optional secondary content rendered at the right side of the
   *  toolbar (e.g. "N of M files" indicator). */
  toolbarExtra?: ReactNode;
  /** Test-id prefix for inputs / buttons. Defaults to `hierarchy`. */
  testIdPrefix?: string;
  /** Empty-state message when `nodes` is empty (or after a search
   *  filter prunes everything). */
  emptyLabel?: string;
}

/**
 * Generic hierarchical viewer used by Change Analysis's File-list and
 * Semantic views. Owns the toolbar (filter + Expand-all + Collapse-
 * all), the chevron toggle, and the status-badge rendering. Callers
 * shape their data into `HierarchyNode[]` once and the rest is
 * uniform.
 */
export function HierarchyView({
  nodes,
  showToolbar = true,
  searchPlaceholder = "Filter…",
  toolbarExtra,
  testIdPrefix = "hierarchy",
  emptyLabel = "Nothing to show.",
}: HierarchyViewProps) {
  const [search, setSearch] = useState("");
  // Collapse-state is stored as user *overrides* on top of the
  // computed default. The default collapses every branch deeper than
  // the first nested layer (top-level nodes are expanded so their
  // direct children render; grandchildren start collapsed). Storing
  // overrides — rather than the absolute collapsed set — means that
  // when the data refreshes with new ids, those new ids automatically
  // pick up the default state instead of leaking into "expanded".
  // They are kept as two sets in this step's history state: the rows the
  // person closed and the rows they opened.
  const closed = useCollapsedSet(`${testIdPrefix}.closed`);
  const opened = useCollapsedSet(`${testIdPrefix}.opened`);

  const filtered = useMemo(() => {
    if (!search.trim()) return nodes;
    const needle = search.toLowerCase();
    return nodes.map((n) => filterTree(n, needle)).filter((n): n is HierarchyNode => n != null);
  }, [nodes, search]);

  const allIds = useMemo(() => collectAllIds(filtered), [filtered]);
  const defaultCollapsed = useMemo(() => collectDefaultCollapsedIds(filtered), [filtered]);
  // Largest row total in the tree. Bars get a fixed-width track on
  // every row, but the colored fill inside the track scales to
  // (row.total / barMax) so a row with fewer changes shows a
  // shorter colored portion. Zero means no row carries metrics
  // and the slot is hidden entirely.
  const barMax = useMemo(() => collectBarMax(filtered), [filtered]);

  // Merge default + overrides into the effective collapsed set.
  const effectivelyCollapsed = useMemo(() => {
    if (search.trim()) return new Set<string>(); // search forces all expanded
    const out = new Set<string>(defaultCollapsed);
    for (const id of opened.ids) out.delete(id);
    for (const id of closed.ids) out.add(id);
    return out;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [defaultCollapsed, opened.ids, closed.ids, search]);

  const toggle = (id: string) => {
    if (effectivelyCollapsed.has(id)) {
      closed.set(closed.ids.filter((x) => x !== id));
      opened.set([...opened.ids, id]);
    } else {
      opened.set(opened.ids.filter((x) => x !== id));
      closed.set([...closed.ids, id]);
    }
  };

  const expandAll = () => {
    // Mark every branch as user-expanded so they override any default-
    // collapsed entries.
    closed.set([]);
    opened.set([...allIds]);
  };
  const collapseAll = () => {
    opened.set([]);
    closed.set([...allIds]);
  };

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
      {showToolbar ? (
        <div style={toolbarRow}>
          <input
            type="text"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            placeholder={searchPlaceholder}
            data-testid={`${testIdPrefix}-search`}
            style={searchInput}
          />
          <button
            type="button"
            data-testid={`${testIdPrefix}-expand-all`}
            onClick={expandAll}
            disabled={!!search.trim()}
            title={search.trim() ? "Search forces all rows expanded" : "Expand all branches"}
            style={smallButton}
          >
            Expand all
          </button>
          <button
            type="button"
            data-testid={`${testIdPrefix}-collapse-all`}
            onClick={collapseAll}
            disabled={!!search.trim()}
            title={search.trim() ? "Search forces all rows expanded" : "Collapse all branches"}
            style={smallButton}
          >
            Collapse all
          </button>
          {toolbarExtra ? <div style={{ marginLeft: "auto" }}>{toolbarExtra}</div> : null}
        </div>
      ) : null}
      {filtered.length === 0 ? (
        <div style={emptyStyle}>{emptyLabel}</div>
      ) : (
        <div data-testid={`${testIdPrefix}-tree`} style={{ display: "flex", flexDirection: "column" }}>
          {filtered.map((node) => (
            <Branch
              key={node.id}
              node={node}
              depth={0}
              collapsed={effectivelyCollapsed}
              onToggle={toggle}
              barMax={barMax}
            />
          ))}
        </div>
      )}
    </div>
  );
}

function Branch({
  node,
  depth,
  collapsed,
  onToggle,
  barMax,
}: {
  node: HierarchyNode;
  depth: number;
  collapsed: Set<string>;
  onToggle(id: string): void;
  barMax: number;
}) {
  const isLeaf = node.children.length === 0;
  const expanded = !collapsed.has(node.id);
  const hasMetrics = barMax > 0;
  return (
    <>
      <div data-testid={node.testId} style={rowOuter}>
        {hasMetrics ? <MetricsLeftSlot metrics={node.metrics} barMax={barMax} /> : null}
        <div style={contentArea(depth)}>
          {isLeaf ? (
            <span style={chevronSpacer} aria-hidden />
          ) : (
            <ChevronToggle expanded={expanded} onClick={() => onToggle(node.id)} />
          )}
          {node.icon ? <span style={iconWrapper}>{node.icon}</span> : null}
          {!hasMetrics && node.statuses && node.statuses.size > 0 ? (
            <StatusBadges statuses={node.statuses} />
          ) : null}
          {node.onDrill ? (
            <button
              type="button"
              onClick={node.onDrill}
              title={node.drillTitle}
              style={node.labelColor ? { ...labelButton, color: node.labelColor } : labelButton}
            >
              {node.label}
            </button>
          ) : (
            <span style={node.labelColor ? { ...labelText, color: node.labelColor } : labelText}>
              {node.label}
            </span>
          )}
          {node.detail ? <span style={detailText}>{node.detail}</span> : null}
          {typeof node.count === "number" ? (
            <span style={countText}>({node.count})</span>
          ) : null}
        </div>
      </div>
      {!isLeaf && expanded
        ? node.children.map((child) => (
            <Branch
              key={child.id}
              node={child}
              depth={depth + 1}
              collapsed={collapsed}
              onToggle={onToggle}
              barMax={barMax}
            />
          ))
        : null}
    </>
  );
}

function MetricsLeftSlot({
  metrics,
  barMax,
}: {
  metrics: HierarchyMetrics | undefined;
  barMax: number;
}) {
  // Always reserve the same horizontal real-estate so rows without
  // metrics line up cleanly with rows that have them.
  if (!metrics) {
    return (
      <div style={metricsLeftWrap}>
        <span style={addColStyle} />
        <span style={delColStyle} />
        <div style={barTrackStyle} />
      </div>
    );
  }
  const total = metrics.added + metrics.modified + metrics.deleted;
  // Track width is fixed; the COLORED FILL inside the track is sized
  // to (total / barMax) so a row with fewer changes shows a shorter
  // colored portion. Status segments split the colored fill
  // proportionally.
  const totalPct = barMax === 0 ? 0 : (total / barMax) * 100;
  const addPct = total === 0 ? 0 : (metrics.added / total) * 100;
  const modPct = total === 0 ? 0 : (metrics.modified / total) * 100;
  const delPct = total === 0 ? 0 : (metrics.deleted / total) * 100;
  return (
    <div style={metricsLeftWrap}>
      <span style={addColStyle}>{metrics.additions > 0 ? `+${metrics.additions}` : ""}</span>
      <span style={delColStyle}>{metrics.deletions > 0 ? `−${metrics.deletions}` : ""}</span>
      <div
        style={barTrackStyle}
        title={`${metrics.added} added · ${metrics.modified} modified · ${metrics.deleted} deleted`}
      >
        <div style={{ display: "flex", height: "100%", width: `${totalPct}%` }}>
          {metrics.added > 0 ? (
            <span style={{ width: `${addPct}%`, background: "var(--text-success, #16a34a)" }} />
          ) : null}
          {metrics.modified > 0 ? (
            <span style={{ width: `${modPct}%`, background: "var(--text-warning, #d97706)" }} />
          ) : null}
          {metrics.deleted > 0 ? (
            <span style={{ width: `${delPct}%`, background: "var(--text-danger, #dc2626)" }} />
          ) : null}
        </div>
      </div>
    </div>
  );
}

function ChevronToggle({ expanded, onClick }: { expanded: boolean; onClick(): void }) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-expanded={expanded}
      aria-label={expanded ? "Collapse" : "Expand"}
      title={expanded ? "Collapse" : "Expand"}
      style={chevronButton}
    >
      <svg
        width="0.75em"
        height="0.75em"
        viewBox="0 0 10 10"
        aria-hidden
        style={{
          transform: expanded ? "rotate(90deg)" : "rotate(0deg)",
          transition: "transform 120ms ease",
          display: "block",
        }}
      >
        <path
          d="M3 1.5 L7 5 L3 8.5"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.6"
          strokeLinecap="round"
          strokeLinejoin="round"
        />
      </svg>
    </button>
  );
}

export function StatusBadges({ statuses }: { statuses: Set<HierarchyStatus> }) {
  const order: HierarchyStatus[] = ["added", "modified", "deleted"];
  return (
    <span style={badgeRow}>
      {order
        .filter((s) => statuses.has(s))
        .map((s) => (
          <StatusBadge key={s} status={s} />
        ))}
    </span>
  );
}

export function StatusBadge({ status }: { status: HierarchyStatus }) {
  const cfg = STATUS_STYLE[status];
  return (
    <span title={cfg.title} data-testid={`hierarchy-status-${status}`} style={badgeStyle(cfg)}>
      {cfg.glyph}
    </span>
  );
}

const STATUS_STYLE: Record<HierarchyStatus, { glyph: string; fg: string; bg: string; title: string }> = {
  added: {
    glyph: "A",
    fg: "var(--text-success, #16a34a)",
    bg: "rgba(22, 163, 74, 0.18)",
    title: "Added",
  },
  modified: {
    glyph: "M",
    fg: "var(--text-link, #2563eb)",
    bg: "rgba(37, 99, 235, 0.18)",
    title: "Modified",
  },
  deleted: {
    glyph: "D",
    fg: "var(--text-danger, #dc2626)",
    bg: "rgba(220, 38, 38, 0.18)",
    title: "Deleted",
  },
};

/** Filter the tree to nodes whose label matches `needle` plus their
 *  ancestors. Returns null when neither the node nor any descendant
 *  matches. */
function filterTree(node: HierarchyNode, needle: string): HierarchyNode | null {
  const selfMatch = node.label.toLowerCase().includes(needle);
  const matchedChildren = node.children
    .map((c) => filterTree(c, needle))
    .filter((c): c is HierarchyNode => c != null);
  if (!selfMatch && matchedChildren.length === 0) return null;
  return { ...node, children: matchedChildren };
}

function collectBarMax(nodes: HierarchyNode[]): number {
  let max = 0;
  const walk = (list: HierarchyNode[]) => {
    for (const n of list) {
      if (n.metrics) {
        const total = n.metrics.added + n.metrics.modified + n.metrics.deleted;
        if (total > max) max = total;
      }
      if (n.children.length > 0) walk(n.children);
    }
  };
  walk(nodes);
  return max;
}

function collectAllIds(nodes: HierarchyNode[], out: string[] = []): string[] {
  for (const n of nodes) {
    if (n.children.length > 0) {
      out.push(n.id);
      collectAllIds(n.children, out);
    }
  }
  return out;
}

/**
 * Collect ids of branch nodes that should start collapsed by default —
 * every branch whose depth is >= 1. Top-level nodes (depth 0) stay
 * expanded so their direct children render; deeper nesting is hidden
 * until the user expands it.
 */
function collectDefaultCollapsedIds(
  nodes: HierarchyNode[],
  depth = 0,
  out: Set<string> = new Set(),
): Set<string> {
  for (const n of nodes) {
    if (n.children.length === 0) continue;
    if (depth >= 1) out.add(n.id);
    collectDefaultCollapsedIds(n.children, depth + 1, out);
  }
  return out;
}

const CHEVRON_GUTTER = 16;
const ROW_FONT_SIZE = 12;
const toolbarRow: React.CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 6,
};
const searchInput: React.CSSProperties = {
  flex: 1,
  minWidth: 120,
  maxWidth: 320,
  padding: "4px 8px",
  background: "var(--surface-app)",
  color: "var(--text-primary)",
  border: "1px solid var(--border-subtle)",
  borderRadius: 4,
  fontSize: "var(--text-xs)",
};
const smallButton: React.CSSProperties = {
  padding: "4px 10px",
  background: "var(--surface-card)",
  color: "var(--text-primary)",
  border: "1px solid var(--border-subtle)",
  borderRadius: 4,
  cursor: "pointer",
  fontSize: "var(--text-xs)",
};
const emptyStyle: React.CSSProperties = {
  color: "var(--text-muted)",
  fontSize: "var(--text-xs)",
  padding: 8,
};
const rowOuter: React.CSSProperties = {
  display: "flex",
  alignItems: "center",
  fontSize: ROW_FONT_SIZE,
  lineHeight: 1.3,
  padding: "1px 0",
  userSelect: "none",
  gap: 8,
};
const contentArea = (depth: number): React.CSSProperties => ({
  display: "flex",
  alignItems: "center",
  gap: 4,
  paddingLeft: depth * 12,
  flex: 1,
  minWidth: 0,
});
const metricsLeftWrap: React.CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 6,
  flexShrink: 0,
};
const chevronSpacer: React.CSSProperties = {
  display: "inline-block",
  width: CHEVRON_GUTTER,
  flexShrink: 0,
};
const chevronButton: React.CSSProperties = {
  display: "inline-flex",
  alignItems: "center",
  justifyContent: "center",
  width: CHEVRON_GUTTER,
  height: "1em",
  padding: 0,
  background: "transparent",
  border: "none",
  color: "var(--text-muted)",
  cursor: "pointer",
  flexShrink: 0,
  fontSize: "inherit",
  lineHeight: 1,
};
const iconWrapper: React.CSSProperties = {
  display: "inline-flex",
  alignItems: "center",
  justifyContent: "center",
  // Force the icon to render at the same height as the text so a
  // larger inline emoji / svg can't push the row taller than the
  // others (which would create uneven row heights).
  width: "1em",
  height: "1em",
  fontSize: "1em",
  lineHeight: 1,
  flexShrink: 0,
  color: "var(--text-muted)",
};
const labelButton: React.CSSProperties = {
  background: "transparent",
  border: "none",
  padding: 0,
  margin: 0,
  cursor: "pointer",
  color: "var(--text-primary)",
  fontSize: "inherit",
  fontFamily: "inherit",
  textAlign: "left",
};
const labelText: React.CSSProperties = {
  color: "var(--text-primary)",
};
const detailText: React.CSSProperties = {
  color: "var(--text-muted)",
  marginLeft: 4,
};
const countText: React.CSSProperties = {
  color: "var(--text-muted)",
  fontSize: 11,
  marginLeft: 4,
};
const badgeRow: React.CSSProperties = {
  display: "inline-flex",
  gap: 2,
  flexShrink: 0,
};
const barTrackStyle: React.CSSProperties = {
  width: 100,
  height: 8,
  background: "var(--surface-app)",
  borderRadius: 4,
  overflow: "hidden",
  flexShrink: 0,
};
const addColStyle: React.CSSProperties = {
  color: "var(--text-success, #16a34a)",
  width: 44,
  textAlign: "right",
  fontSize: 11,
  fontFamily: "var(--font-mono)",
  flexShrink: 0,
};
const delColStyle: React.CSSProperties = {
  color: "var(--text-danger, #dc2626)",
  width: 44,
  textAlign: "right",
  fontSize: 11,
  fontFamily: "var(--font-mono)",
  flexShrink: 0,
};
const badgeStyle = (cfg: { fg: string; bg: string }): React.CSSProperties => ({
  display: "inline-flex",
  alignItems: "center",
  justifyContent: "center",
  // Pixel sizing — em-relative was rendering at ~10px and the
  // badges effectively disappeared. Fixed 14×14 with 10px glyph
  // matches the row's 12px line height without growing it.
  width: 14,
  height: 14,
  borderRadius: 3,
  fontSize: 10,
  fontWeight: "var(--weight-bold)",
  color: cfg.fg,
  background: cfg.bg,
  lineHeight: 1,
  flexShrink: 0,
});

