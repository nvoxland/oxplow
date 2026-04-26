import { describe, expect, test } from "bun:test";
import { handleAutoCommitToggle } from "./ThreadSettingsPage.js";

describe("handleAutoCommitToggle", () => {
  test("invokes setAutoCommit with the new enabled value", async () => {
    const calls: Array<{ streamId: string; threadId: string; enabled: boolean }> = [];
    const fakeSetAutoCommit = async (streamId: string, threadId: string, enabled: boolean) => {
      calls.push({ streamId, threadId, enabled });
      return [];
    };

    await handleAutoCommitToggle("s-1", "t-1", true, fakeSetAutoCommit);

    expect(calls).toEqual([{ streamId: "s-1", threadId: "t-1", enabled: true }]);
  });

  test("propagates the disable transition", async () => {
    const calls: Array<{ enabled: boolean }> = [];
    const fakeSetAutoCommit = async (_s: string, _t: string, enabled: boolean) => {
      calls.push({ enabled });
      return [];
    };

    await handleAutoCommitToggle("s-1", "t-1", false, fakeSetAutoCommit);

    expect(calls).toEqual([{ enabled: false }]);
  });

  test("returns the updated thread list from the saver", async () => {
    const updated = [{ id: "t-1" }] as never;
    const fakeSetAutoCommit = async () => updated;
    const result = await handleAutoCommitToggle("s-1", "t-1", true, fakeSetAutoCommit);
    expect(result).toBe(updated);
  });
});
