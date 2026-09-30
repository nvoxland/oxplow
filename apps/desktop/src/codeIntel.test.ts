import { expect, test } from "bun:test";

import type { SqlQueryResult } from "./tauri-bridge/generated/bindings.js";
import { diagnosticsFromResult, problemsByFile, symbolLocation, symbolsFromResult, symbolTree } from "./codeIntel.js";
import { refFromTabId, symbolRef } from "./tabs/pageRefs.js";

const result = (columns: string[], rows: SqlQueryResult["rows"]): SqlQueryResult =>
  ({ columns, rows, truncated: false, reads: { models: [], tables: [], measures: [] }, freshness: {} }) as unknown as SqlQueryResult;

test("problems group by file, errors first, with counts; a severity filter narrows them", () => {
  const diags = diagnosticsFromResult(
    result(
      ["path", "severity", "message", "source", "code", "line", "col"],
      [
        ["src/b.rs", "warning", "unused", "rustc", null, 3, 1],
        ["src/a.rs", "error", "mismatched types", "rustc", "E0308", 10, 5],
        ["src/a.rs", "warning", "dead code", "rustc", null, 2, 1],
      ],
    ),
  );
  const files = problemsByFile(diags, null);
  expect(files.map((f) => [f.path, f.errors, f.warnings])).toEqual([
    ["src/a.rs", 1, 1],
    ["src/b.rs", 0, 1],
  ]);
  expect(files[0]!.problems.map((p) => p.line)).toEqual([10, 2]);
  expect(problemsByFile(diags, "error").map((f) => f.path)).toEqual(["src/a.rs"]);
});

test("symbols nest under their container, in file order", () => {
  const symbols = symbolsFromResult(
    result(
      ["ref", "path", "name", "kind", "container", "line", "col"],
      [
        ["symbol:a.rs/Widget@snap:1", "a.rs", "Widget", "struct", null, 1, 1],
        ["symbol:a.rs/Widget::spin@snap:1", "a.rs", "spin", "method", "Widget", 3, 5],
        ["symbol:a.rs/main@snap:1", "a.rs", "main", "function", null, 9, 1],
      ],
    ),
  );
  const tree = symbolTree(symbols);
  expect(tree.map((n) => [n.symbol.name, n.children.map((c) => c.symbol.name)])).toEqual([
    ["Widget", ["spin"]],
    ["main", []],
  ]);
});

test("a symbol: ref is a page kind of its own, resolved to its file's line", () => {
  const ref = refFromTabId("symbol:src/a.rs/Widget::spin@snap:4");
  expect(ref).toEqual(symbolRef("symbol:src/a.rs/Widget::spin@snap:4"));
  expect(ref?.kind).toBe("symbol");
  expect(symbolLocation(result(["path", "line", "col"], [["src/a.rs", 3, 5]]))).toEqual({ path: "src/a.rs", line: 3, col: 5 });
  expect(symbolLocation(result(["path", "line", "col"], []))).toBeNull();
});
