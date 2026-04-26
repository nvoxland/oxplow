import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { monacoThemeFor, getActiveMonacoTheme, subscribeMonacoTheme } from "./monaco-theme.js";
import { setStoredThemePref } from "./theme.js";

class FakeStorage {
  private store = new Map<string, string>();
  getItem(key: string): string | null {
    return this.store.has(key) ? (this.store.get(key) as string) : null;
  }
  setItem(key: string, value: string): void {
    this.store.set(key, value);
  }
  removeItem(key: string): void {
    this.store.delete(key);
  }
  clear(): void {
    this.store.clear();
  }
}

describe("monaco-theme helper", () => {
  let storage: FakeStorage;
  let attrs: Map<string, string>;

  beforeEach(() => {
    storage = new FakeStorage();
    attrs = new Map<string, string>();
    (globalThis as any).window = {
      localStorage: storage,
      matchMedia: (_q: string) => ({
        matches: false,
        addEventListener: () => {},
        removeEventListener: () => {},
      }),
    };
    (globalThis as any).document = {
      documentElement: {
        getAttribute: (k: string) => attrs.get(k) ?? null,
        setAttribute: (k: string, v: string) => attrs.set(k, v),
      },
    };
  });

  afterEach(() => {
    delete (globalThis as any).window;
    delete (globalThis as any).document;
  });

  test("monacoThemeFor maps light → vs and dark → vs-dark", () => {
    expect(monacoThemeFor("light")).toBe("vs");
    expect(monacoThemeFor("dark")).toBe("vs-dark");
  });

  test("getActiveMonacoTheme reads data-theme attribute", () => {
    attrs.set("data-theme", "light");
    expect(getActiveMonacoTheme()).toBe("vs");
    attrs.set("data-theme", "dark");
    expect(getActiveMonacoTheme()).toBe("vs-dark");
  });

  test("getActiveMonacoTheme defaults to vs (light) when attr missing", () => {
    expect(getActiveMonacoTheme()).toBe("vs");
  });

  test("subscribeMonacoTheme fires when stored pref changes", () => {
    setStoredThemePref("light");
    const seen: string[] = [];
    const off = subscribeMonacoTheme((t) => seen.push(t));
    setStoredThemePref("dark");
    setStoredThemePref("light");
    off();
    setStoredThemePref("dark"); // should not be seen
    expect(seen).toEqual(["vs-dark", "vs"]);
  });
});
