import { describe, expect, test } from "bun:test";
import type { Extension } from "../tauri-bridge/generated/bindings.js";
import { extensionRowModel } from "./extensionRowModel.js";

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
