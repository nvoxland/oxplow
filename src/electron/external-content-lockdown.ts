import { app, session, shell, type WebContents } from "electron";
import { classifyExternalUrl } from "../ui/external-url-allowlist.js";

/**
 * Centralized hardening for any web content other than the host renderer.
 * Reasoning lives in `.context/agent-model.md` (the "External URL tabs"
 * subsystem doc); this file is the implementation.
 *
 * Why redundant with the per-tag webpreferences in ExternalUrlPage.tsx?
 * Defense in depth. The renderer's <webview> attributes are the first
 * line; this module is the main-process gate that catches anything the
 * renderer forgets or that a future change accidentally loosens. It runs
 * once at app startup.
 */

export const EXTERNAL_PARTITION = "persist:external";

export function registerExternalContentLockdown(): void {
  // Enforce hardened webPreferences on every <webview> guest at attach
  // time. Even if a renderer-side bug or a malicious component tried to
  // mount a <webview> with `nodeintegration` or no sandbox, this strips
  // those attributes before the guest webContents is created.
  app.on("web-contents-created", (_event, contents) => {
    contents.on("will-attach-webview", (_evt, webPreferences, params) => {
      // Hard-strip dangerous flags regardless of what the JSX requested.
      // (Mutating the passed-in object is the documented way to override.)
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      const prefs = webPreferences as any;
      delete prefs.preload;
      delete prefs.preloadURL;
      prefs.nodeIntegration = false;
      prefs.nodeIntegrationInWorker = false;
      prefs.nodeIntegrationInSubFrames = false;
      prefs.contextIsolation = true;
      prefs.sandbox = true;
      prefs.webSecurity = true;
      prefs.allowRunningInsecureContent = false;
      prefs.experimentalFeatures = false;
      prefs.enableRemoteModule = false;
      prefs.javascript = true; // we want JS to render the page; sandbox bounds it

      // Force the guest onto the isolated session partition so cookies/
      // storage never mix with the app session.
      params.partition = EXTERNAL_PARTITION;

      // Block attaching to a non-http(s) URL at the boundary. Anything
      // else (file:, javascript:, custom schemes) is denied by setting
      // src to about:blank and letting the renderer's allowlist surface
      // a refusal to the user.
      const verdict = classifyExternalUrl(params.src ?? "");
      if (!verdict.ok) {
        params.src = "about:blank";
      }

      applyGuestContentsHardening(contents);
    });

    // For the host renderer itself: nothing to harden here beyond what
    // the BrowserWindow already configures, but make sure host nav stays
    // pinned (no SPA-driven `will-navigate` away from the app shell to a
    // foreign origin).
    if (contents.getType() !== "webview") {
      contents.on("will-navigate", (e, url) => {
        // Allow the initial load and same-app navigations (file:// to the
        // bundled index.html). Anything else is treated as an attempted
        // host-frame hijack — block it and route http(s) to OS browser.
        const ok = url.startsWith("file://") || url.startsWith("http://localhost") || url.startsWith("https://localhost");
        if (!ok) {
          e.preventDefault();
          if (/^https?:\/\//i.test(url)) void shell.openExternal(url);
        }
      });
    }
  });

  // Configure the external partition session once. Currently this is a
  // placeholder; CSP / referrer / request hygiene land in the
  // request-hygiene commit.
  configureExternalSession();
}

/**
 * Hook the guest webContents (created via <webview>) to deny popups,
 * permission requests, devtools (in production), and any drag-and-drop
 * file loads. Idempotent — safe to call multiple times.
 */
export function applyGuestContentsHardening(host: WebContents): void {
  host.once("did-attach-webview", (_event, guest) => {
    // Deny window.open from external content. Anything that should
    // actually open routes through the renderer's allowlist again via
    // the host's existing setWindowOpenHandler — guest popups don't
    // get a free pass.
    guest.setWindowOpenHandler(({ url }) => {
      if (/^https?:\/\//i.test(url)) {
        // Best-effort: send the user to their OS browser for popup
        // intents from external content. We don't auto-open another
        // in-app tab because popup-driven flows tend to be ad/tracking.
        void shell.openExternal(url);
      }
      return { action: "deny" };
    });

    // Pin guest navigation to http(s). Anything else (javascript:, app:)
    // gets blocked.
    guest.on("will-navigate", (e, url) => {
      const verdict = classifyExternalUrl(url);
      if (!verdict.ok) e.preventDefault();
    });

    // Drag-and-drop file:// loads — block.
    guest.on("will-redirect", (e, url) => {
      const verdict = classifyExternalUrl(url);
      if (!verdict.ok) e.preventDefault();
    });

    // Permission requests: deny by default. We don't want third-party
    // pages prompting the user for camera/mic/geolocation/notifications
    // through the embedded surface.
    guest.session.setPermissionRequestHandler((_wc, _permission, callback) => {
      callback(false);
    });
    guest.session.setPermissionCheckHandler(() => false);

    // DevTools in production: block. (Allowed in dev for debugging.)
    if (app.isPackaged) {
      guest.on("devtools-opened", () => {
        guest.closeDevTools();
      });
    }
  });
}

function configureExternalSession(): void {
  // Touch the session so it's instantiated up front (Electron creates it
  // lazily on first use). CSP + request-rewriting hooks are wired here
  // by the request-hygiene commit.
  void session.fromPartition(EXTERNAL_PARTITION);
}
