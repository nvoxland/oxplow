import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";

import { formatUri, identityOf, isExternal, oxplowUri, paramOf, parseOxplowUri, resourceOf, uriProblem } from "./oxplowUri.js";

// oxplow's URI grammar against the golden fixture the Rust parser asserts
// too (crates/oxplow-domain/tests/oxplow_uri.rs), so the two can't drift.
const fixture = JSON.parse(
  readFileSync(join(import.meta.dir, "../../../../crates/oxplow-domain/tests/fixtures/uri.json"), "utf8"),
) as {
  valid: Array<{ text: string; resource: { extension: string; name: string }; params: Record<string, string>; view: Record<string, string>; canonical: string; identity: string }>;
  invalid: Array<{ text: string; reason: string }>;
  build: Array<{ resource: string; params: Record<string, string>; view: Record<string, string>; canonical: string }>;
};

describe("the fixture", () => {
  test("every valid case parses to its parts and its canonical form", () => {
    for (const c of fixture.valid) {
      const u = parseOxplowUri(c.text);
      expect(u, c.text).not.toBeNull();
      expect(u!.resource, c.text).toEqual(c.resource);
      expect(u!.params, c.text).toEqual(c.params);
      expect(u!.view, c.text).toEqual(c.view);
      expect(formatUri(u!), c.text).toBe(c.canonical);
      expect(identityOf(c.text), c.text).toBe(c.identity);
      // The canonical form is a fixed point.
      expect(formatUri(parseOxplowUri(c.canonical)!), c.text).toBe(c.canonical);
    }
  });

  test("every invalid case is refused for its reason", () => {
    for (const c of fixture.invalid) {
      expect(parseOxplowUri(c.text), c.text).toBeNull();
      expect(uriProblem(c.text), c.text).toBe(c.reason);
    }
  });

  test("a built URI is canonical", () => {
    for (const c of fixture.build) {
      expect(oxplowUri(c.resource, c.params, c.view)).toBe(c.canonical);
    }
  });
});

test("its readers name the resource and a param", () => {
  const u = oxplowUri("oxplow/work_item", { provider: "oxplow", id: "tsk42" });
  expect(resourceOf(u)).toBe("oxplow/work_item");
  expect(paramOf(u, "id")).toBe("tsk42");
  expect(paramOf(u, "gone")).toBeNull();
  expect(resourceOf("effort:eff1")).toBeNull();
});

// A link may name any resource; oxplow's are the oxplow: ones.
test("an external URI is a URL of another scheme", () => {
  expect(isExternal("https://example.com/a")).toBe(true);
  expect(isExternal("mailto:a@example.com")).toBe(true);
  expect(isExternal("oxplow:oxplow/effort?id=eff1")).toBe(false);
  expect(isExternal("oxplow:effort")).toBe(false);
  expect(isExternal("not a uri")).toBe(false);
});
