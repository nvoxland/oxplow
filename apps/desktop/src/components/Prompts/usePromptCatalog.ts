/// The prompt catalog for a stream, re-read when an extension's files
/// change (its `intent.prompts` may have).
import { useEffect, useState } from "react";

import { promptCatalog, subscribeOxplowEvents } from "../../api.js";
import { lensDefinitionChanged } from "../../lens/lensRerun.js";
import type { CatalogPrompt } from "../../tauri-bridge/generated/bindings.js";

export function usePromptCatalog(streamId: string | null): CatalogPrompt[] {
  const [catalog, setCatalog] = useState<CatalogPrompt[]>([]);
  useEffect(() => {
    let live = true;
    const load = () =>
      promptCatalog(streamId)
        .then((c) => {
          if (live) setCatalog(c);
        })
        .catch(() => {
          // No catalog, no suggestions; the page still works.
        });
    load();
    const off = subscribeOxplowEvents((e) => {
      if (lensDefinitionChanged(e as Record<string, unknown>)) load();
    });
    return () => {
      live = false;
      off();
    };
  }, [streamId]);
  return catalog;
}
