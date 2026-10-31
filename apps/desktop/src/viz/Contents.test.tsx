import { afterEach, expect, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";

import { Contents, CONTENTS_SUBSECTIONS_MAX } from "./Contents.js";

// A page's Contents lists its markdown headings, and its sections'
// headings once it has two or more.

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

test("sections are listed, in order, with the markdown headings", async () => {
  const view = render(<Contents root={body(4, true)} />);
  await waitFor(() => expect(view.getByTestId("viz-rail-contents")).toBeTruthy());
  const items = Array.from(view.getByTestId("viz-rail-contents").querySelectorAll("button")).map((b) => b.textContent);
  expect(items).toEqual(["Intro", "Section 0", "Section 1", "Section 2", "Section 3"]);
});

test("one section gets no Contents; two do", async () => {
  const one = render(<Contents root={body(1)} />);
  expect(one.queryByTestId("viz-rail-contents")).toBeNull();
  cleanup();
  const two = render(<Contents root={body(2)} />);
  await waitFor(() => expect(two.getByTestId("viz-rail-contents")).toBeTruthy());
  const items = Array.from(two.getByTestId("viz-rail-contents").querySelectorAll("button")).map((b) => b.textContent);
  expect(items).toEqual(["Section 0", "Section 1"]);
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

test("a sections viz of three rows lists all three under its parent", async () => {
  const view = render(<Contents root={parentWith(3)} />);
  await waitFor(() => expect(view.getByTestId("viz-rail-contents")).toBeTruthy());
  const items = Array.from(view.getByTestId("viz-rail-contents").querySelectorAll("button")).map((b) => b.textContent);
  expect(items).toEqual(["Pages", "Sub 0", "Sub 1", "Sub 2"]);
});

test("a parent with one subsection is two sections: Contents shows", async () => {
  const view = render(<Contents root={parentWith(1)} />);
  await waitFor(() => expect(view.getByTestId("viz-rail-contents")).toBeTruthy());
});

test("past the cap, the first ten are listed, then one '… N more' that scrolls to the parent", async () => {
  expect(CONTENTS_SUBSECTIONS_MAX).toBe(10);
  const root = parentWith(14);
  root.insertAdjacentHTML("beforeend", `<section><h2 class="ox-section-title">After</h2></section>`);
  const view = render(<Contents root={root} />);
  await waitFor(() => expect(view.getByTestId("viz-rail-contents")).toBeTruthy());
  const buttons = Array.from(view.getByTestId("viz-rail-contents").querySelectorAll("button"));
  expect(buttons.map((b) => b.textContent)).toEqual([
    "Pages",
    ...Array.from({ length: 10 }, (_, i) => `Sub ${i}`),
    "… 4 more",
    "After",
  ]);
  const parent = root.querySelector<HTMLElement>(".ox-section-title")!;
  let scrolled: Element | null = null;
  parent.scrollIntoView = function (this: HTMLElement) {
    scrolled = this;
  };
  fireEvent.click(buttons[11]!);
  expect(scrolled === parent).toBe(true);
});

test("a subsection that has only its heading is still a place to go to", async () => {
  const view = render(<Contents root={parentWith(4)} />);
  await waitFor(() => expect(view.getByTestId("viz-rail-contents")).toBeTruthy());
  expect(view.getByTestId("viz-rail-contents").textContent).toContain("Sub 3");
});

// A grouped `sections` viz: group sections, each with its rows' subsections,
// every container carrying its own row count.
function grouped(groups: number[], extra = ""): HTMLElement {
  const root = document.createElement("div");
  const inner = groups
    .map(
      (rows, g) =>
        `<section><h3 class="ox-section-title">Group ${g}</h3><div data-sections="${rows}">${Array.from(
          { length: rows },
          (_, i) => `<section><h4 class="ox-section-title">G${g} row ${i}</h4></section>`,
        ).join("")}</div></section>`,
    )
    .join("");
  root.innerHTML = `<section><h2 class="ox-section-title">Problems</h2><div data-sections="${groups.length}">${inner}</div></section>${extra}`;
  document.body.appendChild(root);
  return root;
}
const listing = (view: ReturnType<typeof render>) => Array.from(view.getByTestId("viz-rail-contents").querySelectorAll("button"));

test("groups are listed under the parent and their rows under each group, three levels", async () => {
  const view = render(<Contents root={grouped([2, 1])} />);
  await waitFor(() => expect(view.getByTestId("viz-rail-contents")).toBeTruthy());
  const items = listing(view);
  expect(items.map((b) => b.textContent)).toEqual(["Problems", "Group 0", "G0 row 0", "G0 row 1", "Group 1", "G1 row 0"]);
  expect(items.map((b) => b.getAttribute("data-level"))).toEqual(["2", "3", "4", "4", "3", "4"]);
});

test("the cap applies per group: ten rows, then '… N more' scrolling to that group's heading", async () => {
  const root = grouped([13, 2]);
  const view = render(<Contents root={root} />);
  await waitFor(() => expect(view.getByTestId("viz-rail-contents")).toBeTruthy());
  const items = listing(view);
  expect(items.map((b) => b.textContent)).toEqual([
    "Problems",
    "Group 0",
    ...Array.from({ length: 10 }, (_, i) => `G0 row ${i}`),
    "… 3 more",
    "Group 1",
    "G1 row 0",
    "G1 row 1",
  ]);
  const group0 = root.querySelectorAll<HTMLElement>("h3.ox-section-title")[0]!;
  let scrolled: Element | null = null;
  group0.scrollIntoView = function (this: HTMLElement) {
    scrolled = this;
  };
  fireEvent.click(items[12]!);
  expect(scrolled === group0).toBe(true);
});

test("more than ten groups: ten with their rows, then '… N more' to the parent; the rest's rows are not listed", async () => {
  const root = grouped(Array.from({ length: 12 }, () => 1));
  const view = render(<Contents root={root} />);
  await waitFor(() => expect(view.getByTestId("viz-rail-contents")).toBeTruthy());
  const texts = listing(view).map((b) => b.textContent);
  expect(texts.length).toBe(1 + 10 * 2 + 1);
  expect(texts[texts.length - 1]).toBe("… 2 more");
  expect(texts).not.toContain("G10 row 0");
  expect(texts).not.toContain("Group 10");
});

// A last section too short to reach the top of the scroller is still the
// one in view once the person has scrolled to the bottom.
test("scrolled to the bottom, the last section is the active one", async () => {
  const root = body(3);
  root.style.overflowY = "auto";
  Object.defineProperty(root, "scrollHeight", { value: 1000, configurable: true });
  Object.defineProperty(root, "clientHeight", { value: 400, configurable: true });
  Object.defineProperty(root, "scrollTop", { value: 600, configurable: true, writable: true });
  root.getBoundingClientRect = () => ({ top: 0 }) as DOMRect;
  root.querySelectorAll("h2").forEach((h, i) => {
    h.getBoundingClientRect = () => ({ top: i * 300 }) as DOMRect;
  });
  const view = render(<Contents root={root} />);
  await waitFor(() => expect(view.getByTestId("viz-rail-contents")).toBeTruthy());
  const active = Array.from(view.getByTestId("viz-rail-contents").querySelectorAll("button")).find((b) => b.getAttribute("data-active") === "true");
  expect(active?.textContent).toBe("Section 2");
});
