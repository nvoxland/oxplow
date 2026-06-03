import { describe, expect, test } from "bun:test";
import { normalizeSnapshotId } from "./effort-snapshot.js";

describe("normalizeSnapshotId", () => {
  test("coerces a numeric snapshot id to a string", () => {
    // The Tauri binding types effort snapshot ids as numbers; the app
    // (and Rust's TreeVersion.id) want strings. A number reaches Rust
    // as {kind:"snapshot", id: <number>} and serde rejects it → opaque
    // "ipc error" when opening an effort diff.
    expect(normalizeSnapshotId(1500)).toBe("1500");
  });

  test("passes a string id through unchanged", () => {
    expect(normalizeSnapshotId("1500")).toBe("1500");
  });

  test("maps null/undefined to null (→ DISK at the call site)", () => {
    expect(normalizeSnapshotId(null)).toBeNull();
    expect(normalizeSnapshotId(undefined)).toBeNull();
  });
});
