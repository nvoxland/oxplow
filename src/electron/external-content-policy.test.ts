import { expect, test } from "bun:test";
import { isInternalUrl, sanitizeOutboundHeaders, withInjectedCsp } from "./external-content-policy.js";

// The internal-url predicate is the gate that prevents an external page
// (loaded in the persist:external partition) from probing or fetching
// the app's own loopback dev server, file:// resources, or custom
// schemes. Tested as pure helpers because the request hooks themselves
// are integration-shaped — but the policy decisions ARE these
// functions, so covering them directly keeps regressions visible.

test("isInternalUrl: blocks http localhost", () => {
  expect(isInternalUrl("http://localhost/")).toBe(true);
  expect(isInternalUrl("http://localhost:3000/api")).toBe(true);
  expect(isInternalUrl("https://localhost:8443/oauth")).toBe(true);
});

test("isInternalUrl: blocks 127.0.0.1 and ::1", () => {
  expect(isInternalUrl("http://127.0.0.1/")).toBe(true);
  expect(isInternalUrl("http://127.0.0.1:5173/x")).toBe(true);
  expect(isInternalUrl("http://[::1]:9000/")).toBe(true);
});

test("isInternalUrl: blocks file:, app:, chrome:, devtools:", () => {
  expect(isInternalUrl("file:///etc/passwd")).toBe(true);
  expect(isInternalUrl("app://oxplow/api")).toBe(true);
  expect(isInternalUrl("chrome://settings")).toBe(true);
  expect(isInternalUrl("devtools://devtools/bundled/inspector.html")).toBe(true);
});

test("isInternalUrl: allows ordinary public web URLs", () => {
  expect(isInternalUrl("https://example.com")).toBe(false);
  expect(isInternalUrl("https://github.com/anthropics/claude-code")).toBe(false);
  expect(isInternalUrl("http://news.ycombinator.com")).toBe(false);
});

test("isInternalUrl: does not match hostnames containing localhost as a substring", () => {
  expect(isInternalUrl("https://localhost.example.com/")).toBe(false);
  expect(isInternalUrl("https://example.com/localhost/foo")).toBe(false);
});

test("sanitizeOutboundHeaders: strips Authorization, Cookie, Proxy-Authorization (any case)", () => {
  const out = sanitizeOutboundHeaders({
    "Authorization": "Bearer abc",
    "authorization": "Basic def",
    "Cookie": "session=1",
    "cookie": "session=2",
    "Proxy-Authorization": "x",
    "User-Agent": "Mozilla/5.0",
  });
  expect(out.Authorization).toBeUndefined();
  expect(out.authorization).toBeUndefined();
  expect(out.Cookie).toBeUndefined();
  expect(out.cookie).toBeUndefined();
  expect(out["Proxy-Authorization"]).toBeUndefined();
  expect(out["User-Agent"]).toBe("Mozilla/5.0");
});

test("sanitizeOutboundHeaders: pins Referrer-Policy", () => {
  const out = sanitizeOutboundHeaders({});
  expect(out["Referrer-Policy"]).toBe("strict-origin-when-cross-origin");
});

test("withInjectedCsp: injects frame-ancestors policy when absent", () => {
  const out = withInjectedCsp({ "Content-Type": "text/html" });
  expect(out["Content-Security-Policy"]).toEqual(["frame-ancestors 'none';"]);
});

test("withInjectedCsp: leaves existing CSP intact (any header casing)", () => {
  const out = withInjectedCsp({
    "content-security-policy": ["default-src 'self'"],
  });
  expect(out["content-security-policy"]).toEqual(["default-src 'self'"]);
  expect(out["Content-Security-Policy"]).toBeUndefined();
});

test("withInjectedCsp: handles undefined input", () => {
  const out = withInjectedCsp(undefined);
  expect(out["Content-Security-Policy"]).toEqual(["frame-ancestors 'none';"]);
});
