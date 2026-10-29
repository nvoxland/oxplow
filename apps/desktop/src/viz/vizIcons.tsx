/**
 * The fixed vocabularies a viz draws from its rows: a column's
 * `icon` names one of `VIZ_ICON_NAMES`, and its `tone` one of
 * `VIZ_TONES`. Values come from the query, so the loader can't check them:
 * an unknown name draws no icon, an unknown tone leaves the cell plain.
 * See `.context/extensions.md` → Viz nodes.
 */
import { Layers, MessageSquare, type LucideIcon } from "lucide-react";
import type { CSSProperties, ReactElement } from "react";

import { pageKindIconComponent, pageKindLabel } from "../pageKinds.js";
import type { Extension } from "../tauri-bridge/generated/bindings.js";
import { ICONS } from "../icons.js";

/** A semantic tone, drawn with a theme variable (`.context/theming.md`). */
export type VizTone = "accent" | "success" | "warning" | "danger" | "muted";

export const VIZ_TONES: readonly VizTone[] = ["accent", "success", "warning", "danger", "muted"];

const TONE_VARS: Record<VizTone, string> = {
  accent: "var(--accent)",
  success: "var(--status-done)",
  warning: "var(--status-waiting)",
  danger: "var(--severity-critical)",
  muted: "var(--text-muted)",
};

/** The colour of a tone, or null for anything outside the vocabulary. */
export function toneColor(tone: unknown): string | null {
  return typeof tone === "string" && tone in TONE_VARS ? TONE_VARS[tone as VizTone] : null;
}

/** One icon: a work-status glyph (with the tone it draws in unless the
 *  column sets one), or a kind's icon. */
export type VizIconSpec =
  | { glyph: string; tone: VizTone | null; label: string }
  | { Icon: LucideIcon; tone: null; label: string };

/** The work-status glyphs, matching the rail's Work rows (and the status
 *  states `v_work_item` uses, so a query can pass its state), and a file's
 *  change status (`v_file.status`). */
const STATUS: Record<string, { glyph: string; tone: VizTone | null; label: string }> = {
  ready: { glyph: "☐", tone: null, label: "Ready" },
  todo: { glyph: "☐", tone: null, label: "Ready" },
  in_progress: { glyph: "◐", tone: "accent", label: "In progress" },
  blocked: { glyph: "⚠", tone: "danger", label: "Blocked" },
  done: { glyph: "✓", tone: "success", label: "Done" },
  canceled: { glyph: "✗", tone: "muted", label: "Canceled" },
  archived: { glyph: "▣", tone: "muted", label: "Archived" },
  // A file's status, lettered as the file trees letter it (`shortStatus`).
  added: { glyph: "A", tone: "success", label: "Added" },
  modified: { glyph: "M", tone: "warning", label: "Modified" },
  deleted: { glyph: "D", tone: "danger", label: "Deleted" },
  renamed: { glyph: "R", tone: "accent", label: "Renamed" },
  untracked: { glyph: "U", tone: "muted", label: "Untracked" },
  conflicted: { glyph: "C", tone: "danger", label: "Conflicted" },
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
  viz: { Icon: pageKindIconComponent("viz"), label: "Viz" },
  metric: { Icon: pageKindIconComponent("metric"), label: "Metric" },
  dashboard: { Icon: pageKindIconComponent("dashboard"), label: "Dashboard" },
  comment: { Icon: MessageSquare, label: "Comment" },
};

/** Every name a column's `icon` may give: the status glyphs, the kinds,
 *  and the icons an extension's ref kind may name (`ICONS`). */
export const VIZ_ICON_NAMES: readonly string[] = [
  ...Object.keys(STATUS),
  ...Object.keys(KINDS),
  ...Object.keys(ICONS).filter((n) => !(n in KINDS)),
];

/** The icon `name` draws, or null when it isn't in the vocabulary. Past
 *  the names above, any page kind — a declared page's id among
 *  `extensions` too — draws its page's icon, so a row of pages — a
 *  bookmark, a visit — can pass its `page_kind`. */
export function vizIcon(name: unknown, extensions: Extension[] | null = null): VizIconSpec | null {
  if (typeof name !== "string") return null;
  const status = STATUS[name];
  if (status) return { ...status };
  const kind = KINDS[name];
  if (kind?.Icon) return { Icon: kind.Icon, tone: null, label: kind.label };
  const ref = ICONS[name];
  if (ref) return { Icon: ref, tone: null, label: name };
  const page = pageKindIconComponent(name, extensions);
  return page ? { Icon: page, tone: null, label: pageKindLabel(name, extensions) } : null;
}

/** A cell's icon. `tone` (the column's) wins over the glyph's own. */
export function VizIcon({ spec, tone }: { spec: VizIconSpec; tone: string | null }): ReactElement {
  const color = tone ?? toneColor(spec.tone) ?? "var(--text-secondary)";
  const style: CSSProperties = { width: 14, display: "inline-flex", justifyContent: "center", flexShrink: 0, color };
  if ("glyph" in spec) {
    return (
      <span data-testid="viz-icon" role="img" aria-label={spec.label} title={spec.label} style={{ ...style, fontSize: "var(--text-xs)" }}>
        {spec.glyph}
      </span>
    );
  }
  const { Icon } = spec;
  return (
    <span data-testid="viz-icon" role="img" aria-label={spec.label} title={spec.label} style={style}>
      <Icon aria-hidden size={12} />
    </span>
  );
}
