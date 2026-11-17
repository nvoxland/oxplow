import { afterAll, afterEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";
import { useState } from "react";
import { PageNavigationContext, type PageNavigation } from "../tabs/PageNavigationContext.js";
import type { VizRun } from "../api.js";
import { Contents } from "./Contents.js";
import { Section } from "../kit/Section.js";
import { SettingField } from "./SettingField.js";
import { inventory, validate } from "./settingsModel.js";
import { IpcCallError } from "../ipc-error.js";

const realApi = { ...(await import("../api.js")) };
const calls: { name: string; input: unknown }[] = [];
// The next command's refusal, when a test wants one.
let refuse: Error | null = null;
mock.module("../api.js", () => ({ ...realApi, runCommand: async (name: string, input: unknown) => {
  calls.push({ name, input });
  if (refuse) throw refuse;
  return { result: null, audit_id: null, event_id: null, inverse: null };
} }));
const { SettingsViz } = await import("./SettingsViz.js");
afterEach(() => { cleanup(); calls.length = 0; refuse = null; });
afterAll(() => {
  mock.module("../api.js", () => realApi);
});
function run(overrides: Record<string, unknown> = {}, params = {}): VizRun {
  const row = { key: "oxplow.example", title: "Example", doc: "", category: "General", schema: JSON.stringify({ type: "string", default: "default" }), value: '"local saved"', origin: "local", local_value: '"local saved"', project_value: '"shared"', global_value: null, local_status: "set", project_status: "set", global_status: "unset", ...overrides };
  return { config: { id: "settings" }, params, result: { columns: Object.keys(row), rows: [Object.values(row)] } } as unknown as VizRun;
}

test("level selector is independent of effective source and save explicitly targets it", async () => {
  const ui = render(<SettingsViz run={run()} />);
  const level = ui.getByLabelText("Saves to for Example");
  expect((level as HTMLSelectElement).value).toBe("local");
  expect(ui.container.querySelectorAll("details[open]").length).toBe(0);
  fireEvent.change(level, { target: { value: "project" } });
  expect(ui.getByText(/Source: Just me in this project/)).toBeTruthy();
  const input = ui.getByLabelText("Example", { selector: "input" });
  expect((input as HTMLInputElement).value).toBe("shared");
  expect((ui.getByText("Save") as HTMLButtonElement).disabled).toBe(true);
  fireEvent.change(input, { target: { value: "new shared" } });
  fireEvent.click(ui.getByText("Save"));
  await waitFor(() => expect(calls).toEqual([{ name: "oxplow.config.set", input: { key: "oxplow.example", level: "project", value: "new shared" } }]));
});

test("refresh preserves draft and reports a conflicting change; discard follows latest", () => {
  const ui = render(<SettingsViz run={run()} />);
  fireEvent.change(ui.getByLabelText("Example", { selector: "input" }), { target: { value: "draft" } });
  ui.rerender(<SettingsViz run={run({ local_value: '"external"', value: '"external"' })} />);
  expect((ui.getByLabelText("Example", { selector: "input" }) as HTMLInputElement).value).toBe("draft");
  expect(ui.getByText(/saved value changed/)).toBeTruthy();
  fireEvent.click(ui.getByText("Discard"));
  expect((ui.getByLabelText("Example", { selector: "input" }) as HTMLInputElement).value).toBe("external");
});

test("reset previews inheritance and runs unset only on Save", async () => {
  const ui = render(<SettingsViz run={run()} />);
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
  fireEvent.click(ui.getByText("Edit value: Just me in this project"));
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

test("a oneOf of titled consts is one labeled choice, with the chosen one's description", () => {
  const values: unknown[] = [];
  const schema = {
    type: "string",
    oneOf: [
      { const: "off", title: "Off", description: "Agents hear nothing about tasks." },
      { const: "agent_tracks", title: "The agent tracks its work", description: "The agent files a task for each piece of work." },
    ],
  };
  const ui = render(<SettingField schema={schema} value="agent_tracks" label="Workflow" onChange={(v) => values.push(v)} />);
  const select = ui.getByLabelText("Workflow") as HTMLSelectElement;
  expect([...select.options].map((o) => o.textContent)).toEqual(["Choose a value", "Off", "The agent tracks its work"]);
  expect(ui.getByText("The agent files a task for each piece of work.")).toBeTruthy();
  expect(ui.queryByLabelText("Workflow format")).toBeNull();
  fireEvent.change(select, { target: { value: "0" } });
  expect(values).toEqual(["off"]);
});

test("editing global starts with its own inherited default, never a higher-priority local value", () => {
  const ui = render(<SettingsViz run={run()} />);
  fireEvent.change(ui.getByLabelText("Saves to for Example"), { target: { value: "global" } });
  expect((ui.getByLabelText("Example", { selector: "input" }) as HTMLInputElement).value).toBe("default");
});

test("an unavailable secret can be repaired or removed at the explicitly selected level", async () => {
  const ui = render(<SettingsViz run={run({ secret: 1, value: null, origin: null, unavailable: 1, project_status: "unavailable", project_value: null })} />);
  fireEvent.click(ui.getByText("Edit value: This project"));
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
  fireEvent.change(ui.getByLabelText("Example", { selector: "input" }), { target: { value: "ordinary draft" } });
  fireEvent.change(ui.getByLabelText("Saves to for Example"), { target: { value: "global" } });
  fireEvent.change(ui.getByLabelText("Saves to for Example"), { target: { value: "local" } });
  expect((ui.getByLabelText("Example", { selector: "input" }) as HTMLInputElement).value).toBe("ordinary draft");
  expect(JSON.stringify(persisted)).toContain("ordinary draft");
  ui.unmount();
  persisted = {};
  const secrets = render(<InTab secret />);
  fireEvent.click(secrets.getByText("Edit value: Just me in this project"));
  fireEvent.change(secrets.getByLabelText("Example", { selector: "input" }), { target: { value: "private-token" } });
  expect(JSON.stringify(persisted)).not.toContain("private-token");
});


test("saving or closing a secret editor clears its input rather than retaining the submitted value", async () => {
  const ui = render(<SettingsViz run={run({ secret: 1, value: null, local_value: null, is_set: 1 })} />);
  fireEvent.click(ui.getByText("Edit value: Just me in this project"));
  const input = ui.getByLabelText("Example", { selector: "input" }) as HTMLInputElement;
  fireEvent.change(input, { target: { value: "private-secret-draft" } });
  fireEvent.click(ui.getByText("Save"));
  await waitFor(() => expect(input.value).toBe(""));
  fireEvent.change(input, { target: { value: "another-private-draft" } });
  fireEvent.click(ui.getByText("Collapse editor"));
  fireEvent.click(ui.getByText("Edit value: Just me in this project"));
  expect((ui.getByLabelText("Example", { selector: "input" }) as HTMLInputElement).value).toBe("");
});


test("settings use standard nested sections and Contents links without listing editor disclosures", async () => {
  const data = run();
  data.result.rows.push(run({ key: "oxplow.second", title: "Second setting" }).result.rows[0]!);
  function Page() {
    const [root, setRoot] = useState<HTMLDivElement | null>(null);
    return <><div ref={setRoot}><Section id="category" title="Preferences"><SettingsViz run={data} /></Section></div><Contents root={root} /></>;
  }
  const ui = render(<Page />);
  const contents = await ui.findByRole("navigation", { name: "Contents" });
  expect(Array.from(contents.querySelectorAll("button")).map((button) => button.textContent)).toEqual(["Preferences", "Example", "Second setting"]);
  const heading = ui.getByRole("heading", { name: "Second setting", level: 3 });
  let scrolled = false;
  heading.scrollIntoView = () => { scrolled = true; };
  fireEvent.click(ui.getByRole("button", { name: "Second setting" }));
  expect(scrolled).toBe(true);
  expect(heading.closest("section")?.getAttribute("data-section")).toBe("settings/oxplow.second");
  expect(contents.textContent).not.toContain("All levels");
  expect(contents.textContent).not.toContain("Technical details");
});

test("a value the setting refuses shows the reason the save gave, once", async () => {
  refuse = new IpcCallError("`/1` must be a string", "INVALID", "/value/1");
  const ui = render(<SettingsViz run={run()} />);
  fireEvent.change(ui.getByLabelText("Example", { selector: "input" }), { target: { value: "draft" } });
  fireEvent.click(ui.getByText("Save"));
  const alert = await waitFor(() => ui.getByText("`/1` must be a string"));
  expect(alert.getAttribute("role")).toBe("alert");
});

test("a scalar is edited in its section: change it and Save, the level is secondary", async () => {
  const ui = render(<SettingsViz run={run({ schema: JSON.stringify({ type: "boolean", default: false }), value: "true", origin: "project", local_value: null, local_status: "unset", project_value: "true" })} />);
  // No Edit button to press first: the control is there.
  expect(ui.queryByText(/^Edit .* value$/)).toBeNull();
  const box = ui.getByLabelText("Example", { selector: "input" }) as HTMLInputElement;
  expect(box.checked).toBe(true);
  expect((ui.getByLabelText("Saves to for Example") as HTMLSelectElement).value).toBe("project");
  fireEvent.click(box);
  fireEvent.click(ui.getByText("Save"));
  await waitFor(() => expect(calls).toEqual([{ name: "oxplow.config.set", input: { key: "oxplow.example", level: "project", value: false } }]));
});

test("levels read as a person knows them, the same everywhere", () => {
  const ui = render(<SettingsViz run={run({ schema: JSON.stringify({ type: "array", items: { type: "string" } }), value: '["a"]', local_value: '["a"]', project_value: '["b"]' })} />);
  const text = ui.container.textContent ?? "";
  expect(text).toContain("Source: Just me in this project");
  expect(text).not.toMatch(/\b(local|global)\b/);
  expect(ui.getByText("Edit value: Just me in this project")).toBeTruthy();
});

test("a setting says where it is changed, and its schema reads as JSON", () => {
  const ui = render(<SettingsViz run={run({ restart_required: 1 })} />);
  expect(ui.getByText("Changed for: Just me in this project, This project")).toBeTruthy();
  expect(ui.getByText("Takes effect after oxplow restarts.")).toBeTruthy();
  const schema = ui.container.querySelector("pre code");
  expect(schema?.textContent).toBe(JSON.stringify({ type: "string", default: "default" }, null, 2));
  ui.unmount();
  const unchanged = render(<SettingsViz run={run({ local_value: null, local_status: "unset", project_value: null, project_status: "unset", origin: "default", value: '"default"' })} />);
  expect(unchanged.queryByText(/Changed for:/)).toBeNull();
});

test("the page's query is the filter: the visualization shows every row it is given", () => {
  const ui = render(<SettingsViz run={run({}, { q: "matches only a value" })} />);
  expect(ui.getByRole("heading", { name: "Example" })).toBeTruthy();
});

test("a list item moves down as well as up", () => {
  let value: unknown = ["a", "b", "c"];
  const ui = render(<SettingField schema={{ type: "array", items: { type: "string" } }} value={value} label="Items" onChange={(next) => { value = next; }} />);
  fireEvent.click(ui.getAllByText("Move down")[0]!);
  expect(value).toEqual(["b", "a", "c"]);
  expect(ui.getAllByText("Move down").length).toBe(2);
});
