/**
 * Pure policy helpers for the external-content sandbox. Kept Electron-free
 * so they can be unit-tested in bun without spinning up a packaged
 * Electron runtime — the lockdown wiring in
 * external-content-lockdown.ts imports these and feeds them to
 * webRequest hooks.
 */

export const EXTERNAL_PARTITION = "persist:external";

/**
 * Patterns that match app-internal origins. Requests originating from
 * the external partition that target one of these are blocked, so a
 * page rendered in an external-url tab can't probe (or fetch from) the
 * app's own dev server, file://, or custom schemes.
 *
 * Kept conservative: only common dev-loopback ports the renderer is
 * known to bind, plus file:// and recognised custom schemes. Adding to
 * the list is cheap; over-broadening it (blocking *.local, etc.) would
 * break legitimate external content.
 */
export const INTERNAL_HOST_PATTERNS: ReadonlyArray<RegExp> = [
  /^https?:\/\/localhost(?::\d+)?(?:\/|$)/i,
  /^https?:\/\/127\.0\.0\.1(?::\d+)?(?:\/|$)/i,
  /^https?:\/\/\[::1\](?::\d+)?(?:\/|$)/i,
  /^file:\/\//i,
  /^app:\/\//i,
  /^chrome:\/\//i,
  /^devtools:\/\//i,
];

export function isInternalUrl(url: string): boolean {
  return INTERNAL_HOST_PATTERNS.some((re) => re.test(url));
}

/**
 * Header sanitization rule for outbound requests on the external
 * partition. Strips any auth headers leaking from the app session and
 * pins a strict-ish referrer policy.
 */
export function sanitizeOutboundHeaders(input: Record<string, string | string[]>): Record<string, string | string[]> {
  const out: Record<string, string | string[]> = {};
  for (const [k, v] of Object.entries(input)) {
    const lower = k.toLowerCase();
    if (lower === "authorization") continue;
    if (lower === "cookie") continue;
    if (lower === "proxy-authorization") continue;
    out[k] = v;
  }
  out["Referrer-Policy"] = "strict-origin-when-cross-origin";
  return out;
}

/**
 * Compute response-header overrides. We layer a minimal CSP on top of
 * whatever the upstream sent so a missing site CSP doesn't leave the
 * door open for the page to be reframed by host content. We only inject
 * when the upstream sent no CSP of its own — sites that ship a real
 * policy keep it.
 */
export function withInjectedCsp(
  responseHeaders: Record<string, string | string[]> | undefined,
): Record<string, string | string[]> {
  const headers: Record<string, string | string[]> = { ...(responseHeaders ?? {}) };
  const hasCsp = Object.keys(headers).some((k) => k.toLowerCase() === "content-security-policy");
  if (!hasCsp) {
    headers["Content-Security-Policy"] = ["frame-ancestors 'none';"];
  }
  return headers;
}
