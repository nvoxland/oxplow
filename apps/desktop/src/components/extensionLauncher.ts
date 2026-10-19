/// The launcher's pages: every enabled extension's declared `pages:` with
/// a category (oxplow's own among them, from oxplow-foundation) and their
/// lenses. A page's `count:` lens is its badge. What extensions offer to
/// run is a command — declared with a `ui`, offered like oxplow's own
/// (`commandOffers`).
import { useEffect, useState } from "react";
import { runLens } from "../api.js";
import type { Extension } from "../tauri-bridge/generated/bindings.js";
import { lensDirectoryEntries, mergeDirectory } from "../lens/lensModel.js";
import { refFromTabId } from "../tabs/pageRefs.js";
import type { PageDirectoryEntry } from "./RailHud/sections.js";

export function launcherPages(extensions: Extension[], counts: Record<string, number> = {}): PageDirectoryEntry[] {
  const pages: PageDirectoryEntry[] = [];
  for (const ext of extensions) {
    if (!ext.enabled) continue;
    for (const page of ext.pages) {
      const ref = page.category ? refFromTabId(page.pageRef) : null;
      if (!page.category || !ref) continue;
      const badge = page.count ? counts[page.count] : undefined;
      pages.push({
        id: ref.id,
        label: page.title,
        ref,
        category: page.category,
        keywords: [page.keywords, ext.name, page.id].filter(Boolean).join(" "),
        ...(badge ? { badge } : {}),
      });
    }
  }
  return [...pages, ...lensDirectoryEntries(extensions)];
}

/** The launcher's whole page list: the declared pages and lenses, then
 *  `more` (the Settings areas) placed by category. Empty until the
 *  extensions load — a partial list would let a quick query pick the wrong
 *  page ("Settings" finding an area before the Settings page). */
export function launcherDirectory(
  extensions: Extension[] | null,
  counts: Record<string, number>,
  more: PageDirectoryEntry[],
): PageDirectoryEntry[] {
  return extensions === null ? [] : mergeDirectory(launcherPages(extensions, counts), more);
}

/** The declared pages' badges, by count lens: each `count:` lens run
 *  (its first cell) whenever the launcher opens. */
export function useLauncherCounts(extensions: Extension[] | null, streamId: string | null, open: boolean): Record<string, number> {
  const [counts, setCounts] = useState<Record<string, number>>({});
  useEffect(() => {
    if (!open || !extensions) return;
    let cancelled = false;
    const lenses = extensions.flatMap((e) => (e.enabled ? e.pages.flatMap((p) => (p.count ? [p.count] : [])) : []));
    void Promise.all(
      lenses.map((lens) =>
        runLens(lens, {}, streamId)
          .then((run) => [lens, Number(run.result.rows[0]?.[0] ?? 0)] as const)
          .catch(() => [lens, 0] as const),
      ),
    ).then((entries) => {
      if (!cancelled) setCounts(Object.fromEntries(entries));
    });
    return () => {
      cancelled = true;
    };
  }, [extensions, streamId, open]);
  return counts;
}
