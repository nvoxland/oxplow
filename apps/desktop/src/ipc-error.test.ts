import { describe, expect, test } from "bun:test";
import { ipcErrorMessage } from "./ipc-error.js";

describe("ipcErrorMessage", () => {
  test("surfaces a plain string error verbatim (arg-deser / panic)", () => {
    // The bug: this used to collapse to "ipc error".
    expect(ipcErrorMessage("invalid type: integer, expected a string")).toBe(
      "invalid type: integer, expected a string",
    );
  });

  test("prefers message on an IpcError object", () => {
    expect(ipcErrorMessage({ message: "boom", code: "E1" })).toBe("boom");
  });

  test("falls back to code when message is missing/blank", () => {
    expect(ipcErrorMessage({ code: "E_NOPE" })).toBe("E_NOPE");
    expect(ipcErrorMessage({ message: "   ", code: "E_NOPE" })).toBe("E_NOPE");
  });

  test("JSON-dumps an unrecognized object rather than hiding it", () => {
    expect(ipcErrorMessage({ reason: "weird" })).toBe('{"reason":"weird"}');
  });

  test("falls back to the generic literal for empty/unknown payloads", () => {
    expect(ipcErrorMessage(null)).toBe("ipc error");
    expect(ipcErrorMessage(undefined)).toBe("ipc error");
    expect(ipcErrorMessage("")).toBe("ipc error");
    expect(ipcErrorMessage("   ")).toBe("ipc error");
    expect(ipcErrorMessage({})).toBe("ipc error");
  });
});
