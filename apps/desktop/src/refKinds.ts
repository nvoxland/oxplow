/**
 * Every ref kind the vocabulary knows (`.context/refs.md` "The kind
 * registry"), read from `v_ref_kind`: core's and those extensions declare
 * (P8.D6–D7, `.context/extensions.md` → "Ref kinds"). How the chrome draws
 * one (an extension's icon and label), where one opens (its page
 * template, `pageTabId`), how a `[[…]]` names one, and which model holds
 * it.
 *
 * One process-wide list, loaded by `useRefKindsLoader` (mounted once, in
 * `App`) and re-read when `v_ref_kind` changes; the pure helpers
 * (`pageKindIconComponent`, `refFromTabId`, `preprocessWikilinks`) read
 * the current list, and components that render refs of extensions' kinds subscribe with
 * `useRefKinds` so they redraw when it changes.
 */
import {
  BookOpen,
  Box,
  Bug,
  Calendar,
  CircleDot,
  Database,
  FileText,
  Flag,
  Folder,
  GitBranch,
  GitCommit,
  GitPullRequest,
  Link,
  type LucideIcon,
  MessageSquare,
  Package,
  Server,
  Shield,
  Star,
  Tag,
  Ticket,
  User,
  Zap,
} from "lucide-react";
import { useCallback, useEffect, useState, useSyncExternalStore } from "react";

import { querySql } from "./api.js";
import { recordOpError } from "./components/opErrorsStore.js";
import { NO_READS, useRerunOnChange } from "./lens/lensRerun.js";
import { parseRef } from "./refs/ref.js";
import type { Reads, SqlQueryResult } from "./tauri-bridge/generated/bindings.js";

/**
 * The icons a ref kind may name — the Rust `REF_KIND_ICONS` allowlist
 * (`extension_ref_kinds.rs`), which a Rust test keeps in step with these
 * keys.
 */
export const REF_KIND_ICONS: Readonly<Record<string, LucideIcon>> = {
  "book-open": BookOpen,
  "box": Box,
  "bug": Bug,
  "calendar": Calendar,
  "circle-dot": CircleDot,
  "database": Database,
  "file-text": FileText,
  "flag": Flag,
  "folder": Folder,
  "git-branch": GitBranch,
  "git-commit": GitCommit,
  "git-pull-request": GitPullRequest,
  "link": Link,
  "message-square": MessageSquare,
  "package": Package,
  "server": Server,
  "shield": Shield,
  "star": Star,
  "tag": Tag,
  "ticket": Ticket,
  "user": User,
  "zap": Zap,
};

/** One ref kind, as `v_ref_kind` lists it. */
export interface RefKind {
  kind: string;
  /** The extension that declares it; null for a core kind. */
  extension: string | null;
  /** What one is called; null for a core kind (the chrome names those). */
  label: string | null;
  /** The anchored regex its ids match. */
  idPattern: string;
  /** Its `[[prefix:…]]` sugar. */
  wikilinks: string[];
  /** The view holding one, by its `ref` column; null when no model does. */
  resolve: string | null;
  /** Where one opens, a tab-id template (`{ref}`, `{thread_ref}`,
   *  `page:acme/pr?ref={ref}`); null when it has no page. */
  page: string | null;
  icon: string | null;
}

export function refKindsFromResult(result: SqlQueryResult): RefKind[] {
  return result.rows.map((row) => {
    const at = (name: string) => row[result.columns.indexOf(name)] ?? null;
    const text = (name: string) => String(at(name) ?? "");
    const maybe = (name: string) => {
      const v = at(name);
      return v === null || v === undefined || v === "" ? null : String(v);
    };
    let wikilinks: string[] = [];
    try {
      const parsed: unknown = JSON.parse(text("wikilinks") || "[]");
      if (Array.isArray(parsed)) wikilinks = parsed.map(String);
    } catch {
      wikilinks = [];
    }
    return {
      kind: text("kind"),
      extension: maybe("extension"),
      label: maybe("label"),
      idPattern: text("id_pattern"),
      wikilinks,
      resolve: maybe("resolve"),
      page: maybe("page"),
      icon: maybe("icon"),
    };
  });
}

let current: ReadonlyMap<string, RefKind> = new Map();
const listeners = new Set<() => void>();

/** Replace the list (the loader; tests). */
export function setRefKinds(kinds: readonly RefKind[]): void {
  current = new Map(kinds.map((k) => [k.kind, k]));
  for (const l of listeners) l();
}

/** Whether the list has been read: before it, a kind that isn't core's is
 *  not yet known to be missing. */
export function refKindsLoaded(): boolean {
  return current.size > 0;
}

/** A ref kind by name, or null (unknown, or the list isn't loaded yet). */
export function refKindInfo(kind: string): RefKind | null {
  return current.get(kind) ?? null;
}

/** The `{…}` placeholders of a page template, each its alternatives in
 *  order (`{effort_ref|thread_ref}`), with where it sits. */
function placeholders(template: string): Array<{ at: number; end: number; names: string[] }> {
  return [...template.matchAll(/\{([^{}]+)\}/g)].map((m) => ({
    at: m.index,
    end: m.index + m[0].length,
    names: m[1]!.split("|"),
  }));
}

/** The columns of its `resolve` row that `ref`'s page template reads, and
 *  the view to read them from; null when it reads none (`{ref}`, a page)
 *  or its kind has no page. */
export function pageColumns(ref: string): { view: string; columns: string[] } | null {
  const kind = parseRef(ref)?.kind;
  const info = kind ? current.get(kind) : undefined;
  if (!info?.page || !info.resolve) return null;
  const columns = [...new Set(placeholders(info.page).flatMap((p) => p.names))].filter((c) => c !== "ref");
  return columns.length ? { view: info.resolve, columns } : null;
}

/**
 * The tab id `ref` opens as, by its kind's page template (`v_ref_kind.page`):
 * `{ref}` is the ref itself, `{col}` that column of `row` (its `resolve`
 * row), `{a|b}` the first of them that isn't empty. A value after the `?`
 * is a query parameter, encoded as one. Null when its kind has no page,
 * or the template names a column `row` doesn't give.
 */
export function pageTabId(ref: string, row: Readonly<Record<string, unknown>> = {}): string | null {
  const kind = parseRef(ref)?.kind;
  const template = kind ? current.get(kind)?.page : null;
  if (!template) return null;
  const query = template.indexOf("?");
  let out = "";
  let last = 0;
  for (const p of placeholders(template)) {
    const value = p.names
      .map((name) => (name === "ref" ? ref : row[name]))
      .find((v) => v !== null && v !== undefined && v !== "");
    if (value === undefined) return null;
    const text = String(value);
    out += template.slice(last, p.at) + (query >= 0 && p.at > query ? encodeURIComponent(text) : text);
    last = p.end;
  }
  return out + template.slice(last);
}

/** The icon of an extension's ref kind, or null. */
export function refKindIcon(kind: string): LucideIcon | null {
  const icon = current.get(kind)?.icon;
  return icon ? (REF_KIND_ICONS[icon] ?? null) : null;
}

/**
 * The canonical ref a `[[…]]` interior names through an extension's kind
 * — `acme_pr:12` itself, or its `wikilink:` sugar `pr:12` — or null. The
 * id must match the kind's pattern, as the backend's `canonical_wikilink`
 * checks.
 */
export function extensionWikilinkRef(interior: string): string | null {
  const colon = interior.indexOf(":");
  if (colon <= 0) return null;
  const head = interior.slice(0, colon).toLowerCase();
  const id = interior.slice(colon + 1).trim();
  // The backend's order (`canonical_wikilink`): a `wikilink:` sugar, then
  // a kind's own name. Load keeps them apart (tsk933); one order keeps the
  // UI and the backend naming the same ref regardless.
  const extensions = [...current.values()].filter((k) => k.extension !== null);
  const kind = extensions.find((k) => k.wikilinks.includes(head)) ?? extensions.find((k) => k.kind === head);
  // The pattern runs in JS's backtracking engine: the load check keeps it
  // in one explicit ASCII form both engines read alike, with no repeats
  // that could share characters unfenced, so no id makes it backtrack
  // (tsk797, tsk917).
  if (!kind || !id) return null;
  let matches = false;
  try {
    matches = new RegExp(kind.idPattern).test(id);
  } catch {
    matches = false;
  }
  return matches ? `${kind.kind}:${id}` : null;
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/** The ref kinds; a component that draws or opens refs reads this so it
 *  redraws when they change. */
export function useRefKinds(): ReadonlyMap<string, RefKind> {
  return useSyncExternalStore(
    subscribe,
    () => current,
    () => current,
  );
}

/** Every ref kind, and what was read. */
export async function readRefKinds(): Promise<{ kinds: RefKind[]; reads: Reads }> {
  const res = await querySql(
    `SELECT kind, extension, label, id_pattern, wikilinks, resolve, page, icon
       FROM v_ref_kind ORDER BY kind`,
    [],
    1_000,
  );
  return { kinds: refKindsFromResult(res), reads: res.reads };
}

/** Load the list now and again whenever `v_ref_kind` changes. Mounted
 *  once, in `App`. */
export function useRefKindsLoader(): void {
  const [reads, setReads] = useState<Reads>(NO_READS);
  const load = useCallback(() => {
    void readRefKinds()
      .then((r) => {
        setRefKinds(r.kinds);
        setReads(r.reads);
      })
      .catch((e: unknown) => {
        recordOpError({ label: "Read ref kinds", message: e instanceof Error ? e.message : String(e) });
      });
  }, []);
  useEffect(load, [load]);
  useRerunOnChange(reads, load);
}

/** The title of an extension's ref from its kind's `resolve` model
 *  (`SELECT title … WHERE ref = ?`); null while loading, or when the
 *  model has no row for it. */
export function useExtensionRefTitle(ref: string | null): string | null {
  const kinds = useRefKinds();
  const kind = ref ? kinds.get(parseRef(ref)?.kind ?? "") : undefined;
  const [title, setTitle] = useState<string | null>(null);
  const [reads, setReads] = useState<Reads>(NO_READS);
  const view = kind?.resolve ?? null;
  const load = useCallback(() => {
    // A view name built by oxplow (`v_<extension>_<model>`); never text a
    // person typed. Checked anyway: it's spliced into the query.
    if (!ref || !view || !/^v_[a-z0-9_]+$/.test(view)) {
      setTitle(null);
      return;
    }
    void querySql(`SELECT title FROM ${view} WHERE ref = ?1 LIMIT 1`, [ref], 1)
      .then((res) => {
        const cell = res.rows[0]?.[0];
        setTitle(cell == null ? null : String(cell));
        setReads(res.reads);
      })
      .catch(() => setTitle(null));
  }, [ref, view]);
  useEffect(load, [load]);
  useRerunOnChange(reads, load);
  return title;
}
