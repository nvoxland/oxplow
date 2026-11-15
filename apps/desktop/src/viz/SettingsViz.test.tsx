import { afterAll, afterEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";
import { useState } from "react";
import { PageNavigationContext, type PageNavigation } from "../tabs/PageNavigationContext.js";
import type { VizRun } from "../api.js";
import { SettingField } from "./SettingField.js";
import { inventory, validate } from "./settingsModel.js";

const realApi = { ...(await import("../api.js")) };
const calls: { name: string; input: unknown }[] = [];
mock.module("../api.js", () => ({ ...realApi, runCommand: async (name: string, input: unknown) => {
  calls.push({ name, input });
  return { result: null, audit_id: null, event_id: null, inverse: null };
} }));
const { SettingsViz } = await import("./SettingsViz.js");
afterEach(() => { cleanup(); calls.length = 0; });
afterAll(() => mock.module("../api.js", () => realApi));
function run(overrides: Record<string, unknown> = {}, params = {}): VizRun {
  const row = { key: "oxplow.example", title: "Example", doc: "", category: "General", schema: JSON.stringify({ type: "string", default: "default" }), value: '"local saved"', origin: "local", local_value: '"local saved"', project_value: '"shared"', global_value: null, local_status: "set", project_status: "set", global_status: "unset", ...overrides };
  return { config: { id: "settings" }, params, result: { columns: Object.keys(row), rows: [Object.values(row)] } } as unknown as VizRun;
}

test("level selector is independent of effective source and save explicitly targets it", async () => {
  const ui = render(<SettingsViz run={run()} />);
  const level = ui.getByLabelText("View/edit level for Example");
  expect((level as HTMLSelectElement).value).toBe("local");
  expect(ui.container.querySelectorAll("details[open]").length).toBe(0);
  fireEvent.change(level, { target: { value: "project" } });
  expect(ui.getByText(/Source: local/)).toBeTruthy();
  fireEvent.click(ui.getByText("Edit project value"));
  const input = ui.getByLabelText("Example", { selector: "input" });
  expect((input as HTMLInputElement).value).toBe("shared");
  expect((ui.getByText("Save") as HTMLButtonElement).disabled).toBe(true);
  fireEvent.change(input, { target: { value: "new shared" } });
  fireEvent.click(ui.getByText("Save"));
  await waitFor(() => expect(calls).toEqual([{ name: "oxplow.config.set", input: { key: "oxplow.example", level: "project", value: "new shared" } }]));
});

test("refresh preserves draft and reports a conflicting change; discard follows latest", () => {
  const ui = render(<SettingsViz run={run()} />);
  fireEvent.click(ui.getByText("Edit local value"));
  fireEvent.change(ui.getByLabelText("Example", { selector: "input" }), { target: { value: "draft" } });
  ui.rerender(<SettingsViz run={run({ local_value: '"external"', value: '"external"' })} />);
  expect((ui.getByLabelText("Example", { selector: "input" }) as HTMLInputElement).value).toBe("draft");
  expect(ui.getByText(/saved value changed/)).toBeTruthy();
  fireEvent.click(ui.getByText("Discard"));
  expect((ui.getByLabelText("Example", { selector: "input" }) as HTMLInputElement).value).toBe("external");
});

test("reset previews inheritance and runs unset only on Save", async () => {
  const ui = render(<SettingsViz run={run()} />);
  fireEvent.click(ui.getByText("Edit local value"));
  fireEvent.click(ui.getByText("Reset to inherited value"));
  expect(ui.getByRole("status").textContent).toContain("shared");
  expect(calls.length).toBe(0);
  fireEvent.click(ui.getByText("Save"));
  await waitFor(() => expect(calls[0]).toEqual({ name: "oxplow.config.unset", input: { key: "oxplow.example", level: "local" } }));
});

test("null, empty and inherited have distinct values; secret statuses disclose no value", () => {
  expect(inventory(run({ project_value: "null", global_value: '""' }))[0]!.overrides).toEqual({ local: "local saved", project: null, global: "" });
  const ui = render(<SettingsViz run={run({ secret: 1, value: null, local_value: null })} />);
  expect(ui.container.textContent).not.toContain("local saved");
  fireEvent.click(ui.getByText("Edit local value"));
  expect(ui.getByLabelText("Example", { selector: "input" }).getAttribute("type")).toBe("password");
  expect((ui.getByText("Save") as HTMLButtonElement).disabled).toBe(true);
});

test("structured enums retain numeric and boolean types; collections validate nested values", () => {
  const values: unknown[] = [];
  const ui = render(<SettingField schema={{ enum: [false, 2, "two"] }} value={false} label="Choice" onChange={(v) => values.push(v)} />);
  fireEvent.change(ui.getByLabelText("Choice"), { target: { value: "1" } });
  expect(values).toEqual([2]);
  expect(validate({ type: "array", items: { type: "object", required: ["count"], properties: { count: { type: "integer", minimum: 1 } } } }, [{ count: 0 }, {}])).toEqual(["Value 1.count: enter a number at least 1.", "Value 2: count is required."]);
  expect(validate({ type: ["string", "null"] }, null)).toEqual([]);
});

test("editing global starts with its own inherited default, never a higher-priority local value", () => {
  const ui = render(<SettingsViz run={run()} />);
  fireEvent.change(ui.getByLabelText("View/edit level for Example"), { target: { value: "global" } });
  fireEvent.click(ui.getByText("Edit global value"));
  expect((ui.getByLabelText("Example", { selector: "input" }) as HTMLInputElement).value).toBe("default");
});

test("an unavailable secret can be repaired or removed at the explicitly selected level", async () => {
  const ui = render(<SettingsViz run={run({ secret: 1, value: null, origin: null, unavailable: 1, project_status: "unavailable", project_value: null })} />);
  fireEvent.click(ui.getByText("Edit project value"));
  fireEvent.change(ui.getByLabelText("Example", { selector: "input" }), { target: { value: "replacement" } });
  fireEvent.click(ui.getByText("Save"));
  await waitFor(() => expect(calls[0]).toEqual({ name: "oxplow.config.set", input: { key: "oxplow.example", level: "project", value: "replacement" } }));
});

test("core-style nested references produce structured LSP object, map and array controls", () => {
  const schema = {
    type: "array", items: { $ref: "#/$defs/LspServer" },
    $defs: {
      LspServer: { type: "object", additionalProperties: false, required: ["language", "command"], properties: { language: { type: "string" }, command: { type: "string" }, args: { type: "array", items: { type: "string" } }, env: { type: "object", additionalProperties: { type: "string" } }, enabled: { type: "boolean" } } },
    },
  };
  const values: unknown[] = [];
  const ui = render(<SettingField schema={schema} value={[{ language: "rust", command: "rust-analyzer", args: ["--stdio"], env: { LOG: "info" }, enabled: true }]} label="Servers" onChange={(v) => values.push(v)} />);
  expect((ui.getByLabelText("command") as HTMLInputElement).value).toBe("rust-analyzer");
  expect((ui.getByLabelText("enabled") as HTMLInputElement).checked).toBe(true);
  expect((ui.getByLabelText("args 1") as HTMLInputElement).value).toBe("--stdio");
  fireEvent.change(ui.getByLabelText("command"), { target: { value: "new-server" } });
  expect(values[0]).toEqual([{ language: "rust", command: "new-server", args: ["--stdio"], env: { LOG: "info" }, enabled: true }]);
  // Only env is an open map; the server object itself has no dynamic-entry editor.
  expect(ui.getAllByLabelText("New entry name").length).toBe(1);
  expect(validate(schema, [{ language: "rust", command: 7 }])).toEqual(["Value 1.command: enter a string."]);
});

test("nullable unions retain branch picker and choose typed arrays rather than raw text", () => {
  const schema = { anyOf: [{ type: "null" }, { type: "string", title: "Text" }, { type: "array", title: "List", items: { type: "integer" } }] };
  let value: unknown = "hello";
  const ui = render(<SettingField schema={schema} value={value} label="Union" onChange={(next) => { value = next; }} />);
  fireEvent.change(ui.getByLabelText("Union format"), { target: { value: "1" } });
  expect(value).toEqual([]);
  ui.rerender(<SettingField schema={schema} value={value} label="Union" onChange={(next) => { value = next; }} />);
  expect(ui.getByText("Add item")).toBeTruthy();
  fireEvent.click(ui.getByText("Add item"));
  expect(value).toEqual([0]);
  expect(validate({ type: "array", uniqueItems: true, items: { type: "boolean" } }, [true, true])).toEqual(["Value: each item must be unique."]);
});

test("ordinary drafts survive changing levels in tab history; secrets never enter history", () => {
  let persisted: Record<string, unknown> = {};
  function InTab({ secret = false }: { secret?: boolean }) {
    const [historyState, setHistoryState] = useState<Record<string, unknown>>({});
    const navigation = { historyState, setHistoryState: (patch: Record<string, unknown>) => setHistoryState((old) => { persisted = { ...old, ...patch }; return persisted; }) } as unknown as PageNavigation;
    return <PageNavigationContext.Provider value={navigation}><SettingsViz run={run(secret ? { secret: 1, value: null, local_value: null } : {})} /></PageNavigationContext.Provider>;
  }
  const ui = render(<InTab />);
  fireEvent.click(ui.getByText("Edit local value"));
  fireEvent.change(ui.getByLabelText("Example", { selector: "input" }), { target: { value: "ordinary draft" } });
  fireEvent.change(ui.getByLabelText("View/edit level for Example"), { target: { value: "global" } });
  fireEvent.change(ui.getByLabelText("View/edit level for Example"), { target: { value: "local" } });
  fireEvent.click(ui.getByText("Edit local value"));
  expect((ui.getByLabelText("Example", { selector: "input" }) as HTMLInputElement).value).toBe("ordinary draft");
  expect(JSON.stringify(persisted)).toContain("ordinary draft");
  ui.unmount();
  persisted = {};
  const secrets = render(<InTab secret />);
  fireEvent.click(secrets.getByText("Edit local value"));
  fireEvent.change(secrets.getByLabelText("Example", { selector: "input" }), { target: { value: "private-token" } });
  expect(JSON.stringify(persisted)).not.toContain("private-token");
});
