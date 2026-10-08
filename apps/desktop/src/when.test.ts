import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";

import { parseWhen, whenHolds, type WhenContext } from "./when.js";

// A command's `when`, read by the window exactly as the daemon checks it:
// both run the same cases (crates/oxplow-domain/fixtures/when_cases.json).

interface Cases {
  cases: Array<{ when: string; context: WhenContext; is: boolean }>;
  refused: Array<{ when: string; says: string }>;
}

const shared: Cases = JSON.parse(
  readFileSync(join(import.meta.dir, "../../../crates/oxplow-domain/fixtures/when_cases.json"), "utf8"),
);

test("the shared cases evaluate alike", () => {
  for (const c of shared.cases) {
    expect([c.when, whenHolds(parseWhen(c.when), c.context)]).toEqual([c.when, c.is]);
  }
});

test("the shared refusals say why", () => {
  for (const c of shared.refused) {
    let said = "";
    try {
      parseWhen(c.when);
    } catch (e) {
      said = e instanceof Error ? e.message : String(e);
    }
    expect([c.when, said.includes(c.says) ? c.says : said]).toEqual([c.when, c.says]);
  }
});
