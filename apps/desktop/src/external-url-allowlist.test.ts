import { expect, test } from "bun:test";
import { classifyExternalUrl, describeRejection, isAllowedExternalUrl } from "./external-url-allowlist.js";

// The allowlist is the single security gate for opening external URLs in
// the in-app external-url tab. Anything that slips through here lands in
// the sandboxed renderer; anything rejected falls back to a refusal so
// the user sees why the link didn't open. Every reject branch is
// exercised so an accidental loosening of the policy fails a test.

test("allows plain https URL", () => {
  const v = classifyExternalUrl("https://example.com/path");
  expect(v.ok).toBe(true);
  if (v.ok) expect(v.url).toBe("https://example.com/path");
});

test("allows http URL", () => {
  const v = classifyExternalUrl("http://example.com");
  expect(v.ok).toBe(true);
});

test("trims surrounding whitespace before parsing", () => {
  const v = classifyExternalUrl("   https://example.com  ");
  expect(v.ok).toBe(true);
});

test("rejects empty input", () => {
  expect(classifyExternalUrl("")).toEqual({ ok: false, reason: "empty" });
  expect(classifyExternalUrl("   ")).toEqual({ ok: false, reason: "empty" });
});

test("rejects malformed input", () => {
  expect(classifyExternalUrl("not a url").ok).toBe(false);
  // The WHATWG parser is lenient — `nope` parses as a relative reference
  // failure but `nope:` is technically a scheme. We still need to verify
  // it gets rejected for a different reason (host-empty / scheme).
  const v = classifyExternalUrl("nope:");
  expect(v.ok).toBe(false);
});

test("rejects file: scheme", () => {
  expect(classifyExternalUrl("file:///etc/passwd")).toEqual({ ok: false, reason: "scheme-not-allowed" });
});

test("rejects javascript: scheme", () => {
  expect(classifyExternalUrl("javascript:alert(1)")).toEqual({ ok: false, reason: "scheme-not-allowed" });
});

test("rejects data: scheme", () => {
  expect(classifyExternalUrl("data:text/html,<script>alert(1)</script>")).toEqual({ ok: false, reason: "scheme-not-allowed" });
});

test("rejects blob: scheme", () => {
  expect(classifyExternalUrl("blob:https://example.com/abc")).toEqual({ ok: false, reason: "scheme-not-allowed" });
});

test("rejects custom app: scheme", () => {
  expect(classifyExternalUrl("app://oxplow/internal")).toEqual({ ok: false, reason: "scheme-not-allowed" });
});

test("rejects http URL with empty host", () => {
  expect(classifyExternalUrl("http://").ok).toBe(false);
});

test("scheme check is case-insensitive (parser canonicalizes to lowercase)", () => {
  expect(classifyExternalUrl("HTTPS://EXAMPLE.COM").ok).toBe(true);
});

test("isAllowedExternalUrl mirrors classifyExternalUrl.ok", () => {
  expect(isAllowedExternalUrl("https://example.com")).toBe(true);
  expect(isAllowedExternalUrl("javascript:1")).toBe(false);
});

test("describeRejection produces a non-empty string for each reason", () => {
  for (const reason of ["empty", "malformed", "scheme-not-allowed", "host-empty"] as const) {
    expect(describeRejection(reason).length).toBeGreaterThan(0);
  }
});
