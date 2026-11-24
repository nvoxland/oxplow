import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";

import { findRuleLinks, rulesOf } from "./linkRules.js";

// An extension's `links:` make text a link, the same as the daemon's
// matcher does (`link_rules.rs`): the cases both run.
const shared = JSON.parse(
  readFileSync(join(import.meta.dir, "../../../crates/oxplow-app/src/extension/extensions/link_rules_cases.json"), "utf8"),
) as { rules: never[]; cases: { text: string; links: { text: string; uri: string }[] }[] };

test("the shared cases match", () => {
  for (const c of shared.cases) {
    expect(findRuleLinks(c.text, shared.rules).map((l) => ({ text: l.text, uri: l.uri }))).toEqual(c.links);
  }
});

test("the rules are the enabled extensions', in order", () => {
  const ext = (name: string, enabled: boolean, ids: string[]) =>
    ({ name, enabled, links: ids.map((id) => ({ id, pattern: "X", uri: "https://x/{{match}}" })) }) as never;
  expect(rulesOf([ext("a", true, ["one"]), ext("b", false, ["two"]), ext("c", true, ["three"])]).map((r) => r.id)).toEqual(["one", "three"]);
  expect(rulesOf(null)).toEqual([]);
});
