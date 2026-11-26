import { expect, test } from "bun:test";

import { VIZ_ICON_NAMES, VIZ_TONES, cellIcon, vizIcon, toneColor } from "./vizIcons.js";
import { MISSING_PAGE_ICON, pageKindIconComponent } from "../pageKinds.js";
import { ICONS } from "../icons.js";
import type { Extension } from "../tauri-bridge/generated/bindings.js";

// A column's `icon` names one from a fixed vocabulary — the work
// glyphs, one per bucket (a query passes `state`), the page kinds' icons and the ref-kind
// icons; an unknown name draws nothing.
test("the icon vocabulary has the work glyphs, kinds and ref-kind icons; unknown names draw nothing", () => {
  expect(vizIcon("in_progress")).toEqual({ glyph: "◐", tone: "accent", label: "In progress" });
  expect(vizIcon("done")).toEqual({ glyph: "✓", tone: "success", label: "Done" });
  expect(vizIcon("blocked")).toEqual({ glyph: "⚠", tone: "danger", label: "Blocked" });
  expect(vizIcon("canceled")).toEqual({ glyph: "✗", tone: "muted", label: "Canceled" });
  expect(vizIcon("todo")).toEqual({ glyph: "☐", tone: null, label: "To do" });
  expect(vizIcon("backlog")).toEqual({ glyph: "◌", tone: "muted", label: "Backlog" });
  // The work glyphs are the buckets: a list's own stage names none.
  expect(vizIcon("ready")).toBeNull();
  expect(vizIcon("archived")).toBeNull();
  for (const name of ["epic", "task", "knowledge", "file", "commit", "bug", "git-pull-request"]) {
    expect(vizIcon(name), name).not.toBeNull();
    expect(VIZ_ICON_NAMES).toContain(name);
  }
  expect(vizIcon("nope")).toBeNull();
  expect(vizIcon(null)).toBeNull();
});

// A file's status draws the letter the file trees use (`shortStatus`).
test("the icon vocabulary has the file statuses, as the file trees letter them", () => {
  const letters = { added: "A", modified: "M", deleted: "D", renamed: "R", untracked: "U", conflicted: "C" };
  for (const [name, glyph] of Object.entries(letters)) {
    expect(vizIcon(name), name).toMatchObject({ glyph });
    expect(VIZ_ICON_NAMES).toContain(name);
  }
  expect(vizIcon("deleted")).toMatchObject({ tone: "danger", label: "Deleted" });
  expect(vizIcon("added")).toMatchObject({ tone: "success" });
});

// A tone is a theme variable, never a raw colour.
test("tones map to theme variables; anything else is plain", () => {
  expect(VIZ_TONES).toEqual(["accent", "success", "warning", "danger", "muted"]);
  for (const t of VIZ_TONES) expect(toneColor(t)).toMatch(/^var\(--[a-z-]+\)$/);
  expect(toneColor("chartreuse")).toBeNull();
  expect(toneColor(null)).toBeNull();
});

// Any page kind names its page's icon, so a row of pages (a
// bookmark, a visit) shows each one's own: an entity's, or the icon a
// declared page names.
test("a page kind draws its page's icon", () => {
  for (const kind of ["task", "dir"]) {
    const spec = vizIcon(kind);
    expect(spec, kind).not.toBeNull();
    expect(spec && "Icon" in spec ? spec.Icon : null, kind).toBe(pageKindIconComponent(kind));
  }
  const extensions = [
    { name: "oxplow-foundation", namespace: "oxplow", enabled: true, pageFiles: [{ id: "oxplow/git-dashboard", extension: "oxplow-foundation", slug: "git-dashboard", title: "Git", icon: "git-branch" }] },
  ] as unknown as Extension[];
  const spec = vizIcon("git-dashboard", extensions);
  expect(spec && "Icon" in spec ? spec.Icon : null).toBe(ICONS["git-branch"]!);
  expect(spec?.label).toBe("git");
});

// A cell that opens a page always has an icon: a kind the vocabulary
// doesn't know (a page no extension declares any more) draws the
// missing-page icon, so a list of pages (Go To) lines up. Any other column keeps
// "an unknown name draws nothing".
test("a cell that opens a page draws the missing-page icon when its kind is unknown", () => {
  const page = { kind: "page" } as const;
  const spec = cellIcon("assistant-settings", page);
  expect(spec && "Icon" in spec ? spec.Icon : null).toBe(MISSING_PAGE_ICON);
  expect(cellIcon(null, page)).not.toBeNull();
  expect(cellIcon("task", page)).toEqual(vizIcon("task"));
  expect(cellIcon("ready", null)).toBeNull();
  expect(cellIcon("ready", { kind: "file" })).toBeNull();
});

// A row about something oxplow ran (an agent session) marks it with
// oxplow's own ox, the app's logo.
test("the oxplow icon is the app's logo", () => {
  expect(VIZ_ICON_NAMES).toContain("oxplow");
  const spec = vizIcon("oxplow");
  expect(spec && "Icon" in spec ? spec.label : null).toBe("oxplow");
});
