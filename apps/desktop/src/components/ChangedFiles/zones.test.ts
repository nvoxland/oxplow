import { describe, expect, test } from "bun:test";
import { classifyZone, compileZoneRules, zoneColor, ZONE_OTHER } from "./zones";
import fixture from "../../../../../fixtures/zone-globs.json";

const rules = (pairs: Array<[string, string | string[]]>) =>
  compileZoneRules(
    pairs.map(([zone, match]) => ({
      zone,
      match: Array.isArray(match) ? match : [match],
      color: null,
    })),
  );

describe("classifyZone", () => {
  // tsk251: oxplow ships no rule table — an unconfigured project has no
  // zone vocabulary at all, and the badges/treemap go quiet rather than
  // guessing from paths.
  test("with no rules everything is `other`", () => {
    const none = compileZoneRules([]);
    expect(classifyZone("crates/oxplow-db/src/lib.rs", none)).toBe(ZONE_OTHER);
    expect(classifyZone("anything.ts", none)).toBe(ZONE_OTHER);
  });

  test("first matching rule wins", () => {
    const r = rules([
      ["test", "**/*_test.rs"],
      ["store", "crates/db/**"],
      ["meta", "**/*.toml"],
    ]);
    expect(classifyZone("crates/db/src/thing_test.rs", r)).toBe("test");
    expect(classifyZone("crates/db/src/thing.rs", r)).toBe("store");
    expect(classifyZone("crates/db/Cargo.toml", r)).toBe("store");
    expect(classifyZone("tools/build.toml", r)).toBe("meta");
    expect(classifyZone("scripts/deploy.sh", r)).toBe(ZONE_OTHER);
  });

  test("a rule matches any of its patterns", () => {
    const r = rules([["test", ["**/*_test.rs", "**/tests/**"]]]);
    expect(classifyZone("src/a_test.rs", r)).toBe("test");
    expect(classifyZone("crates/db/tests/it.rs", r)).toBe("test");
    expect(classifyZone("src/a.rs", r)).toBe(ZONE_OTHER);
  });

  test("windows separators normalize before matching", () => {
    const r = rules([["ui", "apps/desktop/**"]]);
    expect(classifyZone("apps\\desktop\\src\\App.tsx", r)).toBe("ui");
  });
});

describe("zoneColor", () => {
  test("uses the rule's colour, else a stable palette entry", () => {
    const r = compileZoneRules([
      { zone: "store", match: ["crates/db/**"], color: "#ea580c" },
      { zone: "ui", match: ["apps/**"], color: null },
    ]);
    expect(zoneColor("store", r)).toBe("#ea580c");
    const ui = zoneColor("ui", r);
    expect(ui).toMatch(/^#[0-9a-f]{6}$/);
    expect(ui).not.toBe(zoneColor("store", r));
    // Unmatched files read as a neutral, never as a project zone.
    expect(zoneColor(ZONE_OTHER, r)).not.toBe(ui);
  });
});

// The Rust matcher (crates/oxplow-code-deps/src/zones.rs, globset) runs
// this same fixture. Any divergence in glob semantics between the two
// shows up here rather than as a zone that disagrees between the file
// tree and the backend's import edges.
describe("glob semantics parity with the Rust matcher", () => {
  test.each(fixture.cases.map((c) => [c.pattern, c.path, c.matches] as const))(
    "%s vs %s",
    (pattern, path, matches) => {
      const r = rules([["hit", pattern]]);
      expect(classifyZone(path, r) === "hit").toBe(matches);
    },
  );
});
