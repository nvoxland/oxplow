import { afterEach, expect, test } from "bun:test";
import { cleanup, fireEvent, render } from "@testing-library/react";

import { InlineEdit } from "./InlineEdit.js";

afterEach(cleanup);

// Enter with the value unchanged commits nothing: it is a cancel, so a flow
// that opened the field (a rename) ends.
test("an unchanged value is a cancel, not a commit", () => {
  const seen: string[] = [];
  const view = render(<InlineEdit value="Writer" startEditing testId="f" onCommit={(v) => seen.push(`commit ${v}`)} onCancel={() => seen.push("cancel")} />);
  fireEvent.keyDown(view.getByTestId("f"), { key: "Enter" });
  expect(seen).toEqual(["cancel"]);
});

test("a changed value commits", () => {
  const seen: string[] = [];
  const view = render(<InlineEdit value="Writer" startEditing testId="f" onCommit={(v) => seen.push(`commit ${v}`)} onCancel={() => seen.push("cancel")} />);
  fireEvent.change(view.getByTestId("f"), { target: { value: "Editor" } });
  fireEvent.keyDown(view.getByTestId("f"), { key: "Enter" });
  expect(seen).toEqual(["commit Editor"]);
});
