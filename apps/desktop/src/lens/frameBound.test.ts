import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";

// P6b.D3, tsk960: a custom component's frame is the daemon's (its bundle,
// served from 127.0.0.1), and the frame can navigate itself; the page's
// `frame-src` bounds where to, to this machine. Tauri's window gets it in
// its CSP; a plain browser gets the page from whatever serves `dist/`
// (vite, any static server), so the page carries the same bound itself.
// One value, both places.

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

test("the window frames only this machine, and the page says the same", () => {
  const conf = JSON.parse(readFileSync(join(DESKTOP, "src-tauri", "tauri.conf.json"), "utf8")) as {
    app: { security: { csp: string } };
  };
  const tauri = directives(conf.app.security.csp);
  expect(tauri["frame-src"]).toEqual(["http://127.0.0.1:*"]);
  expect(tauri["default-src"]).toEqual(["'self'"]);

  const html = readFileSync(join(DESKTOP, "index.html"), "utf8");
  const meta = /<meta\s+http-equiv="Content-Security-Policy"\s+content="([^"]*)"\s*\/?>/.exec(html);
  expect(meta).not.toBeNull();
  // Only the frame bound: a fuller policy would also bound what the page
  // may connect to, and a daemon reached over a tunnel isn't on loopback.
  expect(directives(meta![1]!)).toEqual({ "frame-src": tauri["frame-src"]! });
});
