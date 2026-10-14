import { afterEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";

// A role's binding can be restricted from tools, SQL, streaming and image
// input: unchecking what it allows saves the binding with `restrict`, and
// the row then says what it forbids.

const realApi = await import("../api.js");
let roleRestrict: string[] = [];
const saved: unknown[] = [];

const settings = () => ({
  providers: [{ id: "or", kind: "openrouter", baseUrl: null, keySet: true }],
  kinds: [{ kind: "openrouter", title: "OpenRouter", defaultBaseUrl: "https://openrouter.ai/api/v1" }],
  roles: [
    {
      role: "summarize",
      description: "Summaries",
      extension: null,
      fallback: null,
      binding: { provider: "or", model: "m", restrict: roleRestrict },
      effective: { provider: "or", model: "m", restrict: roleRestrict },
      overridden: false,
      problem: null,
    },
  ],
});

mock.module("../api.js", () => ({
  ...realApi,
  aiSettings: async () => settings(),
  querySql: async () => ({ columns: [], rows: [], truncated: false, reads: { models: [], tables: [], measures: [] }, freshness: {} }),
  setAiRole: async (_role: string, binding: { restrict?: string[] } | null) => {
    saved.push(binding);
    roleRestrict = binding?.restrict ?? [];
    return settings();
  },
}));

const { AiSection } = await import("./AiSection.js");

afterEach(() => {
  cleanup();
  roleRestrict = [];
  saved.length = 0;
});

test("unchecking what a binding allows saves it restricted, and the row says so", async () => {
  const view = render(<AiSection />);
  const tools = (await waitFor(() => view.getByTestId("ai-role-allow-summarize-tools"))) as HTMLInputElement;
  expect(tools.checked).toBe(true);
  expect(view.queryByTestId("ai-role-restricted-summarize")).toBeNull();

  fireEvent.click(tools);
  fireEvent.click(view.getByTestId("ai-role-allow-summarize-sql"));
  fireEvent.click(view.getByTestId("ai-role-save-summarize"));

  await waitFor(() => expect(saved).toEqual([{ provider: "or", model: "m", restrict: ["tools", "sql"] }]));
  await waitFor(() =>
    expect(view.getByTestId("ai-role-restricted-summarize").textContent).toBe("Restricted: no tools, no SQL"),
  );
  expect((view.getByTestId("ai-role-allow-summarize-tools") as HTMLInputElement).checked).toBe(false);
});

test("Escape puts the boxes back as saved", async () => {
  roleRestrict = ["streaming"];
  const view = render(<AiSection />);
  const streaming = (await waitFor(() => view.getByTestId("ai-role-allow-summarize-streaming"))) as HTMLInputElement;
  expect(streaming.checked).toBe(false);
  fireEvent.click(streaming);
  expect(streaming.checked).toBe(true);
  fireEvent.keyDown(streaming, { key: "Escape" });
  expect((view.getByTestId("ai-role-allow-summarize-streaming") as HTMLInputElement).checked).toBe(false);
  expect(saved).toEqual([]);
});
