/**
 * The fixed vocabularies a lens draws from its rows (tsk1089): a column's
 * `icon` names one of `LENS_ICON_NAMES`, and its `tone` one of
 * `LENS_TONES`. Values come from the query, so the loader can't check them:
 * an unknown name draws no icon, an unknown tone leaves the cell plain.
 * See `.context/extensions.md` → Lenses.
 */
import { Layers, MessageSquare, type LucideIcon } from "lucide-react";
import type { CSSProperties, ReactElement } from "react";

import { pageKindIconComponent, pageKindLabel } from "../pageKinds.js";
import { REF_KIND_ICONS } from "../refKinds.js";

/** A semantic tone, drawn with a theme variable (`.context/theming.md`). */
export type LensTone = "accent" | "success" | "warning" | "danger" | "muted";

export const LENS_TONES: readonly LensTone[] = ["accent", "success", "warning", "danger", "muted"];

const TONE_VARS: Record<LensTone, string> = {
  accent: "var(--accent)",
  success: "var(--status-done)",
  warning: "var(--status-waiting)",
  danger: "var(--severity-critical)",
  muted: "var(--text-muted)",
};

/** The colour of a tone, or null for anything outside the vocabulary. */
export function toneColor(tone: unknown): string | null {
  return typeof tone === "string" && tone in TONE_VARS ? TONE_VARS[tone as LensTone] : null;
}

/** One icon: a work-status glyph (with the tone it draws in unless the
 *  column sets one), or a kind's icon. */
export type LensIconSpec =
  | { glyph: string; tone: LensTone | null; label: string }
  | { Icon: LucideIcon; tone: null; label: string };

/** The work-status glyphs, matching the rail's Work rows (and the status
 *  words `v_task` / `v_work_item` use, so a query can pass its status). */
const STATUS: Record<string, { glyph: string; tone: LensTone | null; label: string }> = {
  ready: { glyph: "☐", tone: null, label: "Ready" },
  todo: { glyph: "☐", tone: null, label: "Ready" },
  in_progress: { glyph: "◐", tone: "accent", label: "In progress" },
  blocked: { glyph: "⚠", tone: "danger", label: "Blocked" },
  done: { glyph: "✓", tone: "success", label: "Done" },
  canceled: { glyph: "✗", tone: "muted", label: "Canceled" },
  archived: { glyph: "▣", tone: "muted", label: "Archived" },
};

/** Kinds of thing, drawn with the icon their pages use. */
const KINDS: Record<string, { Icon: LucideIcon | null; label: string }> = {
  epic: { Icon: Layers, label: "Epic" },
  task: { Icon: pageKindIconComponent("work_item"), label: "Task" },
  wiki: { Icon: pageKindIconComponent("wiki"), label: "Wiki page" },
  file: { Icon: pageKindIconComponent("file"), label: "File" },
  folder: { Icon: pageKindIconComponent("dir"), label: "Folder" },
  commit: { Icon: pageKindIconComponent("commit"), label: "Commit" },
  diff: { Icon: pageKindIconComponent("diff"), label: "Diff" },
  lens: { Icon: pageKindIconComponent("lens"), label: "Lens" },
  metric: { Icon: pageKindIconComponent("metric"), label: "Metric" },
  dashboard: { Icon: pageKindIconComponent("dashboard"), label: "Dashboard" },
  comment: { Icon: MessageSquare, label: "Comment" },
};

/** Every name a column's `icon` may give: the status glyphs, the kinds,
 *  and the icons an extension's ref kind may name (`REF_KIND_ICONS`). */
export const LENS_ICON_NAMES: readonly string[] = [
  ...Object.keys(STATUS),
  ...Object.keys(KINDS),
  ...Object.keys(REF_KIND_ICONS).filter((n) => !(n in KINDS)),
];

/** The icon `name` draws, or null when it isn't in the vocabulary. Past
 *  the names above, any page kind draws its page's icon (tsk1101), so a
 *  row of pages — a bookmark, a visit — can pass its `page_kind`. */
export function lensIcon(name: unknown): LensIconSpec | null {
  if (typeof name !== "string") return null;
  const status = STATUS[name];
  if (status) return { ...status };
  const kind = KINDS[name];
  if (kind?.Icon) return { Icon: kind.Icon, tone: null, label: kind.label };
  const ref = REF_KIND_ICONS[name];
  if (ref) return { Icon: ref, tone: null, label: name };
  const page = pageKindIconComponent(name);
  return page ? { Icon: page, tone: null, label: pageKindLabel(name) } : null;
}

/** A cell's icon. `tone` (the column's) wins over the glyph's own. */
export function LensIcon({ spec, tone }: { spec: LensIconSpec; tone: string | null }): ReactElement {
  const color = tone ?? toneColor(spec.tone) ?? "var(--text-secondary)";
  const style: CSSProperties = { width: 14, display: "inline-flex", justifyContent: "center", flexShrink: 0, color };
  if ("glyph" in spec) {
    return (
      <span data-testid="lens-icon" role="img" aria-label={spec.label} title={spec.label} style={{ ...style, fontSize: "var(--text-xs)" }}>
        {spec.glyph}
      </span>
    );
  }
  const { Icon } = spec;
  return (
    <span data-testid="lens-icon" role="img" aria-label={spec.label} title={spec.label} style={style}>
      <Icon aria-hidden size={12} />
    </span>
  );
}
