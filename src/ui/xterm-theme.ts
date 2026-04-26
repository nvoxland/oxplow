/**
 * Xterm-theme helper. Bridges the app's `data-theme` attribute (set by
 * `theme.ts`) to an xterm.js `ITheme` object so the embedded terminal
 * follows the app theme rather than hard-coding a dark palette.
 *
 * `TerminalPane` calls `getActiveXtermTheme()` once at create time and
 * subscribes via `subscribeXtermTheme` to re-apply on toggle (xterm
 * picks up `terminal.options.theme = next` immediately — no re-mount).
 *
 * Background, foreground, cursor, and selection read from the same
 * `--surface-card` / `--text-primary` / `--accent` / `--accent-soft-bg`
 * variables the rest of the UI uses, so the terminal sits flush with
 * surrounding chrome. The 16 ANSI slots use vetted One Light / One Dark
 * palettes hard-coded here — terminal output uses all 16 and arbitrary
 * UI accents would collide with terminal-color conventions.
 */
import { getStoredThemePref, resolveTheme, subscribeThemePref, type ResolvedTheme } from "./theme.js";

export interface XtermTheme {
  background: string;
  foreground: string;
  cursor: string;
  cursorAccent: string;
  selectionBackground: string;
  black: string;
  red: string;
  green: string;
  yellow: string;
  blue: string;
  magenta: string;
  cyan: string;
  white: string;
  brightBlack: string;
  brightRed: string;
  brightGreen: string;
  brightYellow: string;
  brightBlue: string;
  brightMagenta: string;
  brightCyan: string;
  brightWhite: string;
}

const ANSI_DARK = {
  black: "#3f4451",
  red: "#e05561",
  green: "#8cc265",
  yellow: "#d18f52",
  blue: "#4aa5f0",
  magenta: "#c162de",
  cyan: "#42b3c2",
  white: "#e6e6e6",
  brightBlack: "#4f5666",
  brightRed: "#ff616e",
  brightGreen: "#a5e075",
  brightYellow: "#f0a45d",
  brightBlue: "#4dc4ff",
  brightMagenta: "#de73ff",
  brightCyan: "#4cd1e0",
  brightWhite: "#ffffff",
} as const;

const ANSI_LIGHT = {
  black: "#383a42",
  red: "#e45649",
  green: "#50a14f",
  yellow: "#c18401",
  blue: "#4078f2",
  magenta: "#a626a4",
  cyan: "#0184bc",
  white: "#fafafa",
  brightBlack: "#696c77",
  brightRed: "#ca1243",
  brightGreen: "#50a14f",
  brightYellow: "#986801",
  brightBlue: "#4078f2",
  brightMagenta: "#a626a4",
  brightCyan: "#0997b3",
  brightWhite: "#ffffff",
} as const;

const FALLBACK_DARK = {
  background: "#1c1f24",
  foreground: "#e6e8ec",
  cursor: "#5b8cf5",
  selection: "#1f2a44",
} as const;

const FALLBACK_LIGHT = {
  background: "#ffffff",
  foreground: "#1a1d23",
  cursor: "#2563eb",
  selection: "#eef4ff",
} as const;

function readVar(name: string, fallback: string): string {
  try {
    const v = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
    return v || fallback;
  } catch {
    return fallback;
  }
}

/** Build an xterm `ITheme` for the resolved app theme, reading surface/text
 *  CSS vars at call time so callers always see the live values. */
export function xtermThemeFor(theme: ResolvedTheme): XtermTheme {
  const ansi = theme === "dark" ? ANSI_DARK : ANSI_LIGHT;
  const fb = theme === "dark" ? FALLBACK_DARK : FALLBACK_LIGHT;
  const background = readVar("--surface-card", fb.background);
  return {
    background,
    foreground: readVar("--text-primary", fb.foreground),
    cursor: readVar("--accent", fb.cursor),
    cursorAccent: background,
    selectionBackground: readVar("--accent-soft-bg", fb.selection),
    ...ansi,
  };
}

/** Read the active xterm theme by inspecting `document.documentElement`'s
 *  `data-theme` attribute. Falls back to "light" when unset. */
export function getActiveXtermTheme(): XtermTheme {
  try {
    const attr = document.documentElement.getAttribute("data-theme");
    return xtermThemeFor(attr === "dark" ? "dark" : "light");
  } catch {
    return xtermThemeFor("light");
  }
}

/** Subscribe to xterm-theme changes. Wraps `subscribeThemePref` and
 *  resolves preference → concrete theme so callers receive an ITheme
 *  directly. Returns an unsubscribe function. */
export function subscribeXtermTheme(fn: (theme: XtermTheme) => void): () => void {
  return subscribeThemePref((pref) => {
    fn(xtermThemeFor(resolveTheme(pref)));
  });
}

/** Convenience: resolve the user's current preference to an xterm theme. */
export function currentXtermTheme(): XtermTheme {
  const pref = getStoredThemePref() ?? "light";
  return xtermThemeFor(resolveTheme(pref));
}
