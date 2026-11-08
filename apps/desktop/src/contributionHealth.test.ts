import { expect, test } from "bun:test";

import type { PersonCommands } from "./personCommands.js";
import { enableAgain, enableAllAgain } from "./contributionHealth.js";

const recording = () => {
  const runs: unknown[][] = [];
  const commands = {
    run: async (label: string, command: string, input: unknown) => {
      runs.push([label, command, input]);
      return true;
    },
  } as unknown as PersonCommands;
  return { runs, commands };
};

test("Enable Again runs oxplow.contribution.enable as the person", async () => {
  const { runs, commands } = recording();
  await enableAgain({ namespace: "tracker", contribution: "issues", kind: "collector" }, commands);
  expect(runs).toEqual([["Enable tracker/issues", "oxplow.contribution.enable", { namespace: "tracker", kind: "collector", contribution: "issues" }]]);
});

test("Enable All Disabled runs oxplow.contribution.enable_all as the person", async () => {
  const { runs, commands } = recording();
  await enableAllAgain(commands);
  expect(runs).toEqual([["Enable all disabled contributions", "oxplow.contribution.enable_all", {}]]);
});
