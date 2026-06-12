import { describe, expect, test } from "bun:test";

import { requestNewThread, subscribeNewThreadRequests } from "./new-thread-bus.js";

describe("new-thread bus", () => {
  test("a request reaches the subscriber with the stream id", () => {
    const got: string[] = [];
    const unsub = subscribeNewThreadRequests((streamId) => got.push(streamId));
    try {
      requestNewThread("str1");
      expect(got).toEqual(["str1"]);
    } finally {
      unsub();
    }
  });

  test("unsubscribed listeners stop receiving; requests without listeners are no-ops", () => {
    const got: string[] = [];
    const unsub = subscribeNewThreadRequests((streamId) => got.push(streamId));
    unsub();
    requestNewThread("str2");
    expect(got).toEqual([]);
  });
});
