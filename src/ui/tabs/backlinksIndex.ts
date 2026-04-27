/**
 * Backlinks indexer — given the renderer's already-loaded data slices,
 * compute cross-kind backreferences for any `TabRef`. Pure module: no
 * IPC, no side effects, fully unit-testable.
 *
 * Cross-kind directions covered (see `backlinksIndex.test.ts`):
 *   file        → notes mentioning the path, work items whose efforts
 *                 touched it, findings located at it.
 *   work-item   → notes that link `[[wi-id]]` or contain its id,
 *                 findings on files this item's efforts touched.
 *   finding     → work items whose efforts touched the finding's file,
 *                 notes mentioning that file or the `finding:id` token.
 *   note        → work items, files, findings the note text mentions.
 *
 * The indexer takes plain inputs:
 *   - `notes`: { slug, title, body }
 *   - `workItems`: { id, title, description, acceptance_criteria, touched_files }
 *   - `findings`: { id, path, startLine, endLine, kind, metricValue }
 *
 * Touched-files is the union of paths from all efforts on the work item;
 * the caller passes it in already-computed (App.tsx already loads
 * efforts per work item where the panel needs them).
 */

import type { TabRef } from "./tabState.js";
import { fileRef, findingRef, noteRef, workItemRef } from "./pageRefs.js";

export interface BacklinkNoteEntry {
  slug: string;
  title: string;
  body: string;
}

export interface BacklinkWorkItemEntry {
  id: string;
  title: string;
  description: string;
  acceptance_criteria: string | null;
  /** Union of paths from every effort on this work item (caller-computed). */
  touched_files: string[];
}

export interface BacklinkFindingEntry {
  id: string;
  path: string;
  startLine: number;
  endLine: number;
  kind: string;
  metricValue: number;
}

export interface BacklinkContext {
  notes: BacklinkNoteEntry[];
  workItems: BacklinkWorkItemEntry[];
  findings: BacklinkFindingEntry[];
}

export interface BacklinkEntry {
  ref: TabRef;
  label: string;
  subtitle?: string;
}

function noteMentionsPath(note: BacklinkNoteEntry, path: string): boolean {
  // Look for the path as a substring with a "soft" boundary: the
  // following char must not extend it into a longer path token. We
  // accept sentence punctuation (`.`/`,`/`)`/`]`) as a terminator only
  // when not followed by an extension-character.
  let from = 0;
  while (from <= note.body.length) {
    const idx = note.body.indexOf(path, from);
    if (idx < 0) return false;
    const after = note.body[idx + path.length];
    if (after === undefined) return true;
    // Continues into a longer path-like token — keep searching.
    if (/[A-Za-z0-9_/-]/.test(after)) {
      from = idx + 1;
      continue;
    }
    // A trailing `.` is ambiguous: `src/a.ts.bak` continues the path,
    // but `src/a.ts.` is sentence punctuation. Distinguish by the char
    // after the dot.
    if (after === ".") {
      const after2 = note.body[idx + path.length + 1];
      if (after2 !== undefined && /[A-Za-z0-9]/.test(after2)) {
        from = idx + 1;
        continue;
      }
    }
    return true;
  }
  return false;
}

function noteMentionsId(note: BacklinkNoteEntry, id: string): boolean {
  // Case-sensitive substring check; ids are stable kebab/lowercase.
  return note.body.includes(id);
}

function workItemTouchesFile(item: BacklinkWorkItemEntry, path: string): boolean {
  return item.touched_files.includes(path);
}

function isPathToken(token: string): boolean {
  // crude heuristic — extension and at least one slash, or a dotted relative.
  return /[/]/.test(token) && /\.[A-Za-z0-9]+$/.test(token);
}

function extractFilePathsFromBody(body: string): string[] {
  // Pull tokens that look like workspace-relative file paths. Matches
  // backtick-quoted code and bare paths. Conservative — only tokens with
  // a slash AND a file extension.
  const found = new Set<string>();
  const re = /[A-Za-z0-9_./-]+/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(body)) !== null) {
    const tok = m[0];
    if (isPathToken(tok)) found.add(tok);
  }
  return [...found];
}

function extractWorkItemIdsFromBody(body: string): string[] {
  const found = new Set<string>();
  // [[wi-…]] explicit link form
  const linkRe = /\[\[(wi-[A-Za-z0-9]+)\]\]/g;
  let m: RegExpExecArray | null;
  while ((m = linkRe.exec(body)) !== null) found.add(m[1]);
  // Bare wi-… token
  const bareRe = /\bwi-[A-Za-z0-9]+\b/g;
  while ((m = bareRe.exec(body)) !== null) found.add(m[0]);
  return [...found];
}

function extractFindingIdsFromBody(body: string): string[] {
  const found = new Set<string>();
  const re = /\bfinding:([A-Za-z0-9-]+)\b/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(body)) !== null) found.add(m[1]);
  return [...found];
}

function pushUnique(out: BacklinkEntry[], seen: Set<string>, entry: BacklinkEntry, selfId: string): void {
  if (entry.ref.id === selfId) return;
  if (seen.has(entry.ref.id)) return;
  seen.add(entry.ref.id);
  out.push(entry);
}

export function computeBacklinks(target: TabRef, ctx: BacklinkContext): BacklinkEntry[] {
  const out: BacklinkEntry[] = [];
  const seen = new Set<string>();

  switch (target.kind) {
    case "file": {
      const path = (target.payload as { path?: string } | null)?.path;
      if (!path) return [];

      for (const note of ctx.notes) {
        if (noteMentionsPath(note, path)) {
          pushUnique(out, seen, { ref: noteRef(note.slug), label: note.title || note.slug, subtitle: "note" }, target.id);
        }
      }
      for (const item of ctx.workItems) {
        if (workItemTouchesFile(item, path)) {
          pushUnique(out, seen, { ref: workItemRef(item.id), label: item.title, subtitle: item.id }, target.id);
        }
      }
      for (const f of ctx.findings) {
        if (f.path === path) {
          pushUnique(out, seen, {
            ref: findingRef(f.id),
            label: `${f.kind} (line ${f.startLine})`,
            subtitle: `metric ${f.metricValue}`,
          }, target.id);
        }
      }
      return out;
    }

    case "work-item": {
      const itemId = (target.payload as { itemId?: string } | null)?.itemId;
      if (!itemId) return [];

      const item = ctx.workItems.find((w) => w.id === itemId);
      const touched = item?.touched_files ?? [];

      for (const note of ctx.notes) {
        if (noteMentionsId(note, itemId)) {
          pushUnique(out, seen, { ref: noteRef(note.slug), label: note.title || note.slug, subtitle: "note" }, target.id);
        }
      }
      for (const f of ctx.findings) {
        if (touched.includes(f.path)) {
          pushUnique(out, seen, {
            ref: findingRef(f.id),
            label: `${f.kind} in ${f.path}`,
            subtitle: `line ${f.startLine}`,
          }, target.id);
        }
      }
      return out;
    }

    case "finding": {
      const findingId = (target.payload as { findingId?: string } | null)?.findingId;
      if (!findingId) return [];

      const f = ctx.findings.find((x) => x.id === findingId);
      if (!f) return [];

      for (const item of ctx.workItems) {
        if (workItemTouchesFile(item, f.path)) {
          pushUnique(out, seen, { ref: workItemRef(item.id), label: item.title, subtitle: item.id }, target.id);
        }
      }
      for (const note of ctx.notes) {
        if (noteMentionsPath(note, f.path) || noteMentionsId(note, `finding:${f.id}`)) {
          pushUnique(out, seen, { ref: noteRef(note.slug), label: note.title || note.slug, subtitle: "note" }, target.id);
        }
      }
      return out;
    }

    case "note": {
      const slug = (target.payload as { slug?: string } | null)?.slug;
      if (!slug) return [];
      const note = ctx.notes.find((n) => n.slug === slug);
      if (!note) return [];

      const wiIds = new Set(extractWorkItemIdsFromBody(note.body));
      const findingIds = new Set(extractFindingIdsFromBody(note.body));
      const filePaths = new Set(extractFilePathsFromBody(note.body));

      for (const item of ctx.workItems) {
        if (wiIds.has(item.id)) {
          pushUnique(out, seen, { ref: workItemRef(item.id), label: item.title, subtitle: item.id }, target.id);
        }
      }
      for (const path of filePaths) {
        // Only emit file backlinks — we don't gate by "file exists in
        // workspace" here because the indexer doesn't have an FS slice.
        // If the path doesn't resolve at click time the file tab opener
        // already surfaces a not-found state.
        pushUnique(out, seen, { ref: fileRef(path), label: path, subtitle: "file" }, target.id);
      }
      for (const f of ctx.findings) {
        if (findingIds.has(f.id)) {
          pushUnique(out, seen, {
            ref: findingRef(f.id),
            label: `${f.kind} in ${f.path}`,
            subtitle: `line ${f.startLine}`,
          }, target.id);
        }
      }
      return out;
    }

    default:
      return [];
  }
}
