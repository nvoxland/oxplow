import { expect, test } from "bun:test";

import { capabilitiesFromResult, chosenNote, nextChoices, offWithout } from "./capabilitiesModel.js";

const COLUMNS = [
  "capability", "provider", "extension", "features", "active", "title", "source",
  "available", "chosen_by", "capability_title", "choosable", "optional",
];
const result = (rows: unknown[][]) => ({ columns: COLUMNS, rows, truncated: false, reads: { models: [], tables: [], measures: [] }, freshness: [] }) as never;

// The choosable capabilities, each with its choices, what's active and why.
test("only choosable capabilities are listed, each with its choices", () => {
  const capabilities = capabilitiesFromResult(
    result([
      ["work_items", "oxplow", "oxplow-bundled", '{"hierarchy":true}', 0, "oxplow's tasks", "builtin", 1, null, "Work list", 1, 1],
      ["work_items", "none", null, "{}", 1, "None", "none", 1, "personal", "Work list", 1, 1],
      ["vcs", "git", null, "{}", 1, "git", "core", 1, "default", "Version control", 0, 0],
      ["snapshots", "oxplow", "oxplow-bundled", '{"contents":true}', 1, "Keep every version", "builtin", 1, "default", "Snapshots", 1, 0],
    ]),
  );
  expect(capabilities.map((p) => p.capability)).toEqual(["snapshots", "work_items"]);
  const work = capabilities.find((p) => p.capability === "work_items")!;
  expect(work).toMatchObject({ title: "Work list", optional: true, active: "none", chosenBy: "personal" });
  expect(work.choices.map((c) => c.id)).toEqual(["oxplow", "none"]);
  expect(work.choices[0]!.features).toEqual(["hierarchy"]);
});

// Why the active one is active, in a person's words — a fallback names
// what was chosen and isn't there.
test("the note says why the active one is active", () => {
  const base = { capability: "work_items", title: "Work list", optional: true, choices: [] };
  expect(chosenNote({ ...base, active: "none", chosenBy: "personal", unavailable: null })).toBe("Your own choice.");
  expect(chosenNote({ ...base, active: "oxplow", chosenBy: "project", unavailable: null })).toBe("The project's choice.");
  expect(chosenNote({ ...base, active: "oxplow", chosenBy: "default", unavailable: null })).toBe("The default.");
  expect(chosenNote({ ...base, active: "none", chosenBy: "fallback", unavailable: "issues" })).toBe(
    "`issues` was chosen but isn't available (its extension is disabled or its instance isn't running), so it's none.",
  );
});

// Choosing writes one capability's entry; the default (or "same as the
// project") removes it, and nothing left unsets the key.
test("a choice changes only its capability's entry", () => {
  expect(nextChoices({ effort_policy: "none" }, "work_items", "issues")).toEqual({ effort_policy: "none", work_items: "issues" });
  expect(nextChoices({ effort_policy: "none", work_items: "issues" }, "work_items", null)).toEqual({ effort_policy: "none" });
  expect(nextChoices({ work_items: "issues" }, "work_items", null)).toBeNull();
});

// What "none" turns off: every lens and hint that needs the capability.
test("none names what needs the capability", () => {
  const extensions = [
    {
      enabled: true,
      lenses: [
        { title: "Ready Tasks", needs: ["work_items"] },
        { title: "Coverage", needs: [] },
        { title: "Contents", needs: ["snapshots.contents"] },
      ],
      advisories: [{ id: "landed-in-progress", needs: ["work_items"] }],
    },
    { enabled: false, lenses: [{ title: "Off", needs: ["work_items"] }], advisories: [] },
  ] as never;
  expect(offWithout("work_items", extensions)).toEqual(["Ready Tasks", "the landed-in-progress hint"]);
  expect(offWithout("snapshots", extensions)).toEqual(["Contents"]);
});
