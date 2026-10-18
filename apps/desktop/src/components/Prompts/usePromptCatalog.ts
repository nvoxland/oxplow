/// What a person can ask (`v_prompt`): core's questions and every enabled
/// extension's `intent.prompts`, re-read when the model changes (an
/// extension's prompts may have).
import { useEffect, useState } from "react";

import { querySql } from "../../api.js";
import { NO_READS, useRerunOnChange } from "../../lens/lensRerun.js";
import type { Reads, SqlCell } from "../../tauri-bridge/generated/bindings.js";
import type { CatalogPrompt } from "./promptModel.js";

const SQL = "SELECT source_kind, source, prompt, about FROM v_prompt ORDER BY position";

/** The model's rows as prompts. Pure; exported for tests. */
export function promptsOf(rows: SqlCell[][]): CatalogPrompt[] {
  return rows.map(([kind, source, prompt, about]) => ({
    prompt: String(prompt),
    about: typeof about === "string" ? about : null,
    source: { kind: kind === "area" ? "area" : "extension", name: String(source) },
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
