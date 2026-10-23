import { expect, test } from "bun:test";

import { assembleInstances } from "./providerInstances.js";

const instances = {
  columns: [
    "instance", "extension", "provider", "instance_id", "capability", "scope", "overridden", "enabled",
    "state", "detail", "config", "config_schema", "consecutive_failures", "last_ok_at", "mean_invoke_ms",
    "rate_limited_until", "activity",
  ],
  rows: [
    ["tracker/fake", "tracker", "fake", "fake", "work_items", "project", 0, 1, "ready", null, '{"team":"core"}', '{"type":"object"}', 0, "2026-10-01T00:00:00Z", 12.5, null, null],
    ["tracker/two", "tracker", "fake", "two", "work_items", "global", 1, 0, "unconfigured", "/team: required; a thing", "{}", "{}", 2, null, null, null, "reading"],
  ],
};
const credentials = {
  columns: ["instance", "name", "is_set", "signs_in", "sign_in_state", "signed_in_until", "redirect_port"],
  rows: [
    ["tracker/fake", "TOKEN", 1, 0, null, null, null],
    ["tracker/two", "OAUTH", 1, 1, "signed_in", "2026-12-01T00:00:00Z", 8124],
  ],
};
const collectors = {
  columns: ["instance", "name", "entity", "status", "error", "last_read_at", "records"],
  rows: [["tracker/fake", "work_items", "work_item", "ok", null, "2026-10-01T00:00:00Z", 3]],
};

test("the three models make one view per instance, with its credentials and collectors", () => {
  const [fake, two] = assembleInstances(instances, credentials, collectors);
  expect(fake).toMatchObject({
    instance: "tracker/fake",
    scope: "project",
    enabled: true,
    overridden: false,
    config: { team: "core" },
    health: { state: { state: "ready" }, meanInvokeMs: 12.5, lastOkAt: "2026-10-01T00:00:00Z" },
    credentials: [{ name: "TOKEN", set: true, signIn: null, redirectPort: null }],
    collectors: [{ name: "work_items", entity: "work_item", status: "ok", records: 3 }],
  });
  expect(two.scope).toBe("global");
  expect(two.overridden).toBe(true);
  expect(two.health.state).toEqual({
    state: "unconfigured",
    problems: [{ path: "/team", message: "required" }, { path: "", message: "a thing" }],
  });
  expect(two.health.activity).toBe("reading");
  expect(two.credentials[0].signIn).toEqual({ state: "signed_in", until: "2026-12-01T00:00:00Z" });
  expect(two.credentials[0].redirectPort).toBe(8124);
  expect(two.collectors).toEqual([]);
});
