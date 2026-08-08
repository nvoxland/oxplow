import { afterEach, describe, expect, test } from "bun:test";
import { act, cleanup, fireEvent, renderHook } from "@testing-library/react";
import type { MouseEvent as ReactMouseEvent } from "react";

import {
  isInteractiveTarget,
  isPointInRect,
  useSlideoutStrip,
} from "./useSlideoutStrip.js";

afterEach(cleanup);

/** The hook's pointer-leave grace (180ms) plus slack. */
const settle = () => new Promise((r) => setTimeout(r, 260));

/**
 * A stand-in panel element with a real rect — happy-dom reports all-zero
 * rects, which would make every pointer position read as "outside".
 */
function mountPanel(right = 280, bottom = 800) {
  const el = document.createElement("div");
  document.body.appendChild(el);
  Object.defineProperty(el, "getBoundingClientRect", {
    configurable: true,
    value: () => ({
      left: 0,
      top: 0,
      right,
      bottom,
      width: right,
      height: bottom,
      x: 0,
      y: 0,
      toJSON: () => ({}),
    }),
  });
  return el;
}

/** Minimal synthetic-click stub — the hook only reads `target`. */
const clickOn = (target: Element) =>
  ({ target }) as unknown as ReactMouseEvent<HTMLElement>;

function setup(opts?: { guard?: boolean; onClose?(): void }) {
  const panel = mountPanel();
  const view = renderHook(({ guard }) => useSlideoutStrip({ guard, onClose: opts?.onClose }), {
    initialProps: { guard: opts?.guard ?? false },
  });
  act(() => view.result.current.panelRef(panel));
  return { ...view, panel };
}

// --- Pure helpers ---------------------------------------------------------

describe("isPointInRect", () => {
  const rect = { left: 0, top: 0, right: 280, bottom: 800 } as DOMRect;

  test("a point within the bounds is inside", () => {
    expect(isPointInRect(rect, 120, 300)).toBe(true);
  });

  test("the edges count as inside", () => {
    // The pointer sitting exactly on the panel's right edge is still on
    // the panel — closing there would fight the user's own cursor.
    expect(isPointInRect(rect, 280, 400)).toBe(true);
    expect(isPointInRect(rect, 0, 0)).toBe(true);
  });

  test("a point beyond any edge is outside", () => {
    expect(isPointInRect(rect, 281, 400)).toBe(false);
    expect(isPointInRect(rect, 120, 801)).toBe(false);
    expect(isPointInRect(rect, -1, 400)).toBe(false);
  });
});

describe("isInteractiveTarget", () => {
  test("controls and their descendants are interactive", () => {
    const host = document.createElement("div");
    host.innerHTML = `
      <button id="b"><span id="inner">x</span></button>
      <input id="i" />
      <select id="s"></select>
      <textarea id="t"></textarea>
      <a id="a"></a>
      <label id="l"></label>
      <div id="r" role="button"></div>`;
    document.body.appendChild(host);

    for (const id of ["b", "inner", "i", "s", "t", "a", "l", "r"]) {
      expect(isInteractiveTarget(host.querySelector(`#${id}`))).toBe(true);
    }
  });

  test("plain background elements are not", () => {
    const host = document.createElement("div");
    host.innerHTML = `<div id="bg"><span id="label">title</span></div>`;
    document.body.appendChild(host);

    expect(isInteractiveTarget(host.querySelector("#bg"))).toBe(false);
    expect(isInteractiveTarget(host.querySelector("#label"))).toBe(false);
    expect(isInteractiveTarget(null)).toBe(false);
  });
});

// --- Open / close ---------------------------------------------------------

test("starts closed and opens on demand", () => {
  const { result } = setup();
  expect(result.current.open).toBe(false);

  act(() => result.current.openPanel());
  expect(result.current.open).toBe(true);

  act(() => result.current.closePanel());
  expect(result.current.open).toBe(false);
});

test("dead space in the strip opens the panel, but only when it IS the target", () => {
  const { result } = setup();
  const strip = document.createElement("div");
  const child = document.createElement("div");
  strip.appendChild(child);

  // A click that bubbled up from a row is not a dead-space click.
  act(() =>
    result.current.deadSpaceProps.onClick({
      target: child,
      currentTarget: strip,
    } as unknown as ReactMouseEvent<HTMLElement>),
  );
  expect(result.current.open).toBe(false);

  act(() =>
    result.current.deadSpaceProps.onClick({
      target: strip,
      currentTarget: strip,
    } as unknown as ReactMouseEvent<HTMLElement>),
  );
  expect(result.current.open).toBe(true);
});

test("Escape closes", () => {
  const { result } = setup();
  act(() => result.current.openPanel());

  fireEvent.keyDown(document, { key: "Escape" });
  expect(result.current.open).toBe(false);
});

test("a pointerdown outside the panel closes; inside it does not", () => {
  const { result, panel } = setup();
  act(() => result.current.openPanel());

  fireEvent.pointerDown(panel);
  expect(result.current.open).toBe(true);

  fireEvent.pointerDown(document.body);
  expect(result.current.open).toBe(false);
});

test("a click on the panel's dead background closes it", () => {
  const { result, panel } = setup();
  act(() => result.current.openPanel());

  act(() => result.current.panelProps.onClick(clickOn(panel)));
  expect(result.current.open).toBe(false);
});

test("a click on a control inside the panel does not close it", () => {
  const { result, panel } = setup();
  const button = document.createElement("button");
  panel.appendChild(button);
  act(() => result.current.openPanel());

  act(() => result.current.panelProps.onClick(clickOn(button)));
  expect(result.current.open).toBe(true);
});

test("the pointer leaving the panel's bounds closes it after the grace", async () => {
  const { result } = setup();
  act(() => result.current.openPanel());

  fireEvent.pointerMove(document, { clientX: 400, clientY: 300 });
  expect(result.current.open).toBe(true); // grace hasn't elapsed yet
  await act(settle);

  expect(result.current.open).toBe(false);
});

test("a pointer still inside the panel keeps it open", async () => {
  const { result } = setup();
  act(() => result.current.openPanel());

  fireEvent.pointerMove(document, { clientX: 120, clientY: 300 });
  await act(settle);

  expect(result.current.open).toBe(true);
});

test("returning to the panel within the grace cancels the pending close", async () => {
  const { result } = setup();
  act(() => result.current.openPanel());

  fireEvent.pointerMove(document, { clientX: 400, clientY: 300 });
  fireEvent.pointerMove(document, { clientX: 100, clientY: 300 });
  await act(settle);

  // Overshooting and sliding back is the whole reason the grace exists.
  expect(result.current.open).toBe(true);
});

test("onClose fires however the panel closed", () => {
  let closes = 0;
  const { result } = setup({ onClose: () => closes++ });

  act(() => result.current.openPanel());
  act(() => result.current.closePanel());
  expect(closes).toBe(1);

  act(() => result.current.openPanel());
  fireEvent.keyDown(document, { key: "Escape" });
  expect(closes).toBe(2);
});

// --- The guard: passive closes yield, explicit ones don't ------------------

describe("guard", () => {
  test("suppresses the pointer-leave close", async () => {
    const { result } = setup({ guard: true });
    act(() => result.current.openPanel());

    fireEvent.pointerMove(document, { clientX: 400, clientY: 300 });
    await act(settle);

    expect(result.current.open).toBe(true);
  });

  test("suppresses the background click", () => {
    const { result, panel } = setup({ guard: true });
    act(() => result.current.openPanel());

    act(() => result.current.panelProps.onClick(clickOn(panel)));
    expect(result.current.open).toBe(true);
  });

  test("does NOT suppress Escape", () => {
    const { result } = setup({ guard: true });
    act(() => result.current.openPanel());

    // Explicit dismissal — the user is actively saying "go away", which
    // beats "you might be mid-form".
    fireEvent.keyDown(document, { key: "Escape" });
    expect(result.current.open).toBe(false);
  });

  test("does NOT suppress an outside pointerdown", () => {
    const { result } = setup({ guard: true });
    act(() => result.current.openPanel());

    fireEvent.pointerDown(document.body);
    expect(result.current.open).toBe(false);
  });

  test("does NOT suppress an explicit closePanel", () => {
    const { result } = setup({ guard: true });
    act(() => result.current.openPanel());

    act(() => result.current.closePanel());
    expect(result.current.open).toBe(false);
  });

  test("a guard that clears re-arms the passive close", async () => {
    const { result, rerender } = setup({ guard: true });
    act(() => result.current.openPanel());

    fireEvent.pointerMove(document, { clientX: 400, clientY: 300 });
    await act(settle);
    expect(result.current.open).toBe(true);

    // Form committed — the panel should behave normally again.
    rerender({ guard: false });
    fireEvent.pointerMove(document, { clientX: 400, clientY: 300 });
    await act(settle);
    expect(result.current.open).toBe(false);
  });
});
