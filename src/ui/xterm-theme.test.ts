import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import {
  xtermThemeFor,
  getActiveXtermTheme,
  subscribeXtermTheme,
  type XtermTheme,
} from "./xterm-theme.js";
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

describe("xterm-theme helper", () => {
  let storage: FakeStorage;
  let attrs: Map<string, string>;
  let cssVars: Map<string, string>;

  beforeEach(() => {
    storage = new FakeStorage();
    attrs = new Map<string, string>();
    cssVars = new Map<string, string>();
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
    (globalThis as any).getComputedStyle = (_el: unknown) => ({
      getPropertyValue: (name: string) => cssVars.get(name) ?? "",
    });
  });

  afterEach(() => {
    delete (globalThis as any).window;
    delete (globalThis as any).document;
    delete (globalThis as any).getComputedStyle;
  });

  test("xtermThemeFor light vs dark pick distinct ANSI palettes", () => {
    const light = xtermThemeFor("light");
    const dark = xtermThemeFor("dark");
    expect(light.red).not.toBe(dark.red);
    expect(light.background).toBeTruthy();
    expect(dark.background).toBeTruthy();
  });

  test("xtermThemeFor uses CSS vars when present", () => {
    cssVars.set("--surface-card", "#abcdef");
    cssVars.set("--text-primary", "#123456");
    cssVars.set("--accent", "#fedcba");
    cssVars.set("--accent-soft-bg", "#001122");
    const t = xtermThemeFor("light");
    expect(t.background).toBe("#abcdef");
    expect(t.foreground).toBe("#123456");
    expect(t.cursor).toBe("#fedcba");
    expect(t.cursorAccent).toBe("#abcdef");
    expect(t.selectionBackground).toBe("#001122");
  });

  test("xtermThemeFor falls back when CSS vars missing", () => {
    const light = xtermThemeFor("light");
    const dark = xtermThemeFor("dark");
    expect(light.background).toBe("#ffffff");
    expect(dark.background).toBe("#1c1f24");
  });

  test("getActiveXtermTheme reads data-theme attribute", () => {
    attrs.set("data-theme", "dark");
    const dark = getActiveXtermTheme();
    attrs.set("data-theme", "light");
    const light = getActiveXtermTheme();
    expect(dark.background).toBe("#1c1f24");
    expect(light.background).toBe("#ffffff");
  });

  test("getActiveXtermTheme defaults to light when attr missing", () => {
    expect(getActiveXtermTheme().background).toBe("#ffffff");
  });

  test("subscribeXtermTheme fires when stored pref changes", () => {
    setStoredThemePref("light");
    const seen: XtermTheme[] = [];
    const off = subscribeXtermTheme((t) => seen.push(t));
    setStoredThemePref("dark");
    setStoredThemePref("light");
    off();
    setStoredThemePref("dark");
    expect(seen).toHaveLength(2);
    expect(seen[0]?.background).toBe("#1c1f24");
    expect(seen[1]?.background).toBe("#ffffff");
  });
});
