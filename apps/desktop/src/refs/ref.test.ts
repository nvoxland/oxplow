// The canonical ref grammar (.context/refs.md), asserted against the same
// golden fixture the Rust parser uses, so the two can't drift.
import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { formatRef, parseRef, type CanonicalRef, fileLinesRef } from "./ref.js";

interface Fixture {
  valid: Array<CanonicalRef & { text: string }>;
  invalid: Array<{ text: string; reason: string }>;
  format: Array<CanonicalRef & { text: string }>;
}

const here = dirname(fileURLToPath(import.meta.url));
const fixture: Fixture = JSON.parse(
  readFileSync(join(here, "../../../../crates/oxplow-domain/tests/fixtures/ref_grammar.json"), "utf8"),
);

describe("canonical ref grammar (shared fixture)", () => {
  test("valid refs parse to their components and round-trip", () => {
    for (const c of fixture.valid) {
      const r = parseRef(c.text);
      expect(r, c.text).toEqual({ kind: c.kind, id: c.id, rev: c.rev, frag: c.frag });
      expect(formatRef(r!), c.text).toBe(c.text);
    }
  });

  test("invalid refs are rejected", () => {
    for (const c of fixture.invalid) {
      expect(parseRef(c.text), `${JSON.stringify(c.text)} (${c.reason})`).toBeNull();
    }
  });

  test("formatting escapes only the reserved characters", () => {
    for (const c of fixture.format) {
      const r: CanonicalRef = { kind: c.kind, id: c.id, rev: c.rev, frag: c.frag };
      expect(formatRef(r)).toBe(c.text);
      expect(parseRef(c.text)).toEqual(r);
    }
  });
});

describe("fileLinesRef", () => {
  test("a file's lines as a ref: one line, or a range", () => {
    expect(fileLinesRef("src/a.rs", 10, 10)).toBe("file:src/a.rs#L10");
    expect(fileLinesRef("src/a.rs", 10, 20)).toBe("file:src/a.rs#L10-20");
    expect(fileLinesRef("src/a.rs", 3, 3, "git:abc123")).toBe("file:src/a.rs@git:abc123#L3");
  });
});
