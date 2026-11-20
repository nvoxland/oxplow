import { querySql } from "./api.js";

/**
 * The Mason package to suggest as a language's server when none is
 * configured, read from `v_language.lsp_package` (the extensions that
 * declare the language name it): by the editor's language id, which may be
 * the language's id, one of its aliases or one of its LSP ids
 * (`typescriptreact`). A language whose pack is off still suggests its
 * server; `null` when no language declares one.
 */
export async function suggestedLspPackage(languageId: string): Promise<string | null> {
  const res = await querySql(
    `SELECT lsp_package FROM v_language
      WHERE lsp_package IS NOT NULL
        AND (id = ?1
             OR EXISTS (SELECT 1 FROM json_each(aliases) WHERE value = ?1)
             OR EXISTS (SELECT 1 FROM json_each(lsp_ids) WHERE value = ?1))
      ORDER BY enabled DESC, extension
      LIMIT 1`,
    [languageId],
    1,
  );
  const cell = res.rows[0]?.[0];
  return typeof cell === "string" ? cell : null;
}
