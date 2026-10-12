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

// Escape inside a form (or a command's confirmation) is that control's own
// cancel: it stops there, so a container's Escape — the Answers strip
// collapsing — doesn't also fire.
test("Escape in the form, or in a confirmation, stays inside it", async () => {
  const { CommandConfirm } = await import("../CommandConfirm.js");
  let outer = 0;
  const { getByTestId } = render(
    <div onKeyDown={(e) => e.key === "Escape" && outer++}>
      <SchemaForm schema={schema} onSubmit={() => {}} testIdPrefix="f" />
      <CommandConfirm label="Finish" command="oxplow.work_item.transition" onConfirm={() => {}} onCancel={() => {}} testIdPrefix="c" />
    </div>,
  );
  fireEvent.keyDown(getByTestId("f-title"), { key: "Escape" });
  fireEvent.keyDown(getByTestId("c-run"), { key: "Escape" });
  expect(outer).toBe(0);
});

// tsk1054: a host re-reading its data passes a fresh copy of the same
// schema and starting value; that's no reason to throw away an edit. A
// different starting value is.
test("an edit survives a fresh copy of the same starting value", () => {
  const view = render(<SchemaForm schema={schema} initial={{ title: "Start" }} testIdPrefix="f" />);
  const title = () => view.getByTestId("f-title") as HTMLInputElement;
  fireEvent.change(title(), { target: { value: "Edited" } });
  view.rerender(<SchemaForm schema={structuredClone(schema)} initial={{ title: "Start" }} testIdPrefix="f" />);
  expect(title().value).toBe("Edited");
  view.rerender(<SchemaForm schema={schema} initial={{ title: "Saved elsewhere" }} testIdPrefix="f" />);
  expect(title().value).toBe("Saved elsewhere");
});

const wide = {
  type: "object",
  required: ["title"],
  properties: {
    title: { type: "string" },
    note: { type: "string" },
    ref: { type: "string" },
  },
};

test("`fields` renders only those, in that order; the others are still submitted", () => {
  const submitted: unknown[] = [];
  const { getByTestId, queryByTestId, container } = render(
    <SchemaForm
      schema={wide}
      initial={{ title: "T", note: "hidden", ref: "r:1" }}
      fields={["note", "title", "nope"]}
      onSubmit={(v) => submitted.push(v)}
      testIdPrefix="f"
    />,
  );
  expect(queryByTestId("f-ref")).toBe(null);
  const ids = [...container.querySelectorAll("input")].map((i) => i.dataset.testid);
  expect(ids).toEqual(["f-note", "f-title"]);
  fireEvent.change(getByTestId("f-note"), { target: { value: "shown" } });
  fireEvent.submit(getByTestId("f"));
  expect(submitted).toEqual([{ title: "T", note: "shown", ref: "r:1" }]);
});

test("a `fixed` field shows its value as text, is not an input, and is submitted unchanged", () => {
  const submitted: unknown[] = [];
  const { getByTestId, queryByTestId, container } = render(
    <SchemaForm schema={wide} initial={{ title: "T", ref: "r:1" }} fixed={["ref"]} onSubmit={(v) => submitted.push(v)} testIdPrefix="f" />,
  );
  expect(getByTestId("f-ref-fixed").textContent).toContain("r:1");
  expect(queryByTestId("f-ref")).toBe(null);
  // The first editable field takes the focus, not the fixed one above it.
  expect(document.activeElement).toBe(getByTestId("f-title"));
  expect(container.querySelectorAll("input").length).toBe(2);
  fireEvent.keyDown(getByTestId("f-title"), { key: "Escape" });
  fireEvent.submit(getByTestId("f"));
  expect(submitted).toEqual([{ title: "T", ref: "r:1" }]);
});

test("a server error shows under its field until that field is edited; submit stays enabled", () => {
  const view = render(
    <SchemaForm schema={wide} initial={{ title: "T" }} serverErrors={{ title: "taken" }} onSubmit={() => {}} testIdPrefix="f" />,
  );
  expect(view.getByTestId("f-title-error").textContent).toBe("taken");
  expect((view.getByTestId("f-submit") as HTMLButtonElement).disabled).toBe(false);
  // A re-read of the same errors keeps it cleared once edited.
  fireEvent.change(view.getByTestId("f-note"), { target: { value: "n" } });
  expect(view.queryByTestId("f-title-error")).not.toBe(null);
  fireEvent.change(view.getByTestId("f-title"), { target: { value: "T2" } });
  expect(view.queryByTestId("f-title-error")).toBe(null);
  view.rerender(
    <SchemaForm schema={wide} initial={{ title: "T" }} serverErrors={{ title: "taken" }} onSubmit={() => {}} testIdPrefix="f" />,
  );
  expect(view.queryByTestId("f-title-error")).toBe(null);
  view.rerender(
    <SchemaForm schema={wide} initial={{ title: "T" }} serverErrors={{ title: "taken again" }} onSubmit={() => {}} testIdPrefix="f" />,
  );
  expect(view.getByTestId("f-title-error").textContent).toBe("taken again");
});

test("a server error for a field that isn't shown appears once above the submit button", () => {
  const { getByTestId, queryByTestId } = render(
    <SchemaForm
      schema={wide}
      initial={{ title: "T", ref: "r:1" }}
      fields={["title"]}
      serverErrors={{ ref: "no such ref", zzz: "unknown" }}
      onSubmit={() => {}}
      testIdPrefix="f"
    />,
  );
  expect(queryByTestId("f-ref-error")).toBe(null);
  expect(getByTestId("f-form-error").textContent).toContain("no such ref");
  expect(getByTestId("f-form-error").textContent).toContain("unknown");
  expect((getByTestId("f-submit") as HTMLButtonElement).disabled).toBe(false);
});
