// oxplow's client library for custom components (`viz: custom` lenses).
//
// A component runs in a sandboxed frame: scripts only, no network, no
// storage. oxplow hands it one end of a MessageChannel in an `init`
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
// It is a classic script (it defines the global `oxplow`), not a module:
// a sandboxed frame's origin is opaque, and module scripts are fetched
// with CORS, which nothing served to a frame allows.
(function (global) {
  "use strict";

  // The protocol this library speaks; `init` says which one the host does.
  const PROTOCOL = 1;

  function component(port, init) {
    let next = 0;
    let run = init.run;
    const pending = new Map();
    const listeners = new Set();

    port.onmessage = (event) => {
      const data = event.data;
      if (!data || typeof data !== "object") return;
      if (data.type === "update") {
        run = data.run;
        for (const listener of Array.from(listeners)) listener(run);
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

    const kitCss = typeof init.kitCss === "string" ? init.kitCss : "";
    port.postMessage({ type: "ready" });
    return {
      // The lens's latest run: `{ lens, params, result: { columns, rows } }`.
      get run() {
        return run;
      },
      // The lens's `custom.props`, or null.
      props: init.props === undefined ? null : init.props,
      // The theme's CSS variables, by name (`--text-primary`).
      tokens: init.tokens || {},
      // A small stylesheet built from the tokens.
      kitCss,
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
      // Open one of oxplow's pages (`work_item:oxplow:tsk42`).
      navigate: (ref) => call({ method: "navigate", ref }),
      // Adopt oxplow's look: add `kitCss` to the document.
      applyKitCss(doc) {
        const target = doc || global.document;
        const style = target.createElement("style");
        style.textContent = kitCss;
        target.head.appendChild(style);
        return style;
      },
    };
  }

  // Wait for oxplow's `init` and answer `ready`. Rejects when none comes
  // within `timeoutMs` (10 s), or when the host speaks another protocol.
  // A failed request rejects with the host's `{ code, message }`.
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
