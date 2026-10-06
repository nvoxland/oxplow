import { expect, test } from "bun:test";

import { LENS_ICON_NAMES, LENS_TONES, lensIcon, toneColor } from "./lensIcons.js";
import { pageKindIconComponent } from "../pageKinds.js";

// A column's `icon` names one from a fixed vocabulary — the work
// glyphs the rail draws today, the page kinds' icons and the ref-kind
// icons; an unknown name draws nothing.
test("the icon vocabulary has the work glyphs, kinds and ref-kind icons; unknown names draw nothing", () => {
  expect(lensIcon("in_progress")).toEqual({ glyph: "◐", tone: "accent", label: "In progress" });
  expect(lensIcon("done")).toEqual({ glyph: "✓", tone: "success", label: "Done" });
  expect(lensIcon("blocked")).toEqual({ glyph: "⚠", tone: "danger", label: "Blocked" });
  expect(lensIcon("canceled")).toEqual({ glyph: "✗", tone: "muted", label: "Canceled" });
  expect(lensIcon("ready")).toEqual({ glyph: "☐", tone: null, label: "Ready" });
  expect(lensIcon("todo")).toEqual(lensIcon("ready"));
  for (const name of ["epic", "task", "wiki", "file", "commit", "bug", "git-pull-request"]) {
    expect(lensIcon(name), name).not.toBeNull();
    expect(LENS_ICON_NAMES).toContain(name);
  }
  expect(lensIcon("nope")).toBeNull();
  expect(lensIcon(null)).toBeNull();
});

// A tone is a theme variable, never a raw colour.
test("tones map to theme variables; anything else is plain", () => {
  expect(LENS_TONES).toEqual(["accent", "success", "warning", "danger", "muted"]);
  for (const t of LENS_TONES) expect(toneColor(t)).toMatch(/^var\(--[a-z-]+\)$/);
  expect(toneColor("chartreuse")).toBeNull();
  expect(toneColor(null)).toBeNull();
});

// Any page kind names its page's icon, so a row of pages (a
// bookmark, a visit) shows each one's own.
test("a page kind draws its page's icon", () => {
  for (const kind of ["git-dashboard", "alerts", "work_item", "dir", "settings"]) {
    const spec = lensIcon(kind);
    expect(spec, kind).not.toBeNull();
    expect(spec && "Icon" in spec ? spec.Icon : null, kind).toBe(pageKindIconComponent(kind));
  }
});
