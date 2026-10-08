/// The window as a command host (`.context/commands.md` "Where a command
/// runs"): it hosts the capabilities only it can do — its threads' tabs,
/// … — and says so (`register_client_host`). A command the daemon runs
/// over one (an agent's `oxplow.tab.open`) comes as a `clientCall` event;
/// the window does it in the caller's thread and answers. The window's
/// own runs of such a command never leave it ([`runLocally`]).
import { answerClientCall, onRemoteReconnect, registerClientHost, subscribeOxplowEvents } from "./api.js";
import type { CommandSpec, OxplowEvent } from "./tauri-bridge/generated/bindings.js";

/** Where a call runs: the caller's thread (an agent's own), or `null` for
 *  the thread the window shows (a person's). */
export interface ClientCallContext {
  threadId: string | null;
  /** Who ran it: `agent:thr3`, `human`. */
  actor: string;
}

export type ClientHandler = (input: unknown, ctx: ClientCallContext) => unknown | Promise<unknown>;

/** The window's handlers: capability → op → handler. */
export type ClientHandlers = Record<string, Record<string, ClientHandler>>;

export interface ClientHostDeps {
  register(capabilities: string[]): Promise<void>;
  answer(id: string, answer: { result: unknown } | { error: string }): Promise<void>;
  subscribe(fn: (event: OxplowEvent) => void): () => void;
  onReconnect(fn: () => void): () => void;
}

const DEPS: ClientHostDeps = {
  register: registerClientHost,
  answer: answerClientCall,
  subscribe: subscribeOxplowEvents,
  onReconnect: onRemoteReconnect,
};

type ClientCall = Extract<OxplowEvent, { kind: "clientCall" }>;

async function perform(handlers: ClientHandlers, call: ClientCall): Promise<{ result: unknown } | { error: string }> {
  const handler = handlers[call.capability]?.[call.op];
  if (!handler) return { error: `the window doesn't do \`${call.capability}\` \`${call.op}\`` };
  try {
    return { result: (await handler(call.input, { threadId: call.threadId, actor: call.actor })) ?? null };
  } catch (e) {
    return { error: e instanceof Error ? e.message : String(e) };
  }
}

/** Host `handlers()` (read at each call, so they see the window's state
 *  now) until the returned stop. */
export function startClientHost(handlers: () => ClientHandlers, deps: ClientHostDeps = DEPS): () => void {
  const register = () => void deps.register(Object.keys(handlers())).catch(() => {});
  register();
  const offEvents = deps.subscribe((event) => {
    if (event.kind !== "clientCall") return;
    void perform(handlers(), event).then((answer) => deps.answer(event.id, answer).catch(() => {}));
  });
  const offReconnect = deps.onReconnect(register);
  return () => {
    offEvents();
    offReconnect();
  };
}

/** A person's run of `spec` in the window, when it's backed by a
 *  capability the window hosts: done here, never sent to the daemon (a
 *  view: nothing to record). `null` when the daemon runs it. */
export function runLocally(
  handlers: ClientHandlers,
  spec: CommandSpec,
  input: unknown,
): Promise<{ result: unknown }> | null {
  const handler = spec.op ? handlers[spec.op.capability]?.[spec.op.op] : undefined;
  if (!handler) return null;
  return Promise.resolve(handler(input, { threadId: null, actor: "human" })).then((result) => ({ result }));
}
