import { afterEach, expect, test } from "bun:test";
import { cleanup, fireEvent, render } from "@testing-library/react";

import { OpenWith } from "./OpenWith.js";

afterEach(cleanup);

const EDITOR = { name: "oxplow-foundation/editor", title: "Editor" };
const BLAME = { name: "acme/blame", title: "Blame" };

test("with one renderer there is nothing to pick: it is hidden", () => {
  const view = render(<OpenWith options={[EDITOR]} current={EDITOR.name} pick={() => {}} />);
  expect(view.queryByTestId("page-nav-open-with")).toBeNull();
  const none = render(<OpenWith options={[]} current="" pick={() => {}} />);
  expect(none.queryByTestId("page-nav-open-with")).toBeNull();
});

test("with two, it shows the current one, and picking calls `pick` with `<extension>/<page>`", () => {
  const picked: string[] = [];
  const view = render(<OpenWith options={[EDITOR, BLAME]} current={EDITOR.name} pick={(n) => picked.push(n)} />);
  const select = view.getByTestId("page-nav-open-with") as HTMLSelectElement;
  expect(select.value).toBe("oxplow-foundation/editor");
  fireEvent.change(select, { target: { value: "acme/blame" } });
  expect(picked).toEqual(["acme/blame"]);
});
