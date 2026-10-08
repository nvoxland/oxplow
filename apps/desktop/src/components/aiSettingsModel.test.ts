import { describe, expect, test } from "bun:test";
import type { AiSettings } from "../tauri-bridge/generated/bindings.js";
import {
  emptyProviderForm,
  kindLabel,
  providerFormError,
  roleRows,
  testModelFor,
  usageRows,
  USAGE_SQL,
} from "./aiSettingsModel.js";

const settings: AiSettings = {
  providers: [
    { id: "or", kind: "openrouter", baseUrl: null, keySet: true },
    { id: "local", kind: "openai_compatible", baseUrl: "http://localhost:11434/v1", keySet: false },
  ],
  kinds: [
    { kind: "openrouter", title: "OpenRouter", defaultBaseUrl: "https://openrouter.ai/api/v1" },
    { kind: "openai_compatible", title: "An OpenAI-compatible API", defaultBaseUrl: null },
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
  test("a new provider needs an id without spaces, and starts as the first kind", () => {
    expect(emptyProviderForm(settings.kinds).kind).toBe("openrouter");
    expect(providerFormError(emptyProviderForm(settings.kinds), settings)).toBe("Name it (e.g. openrouter).");
    expect(providerFormError({ ...emptyProviderForm(settings.kinds), id: "my key" }, settings)).toBe(
      "The name can't contain spaces.",
    );
  });

  test("a kind with no default URL needs a base URL", () => {
    const form = { ...emptyProviderForm(settings.kinds), id: "ollama", kind: "openai_compatible" };
    expect(providerFormError(form, settings)).toBe("An OpenAI-compatible API needs a base URL.");
    expect(providerFormError({ ...form, baseUrl: "http://localhost:11434/v1" }, settings)).toBeNull();
  });

  test("with no kinds registered there is nothing to add", () => {
    const none = { ...settings, kinds: [] };
    expect(providerFormError({ ...emptyProviderForm([]), id: "x" }, none)).toBe("No AI provider kinds are available.");
  });

  test("adding a name that's taken is refused, editing it isn't", () => {
    const form = { ...emptyProviderForm(settings.kinds), id: "or" };
    expect(providerFormError(form, settings)).toBe("There's already an AI provider named or.");
    expect(providerFormError({ ...form, editing: true }, settings)).toBeNull();
  });

  test("a kind is shown by its title", () => {
    expect(kindLabel(settings.kinds, "openrouter")).toBe("OpenRouter");
    expect(kindLabel(settings.kinds, "gone")).toBe("gone");
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
    expect(decide.problem).toBe("AI provider gone isn't set up.");
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
