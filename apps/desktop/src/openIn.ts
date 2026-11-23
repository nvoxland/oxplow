/// Open In ▸: the one submenu a path or URL the window recognizes gets on
/// right-click — in the terminal, a markdown link, a page row, a tab, the
/// editor. Oxplow's own items are the OS file manager, the default app, a
/// terminal there (a file's: its folder), the system browser and oxplow's
/// web tab, those that apply to the target and to this window; an
/// extension's items for the link join after them (`extra`).
/// `.context/usability.md` "Open In".
import { filesAreLocal, openInSystemBrowser, openPath, revealPath } from "./api.js";
import { offersForRef, refCommandMenuItems } from "./components/refCommands.js";
import type { CommandEntry } from "./components/quickOpenResults.js";
import { classifyExternalUrl } from "./external-url-allowlist.js";
import type { MenuItem } from "./menu.js";
import { recordOpError } from "./opErrors.js";
import { currentPlatform, revealLabel, type Platform } from "./platform.js";
import { WORKING } from "./revision.js";
import { dirUri, fileUri, isExternal, paramOf, typeOf } from "./uri/uri.js";

/** What is opened: a local path (absolute) or a web URL. */
export type OpenTarget = { kind: "file" | "dir"; path: string } | { kind: "url"; url: string };

/** What the window can do with a target, and where it is. */
export interface OpenInEnv {
  platform: Platform;
  /** The window's files are on this machine (`filesAreLocal`). */
  filesLocal: boolean;
  reveal(path: string): Promise<void>;
  openPath(path: string): Promise<void>;
  openBrowser(url: string): Promise<void>;
  /** Open `url` in oxplow's web tab; null outside a page. */
  openInApp: ((url: string) => void) | null;
  inAppAllowed(url: string): boolean;
  /** Open an oxplow terminal at directory `dir`; null where none can. */
  openTerminalAt: ((dir: string) => void) | null;
}

/** The directory `path` is in. */
export function parentDir(path: string): string {
  const trimmed = path.replace(/\/+$/, "");
  const at = trimmed.lastIndexOf("/");
  return at <= 0 ? "/" : trimmed.slice(0, at);
}

const report = (label: string) => (e: unknown) =>
  recordOpError({ label, message: e instanceof Error ? e.message : String(e) });

/** Oxplow's own Open In items for `target`. */
function builtinItems(target: OpenTarget, env: OpenInEnv): MenuItem[] {
  const items: MenuItem[] = [];
  if (target.kind === "url") {
    const { url } = target;
    items.push({ id: "open-in.browser", label: "Browser", enabled: true, run: () => void env.openBrowser(url).catch(report("Open in browser")) });
    const inApp = env.openInApp;
    if (inApp && env.inAppAllowed(url)) items.push({ id: "open-in.oxplow", label: "Oxplow", enabled: true, run: () => inApp(url) });
    return items;
  }
  const { path } = target;
  if (env.filesLocal) {
    items.push({ id: "open-in.reveal", label: revealLabel(env.platform), enabled: true, run: () => void env.reveal(path).catch(report("Reveal a path")) });
    if (target.kind === "file") {
      items.push({ id: "open-in.default-app", label: "Default App", enabled: true, run: () => void env.openPath(path).catch(report("Open with the default app")) });
    }
  }
  const terminal = env.openTerminalAt;
  if (terminal) {
    const dir = target.kind === "file" ? parentDir(path) : path;
    items.push({ id: "open-in.terminal", label: "Terminal", enabled: true, run: () => terminal(dir) });
  }
  return items;
}

/** The Open In ▸ item for `target`: oxplow's items, then `extra` (an
 *  extension's, for this link). Null when nothing applies. */
export function openInMenu(target: OpenTarget, env: OpenInEnv, extra: MenuItem[] = []): MenuItem | null {
  const submenu = [...builtinItems(target, env), ...extra];
  if (submenu.length === 0) return null;
  return { id: "open-in", label: "Open In", enabled: true, submenu };
}

/** The target a ref names, with a file or directory URI's path joined
 *  onto `worktree` (the stream's `worktree_path`): only a working-copy
 *  path on the local filesystem is a file on disk. A web URL is itself.
 *  Null for anything else. */
export function localTarget(ref: string, worktree: string | null): OpenTarget | null {
  if (isExternal(ref)) return /^https?:\/\//i.test(ref) ? { kind: "url", url: ref } : null;
  const type = typeOf(ref);
  if (type !== "oxplow/file" && type !== "oxplow/dir") return null;
  const rev = paramOf(ref, "rev");
  if ((rev && rev !== WORKING) || paramOf(ref, "filesystem") || !worktree) return null;
  const root = worktree.replace(/\/+$/, "");
  const rel = (paramOf(ref, "path") ?? "").replace(/^\/+/, "").replace(/\/+$/, "");
  return { kind: type === "oxplow/file" ? "file" : "dir", path: rel ? `${root}/${rel}` : root };
}

/** Whether oxplow's web tab may show `url` (the allowlist). */
export const inAppAllowed = (url: string): boolean => classifyExternalUrl(url).ok;

/** Open `url` in the system browser; a failure is reported, not thrown. */
export function openInBrowser(url: string): void {
  void openInSystemBrowser(url).catch(report("Open in browser"));
}

/** What only the window knows: its stream's worktree, how it opens a web
 *  tab and a terminal. `App` sets it (`setOpenInHost`). */
export interface OpenInHost {
  worktree: string | null;
  openInApp(url: string): void;
  openTerminalAt(dir: string): void;
}

let host: OpenInHost | null = null;

/** The window says how it opens things (`App`); null when it closes. */
export function setOpenInHost(next: OpenInHost | null): void {
  host = next;
}

/** This window's Open In environment. */
export function openInEnv(): OpenInEnv {
  return {
    platform: currentPlatform(),
    filesLocal: filesAreLocal(),
    reveal: revealPath,
    openPath,
    openBrowser: openInSystemBrowser,
    openInApp: host ? host.openInApp : null,
    inAppAllowed,
    openTerminalAt: host ? host.openTerminalAt : null,
  };
}

/** The Open In ▸ item for `ref` (a file or directory URI, or a web URL)
 *  in this window, with `extra` after oxplow's items; null when none
 *  applies. */
export function openInForRef(ref: string, extra: MenuItem[] = []): MenuItem | null {
  const target = localTarget(ref, host?.worktree ?? null);
  if (!target) return extra.length > 0 ? { id: "open-in", label: "Open In", enabled: true, submenu: extra } : null;
  return openInMenu(target, openInEnv(), extra);
}

/** The Open In ▸ item for a local `path` (absolute) in this window.
 *  `terminal: false` leaves out Terminal: a path of another stream (a
 *  terminal opens in this window's stream). */
export function openInForPath(
  kind: "file" | "dir",
  path: string,
  extra: MenuItem[] = [],
  opts: { terminal?: boolean } = {},
): MenuItem | null {
  const env = openInEnv();
  return openInMenu({ kind, path }, opts.terminal === false ? { ...env, openTerminalAt: null } : env, extra);
}

/** The Open In ▸ item for a path relative to this window's worktree;
 *  null outside a window. */
export function openInForWorktreePath(kind: "file" | "dir", relPath: string, extra: MenuItem[] = []): MenuItem | null {
  const worktree = host?.worktree;
  if (!worktree) return null;
  const root = worktree.replace(/\/+$/, "");
  const rel = relPath.replace(/^\/+/, "").replace(/\/+$/, "");
  return openInForPath(kind, rel ? `${root}/${rel}` : root, extra);
}

/** The `ui.group` whose commands about a link join its Open In ▸. */
export const OPEN_IN_GROUP = "Open In";

/** `items`, a link's menu, with the extensions' commands about `ref`
 *  as it opens: those in the Open In group after oxplow's items in its
 *  Open In ▸ (one is added when `items` has none), the rest a submenu per
 *  group at the end (`refCommandMenuItems`). `offers` is the daemon's say
 *  on which apply (`offersForRef`). */
export async function withLinkOffers(
  items: MenuItem[],
  ref: string | null,
  offers: (ref: string) => Promise<CommandEntry[]> = offersForRef,
): Promise<MenuItem[]> {
  const entries = ref ? await offers(ref) : [];
  if (entries.length === 0) return items;
  const { items: merged, rest } = mergeOpenInOffers(items, entries);
  return [...merged, ...refCommandMenuItems(rest)];
}

/** `items` with `entries`' Open In group in its Open In ▸ (one is added
 *  when `items` has none), and the entries of other groups, for the caller
 *  to place. */
export function mergeOpenInOffers(items: MenuItem[], entries: CommandEntry[]): { items: MenuItem[]; rest: CommandEntry[] } {
  const extra: MenuItem[] = entries
    .filter((e) => e.group === OPEN_IN_GROUP)
    .map((e) => ({ id: `ref-command-${e.id}`, label: e.label, enabled: e.enabled ?? true, run: e.run }));
  const rest = entries.filter((e) => e.group !== OPEN_IN_GROUP);
  if (extra.length === 0) return { items, rest };
  const at = items.findIndex((i) => i.id === "open-in");
  const merged =
    at === -1
      ? [...items, { id: "open-in", label: "Open In", enabled: true, submenu: extra }]
      : items.map((i, n) => (n === at ? { ...i, submenu: [...(i.submenu ?? []), ...extra] } : i));
  return { items: merged, rest };
}

/** The ref the daemon is asked about for `target` (a link's commands):
 *  a web URL itself, a path inside this window's worktree as its file or
 *  directory URI; null for a path outside it. */
export function linkRefOf(target: OpenTarget): string | null {
  if (target.kind === "url") return target.url;
  const root = host?.worktree?.replace(/\/+$/, "");
  if (!root) return null;
  if (target.path !== root && !target.path.startsWith(`${root}/`)) return null;
  const rel = target.path === root ? "" : target.path.slice(root.length + 1);
  return target.kind === "file" ? fileUri(rel) : dirUri(rel);
}
