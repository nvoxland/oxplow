/// The window as a command host (`.context/commands.md` "Where a command
/// runs"): it hosts the scopes only it can do — its threads' tabs,
/// … — and says so under an id of its own (`register_client_host`). A
/// command the daemon runs over one (an agent's `oxplow.tab.open`) comes
/// as a `clientCall` event addressed to one window; that window does it in
/// the caller's thread and answers. It says when it closes
/// (`unregister_client_host`), so calls stop coming. The window's own runs
/// of such a command never leave it ([`runLocally`]).
import {
  answerClientCall,
  onRemoteReconnect,
  registerClientHost,
  subscribeOxplowEvents,
  unregisterClientHost,
} from "./api.js";
import type { CommandSpec, OxplowEvent } from "./tauri-bridge/generated/bindings.js";

/** The daemon's call a handler answers: what it runs on the daemon for it
 *  runs as the call's actor (`runCommandForCall`). */
export interface ClientCallRef {
  client: string;
  id: string;
}

/** Where a call runs: the caller's thread (an agent's own), or `null` for
 *  the thread the window shows (a person's). */
export interface ClientCallContext {
  threadId: string | null;
  /** Who ran it: `agent:thr3`, `human`. */
  actor: string;
  /** The daemon's call, or `null` for the window's own run. */
  call: ClientCallRef | null;
}

export type ClientHandler = (input: unknown, ctx: ClientCallContext) => unknown | Promise<unknown>;

/** The window's handlers: scope → op → handler. */
export type ClientHandlers = Record<string, Record<string, ClientHandler>>;

export interface ClientHostDeps {
  register(client: string, scopes: string[]): Promise<void>;
  unregister(client: string): Promise<void>;
  answer(client: string, id: string, answer: { result: unknown } | { error: string }): Promise<void>;
  subscribe(fn: (event: OxplowEvent) => void): () => void;
  onReconnect(fn: () => void): () => void;
  /** The window is going away (closed, reloaded). */
  onClose(fn: () => void): () => void;
}

const DEPS: ClientHostDeps = {
  register: registerClientHost,
  unregister: unregisterClientHost,
  answer: answerClientCall,
  subscribe: subscribeOxplowEvents,
  onReconnect: onRemoteReconnect,
  onClose: (fn) => {
    window.addEventListener("pagehide", fn);
    return () => window.removeEventListener("pagehide", fn);
  },
};

type ClientCall = Extract<OxplowEvent, { kind: "clientCall" }>;

async function perform(handlers: ClientHandlers, call: ClientCall): Promise<{ result: unknown } | { error: string }> {
  const handler = handlers[call.scope]?.[call.op];
  if (!handler) return { error: `the window doesn't do \`${call.scope}\` \`${call.op}\`` };
  try {
    const ctx = { threadId: call.threadId, actor: call.actor, call: { client: call.client, id: call.id } };
    return { result: (await handler(call.input, ctx)) ?? null };
  } catch (e) {
    return { error: e instanceof Error ? e.message : String(e) };
  }
}

/** Host `handlers()` (read at each call, so they see the window's state
 *  now) as window `client` until the returned stop. */
export function startClientHost(
  handlers: () => ClientHandlers,
  deps: ClientHostDeps = DEPS,
  client: string = crypto.randomUUID(),
): () => void {
  const register = () => void deps.register(client, Object.keys(handlers())).catch(() => {});
  const unregister = () => void deps.unregister(client).catch(() => {});
  register();
  const offEvents = deps.subscribe((event) => {
    if (event.kind !== "clientCall" || event.client !== client) return;
    void perform(handlers(), event).then((answer) => deps.answer(client, event.id, answer).catch(() => {}));
  });
  const offReconnect = deps.onReconnect(register);
  const offClose = deps.onClose(unregister);
  return () => {
    offEvents();
    offReconnect();
    offClose();
    unregister();
  };
}

/** A person's run of `spec` in the window, when it's backed by a
 *  scope the window hosts: done here, never sent to the daemon (a
 *  view: nothing to record). `null` when the daemon runs it. */
export function runLocally(
  handlers: ClientHandlers,
  spec: CommandSpec,
  input: unknown,
): Promise<{ result: unknown }> | null {
  const handler = spec.op ? handlers[spec.op.scope]?.[spec.op.op] : undefined;
  if (!handler) return null;
  return Promise.resolve(handler(input, { threadId: null, actor: "human", call: null })).then((result) => ({ result }));
}
