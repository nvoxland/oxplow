import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";

// P6b.D3: the main window may frame only the daemon (a custom component's
// bundle, served from 127.0.0.1), and nothing else.
test("the window's CSP frames only the daemon", () => {
  const conf = JSON.parse(readFileSync(join(import.meta.dir, "../src-tauri/tauri.conf.json"), "utf8")) as {
    app: { security: { csp: string } };
  };
  const directives = Object.fromEntries(
    conf.app.security.csp.split(";").map((d) => {
      const [name, ...values] = d.trim().split(/\s+/);
      return [name, values];
    }),
  );
  expect(directives["frame-src"]).toEqual(["http://127.0.0.1:*"]);
  expect(directives["default-src"]).toEqual(["'self'"]);
});
