import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";

import { fileLinesUri, formatUri, identityOf, isExternal, paramOf, parseUri, resourceUri, typeOf, uriProblem } from "./uri.js";

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
      const u = parseUri(c.text);
      expect(u, c.text).not.toBeNull();
      expect(u!.resource, c.text).toEqual(c.resource);
      expect(u!.params, c.text).toEqual(c.params);
      expect(u!.view, c.text).toEqual(c.view);
      expect(formatUri(u!), c.text).toBe(c.canonical);
      expect(identityOf(c.text), c.text).toBe(c.identity);
      // The canonical form is a fixed point.
      expect(formatUri(parseUri(c.canonical)!), c.text).toBe(c.canonical);
    }
  });

  test("every invalid case is refused for its reason", () => {
    for (const c of fixture.invalid) {
      expect(parseUri(c.text), c.text).toBeNull();
      expect(uriProblem(c.text), c.text).toBe(c.reason);
    }
  });

  test("a built URI is canonical", () => {
    for (const c of fixture.build) {
      expect(resourceUri(c.resource, c.params, c.view)).toBe(c.canonical);
    }
  });
});

test("its readers name the resource and a param", () => {
  const u = resourceUri("oxplow/work_item", { provider: "oxplow", id: "tsk42" });
  expect(typeOf(u)).toBe("oxplow/work_item");
  expect(paramOf(u, "id")).toBe("tsk42");
  expect(paramOf(u, "gone")).toBeNull();
  expect(typeOf("oxplow:oxplow/effort?id=eff1")).toBe("oxplow/effort");
  expect(typeOf("effort:eff1")).toBeNull();
});

// A link may name any resource; oxplow's are the oxplow: ones.
test("an external URI is a URL of another scheme", () => {
  expect(isExternal("https://example.com/a")).toBe(true);
  expect(isExternal("mailto:a@example.com")).toBe(true);
  expect(isExternal("oxplow:oxplow/effort?id=eff1")).toBe(false);
  expect(isExternal("oxplow:effort")).toBe(false);
  expect(isExternal("not a uri")).toBe(false);
});

test("a file's lines are the view, never the identity", () => {
  expect(fileLinesUri("src/a.rs", 10, 20)).toBe("oxplow:oxplow/file?path=src/a.rs#lines=10-20");
  expect(fileLinesUri("src/a.rs", 3, 3, "git:abc123")).toBe("oxplow:oxplow/file?path=src/a.rs&rev=git:abc123#lines=3-3");
  expect(identityOf(fileLinesUri("src/a.rs", 10, 20))).toBe("oxplow:oxplow/file?path=src/a.rs");
});
