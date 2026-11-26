import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";

// P6b.D3, a custom component's frame is the daemon's (its bundle,
// served from 127.0.0.1), and the frame can navigate itself; the page's
// `frame-src` bounds where to, to this machine. Tauri's window gets it in
// its CSP; a plain browser gets the page from whatever serves `dist/`
// (vite, any static server), so the page carries the same bound itself.
// One value, both places. Every loopback name the daemon serves a bundle
// to: a browser that reached the daemon as `localhost` builds its
// frames' URLs from that base. Not `[::1]`: a CSP host source can't be an
// IPv6 literal (Chromium logs it invalid and ignores it), so the
// daemon doesn't serve bundles there either. And `https:`: a window on a
// host on another machine (`.context/host.md` "Other computers") gets its
// frames from that host's origin, which the app cannot know when it is
// built; such a host is HTTPS (plain HTTP only through a tunnel to
// loopback).

const DESKTOP = join(import.meta.dir, "..", "..");

/** A policy's directives, by name. */
function directives(policy: string): Record<string, string[]> {
  return Object.fromEntries(
    policy
      .split(";")
      .map((d) => d.trim())
      .filter(Boolean)
      .map((d) => {
        const [name, ...values] = d.split(/\s+/);
        return [name!, values];
      }),
  );
}

test("the window frames this machine and HTTPS hosts, and the page says the same", () => {
  const conf = JSON.parse(readFileSync(join(DESKTOP, "src-tauri", "tauri.conf.json"), "utf8")) as {
    app: { security: { csp: string } };
  };
  const tauri = directives(conf.app.security.csp);
  // `blob:` is the media viz's PDF frame, a file's bytes the page itself
  // holds; the loopback names are the daemon's component bundles.
  expect(tauri["frame-src"]).toEqual(["blob:", "http://127.0.0.1:*", "http://localhost:*", "https:"]);
  expect(tauri["default-src"]).toEqual(["'self'"]);

  const html = readFileSync(join(DESKTOP, "index.html"), "utf8");
  const meta = /<meta\s+http-equiv="Content-Security-Policy"\s+content="([^"]*)"\s*\/?>/.exec(html);
  expect(meta).not.toBeNull();
  // Only the frame bound: a fuller policy would also bound what the page
  // may connect to, and a daemon reached over a tunnel isn't on loopback.
  expect(directives(meta![1]!)).toEqual({ "frame-src": tauri["frame-src"]! });
});
