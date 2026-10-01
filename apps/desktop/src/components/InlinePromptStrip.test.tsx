import { afterEach, expect, test } from "bun:test";
import { cleanup, fireEvent, render } from "@testing-library/react";

import { InlinePromptStrip } from "./InlinePromptStrip.js";

afterEach(cleanup);

const fields = [
  { key: "kind", initialValue: "relates_to" },
  { key: "body", placeholder: "Say it", multiline: true },
];

test("Enter submits every field's value; it needs each one filled", () => {
  const submitted: Array<Record<string, string>> = [];
  const view = render(
    <InlinePromptStrip testId="p" message="M" fields={fields} confirmLabel="Go" onSubmit={(v) => void submitted.push(v)} onCancel={() => {}} />,
  );
  const submit = view.getByTestId("p-submit") as HTMLButtonElement;
  expect(submit.disabled).toBe(true);
  fireEvent.change(view.getByTestId("p-body"), { target: { value: "  hi  " } });
  expect(submit.disabled).toBe(false);
  fireEvent.submit(submit.form!);
  expect(submitted).toEqual([{ kind: "relates_to", body: "hi" }]);
});

test("Cmd/Ctrl+Enter submits from a multiline field; plain Enter doesn't", () => {
  const submitted: Array<Record<string, string>> = [];
  const view = render(
    <InlinePromptStrip testId="p" message="M" fields={fields} confirmLabel="Go" onSubmit={(v) => void submitted.push(v)} onCancel={() => {}} />,
  );
  fireEvent.change(view.getByTestId("p-body"), { target: { value: "x" } });
  fireEvent.keyDown(view.getByTestId("p-body"), { key: "Enter" });
  expect(submitted).toEqual([]);
  fireEvent.keyDown(view.getByTestId("p-body"), { key: "Enter", ctrlKey: true });
  expect(submitted).toEqual([{ kind: "relates_to", body: "x" }]);
});

test("Escape in any field cancels", () => {
  let canceled = 0;
  const view = render(
    <InlinePromptStrip testId="p" message="M" fields={fields} confirmLabel="Go" onSubmit={() => {}} onCancel={() => canceled++} />,
  );
  fireEvent.keyDown(view.getByTestId("p-kind"), { key: "Escape" });
  fireEvent.keyDown(view.getByTestId("p-body"), { key: "Escape" });
  expect(canceled).toBe(2);
});
