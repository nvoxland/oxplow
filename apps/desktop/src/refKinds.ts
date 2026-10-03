/**
 * The ref kinds extensions declare (P8.D6–D7, `.context/extensions.md` →
 * "Ref kinds"), read from `v_ref_kind`: how the chrome draws one (icon,
 * label), where one opens (its extension's page, given `?ref=`), how a
 * `[[…]]` names one, and which model titles it.
 *
 * One process-wide list, loaded by `useRefKindsLoader` (mounted once, in
 * `App`) and re-read when `v_ref_kind` changes; the pure helpers
 * (`pageKindIconComponent`, `refFromTabId`, `preprocessWikilinks`) read
 * the current list, and components that render plugin refs subscribe with
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

/** The longest id a `[[…]]` may name through an extension's kind. */
export const MAX_PLUGIN_ID = 256;

/** One extension's ref kind, as `v_ref_kind` lists it. */
export interface RefKind {
  kind: string;
  extension: string;
  label: string;
  /** The anchored regex its ids match. */
  idPattern: string;
  /** Its `[[prefix:…]]` sugar. */
  wikilinks: string[];
  /** The view whose `title` names one, by `ref`. */
  resolve: string;
  /** The page that opens one: `page:ext.<extension>.<page>`. */
  page: string;
  icon: string;
}

export function refKindsFromResult(result: SqlQueryResult): RefKind[] {
  return result.rows.map((row) => {
    const at = (name: string) => row[result.columns.indexOf(name)] ?? null;
    const text = (name: string) => String(at(name) ?? "");
    let wikilinks: string[] = [];
    try {
      const parsed: unknown = JSON.parse(text("wikilinks") || "[]");
      if (Array.isArray(parsed)) wikilinks = parsed.map(String);
    } catch {
      wikilinks = [];
    }
    return {
      kind: text("kind"),
      extension: text("extension"),
      label: text("label"),
      idPattern: text("id_pattern"),
      wikilinks,
      resolve: text("resolve"),
      page: text("page"),
      icon: text("icon"),
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

/** An extension's ref kind by name, or null (a core kind, or unknown). */
export function refKindInfo(kind: string): RefKind | null {
  return current.get(kind) ?? null;
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
export function pluginWikilinkRef(interior: string): string | null {
  const colon = interior.indexOf(":");
  if (colon <= 0) return null;
  const head = interior.slice(0, colon).toLowerCase();
  const id = interior.slice(colon + 1).trim();
  const kind = current.get(head) ?? [...current.values()].find((k) => k.wikilinks.includes(head));
  // The pattern runs in JS's backtracking engine: the load check keeps it
  // to a portable, group-free subset, and a long id never reaches it
  // (tsk797).
  if (!kind || !id || id.length > MAX_PLUGIN_ID) return null;
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

/** The extensions' ref kinds; a component that draws plugin refs reads
 *  this so it redraws when they change. */
export function useRefKinds(): ReadonlyMap<string, RefKind> {
  return useSyncExternalStore(
    subscribe,
    () => current,
    () => current,
  );
}

/** Every extension's ref kind, and what was read. */
export async function readRefKinds(): Promise<{ kinds: RefKind[]; reads: Reads }> {
  const res = await querySql(
    `SELECT kind, extension, label, id_pattern, wikilinks, resolve, page, icon
       FROM v_ref_kind WHERE extension IS NOT NULL ORDER BY kind`,
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
export function usePluginRefTitle(ref: string | null): string | null {
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
