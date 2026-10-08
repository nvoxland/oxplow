import { expect, test } from "bun:test";

import { userInterruptEnvelope } from "./api.js";

/** The Escape-synthesized Interrupt names the agent session it stops, so
 *  only that session's turn closes. */
test("a user interrupt names its agent session", () => {
  expect(userInterruptEnvelope("ses3", "thr1", "str1")).toEqual({
    kind: "interrupt",
    thread_id: "thr1",
    stream_id: "str1",
    agent_session_id: "ses3",
    session_id: null,
    payload_json: JSON.stringify({ source: "user-escape" }),
    prompt: null,
    decision: null,
  });
});
