import { describe, expect, test } from "bun:test";
import { availableVariants, selectVariantBody, type ProseVariants } from "./selectVariant.js";

const FULL: ProseVariants = { developer: "dev", executive: "exec", terse: "ugh" };

describe("selectVariantBody", () => {
  test("returns the requested variant when present", () => {
    expect(selectVariantBody(FULL, "developer")).toBe("dev");
    expect(selectVariantBody(FULL, "executive")).toBe("exec");
    expect(selectVariantBody(FULL, "terse")).toBe("ugh");
  });

  test("falls back to developer when a variant is null/undefined/empty", () => {
    expect(selectVariantBody({ developer: "dev", executive: null }, "executive")).toBe("dev");
    expect(selectVariantBody({ developer: "dev" }, "terse")).toBe("dev");
    expect(selectVariantBody({ developer: "dev", terse: "" }, "terse")).toBe("dev");
  });
});

describe("availableVariants", () => {
  test("developer always available; others reflect presence", () => {
    expect(availableVariants(FULL)).toEqual({ developer: true, executive: true, terse: true });
    expect(availableVariants({ developer: "dev" })).toEqual({
      developer: true,
      executive: false,
      terse: false,
    });
    expect(availableVariants({ developer: "dev", executive: "", terse: null })).toEqual({
      developer: true,
      executive: false,
      terse: false,
    });
  });
});
