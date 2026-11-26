import { expect, test } from "bun:test";
import { readdirSync, readFileSync, statSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";

// Rules that keep the page layer usable on a touch screen. Each rule has a
// ratchet list of today's sites (file -> count). The list must match the
// source exactly: a new site fails, and a fixed site must leave the list.

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = join(HERE, "..");
const SRC = HERE;

function sources(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) return name === "generated" || name === "node_modules" ? [] : sources(path);
    return /\.(tsx?|css)$/.test(name) && !/\.(test|spec)\.tsx?$/.test(name) && !/\.d\.ts$/.test(name) ? [path] : [];
  });
}

/** Code without its comments (a doc comment may name an example). */
function code(text: string): string {
  return text.replace(/\/\*[\s\S]*?\*\//g, (m) => m.replace(/[^\n]/g, "")).replace(/(^|[^:])\/\/.*$/gm, "$1");
}

/** Count matches of `re` per file (path relative to src, or to the repo for the kit sheet). */
function count(files: string[], base: string, re: RegExp, exempt: string[] = []): Record<string, number> {
  const out: Record<string, number> = {};
  for (const file of files) {
    const rel = relative(base, file);
    if (exempt.includes(rel)) continue;
    const n = code(readFileSync(file, "utf8")).split("\n").filter((l) => re.test(l)).length;
    if (n > 0) out[rel] = n;
  }
  return out;
}

const srcFiles = sources(SRC);

// a. A context menu opens through `useRowContextMenu`, which also gives a
// finger's long press. The hook is the one place that wires the event.
const CONTEXT_MENU_EXEMPT = ["components/useRowContextMenu.tsx"];
/** Raw `onContextMenu=` sites still to move onto the hook (file -> count). */
const CONTEXT_MENU_KEPT: Record<string, number> = {
  "components/CenterTabs/CenterTabs.tsx": 1,
  "components/Context/ContextDockPanel.tsx": 4,
  "components/EditorPane.tsx": 2,
  "components/Markdown/MarkdownView.tsx": 1,
  "components/Markdown/RichTextField.tsx": 1,
  "components/Navigator.tsx": 2,
  "components/RailHud/RailHud.tsx": 2,
  "components/TitleBar.tsx": 2,
  "launcher/Launcher.tsx": 1,
  "pages/DashboardGrid.tsx": 1,
  "tabs/BacklinksList.tsx": 2,
  "tabs/RouteLink.tsx": 1,
  "terminal/TerminalSurface.tsx": 1,
  "terminal/TerminalTabStrip.tsx": 1,
};

test("no raw onContextMenu= outside the shared hook (ratchet)", () => {
  expect(count(srcFiles, SRC, /\bonContextMenu=/, CONTEXT_MENU_EXEMPT)).toEqual(CONTEXT_MENU_KEPT);
});

// b. A drag MIME is declared once, in `dragMimes.ts`.
const MIME_EXEMPT = ["dragMimes.ts"];
/** Drag MIME literals still to move into `dragMimes.ts` (file -> count). */
const MIME_KEPT: Record<string, number> = {
  "components/CenterTabs/CenterTabs.tsx": 1,
  "pages/DashboardGrid.tsx": 1,
};

test("no drag MIME literal outside dragMimes.ts (ratchet)", () => {
  expect(count(srcFiles, SRC, /["'`]application\/x-oxplow/, MIME_EXEMPT)).toEqual(MIME_KEPT);
});

// c. Text in the page layer reads the size tokens (`var(--text-xs)` ...): a
// touch screen makes the smallest ones bigger. A pixel size of 12 or less
// does not follow.
const LAYER = ["viz", "kit", "pages", "tabs", "components"].map((d) => join(SRC, d));
const KIT_SHEET = join(ROOT, "..", "..", "crates/oxplow-app/assets/component/oxplow-kit.css");
const SMALL_FONT = /font-?[sS]ize: *["']?([0-9]|1[0-2])(\.[0-9]+)?(px)?["']?[,; }]/;
/** Small pixel font sizes still to move onto tokens (file -> count). */
const FONT_KEPT: Record<string, number> = {
  "components/Analytics/DailyBarChart.tsx": 1,
  "components/BackgroundTaskIndicator.tsx": 6,
  "components/BranchPicker.tsx": 9,
  "components/ContextMenu.tsx": 1,
  "components/Diff/DiffPane.tsx": 2,
  "components/Dock/DockBar.tsx": 1,
  "components/EditorPane.tsx": 3,
  "components/ErrorBoundary.tsx": 2,
  "components/Git/GitDialogs.tsx": 2,
  "components/InlineConfirm.tsx": 3,
  "components/InlinePromptStrip.tsx": 4,
  "components/Markdown/FencedBlock.tsx": 1,
  "components/Markdown/mermaidRender.ts": 2,
  "components/QuickOpenOverlay.tsx": 13,
  "components/RailHud/RailHud.tsx": 6,
  "components/RefBadge.tsx": 1,
  "components/TitleBar.tsx": 1,
  "components/UndoToast.tsx": 1,
  "tabs/BacklinksList.tsx": 4,
  "tabs/Page.tsx": 1,
  "tabs/PageNavBar.tsx": 5,
  "viz/VizView.tsx": 2,
};

test("no font size of 12px or less in the page layer (ratchet)", () => {
  const files = LAYER.flatMap((d) => sources(d));
  const got = count(files, SRC, SMALL_FONT);
  Object.assign(got, count([KIT_SHEET], join(ROOT, "..", ".."), SMALL_FONT));
  expect(got).toEqual(FONT_KEPT);
});
