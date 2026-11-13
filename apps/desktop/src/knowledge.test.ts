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

test("the enabled stores read v_capability_provider rows; a store that does not say it edits, doesn't", () => {
  const stores = storesFromResult(
    result(
      ["store", "title", "kinds", "features"],
      [
        ["oxwiki", "oxwiki", '[{"id":"note","title":"Note","bucket":"note"}]', '{"edit":true}'],
        ["docs", "Docs", "[]", "{}"],
      ],
    ),
  );
  expect(stores).toEqual([
    { store: "oxwiki", title: "oxwiki", kinds: [{ id: "note", title: "Note", bucket: "note" }], edit: true },
    { store: "docs", title: "Docs", kinds: [], edit: false },
  ]);
  expect(NO_KNOWLEDGE_STORE).toBe("No knowledge store is enabled — turn oxwiki on in Settings → Integrations → Knowledge stores");
});
