# External URL tabs

What this doc covers: the security model for the in-app `external-url`
tab — when external links open inside oxplow instead of the OS browser,
how the embedded webview is sandboxed, and where each invariant is
enforced. Read this before changing anything that loosens the lockdown
(adding webPreferences, allowing more schemes, exposing IPC to the
guest, etc.).

## Why open external links in-app at all

Wiki notes and work items frequently reference public URLs (GitHub
issues, vendor docs, dashboards). Bouncing every click to the OS
browser breaks flow. The trade-off is that any embedded surface is a
new attack surface — a malicious page rendered inside the app could
try to talk to app-internal endpoints, exfiltrate cookies/auth, or
escape the sandbox into the host renderer. Defense in depth keeps the
trade safe.

## Security stance

| Layer | Where | What it enforces |
|---|---|---|
| Scheme allowlist (renderer) | `src/ui/external-url-allowlist.ts` | Only http(s) URLs get a tab. Everything else (file:, javascript:, data:, blob:, app:, custom protocols) returns a structured rejection that surfaces a refusal in `ExternalUrlPage` instead of attaching the webview. |
| Scheme allowlist (main) | `oxplow:openExternalUrl` IPC in `src/electron/main.ts` | Re-validates before calling `shell.openExternal`. The renderer can't smuggle a non-http(s) URL into the OS browser through this IPC. |
| Per-tag webPreferences | `src/ui/pages/ExternalUrlPage.tsx` | `<webview webpreferences="contextIsolation=yes,sandbox=yes,nodeIntegration=no">` plus `partition="persist:external"`. First line of defense. |
| `will-attach-webview` (main) | `src/electron/external-content-lockdown.ts` | Hard-strips `preload`, forces `sandbox=true`/`contextIsolation=true`/`webSecurity=true`/`allowRunningInsecureContent=false`/`experimentalFeatures=false`/`nodeIntegration*=false`, and pins `params.partition` to `persist:external` regardless of what the JSX requested. Non-http(s) `params.src` is rewritten to `about:blank` at the boundary. |
| Guest hardening | `applyGuestContentsHardening` (same file) | On `did-attach-webview`: `setWindowOpenHandler` denies popups (http(s) intents fall through to `shell.openExternal`); `will-navigate` and `will-redirect` block non-http(s); permission requests + checks deny by default; `devtools-opened` is auto-closed in packaged builds. |
| Host renderer pin | `web-contents-created` `will-navigate` listener | The host frame can't be navigated away from `file://` (bundled index.html) or localhost. Anything else is blocked, with http(s) routed to the OS browser instead of taking over the app. |
| Session policy | `configureExternalSession()` | On `persist:external`: strips Authorization / Cookie / Proxy-Authorization on outbound requests (`sanitizeOutboundHeaders`), pins `Referrer-Policy: strict-origin-when-cross-origin`, blocks any request aimed at app-internal origins (`isInternalUrl`), and injects `Content-Security-Policy: frame-ancestors 'none';` when the upstream sent no CSP. |

The intent is **defense in depth**: the renderer-side attribute set is
the first gate, but the main-process `will-attach-webview` hook is the
authoritative gate — even if a future renderer change accidentally
loosens the JSX attributes, the main process re-applies the policy
before the guest webContents starts.

## Modules

| File | Purpose |
|---|---|
| `src/ui/external-url-allowlist.ts` | Pure: `classifyExternalUrl(url)` → `{ ok, url } \| { ok: false, reason }`, `isAllowedExternalUrl`, `describeRejection`. Default policy: http(s) only. Tested in `external-url-allowlist.test.ts`. |
| `src/ui/tabs/pageRefs.ts` | `externalUrlRef(url)` — must be called only after passing through the allowlist. |
| `src/ui/pages/ExternalUrlPage.tsx` | The `<webview>`-rendering page. Loading / error states, "Open in browser" header action, page-title-updated wiring. |
| `src/electron/external-content-policy.ts` | Pure: `isInternalUrl`, `sanitizeOutboundHeaders`, `withInjectedCsp`, `EXTERNAL_PARTITION`. Tested in `external-content-policy.test.ts`. |
| `src/electron/external-content-lockdown.ts` | Wires the pure policy into Electron: `web-contents-created` / `will-attach-webview` / `did-attach-webview` / `webRequest` hooks on the external partition. Registered once from `main.ts` after `app.whenReady()`. |
| `src/electron/main.ts` (`oxplow:openExternalUrl`) | IPC for routing http(s) URLs to `shell.openExternal`, allowlist-gated. |

## Adding a new scheme to the allowlist

Don't, unless you've thought about every layer in the table above. The
allowlist is intentionally narrow. If a feature needs a new scheme:

1. Update `ALLOWED_SCHEMES` in `src/ui/external-url-allowlist.ts` and
   add tests covering the new scheme + a representative reject case.
2. Decide whether the new scheme should also be allowed by the host's
   `will-navigate` host-frame guard in `external-content-lockdown.ts` —
   in most cases the answer is no.
3. Audit `INTERNAL_HOST_PATTERNS` to see whether the new scheme could
   slip past the internal-origin block.
4. Update this doc.

## Adding a new origin block

The internal-origin blocklist (`INTERNAL_HOST_PATTERNS` in
`external-content-policy.ts`) is over-specified on purpose: easier to
add a new pattern than to debug a leak through an over-broad rule. If
the renderer starts binding a new dev port or custom scheme, add it to
the list and to the test file. Don't expand to wildcards (`*.local`,
`172.*.*.*`) — they'll bite legitimate external content.
