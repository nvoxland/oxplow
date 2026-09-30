import { afterEach, expect, test } from "bun:test";
import { cleanup, fireEvent, render } from "@testing-library/react";

import { SchemaForm } from "./SchemaForm.js";

afterEach(cleanup);

const schema = {
  type: "object",
  required: ["title"],
  properties: {
    title: { type: "string" },
    count: { type: ["integer", "null"] },
  },
};

test("submit waits for a valid value; Enter submits it", () => {
  const submitted: unknown[] = [];
  const { getByTestId } = render(
    <SchemaForm schema={schema} onSubmit={(v) => submitted.push(v)} submitLabel="Create" testIdPrefix="f" />,
  );
  const submit = getByTestId("f-submit") as HTMLButtonElement;
  expect(submit.disabled).toBe(true);
  fireEvent.change(getByTestId("f-title"), { target: { value: "Fix it" } });
  fireEvent.change(getByTestId("f-count"), { target: { value: "2" } });
  expect(submit.disabled).toBe(false);
  fireEvent.submit(getByTestId("f"));
  expect(submitted).toEqual([{ title: "Fix it", count: 2 }]);
});

test("a field's problem shows under it and keeps submit disabled", () => {
  const { getByTestId } = render(<SchemaForm schema={schema} onSubmit={() => {}} testIdPrefix="f" />);
  fireEvent.change(getByTestId("f-title"), { target: { value: "x" } });
  fireEvent.change(getByTestId("f-count"), { target: { value: "1.5" } });
  expect(getByTestId("f-count-error").textContent).toContain("whole number");
  expect((getByTestId("f-submit") as HTMLButtonElement).disabled).toBe(true);
});

test("Escape returns to the initial value; onChange reports each edit", () => {
  const seen: unknown[] = [];
  const { getByTestId } = render(
    <SchemaForm schema={schema} initial={{ title: "Start" }} onChange={(v) => seen.push(v)} testIdPrefix="f" />,
  );
  const title = getByTestId("f-title") as HTMLInputElement;
  fireEvent.change(title, { target: { value: "" } });
  expect(seen.at(-1)).toBe(null);
  fireEvent.keyDown(title, { key: "Escape" });
  expect(title.value).toBe("Start");
  expect(seen.at(-1)).toEqual({ title: "Start" });
});
