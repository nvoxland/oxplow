/**
 * What `[[` lists in the editor (`RefCompletion.tsx`): the site search's
 * hits (a wiki page, a work item, a commit, an extension's resource — the
 * quick open's `searchSite`) and the stream's files by name (its
 * workspace index). A file's content match isn't a name, so the search's
 * file hits are left out. Pure; the editor's owner fetches.
 */
import type { SearchHit } from "../../api.js";
import { fileUri, shortResource } from "../../uri/uri.js";

/** One row of the list: what it opens, what it says, its kind's icon. */
export interface RefOption {
  uri: string;
  title: string;
  /** `PageKindIcon`'s kind. */
  kind: string;
}

/** Where `[[` looks: the matches for `query` (`signal` aborts a stale one). */
export type RefSearch = (query: string, signal: AbortSignal) => Promise<RefOption[]>;

/** The files whose path holds every word of `query`: a name starting with
 *  one of the words first, then a name holding one, then the rest; a
 *  shorter path first within each. */
export function fileOptions(paths: readonly string[], query: string, limit: number): RefOption[] {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (words.length === 0) return [];
  const scored: Array<{ path: string; score: number }> = [];
  for (const path of paths) {
    const lower = path.toLowerCase();
    if (!words.every((w) => lower.includes(w))) continue;
    const name = lower.slice(lower.lastIndexOf("/") + 1);
    const score = words.some((w) => name.startsWith(w)) ? 0 : words.some((w) => name.includes(w)) ? 1 : 2;
    scored.push({ path, score });
  }
  scored.sort((a, b) => a.score - b.score || a.path.length - b.path.length || a.path.localeCompare(b.path));
  return scored.slice(0, limit).map(({ path }) => ({ uri: fileUri(path), title: path, kind: "file" }));
}

/** The search's hits (no file content matches), then the files, up to
 *  `limit`. */
export function mergeRefOptions(hits: readonly SearchHit[], files: readonly RefOption[], limit: number): RefOption[] {
  const titled = hits
    .filter((h) => h.resource !== "oxplow/file")
    .map((h) => ({ uri: h.uri, title: h.title || h.uri, kind: shortResource(h.resource) }));
  return [...titled, ...files].slice(0, limit);
}
