import { expect, test } from "bun:test";
import { settingsAreaEntries } from "./sections.js";
import { settingsRef } from "../../tabs/pageRefs.js";

test("each Settings area is a System launcher entry that opens its area", () => {
  const entries = settingsAreaEntries();
  const agents = entries.find((e) => e.id === "settings-agents");
  expect(agents?.ref).toEqual(settingsRef("agents"));
  expect(agents?.category).toBe("System");
  expect(entries.find((e) => e.id === "settings-overview")).toBeUndefined();
});
