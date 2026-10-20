// oxplow's client library for custom components: a `viz: custom` lens's,
// or a page's own (`pages: [{ component: <id> }]`).
//
// A component runs in a sandboxed frame: scripts and forms, no network,
// no storage. A form submits nowhere (the frame's CSP): handle `submit` in
// your script and run a command with `invoke`; a refused one rejects with
// `{ code, message, field? }`, `field` the input it names (`/title`). oxplow hands it one end of a MessageChannel in an `init`
// message; this wraps that protocol. Load it before your own script:
//
//   <script src="/component-lib/oxplow-component.js"></script>
//   <script src="app.js"></script>
//
// and, in app.js:
//
//   oxplow.connect().then((component) => {
//     render(component.run);                 // the lens's own rows
//     component.onUpdate(render);            // the lens re-ran
//   });
//
// A component that is a page has no run (`run` is null); `page` carries
// its params, stream and thread, and it reads with `querySql` (when it
// declares `sql: read`), titles its page with `setTitle`, puts actions in
// the page's bar with `setActions` and hears them with `onAction`, and
// opens a command's form in oxplow with `openForm`.
//
// For oxplow's look, link the kit's stylesheet too —
//   <link rel="stylesheet" href="/component-lib/oxplow-kit.css">
// — and call `component.applyTheme()` once connected.
//
// It is a classic script (it defines the global `oxplow`), not a module:
// a sandboxed frame's origin is opaque, and module scripts are fetched
// with CORS, which nothing served to a frame allows.
(function (global) {
  "use strict";

  // The protocol this library speaks; `init` says which one the host does.
  const PROTOCOL = 5;

  function component(port, init) {
    let next = 0;
    let run = init.run;
    const pending = new Map();
    const listeners = new Set();
    const actionListeners = new Set();

    port.onmessage = (event) => {
      const data = event.data;
      if (!data || typeof data !== "object") return;
      if (data.type === "update") {
        run = data.run;
        for (const listener of Array.from(listeners)) listener(run);
        return;
      }
      if (data.type === "action") {
        for (const listener of Array.from(actionListeners)) listener(data.id);
        return;
      }
      const waiting = pending.get(data.id);
      if (!waiting) return;
      pending.delete(data.id);
      if (data.ok) waiting.resolve(data.result);
      else waiting.reject(data.error);
    };

    const call = (message) =>
      new Promise((resolve, reject) => {
        next += 1;
        const id = String(next);
        pending.set(id, { resolve, reject });
        port.postMessage(Object.assign({ id }, message));
      });

    const tokens = init.tokens || {};
    port.postMessage({ type: "ready" });
    return {
      // The lens's latest run: `{ lens, params, result: { columns, rows } }`;
      // null for a component that is a page.
      get run() {
        return run;
      },
      // The page it is — `{ params, streamId, threadId }` — or null when a
      // lens shows it.
      page: init.page === undefined ? null : init.page,
      // The lens's `custom.props`, or null.
      props: init.props === undefined ? null : init.props,
      // The theme's CSS variables, by name (`--text-primary`).
      tokens,
      protocol: init.protocol,
      // Hear each re-run of the lens; returns how to stop.
      onUpdate(listener) {
        listeners.add(listener);
        return () => {
          listeners.delete(listener);
        };
      },
      // Run a lens this component declares in `assets`.
      query: (asset, params) => call({ method: "query", asset, params: params || {} }),
      // Run a command this component declares in `commands`, as the person
      // looking at it; one that asks is confirmed by them, in oxplow.
      invoke: (command, input) => call({ method: "invoke", command, input: input === undefined ? {} : input }),
      // Open one of oxplow's pages (`work_item:oxplow:tsk42`); `{ newTab:
      // true }` opens it in a new tab (a Cmd-click).
      navigate: (ref, options) =>
        call({ method: "navigate", ref, newTab: !!(options && options.newTab) }),
      // Run read-only SQL over oxplow's models (`?1` parameters): needs
      // `sql: read` on the component. Resolves `{ columns, rows, truncated }`.
      querySql: (sql, params) => call({ method: "querySql", sql, params: params || [] }),
      // Open a declared command's form in oxplow, starting from `input`;
      // resolves with its result once submitted, rejects `CANCELLED` when
      // the person closes it. A page only.
      openForm: (command, input) => call({ method: "openForm", command, input: input === undefined ? {} : input }),
      // The page's title. A page only.
      setTitle: (title) => call({ method: "setTitle", title }),
      // The page's actions — `[{ id, label, primary?, disabled? }]` — drawn
      // by oxplow in the page's bar. A page only.
      setActions: (actions) => call({ method: "setActions", actions }),
      // Hear each press of a page action, by its id; returns how to stop.
      onAction(listener) {
        actionListeners.add(listener);
        return () => {
          actionListeners.delete(listener);
        };
      },
      // Adopt oxplow's theme: set each token on the document's root, where
      // the kit's stylesheet (and your own CSS) reads them. `color-scheme`
      // is one of them: it keeps the frame's canvas transparent.
      applyTheme(doc) {
        const root = (doc || global.document).documentElement;
        for (const name of Object.keys(tokens)) root.style.setProperty(name, tokens[name]);
      },
    };
  }

  // Wait for oxplow's `init` and answer `ready`. Rejects when none comes
  // within `timeoutMs` (10 s), or when the host speaks another protocol.
  // A failed request rejects with the host's `{ code, message, field? }`.
  function connect(options) {
    const target = (options && options.target) || global;
    const timeoutMs = (options && options.timeoutMs) || 10000;
    return new Promise((resolve, reject) => {
      const onMessage = (event) => {
        const data = event.data;
        const port = event.ports && event.ports[0];
        if (!data || data.type !== "init" || !port) return;
        target.removeEventListener("message", onMessage);
        clearTimeout(timer);
        if (data.protocol !== PROTOCOL) {
          reject(
            new Error(
              "oxplow speaks component protocol " + data.protocol + "; this library speaks " + PROTOCOL,
            ),
          );
          return;
        }
        resolve(component(port, data));
      };
      const timer = setTimeout(() => {
        target.removeEventListener("message", onMessage);
        reject(new Error("no init from oxplow within " + timeoutMs + " ms"));
      }, timeoutMs);
      target.addEventListener("message", onMessage);
    });
  }

  global.oxplow = { PROTOCOL, connect };
})(typeof globalThis !== "undefined" ? globalThis : self);
