import { expect, test } from "bun:test";

// The SDK serves the harness's page script from a committed build
// (`crates/oxplow-sdk/assets/harness-host.js`, `bun run build:harness`):
// it must be what the source builds to, and stay free of React.
test("the committed harness script is what its source builds to", async () => {
  const built = await Bun.build({
    entrypoints: [`${import.meta.dir}/harnessEntry.ts`],
    target: "browser",
    format: "iife",
  });
  expect(built.success).toBe(true);
  const fresh = await built.outputs[0].text();
  const committed = await Bun.file(`${import.meta.dir}/../../../../crates/oxplow-sdk/assets/harness-host.js`).text();
  expect(fresh === committed, "stale: run `bun run build:harness` in apps/desktop").toBe(true);
  expect(fresh.toLowerCase()).not.toContain("react");
});
