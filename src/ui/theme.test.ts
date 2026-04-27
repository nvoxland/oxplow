import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import {
  applyTheme,
  getStoredThemePref,
  initTheme,
  resolveTheme,
  setStoredThemePref,
  subscribeThemePref,
  THEME_STORAGE_KEY,
  type ThemePreference,
} from "./theme.js";

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

describe("theme", () => {
  let storage: FakeStorage;
  let root: { setAttribute: (k: string, v: string) => void; attrs: Map<string, string> };
  let mqlListeners: Array<(ev: { matches: boolean }) => void>;
  let mqlMatches: boolean;

  beforeEach(() => {
    storage = new FakeStorage();
    mqlListeners = [];
    mqlMatches = false;
    const attrs = new Map<string, string>();
    root = {
      attrs,
      setAttribute: (k: string, v: string) => {
        attrs.set(k, v);
      },
    };
    (globalThis as any).window = {
      localStorage: storage,
      matchMedia: (_q: string) => ({
        get matches() {
          return mqlMatches;
        },
        addEventListener: (_evt: string, fn: (ev: { matches: boolean }) => void) => {
          mqlListeners.push(fn);
        },
        removeEventListener: (_evt: string, fn: (ev: { matches: boolean }) => void) => {
          const i = mqlListeners.indexOf(fn);
          if (i >= 0) mqlListeners.splice(i, 1);
        },
      }),
    };
    (globalThis as any).document = { documentElement: root };
  });

  afterEach(() => {
    delete (globalThis as any).window;
    delete (globalThis as any).document;
  });

  test("getStoredThemePref returns null when nothing stored", () => {
    expect(getStoredThemePref()).toBeNull();
  });

  test("setStoredThemePref persists value and getStoredThemePref reads it back", () => {
    setStoredThemePref("dark");
    expect(getStoredThemePref()).toBe("dark");
    setStoredThemePref("system");
    expect(getStoredThemePref()).toBe("system");
  });

  test("setStoredThemePref clears storage when null is passed", () => {
    setStoredThemePref("light");
    setStoredThemePref(null);
    expect(getStoredThemePref()).toBeNull();
  });

  test("getStoredThemePref ignores invalid stored values", () => {
    storage.setItem(THEME_STORAGE_KEY, "neon");
    expect(getStoredThemePref()).toBeNull();
  });

  test("resolveTheme(light) returns light regardless of system preference", () => {
    mqlMatches = true; // system says dark
    expect(resolveTheme("light")).toBe("light");
  });

  test("resolveTheme(dark) returns dark", () => {
    expect(resolveTheme("dark")).toBe("dark");
  });

  test("resolveTheme(system) follows prefers-color-scheme: dark", () => {
    mqlMatches = true;
    expect(resolveTheme("system")).toBe("dark");
    mqlMatches = false;
    expect(resolveTheme("system")).toBe("light");
  });

  test("applyTheme writes data-theme attribute on documentElement", () => {
    applyTheme("dark");
    expect(root.attrs.get("data-theme")).toBe("dark");
    applyTheme("light");
    expect(root.attrs.get("data-theme")).toBe("light");
  });

  test("initTheme defaults to light when no preference stored", () => {
    initTheme();
    expect(root.attrs.get("data-theme")).toBe("light");
  });

  test("initTheme honors stored preference", () => {
    setStoredThemePref("dark");
    initTheme();
    expect(root.attrs.get("data-theme")).toBe("dark");
  });

  test("initTheme follows system when preference is system", () => {
    setStoredThemePref("system");
    mqlMatches = true;
    initTheme();
    expect(root.attrs.get("data-theme")).toBe("dark");
  });

  test("initTheme reacts to system preference changes when preference is system", () => {
    setStoredThemePref("system");
    mqlMatches = false;
    initTheme();
    expect(root.attrs.get("data-theme")).toBe("light");
    // Simulate OS switching to dark
    mqlMatches = true;
    for (const listener of mqlListeners) listener({ matches: true });
    expect(root.attrs.get("data-theme")).toBe("dark");
  });

  test("initTheme returned cleanup detaches the system listener", () => {
    setStoredThemePref("system");
    const cleanup = initTheme();
    expect(mqlListeners.length).toBe(1);
    cleanup();
    expect(mqlListeners.length).toBe(0);
  });

  test("changing pref to a non-system value detaches the system listener", () => {
    setStoredThemePref("system");
    initTheme();
    expect(mqlListeners.length).toBe(1);
    setStoredThemePref("dark");
    initTheme();
    expect(mqlListeners.length).toBe(0);
    expect(root.attrs.get("data-theme")).toBe("dark");
  });

  test("notifies subscribers when stored pref changes via setStoredThemePref", () => {
    const seen: ThemePreference[] = [];
    const off = subscribeThemePref((p) => {
      seen.push(p);
    });
    setStoredThemePref("dark");
    setStoredThemePref("system");
    off();
    setStoredThemePref("light"); // should not be seen
    expect(seen).toEqual(["dark", "system"]);
  });
});
