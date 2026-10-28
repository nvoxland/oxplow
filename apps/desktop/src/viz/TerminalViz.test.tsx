import { afterAll, afterEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";

import type { VizRun, SqlCell, Stream } from "../api.js";

// The terminal viz: a terminal per row, all kept, the picked one shown;
// with several, the strip picks (the viz's selection) and its menu
// renames and closes through the viz's actions; a stream with none opens
// one the first time the page shows.

const realApi = { ...(await import("../api.js")) };
const realView = { ...(await import("../terminal/TerminalView.js")) };
afterAll(() => {
  mock.module("../api.js", () => realApi);
  mock.module("../terminal/TerminalView.js", () => realView);
});
const commands: [string, unknown][] = [];
const actions: [string, unknown, unknown][] = [];
mock.module("../api.js", () => ({
  ...realApi,
  runCommand: async (name: string, input: unknown) => {
    commands.push([name, input]);
    return { result: { id: "trm9", uri: "oxplow:oxplow/terminal?id=trm9", title: "Terminal 9" }, audit_id: 1, event_id: null, inverse: null };
  },
}));
mock.module("../terminal/TerminalView.js", () => ({
  TerminalView: ({ uri, agent }: { uri: string; agent?: { conversation: string | null } }) => (
    <div data-testid={`view-${uri}`} data-agent={agent ? "yes" : "shell"} data-conversation={agent?.conversation ?? ""}>
      {uri}
    </div>
  ),
}));
const { TerminalViz } = await import("./TerminalViz.js");
const { VizStreamContext } = await import("./VizStream.js");
const { VizActionRunnerContext } = await import("./vizActionRunner.js");

// A viz presses through the page that draws it: a runner stands in for it.
const press = async (_id: string, action: string, _p: unknown, row: unknown, _s: unknown, _c: boolean, input?: unknown) => {
  actions.push([action, row, input]);
  return { result: null, audit_id: 2, event_id: null, inverse: null } as never;
};

afterEach(() => {
  cleanup();
  commands.length = 0;
  actions.length = 0;
});

const stream = { id: "str1", worktree_path: "/w" } as Stream;
const A = "oxplow:oxplow/terminal?id=trm1";
const B = "oxplow:oxplow/terminal?id=trm2";

function runOf(rows: SqlCell[][], picked: SqlCell = null): VizRun {
  return {
    config: {
      id: "oxplow-foundation/terminal",
      viz: "terminal",
      terminal: { of: "uri", title: "title", conversation: null },
      columns: [{ key: "title", select: "terminal" }],
      params: [],
      actions: [
        { id: "rename", label: "Rename…", command: "oxplow.terminal.rename", row: true, form: true },
        { id: "close", label: "Close terminal", command: "oxplow.terminal.close", row: true },
      ],
    },
    params: { stream_id: 1, terminal: picked },
    result: { columns: ["uri", "title", "terminal"], rows, truncated: false, reads: { datasets: [], tables: [], measures: [] } },
  } as unknown as VizRun;
}

const view = (run: VizRun, onSetParam?: (n: string, v: SqlCell) => void) =>
  render(
    <VizActionRunnerContext.Provider value={press}>
      <VizStreamContext.Provider value={stream}>
        <TerminalViz run={run} onSetParam={onSetParam} />
      </VizStreamContext.Provider>
    </VizActionRunnerContext.Provider>,
  );

test("two rows: two terminals kept, the first shown, and the strip picks another", async () => {
  const picks: [string, SqlCell][] = [];
  const v = view(runOf([[A, "One", "trm1"], [B, "Two", "trm2"]]), (n, val) => picks.push([n, val]));
  expect(v.getByTestId(`view-${A}`).parentElement!.style.display).toBe("flex");
  expect(v.getByTestId(`view-${B}`).parentElement!.style.display).toBe("none");
  fireEvent.click(v.getByTestId(`terminal-tab-${B}`));
  expect(picks).toEqual([["terminal", "Two"]]);
});

test("the picked row is the one shown", () => {
  const v = view(runOf([[A, "One", "trm1"], [B, "Two", "trm2"]], "Two"));
  expect(v.getByTestId(`view-${B}`).parentElement!.style.display).toBe("flex");
});

test("one row draws no strip", () => {
  const v = view(runOf([[A, "One", "trm1"]]));
  expect(v.queryByTestId("terminal-tab-strip")).toBeNull();
});

test("a stream with none opens one, once", async () => {
  const v = view(runOf([]));
  await waitFor(() => expect(commands).toEqual([["oxplow.terminal.open", { stream: "str1" }]]));
  v.rerender(
    <VizActionRunnerContext.Provider value={press}>
      <VizStreamContext.Provider value={stream}>
        <TerminalViz run={runOf([])} />
      </VizStreamContext.Provider>
    </VizActionRunnerContext.Provider>,
  );
  await new Promise((r) => setTimeout(r, 10));
  expect(commands.length).toBe(1);
});

test("the strip's Close runs the viz's close action on that row", async () => {
  const v = view(runOf([[A, "One", "trm1"], [B, "Two", "trm2"]]));
  fireEvent.click(v.getByTestId("terminal-tab-expand"));
  fireEvent.contextMenu(v.getByTestId(`terminal-tab-row-${B}`));
  fireEvent.click(v.getByTestId("menu-item-terminal.close"));
  await waitFor(() => expect(actions[0]?.[0]).toBe("close"));
  expect(actions[0]?.[1]).toEqual({ uri: B, title: "Two", terminal: "trm2" });
});

test("an agent session's row is its agent: what it's bound to comes from the row", async () => {
  const S = "oxplow:oxplow/agent_session?id=ses4";
  const run = {
    config: {
      id: "oxplow-foundation/agent-session-terminal",
      viz: "terminal",
      terminal: { of: "uri", title: null, conversation: "conversation", thread: "thread" },
      columns: [],
      params: [],
      actions: [],
    },
    params: { uri: S },
    result: { columns: ["uri", "thread", "conversation"], rows: [[S, "oxplow:oxplow/thread?id=thr1", "abc:0"]], truncated: false, reads: { datasets: [], tables: [], measures: [] } },
  } as unknown as VizRun;
  const v = view(run);
  const pane = v.getByTestId(`view-${S}`);
  expect(pane.dataset.agent).toBe("yes");
  expect(pane.dataset.conversation).toBe("abc:0");
  // A viz of an agent's terminal never opens a shell.
  expect(commands.some(([name]) => name === "oxplow.terminal.open")).toBe(false);
});
