/**
 * Scheme allowlist for external URLs that may be opened inside the app
 * (in a sandboxed external-url tab) or routed to the system browser.
 *
 * Pure — no IPC, no DOM. Shared by the renderer (link click → tab open)
 * and the main process (will-navigate / setWindowOpenHandler / context
 * menu "Open in browser") so both sides apply the same gate.
 *
 * The default policy: allow http: and https: only. Reject every other
 * scheme — file:, javascript:, data:, blob:, app:, custom protocol
 * handlers, etc. — so a malicious page can't trick the renderer into
 * loading attacker-controlled local resources, executing inline JS, or
 * exfiltrating via odd transports.
 */

export type ExternalUrlVerdict =
  | { ok: true; url: string }
  | { ok: false; reason: ExternalUrlRejectionReason };

export type ExternalUrlRejectionReason =
  | "empty"
  | "malformed"
  | "scheme-not-allowed"
  | "host-empty";

const ALLOWED_SCHEMES = new Set(["http:", "https:"]);

/**
 * Validate a URL string for external-tab rendering. Trims whitespace,
 * parses with the WHATWG URL parser, and checks the scheme + host.
 *
 * Returns the canonicalized URL string on success (parser-normalized
 * form, e.g. lowercased host, default ports stripped) or a structured
 * rejection reason on failure.
 */
export function classifyExternalUrl(input: string): ExternalUrlVerdict {
  const trimmed = (input ?? "").trim();
  if (!trimmed) return { ok: false, reason: "empty" };
  let parsed: URL;
  try {
    parsed = new URL(trimmed);
  } catch {
    return { ok: false, reason: "malformed" };
  }
  if (!ALLOWED_SCHEMES.has(parsed.protocol)) {
    return { ok: false, reason: "scheme-not-allowed" };
  }
  if (!parsed.host) return { ok: false, reason: "host-empty" };
  return { ok: true, url: parsed.toString() };
}

/** Convenience predicate for callers that only need a yes/no. */
export function isAllowedExternalUrl(input: string): boolean {
  return classifyExternalUrl(input).ok;
}

/**
 * Human-readable description of a rejection. Used by the toast/log
 * surfaced to the user when a link can't be opened.
 */
export function describeRejection(reason: ExternalUrlRejectionReason): string {
  switch (reason) {
    case "empty":
      return "Link is empty.";
    case "malformed":
      return "Link is not a valid URL.";
    case "scheme-not-allowed":
      return "Only http(s) links can be opened.";
    case "host-empty":
      return "Link has no host.";
  }
}
