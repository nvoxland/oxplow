/// What a person can ask (`v_page_answer`): what every enabled extension's
/// pages say they answer, re-read when the dataset changes.
import { useEffect, useState } from "react";

import { querySql } from "../../api.js";
import { NO_READS, useRerunOnChange } from "../../viz/rerun.js";
import type { Reads, SqlCell } from "../../tauri-bridge/generated/bindings.js";
import type { CatalogPrompt } from "./promptModel.js";

const SQL =
  "SELECT 'extension' AS source_kind, p.extension AS source, a.question AS prompt, a.about " +
  "FROM v_page_answer a JOIN v_page p ON p.uri = a.page_uri ORDER BY p.extension, p.id, a.position";

/** The dataset's rows as prompts. Pure; exported for tests. */
export function promptsOf(rows: SqlCell[][]): CatalogPrompt[] {
  return rows.map(([, source, prompt, about]) => ({
    prompt: String(prompt),
    about: typeof about === "string" ? about : null,
    source: { kind: "extension", name: String(source) },
  }));
}

export function usePromptCatalog(): CatalogPrompt[] {
  const [catalog, setCatalog] = useState<CatalogPrompt[]>([]);
  const [reads, setReads] = useState<Reads>(NO_READS);
  const load = () =>
    querySql(SQL, [], null)
      .then((r) => {
        setCatalog(promptsOf(r.rows));
        setReads(r.reads);
      })
      .catch(() => {
        // No catalog, no suggestions; the page still works.
      });
  useEffect(() => void load(), []);
  useRerunOnChange(reads, () => void load());
  return catalog;
}
