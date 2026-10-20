import { afterEach, expect, test } from "bun:test";
import { cleanup, fireEvent, render } from "@testing-library/react";
import { useState } from "react";

import { ActionButtons } from "./ActionButtons.js";
import { DataTable, type DataColumn } from "./DataTable.js";
import { Loading } from "./Loading.js";
import { Section } from "./Section.js";
import { Stat } from "./Stat.js";
import { Tile, TileGrid, TileSlot } from "./Tiles.js";
import { Toolbar } from "./Toolbar.js";
import { useListNav } from "./useListNav.js";

afterEach(cleanup);

// A section is a heading and a rule, wiki style: no box around it.
test("a Section is an h2 with its description, actions and body, and carries its id", () => {
  const { container, getByRole, getByText } = render(
    <Section id="earlier" title="Earlier sessions" description="What you had before." actions={[{ id: "r", label: "Refresh", run: () => {} }]}>
      <p>body</p>
    </Section>,
  );
  const section = container.querySelector("section")!;
  expect(section.className).toContain("ox-section");
  expect(section.getAttribute("data-section")).toBe("earlier");
  expect(getByRole("heading", { level: 2 }).textContent).toBe("Earlier sessions");
  expect(getByText("What you had before.")).toBeTruthy();
  expect(getByRole("button", { name: "Refresh" })).toBeTruthy();
  expect(getByText("body")).toBeTruthy();
});

// Actions are data; core draws them. A primary one is the accent button.
test("ActionButtons draws each action, the primary one marked, busy and disabled honoured", () => {
  const ran: string[] = [];
  const { getByRole } = render(
    <ActionButtons
      actions={[
        { id: "save", label: "Save", primary: true, run: () => void ran.push("save") },
        { id: "close", label: "Close", run: () => void ran.push("close") },
        { id: "busy", label: "Sync", busy: true, run: () => void ran.push("busy") },
        { id: "off", label: "Off", disabled: true, run: () => void ran.push("off") },
      ]}
    />,
  );
  const save = getByRole("button", { name: "Save" });
  expect(save.className).toContain("ox-primary");
  fireEvent.click(save);
  fireEvent.click(getByRole("button", { name: "Close" }));
  expect((getByRole("button", { name: "Sync…" }) as HTMLButtonElement).disabled).toBe(true);
  expect((getByRole("button", { name: "Off" }) as HTMLButtonElement).disabled).toBe(true);
  expect(ran).toEqual(["save", "close"]);
});

interface Row {
  id: string;
  name: string;
  size: number;
}
const columns: DataColumn<Row>[] = [
  { key: "name", label: "Name", truncate: true, render: (r) => r.name, title: (r) => r.name },
  { key: "size", label: "Size", width: 60, numeric: true, render: (r) => String(r.size) },
];

// One table for core pages and lenses: fixed column widths, numbers
// right-aligned in tabular figures, long text cut with its full text on hover.
test("DataTable lays out its columns: widths, numeric alignment, truncation", () => {
  const { container, getAllByRole } = render(
    <DataTable columns={columns} rows={[{ id: "a", name: "a long session title", size: 12 }]} rowKey={(r) => r.id} testId="t" />,
  );
  const table = container.querySelector("table")!;
  expect(table.className).toContain("ox-table");
  expect(table.getAttribute("data-testid")).toBe("t");
  const cols = container.querySelectorAll("col");
  expect(cols[1]!.getAttribute("style")).toContain("60px");
  expect(getAllByRole("columnheader").map((h) => h.textContent)).toEqual(["Name", "Size"]);
  const cells = container.querySelectorAll("tbody td");
  expect(cells[0]!.className).toContain("ox-truncate");
  expect(cells[0]!.getAttribute("title")).toBe("a long session title");
  expect(cells[1]!.className).toContain("ox-num");
});

// A fixed width is a promise the column keeps: its cells stay one line
// unless the column says it wraps, so no table has to ask for it.
test("DataTable cuts a column with a width to one line unless it wraps", () => {
  const { container } = render(
    <DataTable
      columns={[
        { key: "sha", label: "Commit", width: 90, render: (r: Row) => r.name },
        { key: "note", label: "Note", width: 90, truncate: false, render: (r: Row) => r.name },
        { key: "rest", label: "Rest", render: (r: Row) => r.name },
      ]}
      rows={[{ id: "a", name: "ba74ce57721f3c01a7a96267399953c2a96b163a", size: 1 }]}
      rowKey={(r) => r.id}
    />,
  );
  const [sha, note, rest] = [...container.querySelectorAll("tbody td")];
  expect(sha!.className).toContain("ox-truncate");
  expect(note!.className).not.toContain("ox-truncate");
  expect(rest!.className).not.toContain("ox-truncate");
});

// The whole text on hover comes for free, and only when the cell is cut.
test("DataTable gives a cut cell its whole text on hover, and a cell that fits none", () => {
  const { container } = render(
    <DataTable
      columns={[{ key: "sha", label: "Commit", width: 90, render: (r: Row) => <span>{r.name}</span> }]}
      rows={[
        { id: "a", name: "ba74ce57721f3c01a7a96267399953c2a96b163a", size: 1 },
        { id: "b", name: "short", size: 1 },
      ]}
      rowKey={(r) => r.id}
    />,
  );
  const [cut, fits] = [...container.querySelectorAll("tbody td")] as HTMLElement[];
  Object.defineProperty(cut!, "scrollWidth", { value: 300 });
  Object.defineProperty(cut!, "clientWidth", { value: 90 });
  Object.defineProperty(fits!, "scrollWidth", { value: 40 });
  Object.defineProperty(fits!, "clientWidth", { value: 90 });
  fireEvent.mouseEnter(cut!);
  fireEvent.mouseEnter(fits!);
  expect(cut!.getAttribute("title")).toBe("ba74ce57721f3c01a7a96267399953c2a96b163a");
  expect(fits!.getAttribute("title")).toBeNull();
});

test("DataTable passes each row its props, and draws group headings", () => {
  const clicked: string[] = [];
  const { getByTestId, getByText } = render(
    <DataTable
      columns={columns}
      groups={[
        { key: "g1", heading: "Group one", rows: [{ id: "a", name: "A", size: 1 }] },
        { key: "g2", heading: "Group two", rows: [{ id: "b", name: "B", size: 2 }] },
      ]}
      rowKey={(r) => r.id}
      rowProps={(r) => ({ "data-testid": `row-${r.id}`, onClick: () => clicked.push(r.id) })}
    />,
  );
  expect(getByText("Group one").closest("td")!.getAttribute("colspan")).toBe("2");
  fireEvent.click(getByTestId("row-b"));
  expect(clicked).toEqual(["b"]);
});

test("DataTable can hide its header", () => {
  const { container } = render(<DataTable columns={columns} rows={[]} rowKey={(r) => r.id} header={false} />);
  expect(container.querySelector("thead")).toBeNull();
});

// The one ↑/↓/Enter/Escape contract for a list driven from an input.
function NavHarness({ count, onPick, onEscape }: { count: number; onPick(i: number): void; onEscape(): void }) {
  const nav = useListNav({ count, onPick, onEscape, id: "harness" });
  return (
    <div>
      <input data-testid="filter" {...nav.inputProps} />
      <ul {...nav.listProps}>
        {Array.from({ length: count }, (_, i) => (
          <li key={i} {...nav.optionProps(i)}>
            row {i}
          </li>
        ))}
      </ul>
    </div>
  );
}

test("useListNav moves with the arrows, picks with Enter, hands Escape back", () => {
  const picked: number[] = [];
  let escaped = 0;
  const { getByTestId, getAllByRole } = render(<NavHarness count={3} onPick={(i) => picked.push(i)} onEscape={() => escaped++} />);
  const input = getByTestId("filter");
  const selected = () => getAllByRole("option").findIndex((o) => o.getAttribute("aria-selected") === "true");
  expect(selected()).toBe(0);
  fireEvent.keyDown(input, { key: "ArrowDown" });
  fireEvent.keyDown(input, { key: "ArrowDown" });
  fireEvent.keyDown(input, { key: "ArrowDown" });
  expect(selected()).toBe(2);
  expect(input.getAttribute("aria-activedescendant")).toBe(getAllByRole("option")[2]!.id);
  fireEvent.keyDown(input, { key: "ArrowUp" });
  fireEvent.keyDown(input, { key: "Enter" });
  expect(picked).toEqual([1]);
  fireEvent.keyDown(input, { key: "Escape" });
  expect(escaped).toBe(1);
});

test("useListNav follows the pointer and comes back into range when the list shrinks", () => {
  function Shrinking() {
    const [count, setCount] = useState(3);
    return (
      <>
        <button type="button" onClick={() => setCount(1)}>shrink</button>
        <NavHarness count={count} onPick={() => {}} onEscape={() => {}} />
      </>
    );
  }
  const { getAllByRole, getByText } = render(<Shrinking />);
  fireEvent.mouseEnter(getAllByRole("option")[2]!);
  expect(getAllByRole("option")[2]!.getAttribute("aria-selected")).toBe("true");
  fireEvent.click(getByText("shrink"));
  expect(getAllByRole("option")[0]!.getAttribute("aria-selected")).toBe("true");
});

test("Toolbar holds a filter and its actions", () => {
  let value = "";
  const { getByRole, getByLabelText } = render(
    <Toolbar filter={{ value: "", onChange: (v) => (value = v), label: "Filter rows", placeholder: "Filter" }} actions={[{ id: "a", label: "Add", run: () => {} }]} />,
  );
  fireEvent.change(getByLabelText("Filter rows"), { target: { value: "x" } });
  expect(value).toBe("x");
  expect(getByRole("button", { name: "Add" })).toBeTruthy();
});

test("Stat shows a value with its unit and label", () => {
  const { container, getByText } = render(<Stat label="Tokens" value="1.2k" unit="tok" hint="last 30 days" />);
  expect(container.firstElementChild!.className).toContain("ox-stat");
  expect(getByText("1.2k")).toBeTruthy();
  expect(getByText("tok")).toBeTruthy();
  expect(getByText("Tokens")).toBeTruthy();
  expect(getByText("last 30 days")).toBeTruthy();
});

// Tiles: a responsive grid; a slot spans by its size; a tile is a titled card.
test("tiles size by their slot and carry a title", () => {
  const { container, getByText } = render(
    <TileGrid testId="grid">
      <TileSlot size="wide">
        <Tile title="Tokens">body</Tile>
      </TileSlot>
      <TileSlot size="small">
        <Tile>untitled</Tile>
      </TileSlot>
    </TileGrid>,
  );
  const grid = container.querySelector('[data-testid="grid"]')!;
  expect(grid.className).toContain("ox-tiles");
  const [wide, small] = [...grid.children];
  expect(wide!.className).toContain("ox-tile-wide");
  expect(small!.className).not.toContain("ox-tile-wide");
  expect(getByText("Tokens").className).toContain("ox-tile-title");
});

test("Loading says what it's reading", () => {
  const { getByText } = render(<Loading what="saved sessions" />);
  expect(getByText("Reading saved sessions…").className).toContain("ox-loading");
});
