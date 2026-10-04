import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";

// tsk960: a custom component's frame can navigate itself; the page's
// `frame-src` bounds where to, to this machine. Tauri's window gets it in
// its CSP; a plain browser gets the page from whatever serves `dist/`
// (vite, any static server), so the page carries the same bound itself.
// One value, both places.

const DESKTOP = join(import.meta.dir, "..", "..");

function frameSrc(policy: string): string | null {
  const directive = policy
    .split(";")
    .map((d) => d.trim())
    .find((d) => d.startsWith("frame-src "));
  return directive ? directive.slice("frame-src ".length).trim() : null;
}

test("the page bounds frames to this machine, as Tauri's CSP does", () => {
  const html = readFileSync(join(DESKTOP, "index.html"), "utf8");
  const meta = /<meta\s+http-equiv="Content-Security-Policy"\s+content="([^"]*)"\s*\/?>/.exec(html);
  expect(meta).not.toBeNull();
  // Only the frame bound: a fuller policy would also bound what the page
  // may connect to, and a daemon reached over a tunnel isn't on loopback.
  expect(meta![1]!.split(";").filter((d) => d.trim()).length).toBe(1);
  const conf = JSON.parse(readFileSync(join(DESKTOP, "src-tauri", "tauri.conf.json"), "utf8")) as {
    app: { security: { csp: string } };
  };
  expect(frameSrc(meta![1]!)).toBe("http://127.0.0.1:*");
  expect(frameSrc(conf.app.security.csp)).toBe(frameSrc(meta![1]!));
});
