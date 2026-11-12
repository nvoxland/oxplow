import { useEffect, useState } from "react";
import { listBacklinks, listPageOutbound, type BacklinkEdge } from "../api.js";
import { refTargetNow } from "./refOpener.js";
import { contributionOf, fileUri, idOf, identityOf, keyOf, pageUri, paramOf, typeOf, knowledgeUri } from "../uri/uri.js";
import type { TabRef } from "./tabState.js";
import type { BacklinkEntry } from "./backlinkTypes.js";

/**
 * Backlinks for a page. One IPC call to the unified `page_ref`
 * graph; the SQLite reader joins source labels (page title, work-
 * item title, commit subject) at read time so the renderer doesn't
 * need a second round-trip per row.
 *
 * Replaces the old in-memory `computeBacklinks` indexer + per-kind
 * `appPageBacklinks` providers — every page kind goes through the
 * same code path now.
 */
export function useBacklinks(target: TabRef): BacklinkEntry[] {
  const [entries, setEntries] = useState<BacklinkEntry[]>([]);

  useEffect(() => {
    let cancelled = false;
    const targetUri = graphUriOf(target);
    if (!targetUri) {
      setEntries([]);
      return;
    }
    void listBacklinks(targetUri, null)
      .then((edges) => {
        if (cancelled) return;
        setEntries(dedupeEntriesByTarget(edges.map(edgeToInboundEntry)));
      })
      .catch(() => {
        if (!cancelled) setEntries([]);
      });
    return () => {
      cancelled = true;
    };
  }, [target.id]);

  return entries;
}

/**
 * Outbound: what this page points AT. Sibling to `useBacklinks` and
 * drives the new Outbound dropdown / panel in the Page chrome. A page
 * that changes its own edges (Link…, Unlink) bumps `version` to re-read.
 */
export function usePageOutbound(source: TabRef, version = 0): BacklinkEntry[] {
  const [entries, setEntries] = useState<BacklinkEntry[]>([]);

  useEffect(() => {
    let cancelled = false;
    const sourceUri = graphUriOf(source);
    if (!sourceUri) {
      setEntries([]);
      return;
    }
    void listPageOutbound(sourceUri, null)
      .then((edges) => {
        if (cancelled) return;
        setEntries(dedupeEntriesByTarget(edges.map(edgeToOutboundEntry)));
      })
      .catch(() => {
        if (!cancelled) setEntries([]);
      });
    return () => {
      cancelled = true;
    };
  }, [source.id, version]);

  return entries;
}

/**
 * The URI a page's graph edges hang on (`.context/resources.md`): what the
 * graph stores. A file's is its path (not the revision shown), a page's or
 * metric's its key (not its view params). Null for pages that are not a
 * URI's (settings, dialogs, a declared page) so the hook short-circuits.
 */
export function graphUriOf(ref: TabRef): string | null {
  const resource = typeOf(ref.id);
  // A route, or what isn't one of oxplow's URIs, has none.
  if (resource === null || resource === "oxplow/page") return null;
  // A file at a revision is the file.
  if (resource === "oxplow/file") {
    const path = paramOf(ref.id, "path");
    return path ? fileUri(path) : null;
  }
  return identityOf(ref.id) ?? ref.id;
}


/**
 * Exported for unit tests. Internal call sites all go through
 * `edgeToInboundEntry` / `edgeToOutboundEntry`, which already invoke
 * this helper.
 */
export function humanRefTypeForTest(refType: string, sourceExtra: string | null): string {
  return humanRefType(refType, sourceExtra);
}

/** The task link type an edge's `ref_type` names (`task_record_link:about`). */
const LINK_TYPE_PREFIX = "task_record_link:";

function linkTypeOf(refType: string): string | undefined {
  return refType.startsWith(LINK_TYPE_PREFIX) ? refType.slice(LINK_TYPE_PREFIX.length) : undefined;
}

/** Convert one inbound edge (source -> me) into a renderer entry. */
export function edgeToInboundEntry(edge: BacklinkEdge): BacklinkEntry {
  return entryFor(edge, edge.source);
}

/** Convert one outbound edge (me -> target) into a renderer entry. */
export function edgeToOutboundEntry(edge: BacklinkEdge): BacklinkEntry {
  return entryFor(edge, edge.target);
}

/**
 * One row for the far end `uri` of an edge. A URI no page shows and
 * nothing resolves is a dead link — it keeps its raw text and never
 * throws; an extension may come back, or the record.
 */
function entryFor(edge: BacklinkEdge, uri: string): BacklinkEntry {
  const target = uri;
  const open = openFor(uri, edge.resolved);
  const dead = open === null && !edge.resolved;
  return {
    open,
    target,
    label: dead ? target : (edge.source_label ?? keyOf(uri) ?? uri),
    subtitle: humanRefType(edge.ref_type, edge.source_extra),
    dead,
    linkType: linkTypeOf(edge.ref_type),
  };
}

/**
 * Collapse rows whose `ref` resolves to the same logical page.
 * Two cases that need this today:
 *
 *  1. An oxwiki page is referenced both as its knowledge URI (via
 *     `wikilink`) and as `file:.oxplow/oxwiki/<slug>.md` (via
 *     `touched_file`). Same underlying entity, two rows.
 *  2. Multiple ref_types pointing at the same target (e.g. a
 *     wikilink edge AND a touched_file edge to the same slug).
 *
 * Strategy: bucket by canonical key, prefer the knowledge ref over the
 * file ref when both exist, prefer a non-fallback label, and merge
 * each contributor's subtitle into a `·`-separated chip line. Order
 * within the input is preserved across the first appearance of each
 * key.
 */
export function dedupeEntriesByTarget(entries: BacklinkEntry[]): BacklinkEntry[] {
  const buckets = new Map<string, BacklinkEntry[]>();
  const order: string[] = [];
  for (const e of entries) {
    const key = canonicalEntryKey(e);
    if (!buckets.has(key)) {
      buckets.set(key, []);
      order.push(key);
    }
    buckets.get(key)!.push(e);
  }
  return order.map((key) => {
    const group = buckets.get(key)!;
    if (group.length === 1) return group[0];
    // Pick the "best" base entry: knowledge kind wins over file when the
    // file is the on-disk shadow of the same oxwiki page.
    const knowledgeMember = group.find((g) => typeOf(g.target) === "oxplow/knowledge");
    const base = knowledgeMember ?? group[0];
    // Combine subtitles, dropping empties and duplicates while
    // preserving first-seen order.
    const seen = new Set<string>();
    const subs: string[] = [];
    for (const g of group) {
      const s = g.subtitle?.trim();
      if (!s || seen.has(s)) continue;
      seen.add(s);
      subs.push(s);
    }
    return {
      ...base,
      subtitle: subs.length > 0 ? subs.join(" · ") : undefined,
    };
  });
}

/**
 * Identity key for dedup. Same key ⇒ same logical page.
 *
 * `file:.oxplow/oxwiki/<slug>.md` is treated as the oxwiki page `<slug>` so the
 * on-disk shadow of a page doesn't render as a sibling row to
 * the page's tab itself.
 */
function canonicalEntryKey(entry: BacklinkEntry): string {
  const path = idOf(entry.target, "oxplow/file");
  if (path !== null) {
    const m = /^\.oxplow\/oxwiki\/(.+)\.md$/.exec(path);
    if (m) return knowledgeUri("oxwiki", m[1]!);
  }
  return entry.target;
}

/**
 * The URI a row opens: itself, or where its resource's page
 * says (the registry, `refTargetNow`) when it has one. A contribution
 * (`collector:<ext>/<id>`, `job:…`) something resolves, with no page of
 * its own, opens Settings, where extensions are managed. Anything else
 * yields null: a dead link when nothing resolves it.
 */
function openFor(uri: string, resolved: boolean): string | null {
  // Findings are listed by the oxplow-bundled Findings page.
  if (typeOf(uri) === "oxplow/finding") return pageUri("findings");
  // Where it opens by the registry, or as itself when the window shows it.
  if (!("unopenable" in refTargetNow(uri))) return uri;
  return resolved && contributionOf(uri) !== null ? pageUri("settings") : null;
}

/**
 * Short label for the relationship type, used as the row's
 * subtitle.
 *
 * Three buckets:
 *
 *  - **mention** — every body-mention ref_type collapses to this
 *    single label. They all mean "the target was named in some
 *    body text" (description / AC / summary / knowledge page), and the
 *    user doesn't care which body parser found it.
 *
 *  - **action verb** (`created` / `modified` / `deleted` /
 *    `referenced` / `resolved` / `completed` / `reopened`, …) —
 *    rendered for any edge that represents a change. `touched_file`
 *    edges carry their change kind through
 *    `source_extra.change_kind`; `updated` is shown as `modified`.
 *
 *  - **typed link** — `task_record_link:<sub>` becomes the sub-type with
 *    underscores swapped for spaces (e.g. `blocks`, `relates to`).
 *
 *  - **comment** — `comment_*`: a task's comment mentions it (the
 *    item is the source; its comments live on its page).
 *
 *  Plus a `found in` fallback for `finding_path`, and the raw
 *  ref_type for anything unrecognized so a new ref_type never
 *  crashes the renderer.
 */
function humanRefType(refType: string, sourceExtra: string | null): string {
  if (refType.startsWith("task_record_link:")) {
    const sub = refType.slice("task_record_link:".length);
    return sub.replace(/_/g, " ");
  }
  switch (refType) {
    case "knowledge_file_ref":
    case "knowledge_dir_ref":
    case "file_mention":
    case "dir_mention":
    case "wikilink":
    case "task_mention":
    case "finding_mention":
    case "commit_mention":
      return "mention";
    case "comment_file_ref":
    case "comment_dir_ref":
    case "comment_wikilink":
    case "comment_task_mention":
    case "comment_finding_mention":
    case "comment_commit_mention":
      return "comment";
    case "touched_file":
      return normalizeAction(parseExtraField(sourceExtra, "change_kind")) ?? "modified";
    case "finding_path":
      return "found in";
    default:
      return refType;
  }
}

/** Pull a string field out of the `source_extra` JSON blob. */
function parseExtraField(sourceExtra: string | null, field: string): string | null {
  if (!sourceExtra) return null;
  try {
    const parsed = JSON.parse(sourceExtra) as Record<string, unknown>;
    const value = parsed[field];
    return typeof value === "string" && value.length > 0 ? value : null;
  } catch {
    return null;
  }
}

/**
 * Normalize the action vocabulary the backend stores
 * (`created`/`updated`/`deleted`) into the labels the
 * renderer surfaces. Notably: `updated` → "modified" everywhere so
 * a file-change row and a knowledge-page-change row read the same way.
 */
function normalizeAction(action: string | null): string | null {
  if (!action) return null;
  if (action === "updated") return "modified";
  return action;
}
