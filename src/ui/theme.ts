/**
 * Theme module: persists user preference (light / dark / system) in
 * localStorage and applies the resolved theme via a `data-theme` attribute
 * on documentElement. CSS variables in `public/index.html` switch values
 * based on that attribute.
 *
 * Default for new installs: light. The `system` mode follows OS via
 * `prefers-color-scheme` and re-applies whenever the OS preference changes.
 */

export type ThemePreference = "light" | "dark" | "system";
export type ResolvedTheme = "light" | "dark";

export const THEME_STORAGE_KEY = "oxplow.theme";

const VALID_PREFS: ThemePreference[] = ["light", "dark", "system"];

type Subscriber = (pref: ThemePreference) => void;
const subscribers = new Set<Subscriber>();

/** Read the stored preference, returning `null` when unset or invalid. */
export function getStoredThemePref(): ThemePreference | null {
  try {
    const raw = window.localStorage.getItem(THEME_STORAGE_KEY);
    if (raw && (VALID_PREFS as string[]).includes(raw)) {
      return raw as ThemePreference;
    }
  } catch {
    // localStorage may be disabled in some sandboxes; treat as unset.
  }
  return null;
}

/**
 * Persist a preference and notify subscribers. Pass `null` to clear.
 * Triggers a re-init internally so `data-theme` updates immediately.
 */
export function setStoredThemePref(pref: ThemePreference | null): void {
  try {
    if (pref === null) {
      window.localStorage.removeItem(THEME_STORAGE_KEY);
    } else {
      window.localStorage.setItem(THEME_STORAGE_KEY, pref);
    }
  } catch {
    // Best-effort persistence.
  }
  initTheme();
  if (pref !== null) {
    for (const sub of subscribers) sub(pref);
  }
}

/** Subscribe to preference changes. Returns an unsubscribe function. */
export function subscribeThemePref(fn: Subscriber): () => void {
  subscribers.add(fn);
  return () => {
    subscribers.delete(fn);
  };
}

/** Resolve a preference to a concrete theme, querying the OS for `system`. */
export function resolveTheme(pref: ThemePreference): ResolvedTheme {
  if (pref === "light" || pref === "dark") return pref;
  try {
    const mql = window.matchMedia("(prefers-color-scheme: dark)");
    return mql.matches ? "dark" : "light";
  } catch {
    return "light";
  }
}

/** Write `data-theme` onto the document root. */
export function applyTheme(theme: ResolvedTheme): void {
  document.documentElement.setAttribute("data-theme", theme);
}

let mediaListener: ((ev: { matches: boolean }) => void) | null = null;
let mediaQueryList: ReturnType<typeof window.matchMedia> | null = null;

function detachMediaListener(): void {
  if (mediaQueryList && mediaListener) {
    try {
      mediaQueryList.removeEventListener("change", mediaListener);
    } catch {
      // ignore
    }
  }
  mediaQueryList = null;
  mediaListener = null;
}

/**
 * Read the stored preference, apply it, and (if `system`) wire up an
 * OS-preference listener that re-applies on change. Returns a cleanup
 * function that detaches the listener.
 */
export function initTheme(): () => void {
  detachMediaListener();
  const pref = getStoredThemePref() ?? "light";
  applyTheme(resolveTheme(pref));
  if (pref === "system") {
    try {
      mediaQueryList = window.matchMedia("(prefers-color-scheme: dark)");
      mediaListener = (ev) => {
        applyTheme(ev.matches ? "dark" : "light");
      };
      mediaQueryList.addEventListener("change", mediaListener);
    } catch {
      // No matchMedia available; silently skip.
    }
  }
  return detachMediaListener;
}
