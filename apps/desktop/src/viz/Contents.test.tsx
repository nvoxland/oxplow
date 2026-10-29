import { afterEach, expect, test } from "bun:test";
import { cleanup, render, waitFor } from "@testing-library/react";

import { Contents, CONTENTS_SUBSECTIONS_MAX } from "./Contents.js";

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

// A section that drew nothing (its viz had nothing to say) is hidden, so
// it isn't listed — and can't be the one marked as in view.
test("a section with nothing under its heading is not listed", async () => {
  const root = body(4);
  root.insertAdjacentHTML(
    "beforeend",
    `<section class="ox-section" data-title-hidden="true"><div class="ox-section-head ox-sr-only"><h2 class="ox-section-title ox-sr-only">YAML</h2></div></section>`,
  );
  const view = render(<Contents root={root} />);
  await waitFor(() => expect(view.getByTestId("viz-rail-contents")).toBeTruthy());
  const items = Array.from(view.getByTestId("viz-rail-contents").querySelectorAll("button")).map((b) => b.textContent);
  expect(items).toEqual(["Section 0", "Section 1", "Section 2", "Section 3"]);
});

test("a section folded shut is still listed", async () => {
  const root = body(3);
  root.insertAdjacentHTML(
    "beforeend",
    `<section class="ox-section" data-collapsed="true"><div class="ox-section-head"><h2 class="ox-section-title">Folded</h2></div></section>`,
  );
  const view = render(<Contents root={root} />);
  await waitFor(() => expect(view.getByTestId("viz-rail-contents")).toBeTruthy());
  const items = Array.from(view.getByTestId("viz-rail-contents").querySelectorAll("button")).map((b) => b.textContent);
  expect(items).toEqual(["Section 0", "Section 1", "Section 2", "Folded"]);
});

// A `sections` viz's subsections are listed under their parent, as the
// nested level; one with more than the cap is not listed at all.
function parentWith(subsections: number): HTMLElement {
  const root = document.createElement("div");
  root.insertAdjacentHTML("beforeend", `<section><h2 class="ox-section-title">Pages</h2><div data-sections="${subsections}"></div></section>`);
  const group = root.querySelector("[data-sections]")!;
  for (let i = 0; i < subsections; i++) {
    group.insertAdjacentHTML("beforeend", `<section class="ox-section" data-keep="true" data-level="3"><div class="ox-section-head"><h3 class="ox-section-title">Sub ${i}</h3></div></section>`);
  }
  document.body.appendChild(root);
  return root;
}

test("subsections are listed under their parent, one level in", async () => {
  const view = render(<Contents root={parentWith(4)} />);
  await waitFor(() => expect(view.getByTestId("viz-rail-contents")).toBeTruthy());
  const items = Array.from(view.getByTestId("viz-rail-contents").querySelectorAll("button"));
  expect(items.map((b) => b.textContent)).toEqual(["Pages", "Sub 0", "Sub 1", "Sub 2", "Sub 3"]);
  expect(items.map((b) => b.getAttribute("data-level"))).toEqual(["2", "3", "3", "3", "3"]);
});

test("at the cap they are all listed; past it, only the parent is", async () => {
  const at = render(<Contents root={parentWith(CONTENTS_SUBSECTIONS_MAX)} />);
  await waitFor(() => expect(at.getByTestId("viz-rail-contents")).toBeTruthy());
  expect(at.getByTestId("viz-rail-contents").querySelectorAll("button").length).toBe(CONTENTS_SUBSECTIONS_MAX + 1);
  cleanup();
  // The parent alone is one section: below CONTENTS_FROM, no Contents; add three more so it lists.
  const root = parentWith(CONTENTS_SUBSECTIONS_MAX + 1);
  for (const t of ["One", "Two", "Three"]) root.insertAdjacentHTML("beforeend", `<section><h2 class="ox-section-title">${t}</h2></section>`);
  const over = render(<Contents root={root} />);
  await waitFor(() => expect(over.getByTestId("viz-rail-contents")).toBeTruthy());
  const items = Array.from(over.getByTestId("viz-rail-contents").querySelectorAll("button")).map((b) => b.textContent);
  expect(items).toEqual(["Pages", "One", "Two", "Three"]);
});

test("a subsection that has only its heading is still a place to go to", async () => {
  const view = render(<Contents root={parentWith(4)} />);
  await waitFor(() => expect(view.getByTestId("viz-rail-contents")).toBeTruthy());
  expect(view.getByTestId("viz-rail-contents").textContent).toContain("Sub 3");
});
