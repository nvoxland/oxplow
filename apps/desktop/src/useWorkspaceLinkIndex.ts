import { useEffect, useRef } from "react";

import { listWorkspaceFiles, subscribeWorkspaceEvents } from "./api.js";

/** Whether a path should be turned into a clickable link (a real workspace
 * file or directory). */
export type LinkablePredicate = (path: string) => boolean;

/** Normalize a candidate to a workspace-relative path, or `null` if it isn't
 * a relative workspace path we can validate (absolute / `~` paths bypass the
 * check so they're never wrongly suppressed). */
function toWorkspaceRelative(path: string): string | null {
  if (path.startsWith("/") || path.startsWith("~")) return null;
  return path.replace(/^\.\/+/, "").replace(/\/+$/, "");
}

/**
 * Maintains an in-memory set of the stream's workspace file paths plus their
 * ancestor directories, refreshed on workspace changes. Returns a stable
 * predicate the terminal link provider uses so dotted words in prose (e.g. a
 * plugin name like `oxplow.junit`) aren't linkified as files.
 *
 * Deliberately conservative — returns `true` (link it) until the index has
 * loaded, and for anything it can't classify (absolute/`~` paths), so it never
 * suppresses a legitimate link; it only suppresses a *relative* path that the
 * loaded index doesn't contain.
 */
export function useWorkspaceLinkIndex(streamId: string | undefined): LinkablePredicate {
  const filesRef = useRef<Set<string>>(new Set());
  const dirsRef = useRef<Set<string>>(new Set());
  const loadedRef = useRef(false);

  useEffect(() => {
    loadedRef.current = false;
    filesRef.current = new Set();
    dirsRef.current = new Set();
    if (!streamId) return;
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | undefined;

    const load = () => {
      listWorkspaceFiles(streamId)
        .then((result) => {
          if (cancelled) return;
          const files = new Set<string>();
          const dirs = new Set<string>();
          for (const f of result.files) {
            const p = f.path.replace(/\/+$/, "");
            files.add(p);
            for (let slash = p.lastIndexOf("/"); slash > 0; slash = p.lastIndexOf("/", slash - 1)) {
              dirs.add(p.slice(0, slash));
            }
          }
          filesRef.current = files;
          dirsRef.current = dirs;
          loadedRef.current = true;
        })
        .catch(() => {
          // Leave the index empty + unloaded so the predicate stays permissive.
        });
    };
    load();
    const unsubscribe = subscribeWorkspaceEvents(streamId, (event) => {
      if (event.kind === "updated") return;
      if (timer) clearTimeout(timer);
      timer = setTimeout(load, 200);
    });
    return () => {
      cancelled = true;
      if (timer) clearTimeout(timer);
      unsubscribe();
    };
  }, [streamId]);

  // Ref-backed so the returned predicate identity is stable (the link provider
  // registers once) while always reading the latest index.
  const predicate = useRef<LinkablePredicate>((path: string) => {
    if (!loadedRef.current) return true;
    const rel = toWorkspaceRelative(path);
    if (rel == null || rel === "") return true;
    return filesRef.current.has(rel) || dirsRef.current.has(rel);
  });
  return predicate.current;
}
