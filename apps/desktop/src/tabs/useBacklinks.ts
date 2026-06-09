import { useEffect, useState } from "react";
import { listBacklinks, listPageOutbound, type BacklinkEdge } from "../api.js";
import {
  directoryRef,
  fileRef,
  findingRef,
  gitCommitRef,
  wikiPageRef,
  taskRef,
} from "./pageRefs.js";
import type { TabRef } from "./tabState.js";
import type { BacklinkEntry } from "./backlinkTypes.js";

/**
 * Backlinks for a page. One IPC call to the unified `page_ref`
 * graph; the SQLite reader joins source labels (wiki title, work-
 * item title, commit subject) at read time so the renderer doesn't
 * need a second round-trip per row.
 *
 * Replaces the old in-memory `computeBacklinks` indexer + per-kind
 * `appPageBacklinks` providers — every page kind goes through the
 * same code path now, including FilePage (which used to render an
 * empty list because it never wired the indexer).
 */
export function useBacklinks(target: TabRef): BacklinkEntry[] {
  const [entries, setEntries] = useState<BacklinkEntry[]>([]);

  useEffect(() => {
    let cancelled = false;
    const targetId = canonicalIdForTarget(target);
    if (!targetId) {
      setEntries([]);
      return;
    }
    void listBacklinks(target.kind, targetId, null)
      .then((edges) => {
        if (cancelled) return;
        const mapped = edges
          .map(edgeToInboundEntry)
          .filter((e): e is BacklinkEntry => e !== null);
        setEntries(dedupeEntriesByTarget(mapped));
      })
      .catch(() => {
        if (!cancelled) setEntries([]);
      });
    return () => {
      cancelled = true;
    };
  }, [target.kind, target.id]);

  return entries;
}

/**
 * Outbound: what this page points AT. Sibling to `useBacklinks` and
 * drives the new Outbound dropdown / panel in the Page chrome.
 */
export function usePageOutbound(source: TabRef): BacklinkEntry[] {
  const [entries, setEntries] = useState<BacklinkEntry[]>([]);

  useEffect(() => {
    let cancelled = false;
    const sourceId = canonicalIdForTarget(source);
    if (!sourceId) {
      setEntries([]);
      return;
    }
    void listPageOutbound(source.kind, sourceId, null)
      .then((edges) => {
        if (cancelled) return;
        const mapped = edges
          .map(edgeToOutboundEntry)
          .filter((e): e is BacklinkEntry => e !== null);
        setEntries(dedupeEntriesByTarget(mapped));
      })
      .catch(() => {
        if (!cancelled) setEntries([]);
      });
    return () => {
      cancelled = true;
    };
  }, [source.kind, source.id]);

  return entries;
}

/**
 * Extract the canonical SQLite id from a TabRef. Most kinds carry
 * their canonical id directly in `payload`; some encode it inside
 * the prefixed `ref.id`. Returns null for kinds the page-ref graph
 * doesn't track (settings, dialogs, …) so the hook short-circuits.
 */
export function canonicalIdForTarget(ref: TabRef): string | null {
  switch (ref.kind) {
    case "wiki": {
      const p = ref.payload as { slug?: string } | null;
      return p?.slug ?? null;
    }
    case "task": {
      // payload.itemId is the prefixed `tsk…` id string (see `taskRef`
      // in pageRefs.ts).
      const p = ref.payload as { itemId?: string } | null;
      return p?.itemId ?? null;
    }
    case "file": {
      const p = ref.payload as { path?: string } | null;
      return p?.path ?? null;
    }
    case "directory": {
      const p = ref.payload as { path?: string } | null;
      return p?.path ?? null;
    }
    case "git-commit": {
      const p = ref.payload as { sha?: string } | null;
      return p?.sha ?? null;
    }
    case "finding": {
      const p = ref.payload as { findingId?: string } | null;
      return p?.findingId ?? null;
    }
    default:
      return null;
  }
}

/**
 * Exported for unit tests. Internal call sites all go through
 * `edgeToInboundEntry` / `edgeToOutboundEntry`, which already invoke
 * this helper.
 */
export function humanRefTypeForTest(refType: string, sourceExtra: string | null): string {
  return humanRefType(refType, sourceExtra);
}

/** Convert one inbound edge (source -> me) into a renderer entry. */
function edgeToInboundEntry(edge: BacklinkEdge): BacklinkEntry | null {
  const ref = refFor(edge.source_kind, edge.source_id);
  if (!ref) return null;
  const label = edge.source_label ?? edge.source_id;
  const subtitle = humanRefType(edge.ref_type, edge.source_extra);
  return { ref, label, subtitle };
}

/** Convert one outbound edge (me -> target) into a renderer entry. */
function edgeToOutboundEntry(edge: BacklinkEdge): BacklinkEntry | null {
  const ref = refFor(edge.target_kind, edge.target_id);
  if (!ref) return null;
  const label = edge.source_label ?? edge.target_id;
  const subtitle = humanRefType(edge.ref_type, edge.source_extra);
  return { ref, label, subtitle };
}

/**
 * Collapse rows whose `ref` resolves to the same logical page.
 * Two cases that need this today:
 *
 *  1. A wiki page is referenced both as `wiki:<slug>` (via
 *     `summary_wikilink` / `impact`) and as `file:.oxplow/wiki/<slug>.md`
 *     (via `touched_file`). Same underlying entity, three rows.
 *  2. Multiple ref_types pointing at the same target (e.g. an
 *     impact edge AND a summary_wikilink edge to the same slug).
 *
 * Strategy: bucket by canonical key, prefer the wiki ref over the
 * file ref when both exist, prefer a non-fallback label, and merge
 * each contributor's subtitle into a `·`-separated chip line. Order
 * within the input is preserved across the first appearance of each
 * key.
 */
export function dedupeEntriesByTarget(entries: BacklinkEntry[]): BacklinkEntry[] {
  const buckets = new Map<string, BacklinkEntry[]>();
  const order: string[] = [];
  for (const e of entries) {
    const key = canonicalEntryKey(e.ref);
    if (!buckets.has(key)) {
      buckets.set(key, []);
      order.push(key);
    }
    buckets.get(key)!.push(e);
  }
  return order.map((key) => {
    const group = buckets.get(key)!;
    if (group.length === 1) return group[0];
    // Pick the "best" base entry: wiki kind wins over file when the
    // file is the on-disk shadow of the same wiki page.
    const wikiMember = group.find((g) => g.ref.kind === "wiki");
    const base = wikiMember ?? group[0];
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
      ref: base.ref,
      label: base.label,
      subtitle: subs.length > 0 ? subs.join(" · ") : undefined,
    };
  });
}

/**
 * Identity key for dedup. Same key ⇒ same logical page.
 *
 * `file:.oxplow/wiki/<slug>.md` is treated as `wiki:<slug>` so the
 * on-disk shadow of a wiki page doesn't render as a sibling row to
 * the wiki tab itself.
 */
function canonicalEntryKey(ref: TabRef): string {
  if (ref.kind === "file") {
    const path = (ref.payload as { path?: string } | null)?.path;
    if (path) {
      const m = /^\.oxplow\/wiki\/(.+)\.md$/.exec(path);
      if (m) return `wiki:${m[1]}`;
    }
  }
  const id = canonicalIdForTarget(ref) ?? ref.id;
  return `${ref.kind}:${id}`;
}

/**
 * Build a navigable `TabRef` from a (kind, id) pair. Kinds the
 * frontend doesn't render directly (rare today but possible for
 * future page kinds) yield null so the row is dropped rather than
 * rendering as a dead button.
 */
function refFor(kind: string, id: string): TabRef | null {
  switch (kind) {
    case "wiki":
      return wikiPageRef(id);
    case "task":
      return taskRef(id);
    case "file":
      return fileRef(id);
    case "directory":
      return directoryRef(id);
    case "git-commit":
      return gitCommitRef(id);
    case "finding":
      return findingRef(id);
    default:
      return null;
  }
}

/**
 * Short label for the relationship type, used as the row's
 * subtitle.
 *
 * Three buckets:
 *
 *  - **mention** — every body-mention ref_type collapses to this
 *    single label. They all mean "the target was named in some
 *    body text" (description / AC / summary / wiki page), and the
 *    user doesn't care which body parser found it.
 *
 *  - **action verb** (`created` / `modified` / `deleted` /
 *    `referenced` / `resolved` / `completed` / `reopened`, …) —
 *    rendered for any edge that represents a change. `touched_file`
 *    edges carry the `task_effort_file.change_kind` through
 *    `source_extra.change_kind`; `impact` edges carry the declared
 *    action through `source_extra.action`. Both paths normalize
 *    `updated` to `modified` for display.
 *
 *  - **typed link** — `task_link:<sub>` becomes the sub-type with
 *    underscores swapped for spaces (e.g. `blocks`, `relates to`).
 *
 *  Plus a `found in` fallback for `finding_path`, and the raw
 *  ref_type for anything unrecognized so a new ref_type never
 *  crashes the renderer.
 */
function humanRefType(refType: string, sourceExtra: string | null): string {
  if (refType.startsWith("task_link:")) {
    const sub = refType.slice("task_link:".length);
    return sub.replace(/_/g, " ");
  }
  switch (refType) {
    case "wiki_file_ref":
    case "wiki_dir_ref":
    case "wikilink":
    case "summary_wikilink":
    case "summary_file_ref":
    case "summary_dir_ref":
    case "task_body_mention":
    case "summary_task_mention":
    case "finding_mention":
    case "summary_finding_mention":
    case "commit_mention":
    case "summary_commit_mention":
      return "mention";
    case "touched_file":
      return normalizeAction(parseExtraField(sourceExtra, "change_kind")) ?? "modified";
    case "finding_path":
      return "found in";
    case "impact":
      return normalizeAction(parseExtraField(sourceExtra, "action")) ?? "impact";
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
 * (`created`/`updated`/`deleted` from `task_effort_file`, plus the
 * looser impact verbs the agent declares) into the labels the
 * renderer surfaces. Notably: `updated` → "modified" everywhere so
 * a file-change row and a wiki-page-change row read the same way.
 */
function normalizeAction(action: string | null): string | null {
  if (!action) return null;
  if (action === "updated") return "modified";
  return action;
}
