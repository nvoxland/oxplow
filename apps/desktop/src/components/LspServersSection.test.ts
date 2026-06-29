import { describe, expect, test } from "bun:test";

import type { LspServerListing } from "../api.js";
import { availableSuggestions, describeServerRow } from "./LspServersSection.js";

function listing(overrides: Partial<LspServerListing>): LspServerListing {
  return {
    languageId: "rust",
    extensions: [],
    command: "/x/rust-analyzer",
    args: [],
    source: "installed",
    packageName: "rust-analyzer",
    version: "2026-01-01",
    binaryExists: true,
    runningStreams: [],
    completionTriggerCharacters: null,
    ...overrides,
  };
}

describe("describeServerRow", () => {
  test("installed server shows version badge and Remove action", () => {
    const row = describeServerRow(listing({}));
    expect(row.badges).toEqual(["installed 2026-01-01"]);
    expect(row.canRemove).toBe(true);
    expect(row.canRestart).toBe(false);
    expect(row.binaryMissing).toBe(false);
  });

  test("yaml server is read-only and badged with its source", () => {
    const row = describeServerRow(
      listing({ source: "yaml", packageName: null, version: null }),
    );
    expect(row.badges).toEqual(["project.yaml"]);
    expect(row.canRemove).toBe(false);
  });

  test("running streams produce a running badge and Restart action", () => {
    expect(describeServerRow(listing({ runningStreams: ["s-1"] })).badges).toContain("running");
    const multi = describeServerRow(listing({ runningStreams: ["s-1", "s-2"] }));
    expect(multi.badges).toContain("running ×2");
    expect(multi.canRestart).toBe(true);
  });

  test("missing binary is flagged", () => {
    expect(describeServerRow(listing({ binaryExists: false })).binaryMissing).toBe(true);
  });
});

describe("availableSuggestions", () => {
  test("excludes languages that already have a server and dedupes packages", () => {
    const out = availableSuggestions([listing({ languageId: "rust" })]);
    expect(out.find((s) => s.pkg === "rust-analyzer")).toBeUndefined();
    // typescript + javascript share one package — only one chip.
    expect(out.filter((s) => s.pkg === "typescript-language-server")).toHaveLength(1);
  });

  test("empty server list offers every curated package once", () => {
    const out = availableSuggestions([]);
    const pkgs = out.map((s) => s.pkg);
    expect(new Set(pkgs).size).toBe(pkgs.length);
    expect(pkgs).toContain("gopls");
  });
});
