import { afterEach, beforeEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";

import type { AssistantBackend, AssistantItem, AssistantSnapshot } from "./assistantApi.js";

const realApi = await import("../../api.js");
const undone: number[] = [];
mock.module("../../api.js", () => ({
  ...realApi,
  querySql: async () => ({ columns: [], rows: [], truncated: false, reads: { models: [], tables: [], measures: [] } }),
  undoCommand: async (id: number) => {
    undone.push(id);
    return {};
  },
}));
const { setAssistantBackend } = await import("./assistantApi.js");
const { AssistantDockBody, useAssistantBadge } = await import("./AssistantDockPanel.js");
const { askAssistant, resetAssistant, setAssistantScope } = await import("./assistantSession.js");
const { dockStore } = await import("../Dock/dockStore.js");
const model = await import("./assistantModel.js");

// ---- a scripted backend ---------------------------------------------------------

let snap: AssistantSnapshot;
const calls: string[] = [];
let opened: unknown[] = [];
let notify: (seq: number) => void = () => {};
let snapshots = 0;
const backend: AssistantBackend = {
  open: async (a) => {
    opened.push(a);
    return { id: "ast1" };
  },
  prompt: async (_id, text) => void calls.push(`prompt:${text}`),
  cancel: async () => void calls.push("cancel"),
  set: async (_id, c) => void calls.push(`set:${JSON.stringify(c)}`),
  answer: async (_id, ask, allow) => void calls.push(`answer:${ask}:${allow}`),
  snapshot: async (_id, since) => {
    snapshots++;
    return { ...snap, items: snap.items.filter((i) => i.seq > since) };
  },
  handoff: async (_id, to) => void calls.push(`handoff:${to}`),
  close: async () => void calls.push("close"),
  subscribe: (_id, f) => {
    notify = f;
    return () => {};
  },
};

const idleSnap = (items: AssistantItem[] = []): AssistantSnapshot => ({
  status: "idle",
  items,
  seq: items.reduce((m, i) => Math.max(m, i.seq), 0),
  model: { provider: "p", model: "m" },
  models: [{ provider: "p", model: "m", label: "p · m" }],
  allowChanges: false,
  problem: null,
});
const user = (id: number, text: string): AssistantItem => ({ id, seq: id, type: "user", text, context: null });
const agent = (id: number, text: string): AssistantItem => ({ id, seq: id, type: "agent", text });

beforeEach(() => {
  snap = idleSnap();
  calls.length = 0;
  opened = [];
  snapshots = 0;
  undone.length = 0;
  setAssistantBackend(backend);
  setAssistantScope({ stream: "str1", thread: "thr1" });
});
afterEach(() => {
  cleanup();
  resetAssistant();
  dockStore.close();
  setAssistantBackend(null);
});

const body = () => <AssistantDockBody streamId="str1" onOpenPage={() => {}} />;

// ---- the model --------------------------------------------------------------------

test("snapshots upsert by id, newer seq winning, and the head follows", () => {
  let s = model.mergeSnapshot(model.initialState(), idleSnap([user(1, "hi"), agent(2, "he")]));
  s = model.mergeSnapshot(s, { ...idleSnap([{ ...agent(2, "hello"), seq: 5 }]), status: "running" });
  expect(s.items.map((i) => (i.type === "agent" ? i.text : i.type))).toEqual(["user", "hello"]);
  expect(s.headSeq).toBe(5);
  expect(model.isBusy(s)).toBe(true);
});

test("the panel's mode: no model, empty, running, a card waiting, idle", () => {
  const base = model.initialState();
  expect(model.panelMode({ ...base, problem: { kind: "no_model", message: "x" } })).toBe("no_model");
  expect(model.panelMode(base)).toBe("empty");
  expect(model.panelMode({ ...base, status: "running" })).toBe("running");
  const card: AssistantItem = { id: 3, seq: 3, type: "permission", requestId: "a1", toolCallId: "t", title: "Rename", options: [], answer: null };
  expect(model.panelMode({ ...base, items: [user(1, "go"), card] })).toBe("waiting");
  expect(model.panelMode({ ...base, items: [user(1, "go"), agent(2, "ok")] })).toBe("idle");
  expect(model.badgeCount({ ...base, items: [card] })).toBe(1);
  expect(model.badgeCount({ ...base, status: "running" })).toBe(1);
  expect(model.badgeCount(base)).toBe(0);
});

test("only the latest turn's completed, audited calls undo together, newest first", () => {
  const tool = (id: number, audit: number | null): AssistantItem => ({
    id,
    seq: id,
    type: "tool",
    call: { id: `c${id}`, title: "t", name: null, kind: "other", status: "completed", locations: [], rawInput: null, rawOutput: null, diffs: [], text: [], auditId: audit },
  });
  const s = { ...model.initialState(), items: [user(1, "a"), tool(2, 10), user(3, "b"), tool(4, 11), tool(5, null), tool(6, 12)] };
  expect(model.lastTurnUndoable(s).map((c) => c.auditId)).toEqual([12, 11]);
});

// ---- the panel --------------------------------------------------------------------

test("with no model, the panel says so and links to Settings → AI", async () => {
  snap = { ...idleSnap(), model: null, models: [], problem: { kind: "no_model", message: "Bind a model to the assistant role." } };
  await askAssistant(null);
  const opens: string[] = [];
  const view = render(<AssistantDockBody streamId="str1" onOpenPage={(r) => opens.push(r.id)} />);
  await waitFor(() => expect(view.getByTestId("assistant-no-model").textContent).toContain("Bind a model"));
  fireEvent.click(view.getByTestId("assistant-open-settings"));
  expect(opens).toEqual(["oxplow:oxplow/page?page=settings#area=ai"]);
  expect((view.getByTestId("assistant-input") as HTMLTextAreaElement).disabled).toBe(true);
});

test("a fresh session offers suggestion chips that fill the box and send nothing", async () => {
  await askAssistant({ kind: "page", uri: "oxplow:oxplow/commit?sha=abc1234", label: "abc1234" });
  const view = render(body());
  await waitFor(() => expect(view.getByTestId("assistant-about").textContent).toContain("abc1234"));
  const chip = (await waitFor(() => view.getAllByTestId("assistant-suggestion")))[0]!;
  fireEvent.click(chip);
  expect((view.getByTestId("assistant-input") as HTMLTextAreaElement).value).toBe(chip.textContent!);
  expect(calls).toEqual([]);
  expect(opened).toEqual([{ stream: "str1", thread: "thr1", context: { kind: "page", uri: "oxplow:oxplow/commit?sha=abc1234", label: "abc1234" } }]);
});

test("Enter sends, Shift+Enter doesn't", async () => {
  await askAssistant(null);
  const view = render(body());
  const input = await waitFor(() => view.getByTestId("assistant-input") as HTMLTextAreaElement);
  fireEvent.change(input, { target: { value: "what changed?" } });
  fireEvent.keyDown(input, { key: "Enter", shiftKey: true });
  expect(calls).toEqual([]);
  fireEvent.keyDown(input, { key: "Enter" });
  await waitFor(() => expect(calls).toEqual(["prompt:what changed?"]));
  expect(input.value).toBe("");
});

test("while a turn runs the box is read-only, Escape cancels it, and the dock panel isn't closed", async () => {
  snap = { ...idleSnap([user(1, "go")]), status: "running" };
  await askAssistant(null);
  let reachedDock = false;
  const view = render(<div onKeyDown={() => (reachedDock = true)}>{body()}</div>);
  const input = await waitFor(() => view.getByTestId("assistant-input") as HTMLTextAreaElement);
  await waitFor(() => expect(view.getByTestId("assistant-panel").getAttribute("data-mode")).toBe("running"));
  expect(input.readOnly).toBe(true);
  fireEvent.keyDown(input, { key: "Escape" });
  await waitFor(() => expect(calls).toEqual(["cancel"]));
  expect(reachedDock).toBe(false);
  fireEvent.click(view.getByTestId("assistant-stop"));
});

test("a waiting card is answered with Allow or Decline", async () => {
  snap = idleSnap([
    user(1, "rename it"),
    { id: 2, seq: 2, type: "permission", requestId: "a7", toolCallId: "t", title: "oxplow.code.rename", options: [], answer: null },
  ]);
  await askAssistant(null);
  const view = render(body());
  await waitFor(() => view.getByTestId("assistant-permission-a7"));
  expect(view.getByTestId("assistant-panel").getAttribute("data-mode")).toBe("waiting");
  fireEvent.click(view.getByTestId("assistant-allow-a7"));
  fireEvent.click(view.getByTestId("assistant-decline-a7"));
  await waitFor(() => expect(calls).toEqual(["answer:a7:true", "answer:a7:false"]));
});

test("a finished call can be undone, and the turn's calls together", async () => {
  snap = idleSnap([
    user(1, "tidy"),
    { id: 2, seq: 2, type: "tool", call: { id: "c", title: "Rename", name: null, kind: "other", status: "completed", locations: [], rawInput: null, rawOutput: null, diffs: [], text: ["done"], auditId: 42 } },
    agent(3, "Renamed."),
  ]);
  await askAssistant(null);
  const view = render(body());
  fireEvent.click(await waitFor(() => view.getByTestId("assistant-undo")));
  await waitFor(() => expect(undone).toEqual([42]));
  await waitFor(() => expect(view.queryByTestId("assistant-undo")).toBeNull());
});

test("the transcript follows backend changes, and the badge counts a running turn", async () => {
  await askAssistant(null);
  const Badge = () => <span data-testid="badge">{useAssistantBadge()?.count ?? 0}</span>;
  const view = render(
    <>
      <Badge />
      {body()}
    </>,
  );
  await waitFor(() => view.getByTestId("assistant-input"));
  snap = { ...idleSnap([user(1, "hi"), agent(2, "hello")]), status: "running" };
  notify(2);
  await waitFor(() => expect(view.getByTestId("assistant-item-2").textContent).toContain("hello"));
  expect(view.getByTestId("badge").textContent).toBe("1");
});

test("asking about something new starts a new session when the current one has turns", async () => {
  snap = idleSnap([user(1, "hi"), agent(2, "hello")]);
  await askAssistant({ kind: "page", uri: "u1" });
  await askAssistant({ kind: "page", uri: "u2" });
  expect(opened.length).toBe(2);
  expect(calls).toContain("close");
  expect(dockStore.get().open).toBe("assistant");
});

test("a flood of changes fetches at most one snapshot in flight and one after", async () => {
  await askAssistant(null);
  const view = render(body());
  await waitFor(() => view.getByTestId("assistant-input"));
  const before = snapshots;
  snap = idleSnap([user(1, "hi"), agent(2, "hello")]);
  for (let seq = 1; seq <= 50; seq++) notify(seq);
  await waitFor(() => expect(view.getByTestId("assistant-item-2")).toBeTruthy());
  await new Promise((r) => setTimeout(r, 30));
  expect(snapshots - before).toBeLessThanOrEqual(2);
});
