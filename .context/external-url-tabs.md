# External URL tabs

What this doc covers: the security model for the in-app `external-url`
tab — when external links open inside oxplow instead of the OS browser,
how the spawned webview window is sandboxed, and where each invariant
is enforced. Read this before changing anything that loosens the
lockdown (adding new schemes, broadening the capability, exposing
extra plugins, etc.).

## Why open external links in-app at all

Wiki pages and tasks frequently reference public URLs (GitHub
issues, vendor docs, dashboards). Bouncing every click to the OS
browser breaks flow. The trade-off is that any embedded surface is a
new attack surface — a malicious page rendered inside the app could
try to talk to app-internal endpoints, exfiltrate cookies/auth, or
escape the sandbox into the host renderer. Defense in depth keeps the
trade safe.

## How it works under Tauri 2

Tauri 2 doesn't render an Electron-style `<webview>` tag inside the
host renderer. Each external URL opens as its own
`tauri::WebviewWindowBuilder` window that runs **outside** the
oxplow webview entirely. The new window inherits the `external-url`
capability (defined in
`apps/desktop/src-tauri/capabilities/external-url.json`), which
explicitly grants **zero oxplow commands and zero plugin permissions**
— it behaves like a sandboxed browser tab.

`apps/desktop/src/pages/ExternalUrlPage.tsx` is no longer a webview
host. It calls `desktopBridge().openExternalUrl(url)` (which dispatches
to `commands::webview::open_external_url` in the Rust shell), then
renders a small status panel with a "re-open" button. The actual page
content lives in the spawned window, isolated from the main webview.

## Security stance

| Layer | Where | What it enforces |
|---|---|---|
| Scheme allowlist (renderer) | `apps/desktop/src/external-url-allowlist.ts` | Only http(s) URLs reach the bridge call. Anything else (file:, javascript:, data:, blob:, custom protocols) returns a structured rejection that surfaces a refusal in `ExternalUrlPage` instead of opening a window. |
| Scheme allowlist (Rust) | `crates/oxplow-tauri-ipc/src/commands/webview.rs` (`open_external_url`) | Re-validates `http://` / `https://` prefix before constructing the `WebviewWindowBuilder`. The renderer can't smuggle a non-http(s) URL through the IPC. |
| Capability scope | `apps/desktop/src-tauri/capabilities/external-url.json` | `permissions: []` — no `core:default`, no plugin defaults, no oxplow commands. The window glob `ext-url-*` matches the label format `open_external_url` assigns. |
| Capability listing | `apps/desktop/src-tauri/tauri.conf.json` `app.security.capabilities` | Capabilities are listed explicitly so a stray file in `capabilities/` cannot widen the surface — the directory's auto-enable behavior is bypassed. |
| Window labelling | `format!("ext-url-{uuid}")` | The label namespace is fixed; the capability glob (`ext-url-*`) only matches windows the IPC command itself created. |
| OS-browser fallback | `tauri-plugin-shell` capability `shell:allow-open` | Only the URL-open intent is granted; arbitrary `shell:execute` is restricted to the tmux/git/typescript-language-server allowlist in `oxplow-windows.json`. |
| Window-event handling | `apps/desktop/src-tauri/src/main.rs` (`is_project_label`) | Window events are builder-level and fire for every window. Session bookkeeping is gated on the `project-<n>` label so closing an external-URL window isn't mistaken for closing the project. The menu registry has no snapshot for these labels, so focusing one leaves the menu bar as-is. |

The intent is **isolation by separation**: the external page never
shares a webview process or capability set with the oxplow renderer,
so even a full sandbox escape inside the external window cannot reach
oxplow's IPC surface.

## Modules

| File | Purpose |
|---|---|
| `apps/desktop/src/external-url-allowlist.ts` | Pure: `classifyExternalUrl(url)` → `{ ok, url } \| { ok: false, reason }`, `isAllowedExternalUrl`, `describeRejection`. Default policy: http(s) only. Tested in `external-url-allowlist.test.ts`. |
| `apps/desktop/src/tabs/pageRefs.ts` | `externalUrlRef(url)` — must be called only after passing through the allowlist. |
| `apps/desktop/src/pages/ExternalUrlPage.tsx` | Status / re-open panel; auto-invokes `desktopBridge().openExternalUrl(url)` on mount. No `<webview>` element. |
| `crates/oxplow-tauri-ipc/src/commands/webview.rs` | `open_external_url(url)` Tauri command. Validates scheme, generates `ext-url-<uuid>` label, builds the new `WebviewWindow`. |
| `apps/desktop/src-tauri/capabilities/external-url.json` | Empty-permission capability; matches `ext-url-*` window labels. |

## Adding a new scheme to the allowlist

Don't, unless you've thought about every layer in the table above. The
allowlist is intentionally narrow. If a feature needs a new scheme:

1. Update `ALLOWED_SCHEMES` in `apps/desktop/src/external-url-allowlist.ts` and
   add tests covering the new scheme + a representative reject case.
2. Update the same scheme check in `commands/webview.rs::open_external_url`.
3. Update this doc.

## Loosening the capability

The `external-url` capability is the load-bearing isolation layer. If
a feature truly needs to expose something to external pages, prefer
adding a *new* capability with a different window-label namespace
(e.g. `trusted-embed-*`) rather than editing the existing one. Keep
the empty-permission `ext-url-*` capability around so anonymous
external links never gain new privileges.
