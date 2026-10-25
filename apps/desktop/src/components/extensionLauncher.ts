/// The launcher's pages: every enabled extension's page files with a
/// category (oxplow's own among them, from oxplow-foundation), then any
/// declared `pages:` entry with no file and their lenses (until lenses
/// go). A page's `badge:` is its count, one run when the launcher opens.
/// What extensions offer to run is a command — declared with a `ui`,
/// offered like oxplow's own (`commandOffers`).
import { useEffect, useState } from "react";
import { runLens, unwrapResult } from "../api.js";
import { pageFileUri } from "../pages/pageFiles.js";
import { commands, type Extension, type Node, type Page, type SqlCell } from "../tauri-bridge/generated/bindings.js";
import { lensDirectoryEntries, mergeDirectory, viewOptions, viewParam } from "../lens/lensModel.js";
import { refFromTabId, withResourceParams } from "../tabs/pageRefs.js";
import type { PageDirectoryEntry } from "./RailHud/sections.js";

export function launcherPages(extensions: Extension[], counts: Record<string, number> = {}): PageDirectoryEntry[] {
  const pages: PageDirectoryEntry[] = [];
  const listed = new Set<string>();
  for (const ext of extensions) {
    if (!ext.enabled) continue;
    for (const page of ext.pageFiles ?? []) {
      const ref = page.category && !page.hidden ? refFromTabId(pageFileUri(extensions, page)) : null;
      if (!page.category || !ref) continue;
      listed.add(ref.id);
      const badge = page.badge ? counts[page.id] : undefined;
      const keywords = [page.keywords, ext.name, page.slug].filter(Boolean).join(" ");
      pages.push({ id: ref.id, label: page.title, ref, category: page.category, keywords, ...(badge ? { badge } : {}) });
      // A page whose view is a switch lists each of its other sections
      // too, by name ("Settings: Agents"), opening on it.
      const sw = sectionsOf(page);
      if (!sw) continue;
      for (const section of sw.children) {
        if (section.id === sw.first) continue;
        const at = withResourceParams(ref, {}, { [sw.param]: section.id });
        const label = section.title ?? section.id;
        pages.push({ id: at.id, label: `${page.title}: ${label}`, ref: at, category: page.category, keywords: [keywords, label].join(" ") });
      }
    }
  }
  const lenses = new Map(extensions.flatMap((e) => e.lenses.map((l) => [l.id, l] as const)));
  for (const ext of extensions) {
    if (!ext.enabled) continue;
    for (const page of ext.pages) {
      const ref = page.category ? refFromTabId(page.pageRef) : null;
      if (!page.category || !ref || listed.has(ref.id)) continue;
      const badge = page.count ? counts[page.count] : undefined;
      pages.push({
        id: ref.id,
        label: page.title,
        ref,
        category: page.category,
        keywords: [page.keywords, ext.name, page.id].filter(Boolean).join(" "),
        ...(badge ? { badge } : {}),
      });
      // A view document's page lists each of its other sections too, by
      // name ("Settings: Agents"), opening on it.
      const lens = page.body.kind === "lens" ? lenses.get(page.body.lens) : undefined;
      const view = lens ? viewParam(lens) : null;
      if (!lens || !view) continue;
      for (const section of viewOptions(lens)) {
        if (section.id === view.default) continue;
        const at = withResourceParams(ref, {}, { [view.name]: section.id });
        pages.push({
          id: at.id,
          label: `${page.title}: ${section.label}`,
          ref: at,
          category: page.category,
          keywords: [page.keywords, ext.name, page.id, section.label].filter(Boolean).join(" "),
        });
      }
    }
  }
  return mergeDirectory(pages, lensDirectoryEntries(extensions));
}

/** The launcher's whole page list: the declared pages and lenses. Empty
 *  until the extensions load — a partial list would let a quick query pick
 *  the wrong page. */
export function launcherDirectory(extensions: Extension[] | null, counts: Record<string, number>): PageDirectoryEntry[] {
  return extensions === null ? [] : launcherPages(extensions, counts);
}

/** A page file's sections, when its view is a switch (or a stack's
 *  switch): the param, the section shown by default, and each section. */
function sectionsOf(page: Page): { param: string; first: string | null; children: Node[] } | null {
  const find = (n: Node): (Node & { kind: "switch" }) | null =>
    n.kind === "switch" ? n : n.kind === "stack" ? n.children.map(find).find((x) => x !== null) ?? null : null;
  const sw = find(page.view);
  if (!sw) return null;
  const fallback = page.params.find((p) => p.name === sw.param)?.default;
  return { param: sw.param, first: fallback != null ? String(fallback) : (sw.children[0]?.id ?? null), children: sw.children };
}

/** How a page runs once, for its badge (the daemon's `run_page`). */
export type RunPageOnce = (page: string, streamId: string | null) => Promise<{ badge: SqlCell | null }>;

const daemonRunPage: RunPageOnce = async (page, streamId) => unwrapResult(await commands.runPage(page, null, null, streamId));

/** Each badge page's count, by page id: one run of it, its `badge` (0 when
 *  it fails or has none). */
export async function launcherCounts(extensions: Extension[], streamId: string | null, runPage: RunPageOnce = daemonRunPage): Promise<Record<string, number>> {
  const pages = extensions.flatMap((e) => (e.enabled ? (e.pageFiles ?? []).filter((p) => p.badge && p.category && !p.hidden) : []));
  const entries = await Promise.all(
    pages.map((p) =>
      runPage(p.id, streamId)
        .then((run) => [p.id, Number(run.badge ?? 0) || 0] as const)
        .catch(() => [p.id, 0] as const),
    ),
  );
  return Object.fromEntries(entries);
}

/** The pages' badges, by page id (a page file's `badge:`) or count lens (a
 *  `pages:` entry's `count:`), run whenever the launcher opens. */
export function useLauncherCounts(extensions: Extension[] | null, streamId: string | null, open: boolean): Record<string, number> {
  const [counts, setCounts] = useState<Record<string, number>>({});
  useEffect(() => {
    if (!open || !extensions) return;
    let cancelled = false;
    const filed = new Set(extensions.flatMap((e) => (e.pageFiles ?? []).map((p) => `${p.extension}/${p.slug}`)));
    const lenses = extensions.flatMap((e) =>
      e.enabled ? e.pages.flatMap((p) => (p.count && !filed.has(`${p.extension}/${p.id}`) ? [p.count] : [])) : [],
    );
    void Promise.all([
      launcherCounts(extensions, streamId),
      Promise.all(
        lenses.map((lens) =>
          runLens(lens, {}, streamId)
            .then((run) => [lens, Number(run.result.rows[0]?.[0] ?? 0)] as const)
            .catch(() => [lens, 0] as const),
        ),
      ),
    ]).then(([pageCounts, lensCounts]) => {
      if (!cancelled) setCounts({ ...Object.fromEntries(lensCounts), ...pageCounts });
    });
    return () => {
      cancelled = true;
    };
  }, [extensions, streamId, open]);
  return counts;
}
