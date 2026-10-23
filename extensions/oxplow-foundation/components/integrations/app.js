// Settings → Integrations: an extension provider's instances on this
// machine — where each stands, its config, credentials and collectors, and
// what a person can do about it. oxplow's own component, hand-written (no
// build). It reads the `v_provider_*` models and changes them with the
// `oxplow.provider.*` commands; the forms, the confirm of a removal and the
// OAuth sign-in are oxplow's (`openForm`, `invoke`, `signIn`).
(function () {
  "use strict";

  const el = (tag, props, ...children) => {
    const node = document.createElement(tag);
    for (const [k, v] of Object.entries(props || {})) {
      if (v === undefined || v === null || v === false) continue;
      if (k === "className") node.className = v;
      else if (k === "testId") node.setAttribute("data-testid", v);
      else if (k === "onclick") node.addEventListener("click", v);
      else if (k === "disabled") node.disabled = !!v;
      else node.setAttribute(k, v === true ? "" : v);
    }
    for (const c of children) if (c !== null && c !== undefined && c !== false) node.append(c);
    return node;
  };
  const text = (v) => (v === null || v === undefined ? null : String(v));
  const parse = (v) => {
    try {
      return typeof v === "string" ? JSON.parse(v) : null;
    } catch {
      return null;
    }
  };
  const message = (e) => (e && e.message ? e.message : String(e));
  const when = (iso) => (iso ? new Date(iso).toLocaleString([], { dateStyle: "short", timeStyle: "short" }) : "");

  function records(result) {
    return result.rows.map((row) => Object.fromEntries(result.columns.map((c, i) => [c, row[i] === undefined ? null : row[i]])));
  }

  function stateOf(kind, detail) {
    const reason = detail || "";
    switch (kind) {
      case "unconfigured":
        return {
          state: kind,
          problems: reason
            .split("; ")
            .filter((p) => p !== "")
            .map((p) => {
              const at = p.indexOf(": ");
              return p.startsWith("/") && at > 0 ? { path: p.slice(0, at), message: p.slice(at + 2) } : { path: "", message: p };
            }),
        };
      case "failing":
        return { state: kind, errors: reason.split("; ").filter((e) => e !== "") };
      case "missing":
      case "refused":
      case "disabled":
        return { state: kind, reason };
      default:
        return { state: kind };
    }
  }

  /** The views the three models' rows make. */
  function assemble(instances, credentials, collectors) {
    const creds = records(credentials);
    const colls = records(collectors);
    return records(instances).map((r) => ({
      instance: r.instance,
      scope: r.scope,
      overridden: r.overridden === 1,
      extension: r.extension,
      provider: r.provider,
      instanceId: r.instance_id,
      capability: r.capability,
      enabled: r.enabled === 1,
      config: parse(r.config) || {},
      configSchema: parse(r.config_schema) || {},
      state: stateOf(r.state, text(r.detail)),
      failures: r.consecutive_failures || 0,
      meanMs: typeof r.mean_invoke_ms === "number" ? r.mean_invoke_ms : null,
      limitedUntil: text(r.rate_limited_until),
      activity: text(r.activity),
      credentials: creds
        .filter((c) => c.instance === r.instance)
        .map((c) => ({
          name: c.name,
          set: c.is_set === 1,
          signsIn: c.signs_in === 1,
          signIn: c.sign_in_state,
          until: text(c.signed_in_until),
        })),
      collectors: colls
        .filter((c) => c.instance === r.instance)
        .map((c) => ({ name: c.name, status: c.status, error: text(c.error), lastReadAt: text(c.last_read_at), records: c.records || 0 })),
    }));
  }

  function statusOf(v) {
    const s = v.state;
    let status;
    switch (s.state) {
      case "off":
        status = "Off";
        break;
      case "missing":
        status = s.reason;
        break;
      case "refused":
        status = "Not started: " + s.reason;
        break;
      case "unconfigured":
        status = "Config problems: " + s.problems.map((p) => (p.path || "/") + " " + p.message).join("; ");
        break;
      case "checking":
        status = "Checking…";
        break;
      case "ready":
        status = v.meanMs === null ? "Ready" : "Ready · ~" + Math.round(v.meanMs) + " ms a call";
        break;
      case "failing":
        status = "Failing (" + v.failures + " in a row): " + (s.errors[s.errors.length - 1] || "");
        break;
      case "disabled":
        status = "Disabled: " + s.reason;
        break;
      default:
        status = s.state;
    }
    if (v.limitedUntil && new Date(v.limitedUntil).getTime() > Date.now()) {
      status += " · rate limited until " + new Date(v.limitedUntil).toLocaleTimeString();
    }
    if (v.activity) status += " · " + v.activity;
    return { status, problem: ["missing", "refused", "unconfigured", "failing", "disabled"].includes(s.state) };
  }

  const enableLabel = (v) => (v.state.state === "disabled" ? "Enable again" : v.enabled ? "Disable" : "Enable");
  const label = (v) =>
    v.instance + " · " + v.capability.replace(/_/g, " ") + (v.overridden ? " · this project's, replacing yours" : v.scope === "global" ? " · yours, in every project" : "");
  const canTurnOffHere = (v) => v.scope === "global" && !v.overridden;
  const canRemove = (v) => v.state.state === "missing" || v.instanceId !== v.provider || v.overridden;
  const hasConfig = (v) => v.state.state !== "missing" && Object.keys((v.configSchema && v.configSchema.properties) || {}).length > 0;

  function signInLine(c) {
    switch (c.signIn) {
      case "signed_in":
        return { text: c.until ? "Signed in until " + new Date(c.until).toLocaleString() : "Signed in", action: "Sign in again", signedIn: true, problem: false };
      case "sign_in_again":
        return { text: "Sign in again: the sign-in lapsed or was withdrawn", action: "Sign in again", signedIn: false, problem: true };
      default:
        return { text: "Not signed in", action: "Sign in", signedIn: false, problem: false };
    }
  }

  function collectorLine(c) {
    const n = c.records + (c.records === 1 ? " record" : " records");
    switch (c.status) {
      case "never":
        return { text: c.name + ": not read yet", problem: false };
      case "reading":
        return { text: c.name + ": reading… · " + n, problem: false };
      case "error":
        return { text: c.name + ": its last read failed (" + (c.error || "unknown") + ") · " + n + " · " + when(c.lastReadAt), problem: true };
      default:
        return { text: c.name + ": " + n + " · last read " + when(c.lastReadAt), problem: false };
    }
  }

  const NAME_PATTERN = "^[a-z][a-z0-9_]*$";

  void oxplow.connect().then((c) => {
    c.applyTheme();
    const root = document.body;
    /** @type {ReturnType<typeof assemble>} */
    let views = [];
    // A config edited in its form and not yet saved, by instance.
    const edits = new Map();
    // What an instance's last Check said (its view's state), by instance.
    const checked = new Map();
    // A line of news under an instance: what failed, or how a sign-in ended.
    const notes = new Map();
    const waiting = new Set();
    let busy = null;

    const note = (instance, textValue, problem) => {
      if (textValue === null) notes.delete(instance);
      else notes.set(instance, { text: textValue, problem: !!problem });
      render();
    };
    const run = async (instance, key, fn) => {
      busy = key;
      render();
      try {
        await fn();
        note(instance, null);
      } catch (e) {
        note(instance, message(e), true);
      } finally {
        busy = null;
        await load();
      }
    };

    async function load() {
      const [instances, credentials, collectors] = await Promise.all([
        c.querySql("SELECT * FROM v_provider_instance ORDER BY instance"),
        c.querySql("SELECT * FROM v_provider_credential ORDER BY instance, name"),
        c.querySql("SELECT * FROM v_provider_collector ORDER BY instance, name"),
      ]);
      views = assemble(instances, credentials, collectors);
      render();
    }

    const button = (testId, labelText, onclick, opts) =>
      el("button", Object.assign({ type: "button", className: "ox-button", testId, onclick }, opts || {}), labelText);

    function credentialRow(v, cred) {
      const id = v.instance + "-" + cred.name;
      if (cred.signsIn) {
        const line = signInLine(cred);
        return el(
          "div",
          { className: "line", testId: "sign-in-" + id },
          el("code", {}, cred.name),
          el("span", { className: line.problem ? "problem" : "muted" }, line.text),
          waiting.has(id) ? el("span", { className: "muted" }, "Finish signing in in your browser…") : null,
          el("span", { className: "grow" }),
          button("sign-in-button-" + id, line.action, async () => {
            waiting.add(id);
            note(v.instance, null);
            try {
              const result = await c.signIn(v.instance, cred.name);
              if (result.state === "failed") note(v.instance, result.error, true);
            } catch (e) {
              note(v.instance, message(e), true);
            } finally {
              waiting.delete(id);
              await load();
            }
          }),
          line.signedIn
            ? button("sign-out-" + id, "Sign out", () =>
                run(v.instance, "sign-out", () => c.invoke("oxplow.provider.set_credential", { instance: v.instance, name: cred.name })),
              )
            : null,
        );
      }
      return el(
        "div",
        { className: "line", testId: "credential-" + id },
        el("code", {}, cred.name),
        el("span", { className: "muted" }, cred.set ? "Saved in keychain" : "Not set"),
        el("span", { className: "grow" }),
        button("credential-set-" + id, cred.set ? "Replace…" : "Set…", async () => {
          let values;
          try {
            values = await c.openForm({
              title: v.instance + " · " + cred.name,
              schema: { type: "object", properties: { value: { type: "string", title: cred.name, writeOnly: true } }, required: ["value"] },
            });
          } catch (e) {
            if (e && e.code === "CANCELLED") return;
            return note(v.instance, message(e), true);
          }
          await run(v.instance, "credential", () => c.invoke("oxplow.provider.set_credential", { instance: v.instance, name: cred.name, value: values.value }));
        }),
        cred.set
          ? button("credential-forget-" + id, "Forget", () =>
              run(v.instance, "credential", () => c.invoke("oxplow.provider.set_credential", { instance: v.instance, name: cred.name })),
            )
          : null,
      );
    }

    function instanceRow(v) {
      const shown = checked.get(v.instance) ? Object.assign({}, v, { state: checked.get(v.instance) }) : v;
      const m = statusOf(shown);
      const missing = v.state.state === "missing";
      const config = edits.has(v.instance) ? edits.get(v.instance) : v.config;
      const unsaved = edits.has(v.instance);
      const news = notes.get(v.instance);
      return el(
        "div",
        { className: "row", testId: "integration-row-" + v.instance },
        el(
          "div",
          { className: "line" },
          el("code", {}, label(v)),
          el("span", { className: m.problem ? "problem" : "muted", testId: "integration-status-" + v.instance }, m.status),
          unsaved ? el("span", { className: "muted", testId: "integration-unsaved-" + v.instance }, "· config edited, not saved") : null,
          el("span", { className: "grow" }),
          hasConfig(v)
            ? button("integration-configure-" + v.instance, "Configure…", async () => {
                let values;
                try {
                  values = await c.openForm({ title: "Configure " + v.instance, schema: v.configSchema, values: config });
                } catch (e) {
                  if (e && e.code === "CANCELLED") return;
                  return note(v.instance, message(e), true);
                }
                edits.set(v.instance, values);
                checked.delete(v.instance);
                note(v.instance, null);
              }, { disabled: busy !== null })
            : null,
          button(
            "integration-check-" + v.instance,
            busy === "check:" + v.instance ? "Checking…" : "Check",
            () =>
              run(v.instance, "check:" + v.instance, async () => {
                const out = await c.invoke("oxplow.provider.check_instance", { instance: v.instance, config });
                checked.set(v.instance, stateOf(out.health.state.state, detailOf(out.health.state)));
              }),
            { disabled: busy !== null || missing, title: "Start it with this config and ask it to check the config; nothing is saved or enabled" },
          ),
          button(
            "integration-toggle-" + v.instance,
            enableLabel(v),
            () =>
              run(v.instance, "toggle:" + v.instance, async () => {
                const enable = enableLabel(v) !== "Disable";
                await c.invoke("oxplow.provider.set_instance", { instance: v.instance, enabled: enable, config });
                edits.delete(v.instance);
                checked.delete(v.instance);
              }),
            { disabled: busy !== null || missing, title: enableLabel(v) === "Disable" ? "Stop it and turn it off in the project's config" : "Check it, save the config and run it on this machine" },
          ),
          unsaved && v.enabled
            ? button("integration-save-" + v.instance, "Save changes", () =>
                run(v.instance, "save:" + v.instance, async () => {
                  await c.invoke("oxplow.provider.set_instance", { instance: v.instance, enabled: true, config });
                  edits.delete(v.instance);
                  checked.delete(v.instance);
                }),
              )
            : null,
          canTurnOffHere(v)
            ? button("integration-off-here-" + v.instance, "Off in this project", () =>
                run(v.instance, "off-here", () => c.invoke("oxplow.provider.turn_off_here", { instance: v.instance })),
              { disabled: busy !== null, title: "Turn it off in this project only: this project gets its own entry, off, and every other project keeps running yours. Remove that entry to bring yours back here." })
            : null,
          canRemove(v)
            ? button("integration-remove-" + v.instance, "Remove", () =>
                run(v.instance, "remove", () => c.invoke("oxplow.provider.remove_instance", { instance: v.instance })),
              { disabled: busy !== null })
            : null,
        ),
        news ? el("div", { className: news.problem ? "problem" : "muted", testId: "integration-note-" + v.instance }, news.text) : null,
        ...v.collectors.map((col) => {
          const line = collectorLine(col);
          return el(
            "div",
            { className: "line", testId: "integration-collector-" + v.instance + "-" + col.name },
            el("span", { className: line.problem ? "problem" : "muted" }, line.text),
            v.state.state === "ready"
              ? button("integration-sync-" + v.instance + "-" + col.name, "Sync Now", () =>
                  run(v.instance, "sync", () => c.invoke("oxplow.provider.sync", { instance: v.instance, collector: col.name })),
                { disabled: busy !== null, title: "Read this collector now, from where its last read left off" })
              : null,
          );
        }),
        ...v.credentials.map((cred) => credentialRow(v, cred)),
      );
    }

    function detailOf(state) {
      if (state.reason) return state.reason;
      if (state.problems) return state.problems.map((p) => (p.path ? p.path + ": " : "") + p.message).join("; ");
      if (state.errors) return state.errors.join("; ");
      return null;
    }

    /** Each declared provider once, whatever its instances. */
    function programs() {
      const seen = new Map();
      for (const v of views) {
        if (v.state.state === "missing" || !v.provider) continue;
        const key = v.extension + "/" + v.provider;
        if (!seen.has(key)) seen.set(key, { key, extension: v.extension, provider: v.provider });
      }
      return [...seen.values()].sort((a, b) => a.key.localeCompare(b.key));
    }

    function addRow(p) {
      return el(
        "div",
        { className: "add" },
        el("span", { className: "muted" }, "Add another instance of "),
        el("code", {}, p.key),
        button("integration-add-" + p.key, "Add…", async () => {
          let values;
          try {
            values = await c.openForm({
              title: "Add an instance of " + p.key,
              schema: {
                type: "object",
                properties: {
                  name: { type: "string", title: "Name", description: "Lowercase letters, digits and underscores, starting with a letter: the id its refs and commands carry.", pattern: NAME_PATTERN },
                  scope: { type: "string", title: "Whose", enum: ["project", "global"], default: "project", description: "project: this project's, shared with the team; global: yours, in every project." },
                },
                required: ["name"],
              },
              values: { scope: "project" },
            });
          } catch (e) {
            if (e && e.code === "CANCELLED") return;
            return note(p.key, message(e), true);
          }
          if (views.some((v) => v.instanceId === values.name)) return note(p.key, "`" + values.name + "` is already an instance", true);
          await run(p.key, "add", () => c.invoke("oxplow.provider.add_instance", { instance: p.extension + "/" + values.name, provider: p.provider, scope: values.scope || "project" }));
        }, { disabled: busy !== null }),
        notes.get(p.key) ? el("span", { className: "problem" }, notes.get(p.key).text) : null,
      );
    }

    function render() {
      root.replaceChildren(
        el(
          "div",
          { testId: "integrations-section" },
          views.length === 0
            ? el("p", { className: "empty", testId: "integrations-empty" }, "No extension provides an integration yet.")
            : null,
          ...views.map(instanceRow),
          ...programs().map(addRow),
        ),
      );
    }

    c.onUpdate(() => void load());
    void load();
  });
})();
