/// The host side of a custom component's bridge (P6b.D4,
/// `.context/extensions.md` → "Custom components"). The frame is
/// sandboxed (`allow-scripts allow-forms`: an opaque origin, no network,
/// no token; a form submits nowhere, its script handles it); after it loads, the host hands it one end of a MessageChannel
/// with `init`, and every request comes back over that port — never over
/// `window` messages anyone could send:
///
///   frame → host  { type: "ready" }
///                 { type: "size", height }                   its content's height
///                 { id, method: "invoke", command, input }   a declared command
///                 { id, method: "navigate", uri, newTab, replace }    open a page
///                 { id, method: "querySql", sql, params }    its own SQL (`sql: read`)
///                 { id, method: "openForm", command, input } a declared command's form, in the host
///                 { id, method: "openForm", form }           a form of its own schema, in the host: the values submitted
///                 { id, method: "signIn", instance, name }   sign in for a provider instance's credential, in the host
///                 { id, method: "cancelSignIn", instance, name } end that sign-in
///                 { id, method: "setTitle", title }          a page's title (a page only)
///                 { id, method: "setActions", actions }      a page's actions (a page only)
///                 { id, method: "setViewParams", params }    its host's view params (a host with a view)
///                 { id, method: "setHistoryState", patch }   this step of its tab (a host with a view)
///                 { id, method: "pageMemoryGet", key }       its own memory, for the person
///                 { id, method: "pageMemorySet", key, value } (null forgets it)
///                 { id, method: "showView", spec }           an inline page's YAML to draw under it (null clears; a page only)
///   host → frame  { type: "init", protocol, run, data, params, props, page, view, tokens }
///                 { type: "update", run, data, params, changed } its data changed
///                 { type: "action", id }                     a page action was pressed
///                 { type: "viewParams", params }             its view params changed (Back, Forward)
///                 { type: "historyState", state }            its history state changed (Back, Forward)
///                 { id, ok: true, result } | { id, ok: false, error: { code, message, field? } }
///
/// `data` is what it is fed, each loader's `{ columns, rows, truncated }`
/// by name (a page's loaders; a viz's own rows as `rows`), with the
/// `params` they ran with and its `props` (a page's component node's, a
/// `custom` viz's `custom.props`); `update` says which loaders `changed`.
/// `run` is a `custom` viz's own rows as a viz run (a viz's, or a page
/// node's), null for a page's component node. `page` carries the page's
/// stream and thread when it is a page's component, else null. `view` is
/// its host's view — view params and
/// history state — or null: a page has one (the whole tab entry), so does
/// a replacement (the keys its target's contract lists) and a viz's own
/// page's component (the viz's params); a tile, a slot or the rail gives
/// none.
///
/// A command that asks is confirmed by the person in the host, never in
/// the frame. The daemon serves a client library wrapping this
/// (`/component-lib/oxplow-component.js`, `oxplow.connect()`); it checks
/// `protocol` against the one it speaks.
import { ICONS } from "../icons.js";
import { IpcCallError, needsConfirmation } from "../ipc-error.js";
import { parseUri } from "../uri/uri.js";
import type { CommandOutcome, VizRun, PageRun, SqlCell, SqlQueryResult } from "../tauri-bridge/generated/bindings.js";

/// The protocol above, by number: bumped when a message's shape changes,
/// with the served library (`crates/oxplow-app/assets/component/`). 2: `init`
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
/// replacement and a viz page's component have one too. 9: a page's view
/// is its URI's `#` (`viewParams` are the fragment's), and `navigate`
/// carries `replace` as a browser's does. 10: a page shows a viz spec the
/// host draws under it with the kit (`showSpec`, null clearing it), and
/// the frame says its content's height (`size`), so the page lays the
/// frame out above what the host draws. 11: a page asks the host to sign in
/// for a provider instance's credential (`signIn`, `cancelSignIn`: the host
/// runs the whole redirect flow and answers how it ended), and `openForm`
/// also takes a form of its own (`{ title, schema, values }`) and answers
/// the values submitted. 12: a component is fed its page's data — `init`
/// and `update` carry `data` (each loader's rows), `params` and `props`,
/// `update` the loaders that `changed` — and a page shows an inline page
/// under it (`showView`, its YAML) where it showed a viz spec.
export const BRIDGE_PROTOCOL = 12;

/** One loader's rows as a component reads them (and what `showView`
 *  answers: the inline page's first loader's). */
export interface ShownRows {
  columns: string[];
  rows: SqlCell[][];
  truncated: boolean;
  /** Why it has none: the loader failed. */
  error?: string;
}

/** What a component is fed: its loaders' rows by name, the params they
 *  ran with, its props, and a `custom` viz's own rows as a viz run. */
export interface HostedData {
  run: VizRun | null;
  data: Record<string, ShownRows>;
  params: Record<string, SqlCell>;
  props: unknown;
}

const rowsOf = (r: SqlQueryResult, error: string | null = null): ShownRows => ({
  columns: r.columns,
  rows: r.rows,
  truncated: r.truncated,
  ...(error ? { error } : {}),
});

/** A viz's component's data: its own rows (`rows`), its params, its
 *  `custom.props`. */
export function vizData(run: VizRun): HostedData {
  return { run, data: { rows: rowsOf(run.result) }, params: run.params, props: run.config.custom?.props ?? null };
}

/** A page's component's data: every loader of `run`, its params, and
 *  `props` (a component node's own, or a `custom` viz's); `own` is a
 *  `custom` viz node's rows as a viz run. */
export function pageData(run: PageRun, props: unknown, own: VizRun | null = null): HostedData {
  const data: Record<string, ShownRows> = {};
  for (const [name, l] of Object.entries(run.loaders)) if (l) data[name] = rowsOf(l.result, l.error);
  return { run: own, data, params: run.params, props: props ?? null };
}

/** A form of the component's own, shown by the host: a JSON Schema for
 *  its fields, starting from `values`. */
export interface OwnForm {
  title: string;
  schema: Record<string, unknown>;
  values: Record<string, unknown>;
}

/** How a sign-in the host ran ended. */
export type SignInResult = { state: "signed_in" } | { state: "failed"; error: string } | { state: "cancelled" };

/** A page action a component declares; core draws it in the nav bar. */
export interface FrameAction {
  id: string;
  label: string;
  primary: boolean;
  /** A button in the bar rather than an item of its ☰ menu. */
  prominent: boolean;
  disabled: boolean;
  /** The ☰ submenu it is in. */
  submenu?: string;
  /** An icon oxplow draws (`ICONS`); an unknown name draws none. */
  icon?: string;
}

/** What a component shown as a page is told about it. */
export interface PageInit {
  streamId: string | null;
  threadId: string | null;
}

/** Its host's view: what a page, a replacement or a viz page's
 *  component shares of its tab. */
export interface ViewInit {
  /** Its tab's address, as far as it is the component's: what it shows. */
  viewParams: Record<string, string>;
  /** This step of its tab's history: what it kept there. */
  historyState: Record<string, unknown>;
}

export type BridgeRequest =
  | { id: string; method: "invoke"; command: string; input: unknown }
  | { id: string; method: "navigate"; uri: string; newTab: boolean; replace: boolean }
  | { id: string; method: "querySql"; sql: string; params: SqlCell[] }
  | { id: string; method: "openForm"; command: string; input: unknown }
  | { id: string; method: "openForm"; form: OwnForm }
  | { id: string; method: "signIn"; instance: string; name: string }
  | { id: string; method: "cancelSignIn"; instance: string; name: string }
  | { id: string; method: "setTitle"; title: string }
  | { id: string; method: "setActions"; actions: FrameAction[] }
  | { id: string; method: "setViewParams"; params: Record<string, string | null> }
  | { id: string; method: "setHistoryState"; patch: Record<string, unknown> }
  | { id: string; method: "pageMemoryGet"; key: string }
  | { id: string; method: "pageMemorySet"; key: string; value: unknown }
  | { id: string; method: "showView"; spec: string | null };

export type FrameMessage = { type: "ready" } | { type: "size"; height: number } | { type: "request"; request: BridgeRequest };

const isCell = (v: unknown): v is SqlCell =>
  v === null || typeof v === "string" || typeof v === "number" || typeof v === "boolean";

/** A message from the frame, or `null` when it isn't one of ours. */
export function parseFrameMessage(data: unknown): FrameMessage | null {
  if (!data || typeof data !== "object") return null;
  const m = data as Record<string, unknown>;
  if (m.type === "ready") return { type: "ready" };
  if (m.type === "size") {
    return typeof m.height === "number" && Number.isFinite(m.height) && m.height >= 0 ? { type: "size", height: m.height } : null;
  }
  if (typeof m.id !== "string" || !m.id) return null;
  switch (m.method) {
    case "invoke":
      if (typeof m.command !== "string") return null;
      return { type: "request", request: { id: m.id, method: "invoke", command: m.command, input: m.input ?? {} } };
    case "navigate":
      if (typeof m.uri !== "string") return null;
      if (m.newTab !== undefined && typeof m.newTab !== "boolean") return null;
      if (m.replace !== undefined && typeof m.replace !== "boolean") return null;
      return {
        type: "request",
        request: { id: m.id, method: "navigate", uri: m.uri, newTab: m.newTab === true, replace: m.replace === true },
      };
    case "querySql": {
      const params = m.params ?? [];
      if (typeof m.sql !== "string" || !Array.isArray(params) || !params.every(isCell)) return null;
      return { type: "request", request: { id: m.id, method: "querySql", sql: m.sql, params: params as SqlCell[] } };
    }
    case "openForm": {
      if (m.form !== undefined) {
        const f = m.form as Record<string, unknown> | null;
        if (!f || typeof f !== "object" || Array.isArray(f)) return null;
        const values = f.values ?? {};
        if (typeof f.title !== "string" || !f.schema || typeof f.schema !== "object" || Array.isArray(f.schema)) return null;
        if (!values || typeof values !== "object" || Array.isArray(values)) return null;
        return {
          type: "request",
          request: {
            id: m.id,
            method: "openForm",
            form: { title: f.title, schema: f.schema as Record<string, unknown>, values: values as Record<string, unknown> },
          },
        };
      }
      if (typeof m.command !== "string") return null;
      return { type: "request", request: { id: m.id, method: "openForm", command: m.command, input: m.input ?? {} } };
    }
    case "signIn":
    case "cancelSignIn":
      if (typeof m.instance !== "string" || !m.instance || typeof m.name !== "string" || !m.name) return null;
      return { type: "request", request: { id: m.id, method: m.method, instance: m.instance, name: m.name } };
    case "setTitle":
      if (typeof m.title !== "string") return null;
      return { type: "request", request: { id: m.id, method: "setTitle", title: m.title } };
    case "setActions": {
      if (!Array.isArray(m.actions)) return null;
      const actions: FrameAction[] = [];
      for (const raw of m.actions as unknown[]) {
        const a = raw as Record<string, unknown> | null;
        if (!a || typeof a.id !== "string" || !a.id || typeof a.label !== "string") return null;
        actions.push({
          id: a.id,
          label: a.label,
          primary: a.primary === true,
          prominent: a.prominent === true,
          disabled: a.disabled === true,
          ...(typeof a.submenu === "string" && a.submenu.trim() ? { submenu: a.submenu.trim() } : {}),
          ...(typeof a.icon === "string" && a.icon in ICONS ? { icon: a.icon } : {}),
        });
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
    case "showView": {
      // The host checks the page itself; here only that it is text.
      const spec = m.spec ?? null;
      if (spec !== null && typeof spec !== "string") return null;
      return { type: "request", request: { id: m.id, method: "showView", spec: spec as string | null } };
    }
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
 *  speaks, what it is fed (`HostedData`), the page it is (`page`), its
 *  host's view (`view`), and the theme's tokens. One builder, so the
 *  library's tests read the message the host really sends. */
export function initMessage(fed: HostedData, tokens: Record<string, string>, page: PageInit | null = null, view: ViewInit | null = null) {
  return {
    type: "init" as const,
    protocol: BRIDGE_PROTOCOL,
    run: fed.run,
    data: fed.data,
    params: fed.params,
    props: fed.props ?? null,
    page,
    view,
    tokens,
  };
}

export interface BridgeDeps {
  /** Run a declared command as the viz for the person. */
  invoke(command: string, input: unknown, confirmed: boolean): Promise<CommandOutcome>;
  /** Open `ref`'s page, in a new tab when asked; false when it isn't one
   *  a component may open (`componentMayOpen`). */
  navigate(ref: string, opts: { newTab: boolean; replace: boolean }): boolean;
  /** Ask the person, in the host, to confirm `command`. */
  confirm(command: string): Promise<boolean>;
  onReady(): void;
  /** Run its own read-only SQL (`run_component_sql`). */
  querySql?(sql: string, params: SqlCell[]): Promise<SqlQueryResult>;
  /** Show a declared command's form in the host and run it as submitted:
   *  `{ result }`, or null when the person closed it. A host that draws a slideover only. */
  openForm?(command: string, input: unknown): Promise<{ result: unknown } | null>;
  /** Show a form of the component's own in the host: the values as
   *  submitted, or null when the person closed it. A host that draws a slideover only. */
  askForm?(form: OwnForm): Promise<Record<string, unknown> | null>;
  /** Sign in for a provider instance's credential, the host running the
   *  whole redirect flow (the person's browser, the shell's listener, the
   *  core); resolves how it ended. A host that draws a slideover only. */
  signIn?(instance: string, name: string): Promise<SignInResult>;
  /** End that sign-in under way (it resolves `cancelled`). A host that draws a slideover only. */
  cancelSignIn?(instance: string, name: string): Promise<void>;
  /** The bridge closed: the host ends what it ran for the frame. */
  onClose?(): void;
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
  /** Run inline page `spec` (its YAML) and draw it under the frame (null
   *  clears it): its first loader's rows, for the frame's own controls (a
   *  chart's column pickers); rejects `{ code: "INVALID", field? }` for a
   *  page it refuses or a loader that failed. A page only. */
  showView?(spec: string | null): Promise<ShownRows | null>;
  /** The frame's content height, as it changes. */
  size?(height: number): void;
}

/** Why an invoke was refused (`DENIED`: the component isn't approved, or a
 *  policy said no), for the host to show beside the frame — the frame's
 *  own code may not (tsk960). Any other failure is the frame's to show. */
export function componentRefusal(e: unknown): string | null {
  return e instanceof IpcCallError && e.code === "DENIED" ? e.message : null;
}

/** Whether a component's `navigate` may open `ref`: one of oxplow's URIs,
 *  never a web page (the frame has no network, and a web page's tab would
 *  carry whatever it put in the URL out). It opens on the page that
 *  renders it (`openRef`). */
export function componentMayOpen(ref: string): boolean {
  return parseUri(ref) !== null;
}

/** What a component shown by a viz may not do: the page is its host's. */
const PAGE_ONLY = "Only a component shown as a page sets its title, actions or the view it shows.";
/** What a component in a host that draws no slideover may not do. */
const NO_FORMS = "This host shows no forms and signs in for no component.";
/** What a component in a host with no view may not do. */
const NO_VIEW = "This host gives the component no view: only a page, a replacement or a viz's own page does.";

/** Answer the frame's requests arriving on `port`. `sent` is what the
 *  `init` message fed it and `sentView` the view it told; `update`,
 *  `viewParams` and `historyState` post only what the frame hasn't seen,
 *  and `ready` is heard once. */
export function createBridgeHost(
  port: MessagePort,
  deps: BridgeDeps,
  sent: HostedData,
  sentView: ViewInit | null = null,
): {
  update(fed: HostedData): void;
  viewParams(params: Record<string, string>): void;
  historyState(state: Record<string, unknown>): void;
  action(id: string): void;
  close(): void;
} {
  let lastData = Object.fromEntries(Object.entries(sent.data).map(([k, v]) => [k, JSON.stringify(v)]));
  let lastParams = JSON.stringify(sent.params);
  let lastRun = JSON.stringify(sent.run ? [sent.run.params, sent.run.result] : null);
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
        if (deps.navigate(r.uri, { newTab: r.newTab, replace: r.replace })) reply(r.id, null);
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
        if ("form" in r) {
          if (!deps.askForm) return fail(r.id, "INVALID", NO_FORMS);
          try {
            const values = await deps.askForm(r.form);
            if (values === null) return fail(r.id, "CANCELLED", "The person closed the form.");
            reply(r.id, values);
          } catch (e) {
            failWith(r.id, e);
          }
          return;
        }
        if (!deps.openForm) return fail(r.id, "INVALID", NO_FORMS);
        try {
          const out = await deps.openForm(r.command, r.input);
          if (out === null) return fail(r.id, "CANCELLED", `The person closed the form for \`${r.command}\`.`);
          reply(r.id, out.result);
        } catch (e) {
          failWith(r.id, e);
        }
        return;
      }
      case "signIn":
        if (!deps.signIn) return fail(r.id, "INVALID", NO_FORMS);
        try {
          reply(r.id, await deps.signIn(r.instance, r.name));
        } catch (e) {
          failWith(r.id, e);
        }
        return;
      case "cancelSignIn":
        if (!deps.cancelSignIn) return fail(r.id, "INVALID", NO_FORMS);
        try {
          await deps.cancelSignIn(r.instance, r.name);
          reply(r.id, null);
        } catch (e) {
          failWith(r.id, e);
        }
        return;
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
      case "showView":
        if (!deps.showView) return fail(r.id, "INVALID", PAGE_ONLY);
        try {
          reply(r.id, (await deps.showView(r.spec)) ?? null);
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
    if (m?.type === "size") return void deps.size?.(m.height);
    if (m?.type === "request") return void handle(m.request);
    const id = (e.data as { id?: unknown } | null)?.id;
    if (typeof id === "string" && id) fail(id, "BAD_REQUEST", "Not a request this host knows.");
  };
  return {
    update: (fed) => {
      const data = Object.fromEntries(Object.entries(fed.data).map(([k, v]) => [k, JSON.stringify(v)]));
      const changed = [...new Set([...Object.keys(data), ...Object.keys(lastData)])].filter((k) => data[k] !== lastData[k]);
      const params = JSON.stringify(fed.params);
      const run = JSON.stringify(fed.run ? [fed.run.params, fed.run.result] : null);
      if (changed.length === 0 && params === lastParams && run === lastRun) return;
      lastData = data;
      lastParams = params;
      lastRun = run;
      port.postMessage({ type: "update", run: fed.run, data: fed.data, params: fed.params, changed });
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
    close: () => {
      port.close();
      deps.onClose?.();
    },
  };
}
