import { afterEach, expect, test } from "bun:test";
import { act, cleanup, render } from "@testing-library/react";

import { triggerRemoteResync } from "../api.js";
import { NO_READS, useRerunOnChange } from "./lensRerun.js";

afterEach(cleanup);

// tsk1050: an open lens kept what it showed before the daemon restarted,
// though what it read changed meanwhile: events sent while the socket was
// down never arrive. A reconnect re-runs it.
test("a reconnect re-runs every open lens host", async () => {
  let runs = 0;
  function Host() {
    useRerunOnChange(NO_READS, () => {
      runs++;
    });
    return null;
  }
  render(<Host />);
  await act(async () => {
    triggerRemoteResync();
    await new Promise((resolve) => setTimeout(resolve, 150));
  });
  expect(runs).toBe(1);
});
