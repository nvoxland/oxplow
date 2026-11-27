import { afterEach, beforeEach, expect, test } from "bun:test";
import { cleanup, fireEvent, render } from "@testing-library/react";

import { AgentStatusDot } from "../components/AgentStatusDot.js";
import { hintFor } from "../touchHints.js";
import { DataTable, type DataColumn } from "./DataTable.js";
import { Section } from "./Section.js";

// What a mouse learns by hovering must reach a finger: a section's help is
// text on a coarse pointer, a cut table cell opens on a tap, and the agent
// dot's question is what a hold on the dot shows.

const real = globalThis.matchMedia;
function pointer(coarse: boolean) {
  globalThis.matchMedia = ((query: string) => ({
    matches: coarse && query === "(pointer: coarse)",
    addEventListener: () => {},
    removeEventListener: () => {},
  })) as unknown as typeof globalThis.matchMedia;
}
beforeEach(() => pointer(false));
afterEach(() => {
  cleanup();
  globalThis.matchMedia = real;
});

test("a section's help is a hover title with a mouse and visible text under the heading on a finger", () => {
  const mouse = render(<Section title="Runs" help="Runs of the last week.">body</Section>);
  expect(mouse.queryByTestId("section-help-text")).toBeNull();
  expect(mouse.getByTestId("section-help").getAttribute("title")).toBe("Runs of the last week.");
  cleanup();

  pointer(true);
  const touch = render(<Section title="Runs" help="Runs of the last week.">body</Section>);
  expect(touch.getByTestId("section-help-text").textContent).toBe("Runs of the last week.");
  expect(touch.queryByTestId("section-help")).toBeNull();
});

type Row = { id: string; name: string };
const columns: DataColumn<Row>[] = [{ key: "name", label: "Name", truncate: true, render: (r) => r.name }];

function cutCell(container: HTMLElement, cut: boolean): HTMLElement {
  const td = container.querySelector<HTMLElement>("tbody td")!;
  Object.defineProperty(td, "scrollWidth", { configurable: true, value: cut ? 200 : 40 });
  Object.defineProperty(td, "clientWidth", { configurable: true, value: 40 });
  return td;
}

test("on a finger a tap on a cut cell shows its whole text, and another tap cuts it again", () => {
  pointer(true);
  const { container } = render(<DataTable columns={columns} rows={[{ id: "a", name: "a very long session title" }]} rowKey={(r) => r.id} />);
  const td = cutCell(container, true);
  fireEvent.click(td);
  expect(td.getAttribute("data-expanded")).toBe("true");
  fireEvent.click(td);
  expect(td.hasAttribute("data-expanded")).toBe(false);
});

test("a tap on a cell that is not cut, or on a link in it, changes nothing; a mouse's click never expands", () => {
  pointer(true);
  const linked: DataColumn<Row>[] = [{ key: "name", label: "Name", truncate: true, render: (r) => <a href="#x">{r.name}</a> }];
  const rows = [{ id: "a", name: "short" }];
  const whole = render(<DataTable columns={columns} rows={rows} rowKey={(r) => r.id} />);
  fireEvent.click(cutCell(whole.container, false));
  expect(whole.container.querySelector("td")!.hasAttribute("data-expanded")).toBe(false);
  cleanup();

  const link = render(<DataTable columns={linked} rows={rows} rowKey={(r) => r.id} />);
  const td = cutCell(link.container, true);
  fireEvent.click(link.container.querySelector("a")!);
  expect(td.hasAttribute("data-expanded")).toBe(false);
  cleanup();

  pointer(false);
  const mouse = render(<DataTable columns={columns} rows={rows} rowKey={(r) => r.id} />);
  const cell = cutCell(mouse.container, true);
  fireEvent.click(cell);
  expect(cell.hasAttribute("data-expanded")).toBe(false);
});

test("a hold on the agent dot already shows the agent's question: its title is the hint", () => {
  const { container } = render(<AgentStatusDot status="awaiting" question="Merge it?" />);
  expect(hintFor(container.querySelector("[data-agent-status]"))).toBe("Waiting on your answer: Merge it?");
});
