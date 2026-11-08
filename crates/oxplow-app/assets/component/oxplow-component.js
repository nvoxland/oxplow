// oxplow's client library for custom components: a `viz: custom` viz node's,
// or a page's own (`pages: [{ component: <id> }]`).
//
// A component runs in a sandboxed frame: scripts and forms, no network,
// no storage. A form submits nowhere (the frame's CSP): handle `submit` in
// your script and run a command with `invoke`; a refused one rejects with
// `{ code, message, field? }`, `field` the input it names (`/title`). oxplow
// itself records every failure but a field error and shows it under your
// frame, so a component draws only a field error, and nothing for
// `CANCELLED` (the person closed a form or declined). oxplow hands it one end of a MessageChannel in an `init`
// message; this wraps that protocol. Load it before your own script:
//
//   <script src="/component-lib/oxplow-component.js"></script>
//   <script src="app.js"></script>
//
// and, in app.js:
//
//   oxplow.connect().then((component) => {
//     render(component.data);                          // its loaders' rows, by name
//     component.onUpdate(({ data }) => render(data));  // they changed
//   });
//
// A component is fed its page's data: `data` holds each loader's
// `{ columns, rows, truncated }` by name (a viz node's own rows as `rows`),
// `params` what they ran with and `props` its own config; `onUpdate`
// hears new data with the loaders that `changed`. A `viz: custom`'s own
// rows are also `run` (a viz run); a page's component node has none.
// A component that is a page (`page` carries its stream and thread)
// reads more with `querySql` (when it
// declares `sql: read`), titles its page with `setTitle`, puts actions in
// the page's bar with `setActions` and hears them with `onAction`, and
// opens a command's form, or a form of its own schema, in oxplow with
// `openForm`, and signs in for a provider's credential with `signIn`. A page may also hand
// oxplow an inline page (its YAML) to draw under it with the kit
// (`showView`): oxplow runs it, keeps it live and offers Pin to
// Dashboard; the library tells oxplow the page's content height, so
// the frame sits at its content's height above what oxplow draws (leave
// `html` and `body` at `height: auto` for that).
//
// A component keeps no state of its own (a frame has no storage); oxplow
// keeps each kind, named by its scope:
//   - view params — the page's tab's address: what it shows. Shareable:
//     Back, a bookmark and an agent see them. `viewParams`,
//     `setViewParams`, `onViewParams`. A page only.
//   - history state — this step of the tab's history: scroll, what's
//     expanded, a draft. `historyState`, `setHistoryState`,
//     `onHistoryState`. A page only.
//   - page memory — the component's own, for the person, in every tab of
//     it: a layout, a last choice. `pageMemory.get` / `pageMemory.set`
//     (JSON; 64 KB a value, 1 MB a component).
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
  const PROTOCOL = 12;

  function component(port, init) {
    let next = 0;
    let run = init.run === undefined ? null : init.run;
    let fed = init.data || {};
    let params = init.params || {};
    const pending = new Map();
    const listeners = new Set();
    const actionListeners = new Set();
    const viewListeners = new Set();
    const stateListeners = new Set();
    const initView = init.view === undefined || init.view === null ? null : init.view;
    let viewParams = initView ? initView.viewParams || {} : null;
    let historyState = initView ? initView.historyState || {} : null;
    const page = init.page ? { streamId: init.page.streamId, threadId: init.page.threadId } : null;

    port.onmessage = (event) => {
      const data = event.data;
      if (!data || typeof data !== "object") return;
      if (data.type === "update") {
        run = data.run === undefined ? null : data.run;
        fed = data.data || {};
        params = data.params || {};
        const update = { run, data: fed, params, changed: data.changed || [] };
        for (const listener of Array.from(listeners)) listener(update);
        return;
      }
      if (data.type === "action") {
        for (const listener of Array.from(actionListeners)) listener(data.id);
        return;
      }
      if (data.type === "viewParams") {
        viewParams = data.params;
        for (const listener of Array.from(viewListeners)) listener(viewParams);
        return;
      }
      if (data.type === "historyState") {
        historyState = data.state;
        for (const listener of Array.from(stateListeners)) listener(historyState);
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
    // The frame says how tall its content is, as it changes: a page's host
    // sits it above what it draws, a section's fits it to its content.
    if (typeof global.ResizeObserver === "function" && global.document) {
      const root = global.document.documentElement;
      let last = -1;
      const observer = new global.ResizeObserver((entries) => {
        const box = entries[0] && entries[0].borderBoxSize && entries[0].borderBoxSize[0];
        const height = Math.ceil(box ? box.blockSize : root.getBoundingClientRect().height);
        if (height === last) return;
        last = height;
        port.postMessage({ type: "size", height });
      });
      observer.observe(root);
    }
    return {
      // Its loaders' rows, by name: `{ columns, rows, truncated, error? }`
      // each (a viz node's own rows as `rows`).
      get data() {
        return fed;
      },
      // The params its data ran with.
      get params() {
        return params;
      },
      // A `viz: custom`'s own rows as a viz run: `{ config, params, result:
      // { columns, rows } }`; null for a page's component node.
      get run() {
        return run;
      },
      // The page it is — `{ streamId, threadId }` — or null when a viz node
      // shows it.
      page,
      // Its host's view params: a page's, a replacement's (the keys its
      // target lists); null when its host gives none (a
      // tile, a slot).
      get viewParams() {
        return viewParams;
      },
      // Its history state, where its host gives a view; else null.
      get historyState() {
        return historyState;
      },
      // Its props — a component node's `props`, a viz's `custom.props` — or null.
      props: init.props === undefined ? null : init.props,
      // The theme's CSS variables, by name (`--text-primary`).
      tokens,
      protocol: init.protocol,
      // Hear new data — `{ run, data, params, changed }`, `changed` the
      // loaders whose rows differ — each time it changes; returns how to stop.
      onUpdate(listener) {
        listeners.add(listener);
        return () => {
          listeners.delete(listener);
        };
      },
      // Run a command this component declares in `commands`, as the person
      // looking at it; one that asks is confirmed by them, in oxplow.
      invoke: (command, input) => call({ method: "invoke", command, input: input === undefined ? {} : input }),
      // Open one of oxplow's pages (`oxplow:oxplow/wiki?slug=intro`), as a
      // browser follows a link; `{ newTab: true }` opens it in a new tab (a
      // Cmd-click), `{ replace: true }` in place of the page (Back skips it).
      navigate: (uri, options) =>
        call({
          method: "navigate",
          uri,
          newTab: !!(options && options.newTab),
          replace: !!(options && options.replace),
        }),
      // Run read-only SQL over oxplow's datasets (`?1` parameters): needs
      // `sql: read` on the component. Resolves `{ columns, rows, truncated }`.
      querySql: (sql, params) => call({ method: "querySql", sql, params: params || [] }),
      // Open a declared command's form in oxplow, starting from `input`;
      // resolves with its result once submitted, rejects `CANCELLED` when
      // the person closes it. Or a form of the component's own —
      // `openForm({ title, schema, values? })`, `schema` a JSON Schema for
      // its fields (a `writeOnly` string is masked) — which resolves with
      // the values submitted and runs nothing: the component then invokes
      // what it will with them. A page only.
      openForm: (command, input) =>
        command !== null && typeof command === "object"
          ? call({ method: "openForm", form: { title: command.title, schema: command.schema, values: command.values || {} } })
          : call({ method: "openForm", command, input: input === undefined ? {} : input }),
      // Sign in for a provider instance's credential (one its provider
      // declares with `oauth:`): oxplow opens the service's page in the
      // person's browser, listens for the redirect on their machine and
      // finishes the sign-in, then resolves how it ended — `{ state:
      // "signed_in" }`, `{ state: "failed", error }` or `{ state:
      // "cancelled" }`. The component declares `oxplow.provider.begin_sign_in`,
      // `complete_sign_in` and `cancel_sign_in` in `commands`. Rejects `INVALID`
      // without the desktop app. A page only.
      signIn: (instance, name) => call({ method: "signIn", instance, name }),
      // End that sign-in under way: its `signIn` resolves `cancelled`.
      cancelSignIn: (instance, name) => call({ method: "cancelSignIn", instance, name }),
      // The page's title. A page only.
      setTitle: (title) => call({ method: "setTitle", title }),
      // Have oxplow draw an inline page under the page — its YAML (JSON is
      // YAML too): `title`, `data` (read-only `sql` loaders), `view` — live,
      // with Pin to Dashboard; null clears it. Resolves with
      // its first loader's rows (`{ columns, rows, truncated }`; null when
      // cleared), for the page's own controls. Rejects `INVALID` with the
      // `field` it is about (`/loaders/<loader>/sql`) for a page oxplow refuses
      // or a loader that failed. A page only.
      showView: (spec) => call({ method: "showView", spec: spec === undefined ? null : spec }),
      // The page's actions — `[{ id, label, primary?, disabled? }]` — drawn
      // by oxplow in the page's bar. A page only.
      setActions: (actions) => call({ method: "setActions", actions }),
      // Change its host's view params (a value null drops one): its tab's
      // entry is replaced, so Back, a bookmark and a reopened tab keep them.
      // A host with a view only; a key not its to set rejects `INVALID`.
      setViewParams: (params) => call({ method: "setViewParams", params }),
      // Hear its view params each time they change (Back, Forward, an echo
      // of setViewParams); returns how to stop.
      onViewParams(listener) {
        viewListeners.add(listener);
        return () => {
          viewListeners.delete(listener);
        };
      },
      // Merge `patch` into its history state. A host with a view only.
      setHistoryState: (patch) => call({ method: "setHistoryState", patch }),
      // Hear its history state each time it changes (Back, Forward);
      // returns how to stop.
      onHistoryState(listener) {
        stateListeners.add(listener);
        return () => {
          stateListeners.delete(listener);
        };
      },
      // Its page memory: the component's own, for the person, in every tab.
      pageMemory: {
        // Resolves its value, or null.
        get: (key) => call({ method: "pageMemoryGet", key }),
        // Keep `value` (JSON); null forgets it.
        set: (key, value) => call({ method: "pageMemorySet", key, value: value === undefined ? null : value }),
      },
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
