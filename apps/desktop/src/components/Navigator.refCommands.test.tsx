import { afterEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";

import type { Stream, Thread, ThreadState } from "../api.js";
import { personSpec, recordRefOffers } from "./refCommandsTestSupport.js";

// A thread row's menu ends with the commands about its thread
// (usability.md "Menus a page or row gets from extensions"): Hand Off
// Work… among them, bound to that thread.

const realApi = await import("../api.js");
mock.module("../api.js", () => ({
  ...realApi,
  refCommands: async (ref: string) => (ref.startsWith("oxplow:oxplow/thread") ? ["oxplow.handoff.send"] : []),
  listPersonCommands: async () => [
    personSpec("oxplow.handoff.send", {
      label: "Hand Off Work…",
      group: "Threads",
      about: "oxplow/thread",
      input: { from_thread: "{{uri}}" },
    }),
    personSpec("tracker.item.estimate", {
      label: "Estimate",
      group: "Tracker",
      about: "oxplow/work_item",
    }),
  ],
}));
const { Navigator } = await import("./Navigator.js");

afterEach(cleanup);

const STREAM = {
  id: "str1",
  kind: "primary",
  title: "Main",
  branch: "main",
} as unknown as Stream;
const THREADS: Record<string, ThreadState> = {
  str1: {
    selectedThreadId: "thr1",
    threads: [
      { id: "thr1", stream_id: "str1", title: "Writer", mark: null },
      { id: "thr2", stream_id: "str1", title: "Research", mark: null },
    ] as unknown as Thread[],
  },
};

test("a thread's menu offers the commands about that thread", async () => {
  const ran: Array<[string, unknown]> = [];
  recordRefOffers(ran);
  const noop = async () => {};
  const { getByTestId, queryByTestId } = render(
    <Navigator
      streams={[STREAM]}
      currentStreamId="str1"
      threadStates={THREADS}
      streamStatuses={{}}
      agentStatuses={{}}
      enabledAgents={["claude"]}
      onSwitchStream={noop}
      onSelectThread={noop}
      onCreateThread={noop}
      vcsEnabled
    />,
  );
  await new Promise((r) => setTimeout(r, 0));
  fireEvent.click(getByTestId("navigator-expand"));
  fireEvent.contextMenu(getByTestId("navigator-thread-row-thr2"));
  fireEvent.click(
    await waitFor(() => getByTestId("menu-item-ref-commands-Threads")),
  );
  const item = await waitFor(() =>
    getByTestId("menu-item-ref-command-oxplow.handoff.send"),
  );
  expect(item.textContent).toContain("Hand Off Work…");
  expect(
    queryByTestId("menu-item-ref-command-tracker.item.estimate"),
  ).toBeNull();
  fireEvent.click(item);
  await waitFor(() =>
    expect(ran).toEqual([
      ["oxplow.handoff.send", { from_thread: "oxplow:oxplow/thread?id=thr2" }],
    ]),
  );
});
