/**
 * The icons a resource, a page or an action may name — the Rust `ICONS`
 * allowlist (`extension_resources.rs`). Names only, with no React, so what
 * runs without the app (the component bridge) can check a name; `icons.ts`
 * maps each to what draws it, typed by these so the two stay one list.
 */
export const ICON_NAMES = [
  "activity",
  "archive",
  "at-sign",
  "bar-chart-3",
  "bell",
  "book-open",
  "box",
  "bug",
  "calendar",
  "check-square",
  "circle-check",
  "circle-dot",
  "circle-stop",
  "columns-3",
  "copy",
  "database",
  "download",
  "external-link",
  "file-text",
  "flag",
  "folder",
  "folder-tree",
  "git-branch",
  "git-commit",
  "git-compare",
  "git-merge",
  "git-pull-request",
  "history",
  "inbox",
  "layout-dashboard",
  "link",
  "list-tree",
  "maximize-2",
  "message-square",
  "package",
  "pencil",
  "play",
  "plus",
  "refresh-cw",
  "server",
  "settings",
  "shield",
  "sparkles",
  "star",
  "tag",
  "terminal",
  "ticket",
  "trash-2",
  "triangle-alert",
  "user",
  "zap",
] as const;

export type IconName = (typeof ICON_NAMES)[number];

const NAMES: ReadonlySet<string> = new Set(ICON_NAMES);

/** Whether `name` is an icon oxplow draws. */
export function isIconName(name: string): name is IconName {
  return NAMES.has(name);
}
