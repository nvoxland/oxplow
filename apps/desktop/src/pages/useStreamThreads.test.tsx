import { afterEach, expect, mock, test } from "bun:test";
import { act, cleanup, renderHook } from "@testing-library/react";

import { type StreamThreadsApi, useStreamThreads } from "./useStreamThreads.js";

type Handler = (event: Record<string, unknown>) => void;

afterEach(cleanup);

function api(): { api: StreamThreadsApi; handlers: Handler[]; listThreads: ReturnType<typeof mock> } {
  const handlers: Handler[] = [];
  const listThreads = mock(async (streamId: string) => [{ id: `${streamId}-t`, title: "x" }]);
  return {
    handlers,
    listThreads,
    api: {
      listThreads: listThreads as never,
      subscribeOxplowEvents: ((h: Handler) => {
        handlers.push(h);
        return () => {};
      }) as never,
    },
  };
}

// tsk790: a stream's threads load once, and re-read when `v_thread` changes —
// a thread created, renamed or closed shows in the picker without a remount.
test("a loaded stream's threads re-read on a v_thread change, and only then", async () => {
  const { api: injected, handlers, listThreads } = api();
  const { result } = renderHook(() => useStreamThreads(injected, () => {}));
  await act(async () => {
    result.current.requestThreads("str1");
    result.current.requestThreads("str1");
    await Promise.resolve();
  });
  expect(listThreads.mock.calls.map((c) => (c as unknown[])[0])).toEqual(["str1"]);
  expect(result.current.threadsByStream.str1?.length).toBe(1);

  await act(async () => {
    for (const h of handlers) h({ kind: "modelsChanged", models: ["v_commit"] });
    await Promise.resolve();
  });
  expect(listThreads).toHaveBeenCalledTimes(1);

  await act(async () => {
    for (const h of handlers) h({ kind: "modelsChanged", models: ["v_thread"] });
    await Promise.resolve();
  });
  expect(listThreads.mock.calls.map((c) => (c as unknown[])[0])).toEqual(["str1", "str1"]);
});
