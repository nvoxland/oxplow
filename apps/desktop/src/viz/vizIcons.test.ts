import { expect, test } from "bun:test";

import { VIZ_ICON_NAMES, VIZ_TONES, vizIcon, toneColor } from "./vizIcons.js";
import { pageKindIconComponent } from "../pageKinds.js";
import { ICONS } from "../icons.js";
import type { Extension } from "../tauri-bridge/generated/bindings.js";

// A column's `icon` names one from a fixed vocabulary — the work
// glyphs the rail draws today, the page kinds' icons and the ref-kind
// icons; an unknown name draws nothing.
test("the icon vocabulary has the work glyphs, kinds and ref-kind icons; unknown names draw nothing", () => {
  expect(vizIcon("in_progress")).toEqual({ glyph: "◐", tone: "accent", label: "In progress" });
  expect(vizIcon("done")).toEqual({ glyph: "✓", tone: "success", label: "Done" });
  expect(vizIcon("blocked")).toEqual({ glyph: "⚠", tone: "danger", label: "Blocked" });
  expect(vizIcon("canceled")).toEqual({ glyph: "✗", tone: "muted", label: "Canceled" });
  expect(vizIcon("ready")).toEqual({ glyph: "☐", tone: null, label: "Ready" });
  expect(vizIcon("todo")).toEqual(vizIcon("ready"));
  for (const name of ["epic", "task", "wiki", "file", "commit", "bug", "git-pull-request"]) {
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
  for (const kind of ["work_item", "dir"]) {
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
