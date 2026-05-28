import { describe, expect, test } from "bun:test";
import { extractHeadingSlugs, sectionAnchorForQuote, sectionSlug } from "./sectionSlug.js";

describe("sectionSlug — parity with Rust heading_slug", () => {
  // Same fixtures as crates/oxplow-domain/src/prose.rs heading_slug test.
  test("matches the Rust fixtures", () => {
    expect(sectionSlug("Storage model")).toBe("storage-model");
    expect(sectionSlug("## Why we did it")).toBe("why-we-did-it");
    expect(sectionSlug("Per-page (per-tab) selector!")).toBe("per-page-per-tab-selector");
    expect(sectionSlug("  Trailing  ")).toBe("trailing");
    expect(sectionSlug("CRATES & tools")).toBe("crates-tools");
  });
});

describe("extractHeadingSlugs", () => {
  test("collects ATX headings in document order", () => {
    const md = "# Title\n\nintro\n\n## Storage model\n\nbody\n\n### Why\n\n";
    expect(extractHeadingSlugs(md)).toEqual(["title", "storage-model", "why"]);
  });
  test("tolerates trailing # and whitespace", () => {
    expect(extractHeadingSlugs("## Storage model ##  ")).toEqual(["storage-model"]);
  });
  test("ignores non-heading lines", () => {
    expect(extractHeadingSlugs("no headings here\njust prose")).toEqual([]);
  });
});

describe("sectionAnchorForQuote", () => {
  const body = "# Title\n\nintro\n\n## Storage\n\nthe canonical text here\n\n## Other\n\ntail";
  test("returns the nearest preceding heading slug", () => {
    expect(sectionAnchorForQuote(body, "the canonical text")).toBe("storage");
  });
  test("returns null for an empty or missing quote", () => {
    expect(sectionAnchorForQuote(body, "")).toBeNull();
    expect(sectionAnchorForQuote(body, "not present")).toBeNull();
  });
  test("returns null when the quote precedes every heading", () => {
    expect(sectionAnchorForQuote("intro text\n\n# First", "intro text")).toBeNull();
  });
});
