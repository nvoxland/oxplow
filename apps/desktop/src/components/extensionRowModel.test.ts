import { describe, expect, test } from "bun:test";
import type { Extension, SourceListing } from "../tauri-bridge/generated/bindings.js";
import { extensionRowModel, sourceRowModel } from "./extensionRowModel.js";

const ext = (over: Partial<Extension> = {}): Extension => ({
  name: "review",
  description: "Review lenses",
  path: "oxplow/extensions/review",
  errors: [],
  lenses: [],
  source: null,
  sources: [],
  ...over,
});

describe("extensionRowModel", () => {
  test("a local extension can't be updated and says it lives in the repo", () => {
    const m = extensionRowModel(ext());
    expect(m.canUpdate).toBe(false);
    expect(m.origin).toBe("In this repo");
    expect(m.lensCount).toBe(0);
    expect(m.healthy).toBe(true);
  });

  test("an installed extension shows its url, ref and short sha and can update", () => {
    const m = extensionRowModel(
      ext({ source: { git: "https://github.com/acme/lenses", gitRef: "v2", sha: "0123456789abcdef0123456789abcdef01234567" } }),
    );
    expect(m.canUpdate).toBe(true);
    expect(m.origin).toBe("https://github.com/acme/lenses @ v2 (0123456)");
  });

  test("errors mark the row unhealthy", () => {
    expect(extensionRowModel(ext({ errors: ["bad yaml"] })).healthy).toBe(false);
  });
});

describe("sourceRowModel", () => {
  const listing = (over: Partial<SourceListing> = {}): SourceListing => ({
    extension: "my-gh",
    spec: { id: "gh", doc: "", entry: "bin/sync.sh", schedule: { kind: "every", minutes: 10 }, env: ["GITHUB_TOKEN"], entities: [] },
    state: null,
    approved: false,
    ...over,
  });

  test("an unapproved source asks for approval and says what it will run", () => {
    const m = sourceRowModel(listing());
    expect(m.action).toBe("approve");
    expect(m.actionLabel).toBe("Approve & Run");
    expect(m.actionTitle).toContain("bin/sync.sh");
    expect(m.actionTitle).toContain("GITHUB_TOKEN");
    expect(m.status).toBe("Never run");
    expect(m.schedule).toBe("every 10m");
  });

  test("an approved source syncs and summarizes its last run", () => {
    const m = sourceRowModel(
      listing({
        approved: true,
        state: { extension: "my-gh", sourceId: "gh", status: "ok", lastRunAt: "2026-09-27T01:00:00Z", error: null, rowCounts: { pr: 12, review: 3 } },
      }),
    );
    expect(m.action).toBe("sync");
    expect(m.actionLabel).toBe("Sync Now");
    expect(m.status).toBe("12 pr · 3 review");
    expect(m.error).toBeNull();
  });

  test("a failed run surfaces the error", () => {
    const m = sourceRowModel(
      listing({
        approved: true,
        state: { extension: "my-gh", sourceId: "gh", status: "error", lastRunAt: "2026-09-27T01:00:00Z", error: "boom", rowCounts: {} },
      }),
    );
    expect(m.status).toBe("Failed");
    expect(m.error).toBe("boom");
    expect(sourceRowModel(listing({ spec: { ...listing().spec, schedule: { kind: "manual" } } })).schedule).toBe("manual");
  });
});
