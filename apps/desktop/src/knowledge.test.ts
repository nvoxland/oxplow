import { expect, test } from "bun:test";

import type { SqlQueryResult } from "./tauri-bridge/generated/bindings.js";
import { NO_KNOWLEDGE_STORE, pagesFromResult, storesFromResult } from "./knowledge.js";

const result = (columns: string[], rows: SqlQueryResult["rows"]): SqlQueryResult =>
  ({ columns, rows, truncated: false, reads: { datasets: [], tables: [], measures: [] }, freshness: {} }) as unknown as SqlQueryResult;

test("the index reads v_oxplow_knowledge_page rows", () => {
  const pages = pagesFromResult(
    result(
      ["uri", "store", "id", "kind", "title", "excerpt", "updated_at", "stale_ref_count", "outbound_uris"],
      [["oxplow:oxplow/knowledge?id=auth&store=oxwiki", "oxwiki", "auth", "page", "Auth", "How login…", "2026-09-30T00:00:00Z", 2, '["oxplow:oxplow/file?path=src/a.rs","oxplow:oxplow/knowledge?id=other&store=oxwiki"]']],
    ),
  );
  expect(pages).toEqual([
    {
      uri: "oxplow:oxplow/knowledge?id=auth&store=oxwiki",
      store: "oxwiki",
      id: "auth",
      kind: "page",
      title: "Auth",
      excerpt: "How login…",
      updated_at: "2026-09-30T00:00:00Z",
      stale_ref_count: 2,
      outbound_uris: ["oxplow:oxplow/file?path=src/a.rs", "oxplow:oxplow/knowledge?id=other&store=oxwiki"],
    },
  ]);
});

test("the stores read v_model_store rows of the knowledge model, with the id pattern a bare [[id]] is matched by", () => {
  const stores = storesFromResult(
    result(
      ["store", "extension", "id_pattern"],
      [
        ["oxwiki", "oxwiki", "[a-z0-9]+(-[a-z0-9]+)*"],
        ["docs", "docs", null],
      ],
    ),
  );
  expect(stores).toEqual([
    { store: "oxwiki", extension: "oxwiki", idPattern: "[a-z0-9]+(-[a-z0-9]+)*" },
    { store: "docs", extension: "docs", idPattern: null },
  ]);
  expect(NO_KNOWLEDGE_STORE).toBe("No knowledge store is on — turn on an extension that keeps pages, such as oxwiki");
});
