import { expect, test } from "bun:test";

import { doneMessage, resultUri, undoneMessage } from "./commandToast.js";

// A toast says which command ran, by its label without the menu's
// decoration ("+ New", "New dashboard…"), and never "done": that reads as
// a task's state ("State: done." after moving one to in progress).
test("a command's toast names it plainly and never says done", () => {
  expect(doneMessage("State")).toBe("✓ State");
  expect(doneMessage("+ New")).toBe("✓ New");
  expect(doneMessage("New dashboard…")).toBe("✓ New dashboard");
  expect(doneMessage("Pin to Dashboard...")).toBe("✓ Pin to Dashboard");
  expect(undoneMessage("+ New")).toBe("Undone: New");
});

// A result that names a thing (a `uri`) is what an Open link goes to.
test("a result's uri is what the toast opens", () => {
  expect(resultUri({ uri: "task:t1", other: 1 })).toBe("task:t1");
  expect(resultUri({ uri: "" })).toBeNull();
  expect(resultUri({ uri: 5 })).toBeNull();
  expect(resultUri({ page: "my-pages/x" })).toBeNull();
  expect(resultUri(null)).toBeNull();
  expect(resultUri("task:t1")).toBeNull();
});
