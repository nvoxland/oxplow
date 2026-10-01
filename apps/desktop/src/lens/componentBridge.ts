/// The host side of a custom component's bridge (P6b.D4,
/// `.context/extensions.md` → "Custom components"). The frame is
/// sandboxed (`allow-scripts` only: an opaque origin, no network, no
/// token); after it loads, the host hands it one end of a MessageChannel
/// with `init`, and every request comes back over that port — never over
/// `window` messages anyone could send:
///
///   frame → host  { type: "ready" }
///                 { id, method: "query", asset, params }     a declared lens's run
///                 { id, method: "invoke", command, input }   a declared command
///                 { id, method: "navigate", ref }            open a page
///   host → frame  { type: "init", run, props, tokens, kitCss }
///                 { type: "update", run }                    the lens re-ran
///                 { id, ok: true, result } | { id, ok: false, error: { code, message } }
///
/// A command that asks is confirmed by the person in the host, never in
/// the frame.
import { needsConfirmation } from "../ipc-error.js";
import { refFromTabId } from "../tabs/pageRefs.js";
import type { TabRef } from "../tabs/tabState.js";
import type { CommandOutcome, LensRun, SqlCell } from "../tauri-bridge/generated/bindings.js";

export type BridgeRequest =
  | { id: string; method: "query"; asset: string; params: Record<string, SqlCell> }
  | { id: string; method: "invoke"; command: string; input: unknown }
  | { id: string; method: "navigate"; ref: string };

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
      return { type: "request", request: { id: m.id, method: "navigate", ref: m.ref } };
    default:
      return null;
  }
}

/** Where the daemon serves a component's bundle (its folder URL). The
 *  stream is a path segment (`primary` outside any stream), so the
 *  bundle's relative URLs resolve in the same worktree. */
export function componentBundleUrl(base: string, extension: string, component: string, streamId: string | null): string {
  const segments = [streamId ?? "primary", extension, component].map(encodeURIComponent);
  return `${base}/components/${segments.join("/")}/`;
}

/** The theme's tokens: the root's custom properties (`--text-primary`). */
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
  return out;
}

/** CSS a component can adopt to look like the host: the tokens and a body
 *  baseline. (There is no kit stylesheet to share yet.) */
export function kitCss(tokens: Record<string, string>): string {
  const vars = Object.entries(tokens)
    .map(([k, v]) => `${k}: ${v};`)
    .join(" ");
  return `:root { ${vars} } body { margin: 0; font-family: var(--font-ui, system-ui, sans-serif); color: var(--text-primary); background: transparent; }`;
}

export interface BridgeDeps {
  /** Run a declared lens (`run_component_query`). */
  query(asset: string, params: Record<string, SqlCell>): Promise<LensRun>;
  /** Run a declared command as the lens for the person. */
  invoke(command: string, input: unknown, confirmed: boolean): Promise<CommandOutcome>;
  /** Open `ref`'s page; false when it isn't one a component may open
   *  (`componentNavigationTarget`). */
  navigate(ref: string): boolean;
  /** Ask the person, in the host, to confirm `command`. */
  confirm(command: string): Promise<boolean>;
  onReady(): void;
}

/** The page a component's `navigate` opens: an oxplow page for `ref`,
 *  never an outside URL (the frame has no network, and an external-url
 *  tab would carry whatever it put in the URL out). */
export function componentNavigationTarget(ref: string): TabRef | null {
  const tab = refFromTabId(ref);
  return tab && tab.kind !== "external-url" ? tab : null;
}

/** What a run shows the frame: an `update` repeats none of it. */
const shown = (run: LensRun) => JSON.stringify([run.params, run.result]);

/** Answer the frame's requests arriving on `port`. `sent` is the run the
 *  `init` message carried; `update` posts only a run whose params or
 *  result the frame hasn't seen, and `ready` is heard once. */
export function createBridgeHost(
  port: MessagePort,
  deps: BridgeDeps,
  sent: LensRun,
): { update(run: LensRun): void; close(): void } {
  let last = shown(sent);
  let ready = false;
  const reply = (id: string, result: unknown) => port.postMessage({ id, ok: true, result });
  const fail = (id: string, code: string, message: string) => port.postMessage({ id, ok: false, error: { code, message } });
  const failWith = (id: string, e: unknown) => {
    const code = (e as { code?: unknown })?.code;
    fail(id, typeof code === "string" ? code : "FAILED", e instanceof Error ? e.message : String(e));
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
        if (deps.navigate(r.ref)) reply(r.id, null);
        else fail(r.id, "INVALID", "A component may open oxplow's pages only.");
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
    if (typeof id === "string" && id) fail(id, "BAD_REQUEST", "Not a query, invoke or navigate request.");
  };
  return {
    update: (run) => {
      const key = shown(run);
      if (key === last) return;
      last = key;
      port.postMessage({ type: "update", run });
    },
    close: () => port.close(),
  };
}
