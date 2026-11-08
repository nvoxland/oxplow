(() => {
  // src/ipc-error.ts
  class IpcCallError extends Error {
    code;
    field;
    details;
    auditId;
    constructor(message, code, field = null, details = [], auditId = null) {
      super(message);
      this.code = code;
      this.field = field;
      this.details = details;
      this.auditId = auditId;
      this.name = "IpcCallError";
    }
  }
  function needsConfirmation(e) {
    return e instanceof IpcCallError && e.code === "NEEDS_CONFIRMATION";
  }
  function ipcErrorDetails(err) {
    if (err && typeof err === "object") {
      const details = err.details;
      if (Array.isArray(details))
        return details.filter((d) => typeof d === "string");
    }
    return [];
  }

  // src/iconNames.ts
  var ICON_NAMES = [
    "activity",
    "archive",
    "at-sign",
    "bar-chart-3",
    "bell",
    "book-open",
    "box",
    "bug",
    "calendar",
    "check-square",
    "circle-check",
    "circle-dot",
    "circle-stop",
    "columns-3",
    "copy",
    "database",
    "download",
    "external-link",
    "file-text",
    "flag",
    "folder",
    "folder-tree",
    "git-branch",
    "git-commit",
    "git-compare",
    "git-merge",
    "git-pull-request",
    "history",
    "inbox",
    "layout-dashboard",
    "link",
    "list-tree",
    "maximize-2",
    "message-square",
    "package",
    "pencil",
    "play",
    "plus",
    "refresh-cw",
    "server",
    "settings",
    "shield",
    "sparkles",
    "star",
    "tag",
    "terminal",
    "ticket",
    "trash-2",
    "triangle-alert",
    "user",
    "zap"
  ];
  var NAMES = new Set(ICON_NAMES);
  function isIconName(name) {
    return NAMES.has(name);
  }

  // src/filesystem.ts
  var MAX_SHOWN_BYTES = 5 * 1024 * 1024;

  // src/uri/uri.ts
  var CONTRIBUTIONS = new Set([
    "dataset",
    "measure",
    "metric",
    "dimension",
    "collector",
    "event_type",
    "ref_kind",
    "command",
    "job",
    "ai_profile",
    "effect",
    "advisory",
    "skill",
    "implementation",
    "provider",
    "page",
    "panel"
  ]);
  var GLOBAL_CONTRIBUTIONS = new Set(["command", "measure", "metric", "dimension", "event_type", "ref_kind"]);

  // src/viz/componentBridge.ts
  var BRIDGE_PROTOCOL = 12;
  var rowsOf = (r, error = null) => ({
    columns: r.columns,
    rows: r.rows,
    truncated: r.truncated,
    ...error ? { error } : {}
  });
  function pageData(run, props, own = null) {
    const data = {};
    for (const [name, l] of Object.entries(run.loaders))
      if (l)
        data[name] = rowsOf(l.result, l.error);
    return { run: own, data, params: run.params, props: props ?? null };
  }
  var isCell = (v) => v === null || typeof v === "string" || typeof v === "number" || typeof v === "boolean";
  function parseFrameMessage(data) {
    if (!data || typeof data !== "object")
      return null;
    const m = data;
    if (m.type === "ready")
      return { type: "ready" };
    if (m.type === "size") {
      return typeof m.height === "number" && Number.isFinite(m.height) && m.height >= 0 ? { type: "size", height: m.height } : null;
    }
    if (typeof m.id !== "string" || !m.id)
      return null;
    switch (m.method) {
      case "invoke":
        if (typeof m.command !== "string")
          return null;
        return { type: "request", request: { id: m.id, method: "invoke", command: m.command, input: m.input ?? {} } };
      case "navigate":
        if (typeof m.uri !== "string")
          return null;
        if (m.newTab !== undefined && typeof m.newTab !== "boolean")
          return null;
        if (m.replace !== undefined && typeof m.replace !== "boolean")
          return null;
        return {
          type: "request",
          request: { id: m.id, method: "navigate", uri: m.uri, newTab: m.newTab === true, replace: m.replace === true }
        };
      case "querySql": {
        const params = m.params ?? [];
        if (typeof m.sql !== "string" || !Array.isArray(params) || !params.every(isCell))
          return null;
        return { type: "request", request: { id: m.id, method: "querySql", sql: m.sql, params } };
      }
      case "openForm": {
        if (m.form !== undefined) {
          const f = m.form;
          if (!f || typeof f !== "object" || Array.isArray(f))
            return null;
          const values = f.values ?? {};
          if (typeof f.title !== "string" || !f.schema || typeof f.schema !== "object" || Array.isArray(f.schema))
            return null;
          if (!values || typeof values !== "object" || Array.isArray(values))
            return null;
          return {
            type: "request",
            request: {
              id: m.id,
              method: "openForm",
              form: { title: f.title, schema: f.schema, values }
            }
          };
        }
        if (typeof m.command !== "string")
          return null;
        return { type: "request", request: { id: m.id, method: "openForm", command: m.command, input: m.input ?? {} } };
      }
      case "signIn":
      case "cancelSignIn":
        if (typeof m.instance !== "string" || !m.instance || typeof m.name !== "string" || !m.name)
          return null;
        return { type: "request", request: { id: m.id, method: m.method, instance: m.instance, name: m.name } };
      case "setTitle":
        if (typeof m.title !== "string")
          return null;
        return { type: "request", request: { id: m.id, method: "setTitle", title: m.title } };
      case "setActions": {
        if (!Array.isArray(m.actions))
          return null;
        const actions = [];
        for (const raw of m.actions) {
          const a = raw;
          if (!a || typeof a.id !== "string" || !a.id || typeof a.label !== "string")
            return null;
          actions.push({
            id: a.id,
            label: a.label,
            primary: a.primary === true,
            prominent: a.prominent === true,
            disabled: a.disabled === true,
            ...typeof a.submenu === "string" && a.submenu.trim() ? { submenu: a.submenu.trim() } : {},
            ...typeof a.icon === "string" && isIconName(a.icon) ? { icon: a.icon } : {}
          });
        }
        return { type: "request", request: { id: m.id, method: "setActions", actions } };
      }
      case "setViewParams": {
        const params = m.params;
        if (!params || typeof params !== "object" || Array.isArray(params))
          return null;
        if (!Object.values(params).every((v) => v === null || typeof v === "string"))
          return null;
        return { type: "request", request: { id: m.id, method: "setViewParams", params } };
      }
      case "setHistoryState": {
        const patch = m.patch;
        if (!patch || typeof patch !== "object" || Array.isArray(patch))
          return null;
        return { type: "request", request: { id: m.id, method: "setHistoryState", patch } };
      }
      case "pageMemoryGet":
        if (typeof m.key !== "string" || !m.key)
          return null;
        return { type: "request", request: { id: m.id, method: "pageMemoryGet", key: m.key } };
      case "pageMemorySet":
        if (typeof m.key !== "string" || !m.key)
          return null;
        return { type: "request", request: { id: m.id, method: "pageMemorySet", key: m.key, value: m.value ?? null } };
      case "showView": {
        const spec = m.spec ?? null;
        if (spec !== null && typeof spec !== "string")
          return null;
        return { type: "request", request: { id: m.id, method: "showView", spec } };
      }
      default:
        return null;
    }
  }
  function initMessage(fed, tokens, page = null, view = null) {
    return {
      type: "init",
      protocol: BRIDGE_PROTOCOL,
      run: fed.run,
      data: fed.data,
      params: fed.params,
      props: fed.props ?? null,
      page,
      view,
      tokens
    };
  }
  var PAGE_ONLY = "Only a component shown as a page sets its title, actions or the view it shows.";
  var NO_FORMS = "This host shows no forms and signs in for no component.";
  var NO_VIEW = "This host gives the component no view: only a page, a replacement or a viz's own page does.";
  function createBridgeHost(port, deps, sent, sentView = null) {
    let lastData = Object.fromEntries(Object.entries(sent.data).map(([k, v]) => [k, JSON.stringify(v)]));
    let lastParams = JSON.stringify(sent.params);
    let lastRun = JSON.stringify(sent.run ? [sent.run.params, sent.run.result] : null);
    let lastView = sentView ? JSON.stringify(sentView.viewParams) : null;
    let lastState = sentView ? JSON.stringify(sentView.historyState) : null;
    let ready = false;
    const reply = (id, result) => port.postMessage({ id, ok: true, result });
    const fail = (id, code, message, field = null) => port.postMessage({ id, ok: false, error: field === null ? { code, message } : { code, message, field } });
    const failWith = (id, e) => {
      const code = e?.code;
      const field = e?.field;
      fail(id, typeof code === "string" ? code : "FAILED", e instanceof Error ? e.message : String(e), typeof field === "string" && field ? field : null);
    };
    const handle = async (r) => {
      switch (r.method) {
        case "invoke":
          try {
            reply(r.id, (await deps.invoke(r.command, r.input, false)).result);
          } catch (e) {
            if (!needsConfirmation(e))
              return failWith(r.id, e);
            if (!await deps.confirm(r.command, ipcErrorDetails(e))) {
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
          if (deps.navigate(r.uri, { newTab: r.newTab, replace: r.replace }))
            reply(r.id, null);
          else
            fail(r.id, "INVALID", "A component may open oxplow's pages only.");
          return;
        case "querySql":
          if (!deps.querySql)
            return fail(r.id, "INVALID", "This host runs no SQL for a component.");
          try {
            reply(r.id, await deps.querySql(r.sql, r.params));
          } catch (e) {
            failWith(r.id, e);
          }
          return;
        case "openForm": {
          if ("form" in r) {
            if (!deps.askForm)
              return fail(r.id, "INVALID", NO_FORMS);
            try {
              const values = await deps.askForm(r.form);
              if (values === null)
                return fail(r.id, "CANCELLED", "The person closed the form.");
              reply(r.id, values);
            } catch (e) {
              failWith(r.id, e);
            }
            return;
          }
          if (!deps.openForm)
            return fail(r.id, "INVALID", NO_FORMS);
          try {
            const out = await deps.openForm(r.command, r.input);
            if (out === null)
              return fail(r.id, "CANCELLED", `The person closed the form for \`${r.command}\`.`);
            reply(r.id, out.result);
          } catch (e) {
            failWith(r.id, e);
          }
          return;
        }
        case "signIn":
          if (!deps.signIn)
            return fail(r.id, "INVALID", NO_FORMS);
          try {
            reply(r.id, await deps.signIn(r.instance, r.name));
          } catch (e) {
            failWith(r.id, e);
          }
          return;
        case "cancelSignIn":
          if (!deps.cancelSignIn)
            return fail(r.id, "INVALID", NO_FORMS);
          try {
            await deps.cancelSignIn(r.instance, r.name);
            reply(r.id, null);
          } catch (e) {
            failWith(r.id, e);
          }
          return;
        case "setTitle":
          if (!deps.setTitle)
            return fail(r.id, "INVALID", PAGE_ONLY);
          deps.setTitle(r.title);
          return reply(r.id, null);
        case "setActions":
          if (!deps.setActions)
            return fail(r.id, "INVALID", PAGE_ONLY);
          deps.setActions(r.actions);
          return reply(r.id, null);
        case "setViewParams":
          if (!deps.setViewParams)
            return fail(r.id, "INVALID", NO_VIEW);
          try {
            deps.setViewParams(r.params);
            reply(r.id, null);
          } catch (e) {
            failWith(r.id, e);
          }
          return;
        case "setHistoryState":
          if (!deps.setHistoryState)
            return fail(r.id, "INVALID", NO_VIEW);
          try {
            deps.setHistoryState(r.patch);
            reply(r.id, null);
          } catch (e) {
            failWith(r.id, e);
          }
          return;
        case "showView":
          if (!deps.showView)
            return fail(r.id, "INVALID", PAGE_ONLY);
          try {
            reply(r.id, await deps.showView(r.spec) ?? null);
          } catch (e) {
            failWith(r.id, e);
          }
          return;
        case "pageMemoryGet":
          if (!deps.pageMemoryGet)
            return fail(r.id, "INVALID", "This host keeps no page memory.");
          try {
            reply(r.id, await deps.pageMemoryGet(r.key) ?? null);
          } catch (e) {
            failWith(r.id, e);
          }
          return;
        case "pageMemorySet":
          if (!deps.pageMemorySet)
            return fail(r.id, "INVALID", "This host keeps no page memory.");
          try {
            deps.pageMemorySet(r.key, r.value);
            reply(r.id, null);
          } catch (e) {
            failWith(r.id, e);
          }
          return;
      }
    };
    port.onmessage = (e) => {
      const m = parseFrameMessage(e.data);
      if (m?.type === "ready") {
        if (ready)
          return;
        ready = true;
        return deps.onReady();
      }
      if (m?.type === "size")
        return void deps.size?.(m.height);
      if (m?.type === "request")
        return void handle(m.request);
      const id = e.data?.id;
      if (typeof id === "string" && id)
        fail(id, "BAD_REQUEST", "Not a request this host knows.");
    };
    return {
      update: (fed) => {
        const data = Object.fromEntries(Object.entries(fed.data).map(([k, v]) => [k, JSON.stringify(v)]));
        const changed = [...new Set([...Object.keys(data), ...Object.keys(lastData)])].filter((k) => data[k] !== lastData[k]);
        const params = JSON.stringify(fed.params);
        const run = JSON.stringify(fed.run ? [fed.run.params, fed.run.result] : null);
        if (changed.length === 0 && params === lastParams && run === lastRun)
          return;
        lastData = data;
        lastParams = params;
        lastRun = run;
        port.postMessage({ type: "update", run: fed.run, data: fed.data, params: fed.params, changed });
      },
      viewParams: (params) => {
        const key = JSON.stringify(params);
        if (key === lastView)
          return;
        lastView = key;
        port.postMessage({ type: "viewParams", params });
      },
      historyState: (state) => {
        const key = JSON.stringify(state);
        if (key === lastState)
          return;
        lastState = key;
        port.postMessage({ type: "historyState", state });
      },
      action: (id) => port.postMessage({ type: "action", id }),
      close: () => {
        port.close();
        deps.onClose?.();
      }
    };
  }

  // src/viz/nodeRun.ts
  var EMPTY_RESULT = {
    columns: [],
    rows: [],
    truncated: false,
    reads: { datasets: [], tables: [] },
    freshness: []
  };
  function nodeRunOf(run, node) {
    if (node.kind !== "viz")
      return null;
    return {
      config: node.config,
      params: run.params,
      result: node.from !== null && run.loaders[node.from]?.result || EMPTY_RESULT
    };
  }
  function nodeById(page, id) {
    return find(page.view, id) ?? (page.rail ? find(page.rail, id) : null);
  }
  function find(at, id) {
    if (at.id === id)
      return at;
    if (at.kind === "stack" || at.kind === "grid" || at.kind === "switch") {
      for (const c of at.children) {
        const hit = find(c, id);
        if (hit)
          return hit;
      }
    }
    return null;
  }

  // src/viz/harnessHost.ts
  function fedBy(run, node) {
    const def = nodeById(run.page, node);
    if (def?.kind === "viz")
      return pageData(run, def.config.custom?.props ?? null, nodeRunOf(run, def));
    return pageData(run, def?.kind === "component" ? def.props : null);
  }
  function drawsFrames(wait = 2000) {
    return new Promise((drew) => {
      const timer = setTimeout(() => drew(false), wait);
      requestAnimationFrame(() => {
        clearTimeout(timer);
        drew(true);
      });
    });
  }
  async function runHarness(server, frame, agent, frames = drawsFrames) {
    const drawing = await frames();
    server.record("drawing", drawing);
    if (!drawing) {
      while ((await server.next(null)).op !== "done")
        ;
      return;
    }
    const start = await server.start();
    const fed = fedBy(start.run, start.node);
    let showing = Promise.resolve();
    const host = await new Promise((mounted) => {
      frame.addEventListener("load", () => {
        const channel = new MessageChannel;
        const record = (kind) => (value) => server.record(kind, value);
        const bridge = createBridgeHost(channel.port1, {
          invoke: async (command, input, confirmed) => ({ result: await server.rpc("invoke", { command, input, confirmed }) }),
          navigate: (ref) => {
            server.record("navigate", ref);
            return true;
          },
          confirm: async (command) => await server.rpc("confirm", { command }) === true,
          onReady: () => server.record("ready", true),
          querySql: async (sql, params) => await server.rpc("querySql", { sql, params }),
          openForm: async (command, input) => await server.rpc("openForm", { command, input }),
          askForm: async (form) => await server.rpc("askForm", { form }),
          showView: (spec) => {
            const mine = showing.then(async () => {
              if (spec === null) {
                server.record("showView", null);
                return null;
              }
              try {
                const drawn = await server.rpc("showView", { spec });
                server.record("showView", spec);
                return drawn;
              } catch (e) {
                server.record("showView", null);
                throw e;
              }
            });
            showing = mine.then(() => {
              return;
            }, () => {
              return;
            });
            return mine;
          },
          setTitle: record("title"),
          setActions: record("actions"),
          setViewParams: record("viewParams"),
          setHistoryState: record("historyState")
        }, fed, start.view);
        frame.contentWindow?.postMessage(initMessage(fed, {}, start.page, start.view), "*", [channel.port2]);
        mounted(bridge);
      }, { once: true });
      frame.src = `/components/v/${start.version}/`;
    });
    const pending = new Map;
    agent.answers((id, result) => {
      pending.get(id)?.(result);
      pending.delete(id);
    });
    let ids = 0;
    let result = null;
    for (;; ) {
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
  function httpServer() {
    let sent = Promise.resolve();
    const post = async (path, body) => {
      const response = await fetch(path, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(body)
      });
      return response.json();
    };
    return {
      start: async () => (await fetch("/harness/start")).json(),
      rpc: async (method, body) => {
        const out = await post("/harness/rpc", { method, ...body });
        if ("error" in out)
          throw new IpcCallError(out.error.message, out.error.code, out.error.field ?? null);
        return out.result;
      },
      record: (kind, value) => {
        sent = sent.then(() => fetch("/harness/record", {
          method: "POST",
          headers: { "content-type": "application/json" },
          body: JSON.stringify({ kind, value })
        }).then(() => {
          return;
        }, () => {
          return;
        }));
      },
      next: async (result) => {
        await sent;
        return await post("/harness/next", { result });
      }
    };
  }
  function main() {
    const frame = document.getElementById("frame");
    runHarness(httpServer(), frame, {
      send: (id, action) => frame.contentWindow?.postMessage({ oxplowHarness: id, action }, "*"),
      answers: (cb) => window.addEventListener("message", (e) => {
        if (e.source !== frame.contentWindow)
          return;
        const data = e.data;
        if (typeof data?.oxplowHarness === "number")
          cb(data.oxplowHarness, data.result);
      })
    }).then(() => {
      location.href = "/harness/done";
    });
  }

  // src/viz/harnessEntry.ts
  main();
})();
