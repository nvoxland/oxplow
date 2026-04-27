/**
 * Monaco-theme helper. Bridges the app's `data-theme` attribute (set
 * by `theme.ts`) to a Monaco theme id so the embedded code editors
 * (`EditorPane`, `DiffPane`) follow the app theme rather than hard-
 * coding `vs-dark`.
 *
 * Both editors call `getActiveMonacoTheme()` once at create time and
 * subscribe via `subscribeMonacoTheme` to re-apply when the user
 * flips the toggle. We intentionally don't import monaco here so the
 * helper stays lightweight and unit-testable without spinning up the
 * editor module.
 */
import { getStoredThemePref, resolveTheme, subscribeThemePref, type ResolvedTheme } from "./theme.js";

export type MonacoThemeId = "vs" | "vs-dark";

/** Map a resolved app theme to the matching Monaco theme id. */
export function monacoThemeFor(theme: ResolvedTheme): MonacoThemeId {
  return theme === "dark" ? "vs-dark" : "vs";
}

/**
 * Read the active Monaco theme by inspecting `document.documentElement`'s
 * `data-theme` attribute. Falls back to "vs" (light) when the attribute is
 * missing or unrecognized — light is the install default.
 */
export function getActiveMonacoTheme(): MonacoThemeId {
  try {
    const attr = document.documentElement.getAttribute("data-theme");
    if (attr === "dark") return "vs-dark";
    return "vs";
  } catch {
    return "vs";
  }
}

/**
 * Subscribe to monaco-theme changes. Wraps `subscribeThemePref` and
 * resolves preference → concrete theme so callers receive a Monaco
 * theme id directly. Returns an unsubscribe function.
 */
export function subscribeMonacoTheme(fn: (theme: MonacoThemeId) => void): () => void {
  return subscribeThemePref((pref) => {
    fn(monacoThemeFor(resolveTheme(pref)));
  });
}

/** Convenience: resolve the user's current preference to a Monaco theme. */
export function currentMonacoTheme(): MonacoThemeId {
  const pref = getStoredThemePref() ?? "light";
  return monacoThemeFor(resolveTheme(pref));
}
