/// The host side of a component under test (`oxplow extension test`'s
/// component harness, `.context/extensions.md` "Testing a component"): the
/// page the harness serves loads this, which mounts the bundle in a
/// sandboxed frame and speaks the real bridge to it (`createBridgeHost`,
/// `initMessage`, `pageData` — the app's own, so the two can't drift).
/// What the core answers (`invoke`, `querySql`, forms, `showView`) goes to
/// the harness's server, which runs it on the test's oxplow; what only the
/// app's shell answers (a title, actions, the view, navigation) is
/// recorded there for the fixture to check. The server then drives the
/// test, one step at a time: a DOM step goes to the agent script the
/// server put in the bundle's page, a shell step (a nav-bar press, a view
/// change, new rows) to the bridge.
///
/// Built on its own (`bun run build:harness`) into
/// `crates/oxplow-sdk/assets/harness-host.js`, which the SDK serves; React
/// stays out of it.
import { IpcCallError } from "../ipc-error.js";
import type { PageRun, SqlCell } from "../tauri-bridge/generated/bindings.js";
import { createBridgeHost, initMessage, pageData, type HostedData, type PageInit, type ViewInit } from "./componentBridge.js";
import { nodeById, nodeRunOf } from "./nodeRun.js";

/** What the server mounts: the bundle's version, the page run, the node
 *  that shows the component, and what the frame is told of its page and
 *  view. */
export interface Start {
  version: string;
  run: PageRun;
  node: string;
  page: PageInit | null;
  view: ViewInit | null;
}

/** What the server asks the host to do next. */
export type Op =
  | { op: "dom"; action: unknown }
  | { op: "press"; id: string }
  | { op: "viewParams"; params: Record<string, string> }
  | { op: "update"; run: PageRun }
  | { op: "done" };

/** How the host reaches the server. */
export interface Server {
  start(): Promise<Start>;
  /** A call the core answers: its result, or it throws the
   *  `IpcCallError` the app's call would. */
  rpc(method: string, body: Record<string, unknown>): Promise<unknown>;
  /** A call only the shell answers, kept for the fixture. */
  record(kind: string, value: unknown): void;
  /** The last op's result; the next op. */
  next(result: unknown): Promise<Op>;
}

/** What node `node` of `run` feeds its component: a `component` node its
 *  props, a `custom` viz its props and its own rows. */
export function fedBy(run: PageRun, node: string): HostedData {
  const def = nodeById(run.page, node);
  if (def?.kind === "viz") return pageData(run, def.config.custom?.props ?? null, nodeRunOf(run, def));
  return pageData(run, def?.kind === "component" ? def.props : null);
}

/** Whether this window draws: a frame within `wait` ms. WebKit draws none
 *  while the display is locked or asleep, and a component that lays
 *  itself out in a frame then never draws, so its test can't run. */
export function drawsFrames(wait = 2000): Promise<boolean> {
  return new Promise((drew) => {
    const timer = setTimeout(() => drew(false), wait);
    requestAnimationFrame(() => {
      clearTimeout(timer);
      drew(true);
    });
  });
}

/** Mount `start` in `frame` and run the server's ops until it says done:
 *  `dom` ops go to the agent in the frame (`toAgent`, whose answer comes
 *  back through `fromAgent`), the rest to the bridge. First the server is
 *  told whether the window draws (`frames`); one that doesn't mounts
 *  nothing and waits to be told it's done. */
export async function runHarness(
  server: Server,
  frame: HTMLIFrameElement,
  agent: { send(id: number, action: unknown): void; answers(cb: (id: number, result: unknown) => void): void },
  frames: () => Promise<boolean> = drawsFrames,
): Promise<void> {
  const drawing = await frames();
  server.record("drawing", drawing);
  if (!drawing) {
    while ((await server.next(null)).op !== "done");
    return;
  }
  const start = await server.start();
  const fed = fedBy(start.run, start.node);
  let showing: Promise<void> = Promise.resolve();
  const host = await new Promise<ReturnType<typeof createBridgeHost>>((mounted) => {
    frame.addEventListener(
      "load",
      () => {
        const channel = new MessageChannel();
        const record = (kind: string) => (value: unknown) => server.record(kind, value);
        const bridge = createBridgeHost(
          channel.port1,
          {
            invoke: async (command, input, confirmed) =>
              ({ result: await server.rpc("invoke", { command, input, confirmed }) }) as never,
            navigate: (ref) => {
              server.record("navigate", ref);
              return true;
            },
            confirm: async (command) => (await server.rpc("confirm", { command })) === true,
            onReady: () => server.record("ready", true),
            querySql: async (sql: string, params: SqlCell[]) => (await server.rpc("querySql", { sql, params })) as never,
            openForm: async (command, input) => (await server.rpc("openForm", { command, input })) as never,
            askForm: async (form) => (await server.rpc("askForm", { form })) as never,
            // One at a time, in the order asked, and recorded once the core has
            // answered: the spec when it drew it, `null` when it refused it
            // or the component cleared it.
            showView: (spec) => {
              const mine = showing.then(async () => {
                if (spec === null) {
                  server.record("showView", null);
                  return null;
                }
                try {
                  const drawn = (await server.rpc("showView", { spec })) as never;
                  server.record("showView", spec);
                  return drawn;
                } catch (e) {
                  server.record("showView", null);
                  throw e;
                }
              });
              showing = mine.then(
                () => undefined,
                () => undefined,
              );
              return mine;
            },
            setTitle: record("title"),
            setActions: record("actions"),
            setViewParams: record("viewParams"),
            setHistoryState: record("historyState"),
          },
          fed,
          start.view,
        );
        // An opaque-origin frame can only be addressed with "*"; the port
        // goes to this frame's window alone.
        frame.contentWindow?.postMessage(initMessage(fed, {}, start.page, start.view), "*", [channel.port2]);
        mounted(bridge);
      },
      { once: true },
    );
    frame.src = `/components/v/${start.version}/`;
  });
  const pending = new Map<number, (result: unknown) => void>();
  agent.answers((id, result) => {
    pending.get(id)?.(result);
    pending.delete(id);
  });
  let ids = 0;
  let result: unknown = null;
  for (;;) {
    const op = await server.next(result);
    switch (op.op) {
      case "dom": {
        const id = ++ids;
        result = await new Promise((answered) => {
          pending.set(id, answered);
          agent.send(id, op.action);
        });
        break;
      }
      case "press":
        host.action(op.id);
        result = null;
        break;
      case "viewParams":
        host.viewParams(op.params);
        result = null;
        break;
      case "update":
        host.update(fedBy(op.run, start.node));
        result = null;
        break;
      case "done":
        host.close();
        return;
    }
  }
}

/** The server this page was served by, over HTTP. */
export function httpServer(): Server {
  let sent: Promise<void> = Promise.resolve();
  const post = async (path: string, body: unknown) => {
    const response = await fetch(path, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(body),
    });
    return response.json() as Promise<unknown>;
  };
  return {
    start: async () => (await fetch("/harness/start")).json() as Promise<Start>,
    rpc: async (method, body) => {
      const out = (await post("/harness/rpc", { method, ...body })) as
        | { ok: true; result: unknown }
        | { ok: false; error: { code: string; message: string; field?: string } };
      // As the app's IPC fails: a `NEEDS_CONFIRMATION` asks first.
      if ("error" in out) throw new IpcCallError(out.error.message, out.error.code, out.error.field ?? null);
      return out.result;
    },
    // The records go in the order they were made: each post waits for the
    // one before, and the next op waits for them all, so the fixture reads
    // what the frame did in the order it did it.
    record: (kind, value) => {
      sent = sent.then(() =>
        fetch("/harness/record", {
          method: "POST",
          headers: { "content-type": "application/json" },
          body: JSON.stringify({ kind, value }),
        }).then(
          () => undefined,
          () => undefined,
        ),
      );
    },
    next: async (result) => {
      await sent;
      return (await post("/harness/next", { result })) as Op;
    },
  };
}

/** Run the harness on this page: its frame, the agent in it. */
export function main(): void {
  const frame = document.getElementById("frame") as HTMLIFrameElement;
  void runHarness(httpServer(), frame, {
    send: (id, action) => frame.contentWindow?.postMessage({ oxplowHarness: id, action }, "*"),
    answers: (cb) =>
      window.addEventListener("message", (e: MessageEvent) => {
        if (e.source !== frame.contentWindow) return;
        const data = e.data as { oxplowHarness?: unknown; result?: unknown };
        if (typeof data?.oxplowHarness === "number") cb(data.oxplowHarness, data.result);
      }),
  }).then(() => {
    location.href = "/harness/done";
  });
}
