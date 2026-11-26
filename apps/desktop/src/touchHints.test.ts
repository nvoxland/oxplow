import { afterEach, expect, test } from "bun:test";
import { HOLD_MS, MENU_MS, hintFor, installTouchHints } from "./touchHints.js";

let stop: (() => void) | null = null;
afterEach(() => {
  stop?.();
  stop = null;
  document.body.innerHTML = "";
});

function el(html: string): Element {
  document.body.innerHTML = html;
  return document.body.firstElementChild!;
}

// An icon-only control's label is what a hold shows; a control with words
// says what it does already.
test("the hint is an icon control's title or label", () => {
  expect(hintFor(el(`<button title="Refresh"><svg></svg></button>`).querySelector("svg"))).toBe("Refresh");
  expect(hintFor(el(`<button aria-label="Close"><svg></svg></button>`))).toBe("Close");
  expect(hintFor(el(`<button aria-label="Close">Close</button>`))).toBeNull();
  expect(hintFor(el(`<button title="Save">Save</button>`))).toBeNull();
  expect(hintFor(el(`<span>plain</span>`))).toBeNull();
});

function press(target: Element, type: string, pointerType = "touch") {
  const e = new Event(type, { bubbles: true, cancelable: true }) as Event & { pointerType: string; clientX: number; clientY: number };
  Object.assign(e, { pointerType, clientX: 10, clientY: 10 });
  target.dispatchEvent(e);
}

test("a finger's hold shows the label and the press does nothing more", async () => {
  stop = installTouchHints();
  const button = el(`<button title="Refresh"><svg></svg></button>`);
  let clicked = 0;
  button.addEventListener("click", () => clicked++);
  press(button, "pointerdown");
  await new Promise((r) => setTimeout(r, HOLD_MS + 50));
  expect(document.querySelector("[data-testid=touch-hint]")?.textContent).toBe("Refresh");
  press(button, "pointerup");
  button.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
  expect(clicked).toBe(0);
});

test("a quick tap and a mouse are left alone", async () => {
  stop = installTouchHints();
  const button = el(`<button title="Refresh"><svg></svg></button>`);
  let clicked = 0;
  button.addEventListener("click", () => clicked++);
  press(button, "pointerdown");
  press(button, "pointerup");
  button.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
  press(button, "pointerdown", "mouse");
  await new Promise((r) => setTimeout(r, HOLD_MS + 50));
  expect(document.querySelector("[data-testid=touch-hint]")).toBeNull();
  expect(clicked).toBe(1);
});

// Chrome may send no click after a long press: the hold must not swallow
// the next tap somewhere else.
test("after a hold with no click, the next tap still works", async () => {
  stop = installTouchHints();
  document.body.innerHTML = `<button id="a" title="Refresh"><svg></svg></button><button id="b">Go</button>`;
  const a = document.getElementById("a")!;
  const b = document.getElementById("b")!;
  let went = 0;
  b.addEventListener("click", () => went++);
  press(a, "pointerdown");
  await new Promise((r) => setTimeout(r, HOLD_MS + 50));
  press(a, "pointerup");
  // No click follows the hold. The person taps another button.
  press(b, "pointerdown");
  press(b, "pointerup");
  b.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
  expect(went).toBe(1);
});

// A label cut short (its title is its own text) shows whole on a hold; a
// glyph button (×, ☰) shows its aria-label.
test("a cut label and a glyph button have hints", () => {
  const cut = el(`<button title="A very long tab name">A very long tab name</button>`) as HTMLElement;
  Object.defineProperty(cut, "scrollWidth", { value: 300 });
  Object.defineProperty(cut, "clientWidth", { value: 80 });
  expect(hintFor(cut)).toBe("A very long tab name");
  expect(hintFor(el(`<button aria-label="Close tab">×</button>`))).toBe("Close tab");
});

const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));
function menus(target: EventTarget) {
  const seen: MouseEvent[] = [];
  target.addEventListener("contextmenu", (e) => seen.push(e as MouseEvent));
  return seen;
}

// iPad sends no contextmenu on a long press: a finger's long press is one,
// so every menu of the page opens by touch, and its click is swallowed.
test("a long press with no menu of the browser's sends one at the finger", async () => {
  stop = installTouchHints();
  const row = el(`<div id="row">A row</div>`);
  const seen = menus(row);
  let clicked = 0;
  row.addEventListener("click", () => clicked++);
  press(row, "pointerdown");
  await wait(MENU_MS + 50);
  expect(seen.length).toBe(1);
  expect(seen[0]!.clientX).toBe(10);
  press(row, "pointerup");
  row.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
  expect(clicked).toBe(0);
});

// Android sends its own, before or after: the page sees one.
test("a long press where the browser sends its own menu is one menu", async () => {
  stop = installTouchHints();
  const row = el(`<div>A row</div>`);
  const seen = menus(row);
  press(row, "pointerdown");
  row.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true }));
  await wait(MENU_MS + 50);
  expect(seen.length).toBe(1);
  press(row, "pointerup");

  seen.length = 0;
  press(row, "pointerdown");
  await wait(MENU_MS + 50);
  row.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true }));
  expect(seen.length).toBe(1);
  press(row, "pointerup");
});

// Text fields and the code editor keep the OS's own long press (select,
// paste); a mouse and a moving finger send nothing.
test("no long press menu in a text field, by mouse, or while scrolling", async () => {
  stop = installTouchHints();
  document.body.innerHTML = `<input id="i" /><div id="d" contenteditable="true">x</div><div id="r">row</div>`;
  const all = menus(document.body);
  press(document.getElementById("i")!, "pointerdown");
  await wait(MENU_MS + 50);
  press(document.getElementById("i")!, "pointerup");
  press(document.getElementById("d")!, "pointerdown");
  await wait(MENU_MS + 50);
  press(document.getElementById("d")!, "pointerup");
  const r = document.getElementById("r")!;
  press(r, "pointerdown", "mouse");
  await wait(MENU_MS + 50);
  press(r, "pointerup", "mouse");
  press(r, "pointerdown");
  const move = new Event("pointermove", { bubbles: true }) as Event & { clientX: number; clientY: number };
  Object.assign(move, { clientX: 60, clientY: 10 });
  r.dispatchEvent(move);
  await wait(MENU_MS + 50);
  expect(all.length).toBe(0);
});

// The hint stays when the long press opened no menu; a menu of the app's
// takes its place.
test("the hint goes only when a menu opened", async () => {
  stop = installTouchHints();
  const button = el(`<button title="Refresh"><svg></svg></button>`);
  press(button, "pointerdown");
  await wait(MENU_MS + 50);
  expect(document.querySelector("[data-testid=touch-hint]")).not.toBeNull();
  press(button, "pointerup");

  button.addEventListener("contextmenu", () => {
    const menu = document.createElement("div");
    menu.setAttribute("data-oxplow-menu", "");
    document.body.appendChild(menu);
  });
  press(button, "pointerdown");
  await wait(MENU_MS + 50);
  expect(document.querySelector("[data-testid=touch-hint]")).toBeNull();
});
