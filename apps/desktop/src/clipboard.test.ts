import { afterEach, expect, test } from "bun:test";
import { copyText, readText } from "./clipboard.js";

const g = globalThis as any;
const saved = { nav: g.navigator, doc: g.document };
afterEach(() => {
  Object.defineProperty(g, "navigator", { value: saved.nav, configurable: true });
  Object.defineProperty(g, "document", { value: saved.doc, configurable: true });
});
function set(name: "navigator" | "document", value: unknown) {
  Object.defineProperty(g, name, { value, configurable: true });
}

test("copyText uses navigator.clipboard when it exists", async () => {
  let got = "";
  set("navigator", { clipboard: { writeText: async (t: string) => { got = t; } } });
  expect(await copyText("hi")).toBe(true);
  expect(got).toBe("hi");
});

test("copyText reports false when the clipboard write is refused", async () => {
  set("navigator", { clipboard: { writeText: async () => { throw new Error("denied"); } } });
  expect(await copyText("hi")).toBe(false);
});

test("copyText falls back to execCommand without navigator.clipboard", async () => {
  const calls: string[] = [];
  const ta: any = { style: {}, setAttribute() {}, focus() {}, select() {}, remove() { calls.push("remove"); } };
  set("navigator", {});
  set("document", {
    createElement: () => ta,
    body: { appendChild: () => calls.push("append") },
    execCommand: (c: string) => { calls.push(c); return true; },
  });
  expect(await copyText("plain")).toBe(true);
  expect(ta.value).toBe("plain");
  expect(calls).toEqual(["append", "copy", "remove"]);
});

test("copyText returns false with no clipboard and no document", async () => {
  set("navigator", {});
  set("document", undefined);
  expect(await copyText("x")).toBe(false);
});

test("readText returns null without navigator.clipboard, and the text with it", async () => {
  set("navigator", {});
  expect(await readText()).toBeNull();
  set("navigator", { clipboard: { readText: async () => "pasted" } });
  expect(await readText()).toBe("pasted");
  set("navigator", { clipboard: { readText: async () => { throw new Error("denied"); } } });
  expect(await readText()).toBeNull();
});
