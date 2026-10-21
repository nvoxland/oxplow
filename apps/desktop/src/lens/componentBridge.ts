/// The host side of a custom component's bridge (P6b.D4,
/// `.context/extensions.md` → "Custom components"). The frame is
/// sandboxed (`allow-scripts allow-forms`: an opaque origin, no network,
/// no token; a form submits nowhere, its script handles it); after it loads, the host hands it one end of a MessageChannel
/// with `init`, and every request comes back over that port — never over
/// `window` messages anyone could send:
///
///   frame → host  { type: "ready" }
///                 { id, method: "query", asset, params }     a declared lens's run
///                 { id, method: "invoke", command, input }   a declared command
///                 { id, method: "navigate", ref, newTab }    open a page
///                 { id, method: "querySql", sql, params }    its own SQL (`sql: read`)
///                 { id, method: "openForm", command, input } a declared command's form, in the host
///                 { id, method: "setTitle", title }          a page's title (a page only)
///                 { id, method: "setActions", actions }      a page's actions (a page only)
///                 { id, method: "setViewParams", params }    its host's view params (a host with a view)
///                 { id, method: "setHistoryState", patch }   this step of its tab (a host with a view)
///                 { id, method: "pageMemoryGet", key }       its own memory, for the person
///                 { id, method: "pageMemorySet", key, value } (null forgets it)
///   host → frame  { type: "init", protocol, run, props, page, view, tokens }
///                 { type: "update", run }                    the lens re-ran
///                 { type: "action", id }                     a page action was pressed
///                 { type: "viewParams", params }             its view params changed (Back, Forward)
///                 { type: "historyState", state }            its history state changed (Back, Forward)
///                 { id, ok: true, result } | { id, ok: false, error: { code, message, field? } }
///
/// Shown by a lens, `init` carries the lens's run and `page` is null; shown
/// as a page (`pages[].component`), `run` is null and `page` carries the
/// page's stream and thread. `view` is its host's view — view params and
/// history state — or null: a page has one (the whole tab entry), so does
/// a replacement (the keys its target's contract lists) and a lens's own
/// page's component (the lens's params); a tile, a slot or the rail gives
/// none.
///
/// A command that asks is confirmed by the person in the host, never in
/// the frame. The daemon serves a client library wrapping this
/// (`/component-lib/oxplow-component.js`, `oxplow.connect()`); it checks
/// `protocol` against the one it speaks.
import { IpcCallError, needsConfirmation } from "../ipc-error.js";
import { parseRef } from "../refs/ref.js";
import type { CommandOutcome, LensRun, SqlCell, SqlQueryResult } from "../tauri-bridge/generated/bindings.js";

/// The protocol above, by number: bumped when a message's shape changes,
/// with the served library (`crates/oxplow-daemon/assets/`). 2: `init`
/// lost `kitCss` — the kit is a stylesheet the bundle links (tsk961).
/// 3: `navigate` carries `newTab` (a Cmd-click in a component). 4: a
/// refused request's error carries the input `field` it names, when it
/// names one (`INVALID`), so a component's form marks that field.
/// 5: a component can be a page — `init.page`, a null `run` — and asks
/// `querySql`, `openForm`, `setTitle`, `setActions`; the host sends
/// `action`. 6: a page's params are its tab entry's view (`setParams`,
/// `params`). 7: each kind of page state by the name of its scope —
/// **view params** (`page.viewParams`, `setViewParams`, `viewParams`),
/// **history state** (`page.historyState`, `setHistoryState`,
/// `historyState`) and **page memory** (`pageMemoryGet`, `pageMemorySet`:
/// the component's own, for the person, every tab of it). 8: the view is
/// the host's, not the page's — `init.view` beside `init.page` — so a
/// replacement and a lens page's component have one too.
export const BRIDGE_PROTOCOL = 8;

/** A page action a component declares; core draws it in the nav bar. */
export interface FrameAction {
  id: string;
  label: string;
  primary: boolean;
  disabled: boolean;
}

/** What a component shown as a page is told about it. */
export interface PageInit {
  streamId: string | null;
  threadId: string | null;
}

/** Its host's view: what a page, a replacement or a lens page's
 *  component shares of its tab. */
export interface ViewInit {
  /** Its tab's address, as far as it is the component's: what it shows. */
  viewParams: Record<string, string>;
  /** This step of its tab's history: what it kept there. */
  historyState: Record<string, unknown>;
}

export type BridgeRequest =
  | { id: string; method: "query"; asset: string; params: Record<string, SqlCell> }
  | { id: string; method: "invoke"; command: string; input: unknown }
  | { id: string; method: "navigate"; ref: string; newTab: boolean }
  | { id: string; method: "querySql"; sql: string; params: SqlCell[] }
  | { id: string; method: "openForm"; command: string; input: unknown }
  | { id: string; method: "setTitle"; title: string }
  | { id: string; method: "setActions"; actions: FrameAction[] }
  | { id: string; method: "setViewParams"; params: Record<string, string | null> }
  | { id: string; method: "setHistoryState"; patch: Record<string, unknown> }
  | { id: string; method: "pageMemoryGet"; key: string }
  | { id: string; method: "pageMemorySet"; key: string; value: unknown };

export type FrameMessage = { type: "ready" } | { type: "request"; request: BridgeRequest };

const isCell = (v: unknown): v is SqlCell =>
  v === null || typeof v === "string" || typeof v === "number" || typeof v === "boolean";

/** A message from the frame, or `null` when it isn't one of ours. */
export function parseFrameMessage(data: unknown): FrameMessage | null {
  if (!data || typeof data !== "object") return null;
  const m = data as Record<string, unknown>;
  if (m.type === "ready") return { type: "ready" };
  if (typeof m.id !== "string" || !m.id) return null;
  switch (m.method) {
    case "query": {
      const params = m.params ?? {};
      if (typeof m.asset !== "string" || !params || typeof params !== "object" || Array.isArray(params)) return null;
      if (!Object.values(params).every(isCell)) return null;
      return { type: "request", request: { id: m.id, method: "query", asset: m.asset, params: params as Record<string, SqlCell> } };
    }
    case "invoke":
      if (typeof m.command !== "string") return null;
      return { type: "request", request: { id: m.id, method: "invoke", command: m.command, input: m.input ?? {} } };
    case "navigate":
      if (typeof m.ref !== "string") return null;
      if (m.newTab !== undefined && typeof m.newTab !== "boolean") return null;
      return { type: "request", request: { id: m.id, method: "navigate", ref: m.ref, newTab: m.newTab === true } };
    case "querySql": {
      const params = m.params ?? [];
      if (typeof m.sql !== "string" || !Array.isArray(params) || !params.every(isCell)) return null;
      return { type: "request", request: { id: m.id, method: "querySql", sql: m.sql, params: params as SqlCell[] } };
    }
    case "openForm":
      if (typeof m.command !== "string") return null;
      return { type: "request", request: { id: m.id, method: "openForm", command: m.command, input: m.input ?? {} } };
    case "setTitle":
      if (typeof m.title !== "string") return null;
      return { type: "request", request: { id: m.id, method: "setTitle", title: m.title } };
    case "setActions": {
      if (!Array.isArray(m.actions)) return null;
      const actions: FrameAction[] = [];
      for (const raw of m.actions as unknown[]) {
        const a = raw as Record<string, unknown> | null;
        if (!a || typeof a.id !== "string" || !a.id || typeof a.label !== "string") return null;
        actions.push({ id: a.id, label: a.label, primary: a.primary === true, disabled: a.disabled === true });
      }
      return { type: "request", request: { id: m.id, method: "setActions", actions } };
    }
    case "setViewParams": {
      const params = m.params;
      if (!params || typeof params !== "object" || Array.isArray(params)) return null;
      if (!Object.values(params).every((v) => v === null || typeof v === "string")) return null;
      return { type: "request", request: { id: m.id, method: "setViewParams", params: params as Record<string, string | null> } };
    }
    case "setHistoryState": {
      const patch = m.patch;
      if (!patch || typeof patch !== "object" || Array.isArray(patch)) return null;
      return { type: "request", request: { id: m.id, method: "setHistoryState", patch: patch as Record<string, unknown> } };
    }
    case "pageMemoryGet":
      if (typeof m.key !== "string" || !m.key) return null;
      return { type: "request", request: { id: m.id, method: "pageMemoryGet", key: m.key } };
    case "pageMemorySet":
      if (typeof m.key !== "string" || !m.key) return null;
      return { type: "request", request: { id: m.id, method: "pageMemorySet", key: m.key, value: m.value ?? null } };
    default:
      return null;
  }
}

/** Where the daemon serves the bundle its host loaded at `version`
 *  (tsk984): a folder of its own, so the bundle's relative URLs stay in
 *  it. */
export function componentBundleUrl(base: string, version: string): string {
  return `${base}/components/v/${encodeURIComponent(version)}/`;
}

/** The theme's tokens: the root's custom properties (`--text-primary`)
 *  and its `color-scheme`. */
export function tokensFromStyle(style: {
  length: number;
  item(i: number): string;
  getPropertyValue(name: string): string;
}): Record<string, string> {
  const out: Record<string, string> = {};
  for (let i = 0; i < style.length; i++) {
    const name = style.item(i);
    if (name.startsWith("--")) out[name] = style.getPropertyValue(name).trim();
  }
  // A frame whose color-scheme differs from its embedder's is painted
  // opaque white; the component adopts the host's (`applyTheme`).
  const scheme = style.getPropertyValue("color-scheme")?.trim();
  if (scheme) out["color-scheme"] = scheme;
  return out;
}

/** The `init` the host posts to a component's frame: the protocol it
 *  speaks, the first run and the lens's `custom.props` (a lens shows it)
 *  or the page it is (`page`), its host's view (`view`), and the theme's
 *  tokens. One builder, so the library's tests read the message the host
 *  really sends. */
export function initMessage(
  run: LensRun | null,
  tokens: Record<string, string>,
  page: PageInit | null = null,
  view: ViewInit | null = null,
) {
  return {
    type: "init" as const,
    protocol: BRIDGE_PROTOCOL,
    run,
    props: run?.lens.custom?.props ?? null,
    page,
    view,
    tokens,
  };
}

export interface BridgeDeps {
  /** Run a declared lens (`run_component_query`). */
  query(asset: string, params: Record<string, SqlCell>): Promise<LensRun>;
  /** Run a declared command as the lens for the person. */
  invoke(command: string, input: unknown, confirmed: boolean): Promise<CommandOutcome>;
  /** Open `ref`'s page, in a new tab when asked; false when it isn't one
   *  a component may open (`componentMayOpen`). */
  navigate(ref: string, opts: { newTab: boolean }): boolean;
  /** Ask the person, in the host, to confirm `command`. */
  confirm(command: string): Promise<boolean>;
  onReady(): void;
  /** Run its own read-only SQL (`run_component_sql`). */
  querySql?(sql: string, params: SqlCell[]): Promise<SqlQueryResult>;
  /** Show a declared command's form in the host and run it as submitted:
   *  `{ result }`, or null when the person closed it. A page only. */
  openForm?(command: string, input: unknown): Promise<{ result: unknown } | null>;
  /** The page's title, in the tab and the nav bar. A page only. */
  setTitle?(title: string): void;
  /** The page's actions, drawn by core in the nav bar. A page only. */
  setActions?(actions: FrameAction[]): void;
  /** Change its host's view params (null drops one); throws `{ code:
   *  "INVALID" }` for a key not its to set. A host with a view only. */
  setViewParams?(params: Record<string, string | null>): void;
  /** Merge `patch` into its history state. A host with a view only. */
  setHistoryState?(patch: Record<string, unknown>): void;
  /** Its page memory's `key`; null when none is kept. */
  pageMemoryGet?(key: string): Promise<unknown>;
  /** Keep `value` (JSON) as its page memory's `key`, null forgetting it;
   *  throws `{ code: "INVALID" }` past the size caps. */
  pageMemorySet?(key: string, value: unknown): void;
}

/** Why an invoke was refused (`DENIED`: the component isn't approved, or a
 *  policy said no), for the host to show beside the frame — the frame's
 *  own code may not (tsk960). Any other failure is the frame's to show. */
export function componentRefusal(e: unknown): string | null {
  return e instanceof IpcCallError && e.code === "DENIED" ? e.message : null;
}

/** Whether a component's `navigate` may open `ref`: an oxplow ref, never
 *  an outside URL (the frame has no network, and an external-url tab would
 *  carry whatever it put in the URL out). It opens where its kind's page
 *  says (`openRef`). */
export function componentMayOpen(ref: string): boolean {
  const parsed = parseRef(ref);
  return parsed !== null && !(parsed.kind === "page" && ref.startsWith("page:external-url"));
}

/** What a run shows the frame: an `update` repeats none of it. */
const shown = (run: LensRun | null) => (run ? JSON.stringify([run.params, run.result]) : "");

/** What a component shown by a lens may not do: the page is its host's. */
const PAGE_ONLY = "Only a component shown as a page sets its title, actions or forms.";
/** What a component in a host with no view may not do. */
const NO_VIEW = "This host gives the component no view: only a page, a replacement or a lens's own page does.";

/** Answer the frame's requests arriving on `port`. `sent` is the run the
 *  `init` message carried and `sentView` the view it told; `update`,
 *  `viewParams` and `historyState` post only what the frame hasn't seen,
 *  and `ready` is heard once. */
export function createBridgeHost(
  port: MessagePort,
  deps: BridgeDeps,
  sent: LensRun | null,
  sentView: ViewInit | null = null,
): {
  update(run: LensRun): void;
  viewParams(params: Record<string, string>): void;
  historyState(state: Record<string, unknown>): void;
  action(id: string): void;
  close(): void;
} {
  let last = shown(sent);
  let lastView = sentView ? JSON.stringify(sentView.viewParams) : null;
  let lastState = sentView ? JSON.stringify(sentView.historyState) : null;
  let ready = false;
  const reply = (id: string, result: unknown) => port.postMessage({ id, ok: true, result });
  const fail = (id: string, code: string, message: string, field: string | null = null) =>
    port.postMessage({ id, ok: false, error: field === null ? { code, message } : { code, message, field } });
  const failWith = (id: string, e: unknown) => {
    const code = (e as { code?: unknown })?.code;
    const field = (e as { field?: unknown })?.field;
    fail(
      id,
      typeof code === "string" ? code : "FAILED",
      e instanceof Error ? e.message : String(e),
      typeof field === "string" && field ? field : null,
    );
  };
  const handle = async (r: BridgeRequest) => {
    switch (r.method) {
      case "query":
        try {
          reply(r.id, await deps.query(r.asset, r.params));
        } catch (e) {
          failWith(r.id, e);
        }
        return;
      case "invoke":
        try {
          reply(r.id, (await deps.invoke(r.command, r.input, false)).result);
        } catch (e) {
          if (!needsConfirmation(e)) return failWith(r.id, e);
          if (!(await deps.confirm(r.command))) {
            return fail(r.id, "CANCELLED", `The person didn't confirm \`${r.command}\`.`);
          }
          try {
            reply(r.id, (await deps.invoke(r.command, r.input, true)).result);
          } catch (again) {
            failWith(r.id, again);
          }
        }
        return;
      case "navigate":
        if (deps.navigate(r.ref, { newTab: r.newTab })) reply(r.id, null);
        else fail(r.id, "INVALID", "A component may open oxplow's pages only.");
        return;
      case "querySql":
        if (!deps.querySql) return fail(r.id, "INVALID", "This host runs no SQL for a component.");
        try {
          reply(r.id, await deps.querySql(r.sql, r.params));
        } catch (e) {
          failWith(r.id, e);
        }
        return;
      case "openForm": {
        if (!deps.openForm) return fail(r.id, "INVALID", PAGE_ONLY);
        try {
          const out = await deps.openForm(r.command, r.input);
          if (out === null) return fail(r.id, "CANCELLED", `The person closed the form for \`${r.command}\`.`);
          reply(r.id, out.result);
        } catch (e) {
          failWith(r.id, e);
        }
        return;
      }
      case "setTitle":
        if (!deps.setTitle) return fail(r.id, "INVALID", PAGE_ONLY);
        deps.setTitle(r.title);
        return reply(r.id, null);
      case "setActions":
        if (!deps.setActions) return fail(r.id, "INVALID", PAGE_ONLY);
        deps.setActions(r.actions);
        return reply(r.id, null);
      case "setViewParams":
        if (!deps.setViewParams) return fail(r.id, "INVALID", NO_VIEW);
        try {
          deps.setViewParams(r.params);
          reply(r.id, null);
        } catch (e) {
          failWith(r.id, e);
        }
        return;
      case "setHistoryState":
        if (!deps.setHistoryState) return fail(r.id, "INVALID", NO_VIEW);
        try {
          deps.setHistoryState(r.patch);
          reply(r.id, null);
        } catch (e) {
          failWith(r.id, e);
        }
        return;
      case "pageMemoryGet":
        if (!deps.pageMemoryGet) return fail(r.id, "INVALID", "This host keeps no page memory.");
        try {
          reply(r.id, (await deps.pageMemoryGet(r.key)) ?? null);
        } catch (e) {
          failWith(r.id, e);
        }
        return;
      case "pageMemorySet":
        if (!deps.pageMemorySet) return fail(r.id, "INVALID", "This host keeps no page memory.");
        try {
          deps.pageMemorySet(r.key, r.value);
          reply(r.id, null);
        } catch (e) {
          failWith(r.id, e);
        }
        return;
    }
  };
  port.onmessage = (e: MessageEvent) => {
    const m = parseFrameMessage(e.data);
    if (m?.type === "ready") {
      if (ready) return;
      ready = true;
      return deps.onReady();
    }
    if (m?.type === "request") return void handle(m.request);
    const id = (e.data as { id?: unknown } | null)?.id;
    if (typeof id === "string" && id) fail(id, "BAD_REQUEST", "Not a request this host knows.");
  };
  return {
    update: (run) => {
      const key = shown(run);
      if (key === last) return;
      last = key;
      port.postMessage({ type: "update", run });
    },
    viewParams: (params) => {
      const key = JSON.stringify(params);
      if (key === lastView) return;
      lastView = key;
      port.postMessage({ type: "viewParams", params });
    },
    historyState: (state) => {
      const key = JSON.stringify(state);
      if (key === lastState) return;
      lastState = key;
      port.postMessage({ type: "historyState", state });
    },
    action: (id) => port.postMessage({ type: "action", id }),
    close: () => port.close(),
  };
}
