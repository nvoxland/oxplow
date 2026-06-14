import { afterEach, expect, test } from "bun:test";
import { cleanup, fireEvent, render } from "@testing-library/react";

import type { AgentTokenUsage } from "../api.js";
import { TurnRow } from "./EffortTokenUsage.js";

afterEach(cleanup);

function row(overrides: Partial<AgentTokenUsage> = {}): AgentTokenUsage {
  return {
    id: 1,
    stream_id: "str1",
    thread_id: "thr1",
    effort_id: "eff1",
    session_id: "sess-1",
    agent_kind: "claude",
    model: "claude-opus-4-8",
    prompt: "fix the parser",
    input_tokens: 1200,
    output_tokens: 300,
    cache_creation_input_tokens: 0,
    cache_read_input_tokens: 0,
    message_count: 1,
    provenance: "observed",
    recorded_at: "2026-06-14T00:00:00Z" as unknown as AgentTokenUsage["recorded_at"],
    ...overrides,
  };
}

test("turn row renders prompt, model, and token total", () => {
  const { getByTestId } = render(<TurnRow row={row()} />);
  const li = getByTestId("effort-turn-1");
  expect(li.textContent).toContain("fix the parser");
  expect(li.textContent).toContain("claude-opus-4-8");
  // 1200 + 300 = 1500 → humanized to 1.5K.
  expect(li.textContent).toContain("1.5K");
});

test("a long prompt is collapsed until clicked, then expands fully", () => {
  const long = "x".repeat(400);
  const { getByTestId } = render(<TurnRow row={row({ id: 2, prompt: long })} />);
  const btn = getByTestId("effort-turn-prompt-2");

  // Collapsed: truncated with an ellipsis, shorter than the full prompt.
  expect(btn.textContent).toContain("…");
  expect(btn.textContent!.length).toBeLessThan(long.length);

  // Expanding shows the whole prompt, no ellipsis.
  fireEvent.click(btn);
  expect(btn.textContent).toBe(long);

  // Collapsing again restores the truncated preview.
  fireEvent.click(btn);
  expect(btn.textContent).toContain("…");
});
