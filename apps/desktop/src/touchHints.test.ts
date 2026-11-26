import { afterEach, expect, test } from "bun:test";
import { HOLD_MS, hintFor, installTouchHints } from "./touchHints.js";

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
