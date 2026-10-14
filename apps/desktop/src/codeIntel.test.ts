import { expect, test } from "bun:test";

import type { SqlQueryResult } from "./tauri-bridge/generated/bindings.js";
import { diagnosticsFromResult, problemsByFile, symbolsFromResult, symbolsQuery, symbolTree } from "./codeIntel.js";
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
      ["ref", "path", "name", "kind", "container", "signature", "line", "col"],
      [
        ["symbol:a.rs/Widget", "a.rs", "Widget", "struct", null, null, 1, 1],
        ["symbol:a.rs/Widget::spin(int)", "a.rs", "spin", "method", "Widget", "int", 3, 5],
        ["symbol:a.rs/main", "a.rs", "main", "function", null, null, 9, 1],
      ],
    ),
  );
  const tree = symbolTree(symbols);
  expect(tree.map((n) => [n.symbol.name, n.children.map((c) => c.symbol.name)])).toEqual([
    ["Widget", ["spin"]],
    ["main", []],
  ]);
  expect(tree[0]!.children[0]!.symbol.signature).toBe("int");
});

test("a container that is a nested path holds its members under their full path", () => {
  const symbols = symbolsFromResult(
    result(
      ["ref", "path", "name", "kind", "container", "signature", "line", "col"],
      [
        ["symbol:P.cs/Acme::P", "P.cs", "P", "class", "Acme", null, 2, 1],
        ["symbol:P.cs/Acme::P::run", "P.cs", "run", "method", "Acme::P", null, 3, 1],
      ],
    ),
  );
  const tree = symbolTree(symbols);
  expect(tree.map((n) => [n.symbol.name, n.children.map((c) => c.symbol.name)])).toEqual([["P", ["run"]]]);
});

test("a symbol: ref is a page kind of its own, current or pinned", () => {
  const ref = refFromTabId("symbol:Sources/a.rs/Widget::spin");
  expect(ref).toEqual(symbolRef("symbol:Sources/a.rs/Widget::spin"));
  expect(ref?.kind).toBe("symbol");
  expect(refFromTabId("symbol:src/a.rs/Widget::spin@snap:4")?.kind).toBe("symbol");
});

// A name filter is a substring, not a pattern: `%` and `_` in it match
// themselves (escaped, with `ESCAPE '\'`).
test("the symbol filter escapes LIKE's wildcards", () => {
  const q = symbolsQuery(2, { filter: " a_b%c\\d " });
  expect(q.sql).toContain("name LIKE ?2 ESCAPE '\\'");
  expect(q.params).toEqual([2, "%a\\_b\\%c\\\\d%"]);
  expect(symbolsQuery(2, { path: "src/a.rs" }).params).toEqual([2, "src/a.rs"]);
});
