import { describe, expect, test } from "bun:test";
import type { AiSettings } from "../tauri-bridge/generated/bindings.js";
import {
  emptyProviderForm,
  providerFormError,
  roleRows,
  testModelFor,
  usageRows,
  USAGE_SQL,
} from "./aiSettingsModel.js";

const settings: AiSettings = {
  providers: [
    { id: "or", kind: "openrouter", baseUrl: null, keySet: true },
    { id: "local", kind: "openai-compatible", baseUrl: "http://localhost:11434/v1", keySet: false },
  ],
  roles: [
    { role: "main", binding: null, overridden: false },
    { role: "fast", binding: null, overridden: false },
    { role: "summarize", binding: { provider: "or", model: "openai/gpt-5-mini" }, overridden: false },
    { role: "embed", binding: null, overridden: false },
    { role: "decide", binding: { provider: "gone", model: "jev" }, overridden: true },
    { role: "review", binding: null, overridden: false },
  ],
};

describe("providerFormError", () => {
  test("a new provider needs an id without spaces", () => {
    expect(providerFormError(emptyProviderForm(), [])).toBe("Name it (e.g. openrouter).");
    expect(providerFormError({ ...emptyProviderForm(), id: "my key" }, [])).toBe("The name can't contain spaces.");
  });

  test("openai-compatible servers need a base URL", () => {
    const form = { ...emptyProviderForm(), id: "ollama", kind: "openai-compatible" as const };
    expect(providerFormError(form, [])).toBe("Local and compatible servers need a base URL.");
    expect(providerFormError({ ...form, baseUrl: "http://localhost:11434/v1" }, [])).toBeNull();
  });

  test("adding a name that's taken is refused, editing it isn't", () => {
    const form = { ...emptyProviderForm(), id: "or" };
    expect(providerFormError(form, settings.providers)).toBe("There's already a provider named or.");
    expect(providerFormError({ ...form, editing: true }, settings.providers)).toBeNull();
  });
});

describe("roleRows", () => {
  test("every role with what it's for and its assignment", () => {
    const rows = roleRows(settings);
    expect(rows.map((r) => r.role)).toEqual(["main", "fast", "summarize", "embed", "decide", "review"]);
    const summarize = rows.find((r) => r.role === "summarize")!;
    expect(summarize.assigned).toBe("or · openai/gpt-5-mini");
    expect(summarize.usedFor.toLowerCase()).toContain("summar");
    expect(rows.find((r) => r.role === "main")!.assigned).toBeNull();
  });

  test("a binding to a missing provider is flagged, and project overrides are noted", () => {
    const decide = roleRows(settings).find((r) => r.role === "decide")!;
    expect(decide.problem).toBe("Provider gone isn't set up.");
    expect(decide.note).toBe("Set by this project");
    expect(decide.editable).toBe(false);
    expect(decide.lockedReason).toContain(".oxplow/project.yaml");
    expect(roleRows(settings).find((r) => r.role === "summarize")!.editable).toBe(true);
  });
});

test("testModelFor uses a model a role already assigns on that provider", () => {
  expect(testModelFor(settings, "or")).toBe("openai/gpt-5-mini");
  expect(testModelFor(settings, "local")).toBe("");
});

test("usageRows reads the usage query's rows", () => {
  expect(USAGE_SQL).toContain("v_ai_call");
  const rows = usageRows({
    columns: ["role", "caller", "calls", "failed", "input_tokens", "output_tokens", "last_at"],
    rows: [["summarize", "mcp:ai_summarize", 3, 1, 1200, 300, "2026-09-26T10:00:00Z"]],
    truncated: false,
  });
  expect(rows).toEqual([
    {
      role: "summarize",
      caller: "mcp:ai_summarize",
      calls: 3,
      failed: 1,
      inputTokens: 1200,
      outputTokens: 300,
      lastAt: "2026-09-26T10:00:00Z",
    },
  ]);
});
