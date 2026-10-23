import type { DashboardItem } from "../../api.js";
import type { TileOptions } from "../../pages/customDashboardData.js";
import type { MenuItem } from "../../menu.js";
import { InlineEdit } from "../InlineEdit.js";
import { useContextMenu } from "../useRowContextMenu.js";

/** The heading's look — one line, left-aligned, sized between the page h1 and a
 *  tile title so it reads as a divider between groups of tiles. */
const HEADING_STYLE: React.CSSProperties = {
  display: "block",
  textAlign: "left",
  fontSize: 15,
  fontWeight: 600,
  lineHeight: 1.4,
  color: "var(--text, #ddd)",
};

/** A heading band's text: edited in place with `InlineEdit` when it can be
 *  saved (`onCommit`), else plain. The custom-dashboard page's `TextTile`
 *  and a dashboard lens's text row tiles draw it. */
export function TextBand({ text, onCommit, testId }: { text: string; onCommit?: (next: string) => void; testId: string }) {
  if (!onCommit) {
    return (
      <span data-testid={testId} style={HEADING_STYLE}>
        {text}
      </span>
    );
  }
  return (
    <InlineEdit
      value={text}
      onCommit={onCommit}
      allowEmpty
      placeholder="Click to add a heading…"
      ariaLabel="Section heading"
      testId={testId}
      displayStyle={HEADING_STYLE}
      inputStyle={{ ...HEADING_STYLE, width: "100%" }}
    />
  );
}

/** A heading band's frame: no card, only a rule under the text. */
export const BAND_STYLE: React.CSSProperties = {
  // A band, not a panel: no card fill or border, just a rule under the
  // heading, and only as tall as the text itself.
  borderBottom: "1px solid var(--border-subtle)",
  padding: "4px 2px 6px",
  minWidth: 0,
  alignSelf: "start",
};

/**
 * A `text` dashboard tile (tsk142) — a **heading band** labelling the run of
 * tiles beneath it. The body is **plain text**, edited in place with
 * `InlineEdit` and rendered as a single left-aligned heading.
 *
 * It is deliberately NOT markdown (tsk147): rendering through `MarkdownView`
 * pulled in the `.oxplow-md` class, which self-caps at `78ch` and
 * `margin-inline: auto` outside a reading column — so the heading centred itself
 * in the band — and let a stray `##` restyle the whole row. A label above a
 * group of tiles doesn't need a document renderer.
 *
 * Also deliberately not a card: it defaults to the `full` size (spanning every
 * grid column), sizes to its own text height, and wears only a bottom rule. It
 * first shipped as a `wide` card inside a 260px-minimum grid row, which rendered
 * a one-line heading as a big empty panel and implied it *contained* the tiles
 * after it. It doesn't — grouping here is positional only.
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
      label: "Width",
      enabled: !!onConfigure,
      submenu: (["full", "wide", "small"] as const).map((s) => ({
        id: `size:${s}`,
        label: s === "full" ? "Full width" : s === "wide" ? "Two columns" : "One column",
        enabled: true,
        checked: (opts.size ?? "full") === s,
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
      style={BAND_STYLE}
    >
      <TextBand text={text} onCommit={(next) => onConfigure?.({ text: next })} testId={`text-tile-edit-${item.id}`} />
      {ctxMenu.menu}
    </section>
  );
}
