// The person's own browser, behind the transport facade.
//
// Some pages belong in the system browser rather than oxplow's sandboxed
// external-URL window: signing in to a service (RFC 8252 — the person's
// sessions and password manager live there, and services refuse embedded
// webviews). UI code reaches it through here rather than importing
// `@tauri-apps/plugin-shell` directly (see no-tauri-imports.test.ts). The
// shell plugin's `open` is granted to oxplow's own windows as
// `shell:allow-open`, http(s) only. In a plain-browser session (no Tauri
// host) it is a new tab.

import { open as shellOpen } from "@tauri-apps/plugin-shell";
import { shellAvailable } from "./transport";

/// Open an http(s) `url` in the person's default browser.
export async function openInSystemBrowser(url: string): Promise<void> {
  if (!/^https?:\/\//.test(url)) throw new Error(`not an http(s) URL: ${url}`);
  if (shellAvailable()) {
    await shellOpen(url);
    return;
  }
  window.open(url, "_blank", "noopener,noreferrer");
}
