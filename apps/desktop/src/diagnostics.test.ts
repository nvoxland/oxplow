import { expect, test } from "bun:test";

import type { Diagnostic, Extension } from "./tauri-bridge/generated/bindings.js";
import { renderDiagnostic, shownErrors } from "./diagnostics.js";

const d = (over: Partial<Diagnostic>): Diagnostic => ({
  severity: "error",
  code: "manifest.name_mismatch",
  file: null,
  line: null,
  contribution: null,
  message: "name `a` must match its folder `b`",
  fix: null,
  mismatch: null,
  ...over,
});

// The same line the backend's `Diagnostic::render` writes.
test("a diagnostic reads under the folder a person edits", () => {
  expect(renderDiagnostic(d({ file: "extension.yaml", line: 2, fix: "rename the folder" }), "extensions/oxplow-foundation")).toBe(
    "extensions/oxplow-foundation/extension.yaml:2: name `a` must match its folder `b` — fix: rename the folder",
  );
  expect(renderDiagnostic(d({ file: "pages/p.yaml" }), "x/")).toBe("x/pages/p.yaml: name `a` must match its folder `b`");
  expect(renderDiagnostic(d({}), "x")).toBe("name `a` must match its folder `b`");
});

test("an extension's errors read under its source, not the copy that runs", () => {
  const ext = { path: "/p/.oxplow/runtime/extensions/acme/abc", source: ".oxplow/extensions/acme", errors: [d({ file: "extension.yaml", line: 3 })] } as unknown as Extension;
  expect(shownErrors(ext)).toEqual([".oxplow/extensions/acme/extension.yaml:3: name `a` must match its folder `b`"]);
});
