import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";

import { SETTINGS_AREAS, settingsArea, settingsSlot } from "./settingsAreas.js";

test("an unknown area is the overview", () => {
  expect(settingsArea("agents").title).toBe("Agents");
  expect(settingsArea("nope").id).toBe("overview");
  expect(settingsArea(null).id).toBe("overview");
});

// The ids an extension's mount names are the contract: core's SLOTS lists
// each area's parts slot with the same section ids, in the same order.
test("every area's slot and section ids match what core's SLOTS lists", () => {
  // rustfmt may break a call over lines: compare without whitespace or a
  // trailing comma.
  const bare = (text: string) => text.replace(/\s+/g, "").replace(/,\)/g, ")");
  const rust = bare(readFileSync(join(import.meta.dir, "..", "..", "..", "..", "crates", "oxplow-app", "src", "extensions.rs"), "utf8"));
  for (const area of SETTINGS_AREAS) {
    const ids = area.sections.map((s) => `"${s.id}"`).join(",");
    expect(rust.includes(bare(`SlotSpec::sections("${settingsSlot(area.id)}", &[], &[${ids}])`))).toBe(true);
  }
});
