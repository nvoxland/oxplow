import { afterEach, expect, test } from "bun:test";
import { cleanup, render, waitFor } from "@testing-library/react";

import { Contents } from "./Contents.js";

// A page's Contents lists its markdown headings, and its sections'
// headings once it has four or more.

afterEach(cleanup);

function body(sections: number, markdown = false): HTMLElement {
  const root = document.createElement("div");
  if (markdown) root.innerHTML = `<div class="oxplow-md"><h2>Intro</h2></div>`;
  for (let i = 0; i < sections; i++) {
    root.insertAdjacentHTML("beforeend", `<section data-section="s${i}"><h2 class="ox-section-title">Section ${i}</h2></section>`);
  }
  document.body.appendChild(root);
  return root;
}

test("four or more sections are listed, in order, with the markdown headings", async () => {
  const view = render(<Contents root={body(4, true)} />);
  await waitFor(() => expect(view.getByTestId("viz-rail-contents")).toBeTruthy());
  const items = Array.from(view.getByTestId("viz-rail-contents").querySelectorAll("button")).map((b) => b.textContent);
  expect(items).toEqual(["Intro", "Section 0", "Section 1", "Section 2", "Section 3"]);
});

test("three sections are not; a page of them shows no Contents", () => {
  const view = render(<Contents root={body(3)} />);
  expect(view.queryByTestId("viz-rail-contents")).toBeNull();
});

test("a section whose heading is visually hidden is still listed, in order", async () => {
  const root = body(3);
  root.querySelector("section")!.insertAdjacentHTML("beforebegin", `<section><h2 class="ox-section-title ox-sr-only">Hidden</h2></section>`);
  const view = render(<Contents root={root} />);
  await waitFor(() => expect(view.getByTestId("viz-rail-contents")).toBeTruthy());
  const items = Array.from(view.getByTestId("viz-rail-contents").querySelectorAll("button")).map((b) => b.textContent);
  expect(items).toEqual(["Hidden", "Section 0", "Section 1", "Section 2"]);
});
