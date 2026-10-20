/**
 * The only code that touches the browser's storage (a guard,
 * `no-direct-storage.test.ts`, holds every other file to it). What lives
 * here is the shell's own: what must be there before the daemon answers,
 * or belongs to this window — the tab sessions, the open files' paths, the
 * remote daemon to connect to and its token, the recent remotes, and this
 * window's log id.
 *
 * Nothing else is kept in the browser. A page's view is its **view
 * params** and its own memory its **history state** (its tab, which this
 * stores as the tab sessions); what a page or the app remembers for the
 * person is **page memory** and **app preferences** (`prefs/remembered.ts`,
 * the project's database). See `.context/pages-and-tabs.md` "Page state".
 *
 * Every read and write is total: storage that's missing or refuses (a
 * test, private mode) reads as nothing and writes nothing.
 */

/** The shell's keys, and only these. */
export type ShellKey =
  | "oxplow.layout.v4.sessions"
  | "oxplow.layout.v2.fileSessions"
  | "oxplow.remoteBase"
  | "oxplow.remoteToken"
  | "oxplow.recentRemotes";

/** This window's keys, gone when it closes. */
export type WindowKey = "oxplow-ui-client-id";

function area(kind: "local" | "session"): Storage | null {
  try {
    return kind === "local" ? globalThis.localStorage : globalThis.sessionStorage;
  } catch {
    return null;
  }
}

function access<K extends string>(kind: "local" | "session") {
  return {
    getItem(key: K): string | null {
      try {
        return area(kind)?.getItem(key) ?? null;
      } catch {
        return null;
      }
    },
    setItem(key: K, value: string): void {
      try {
        area(kind)?.setItem(key, value);
      } catch {
        // Refused (quota, private mode): not kept.
      }
    },
    removeItem(key: K): void {
      try {
        area(kind)?.removeItem(key);
      } catch {
        // Refused: nothing to do.
      }
    },
  };
}

/** The shell's keys, kept across restarts. */
export const shellStorage = access<ShellKey>("local");

/** This window's keys, for its life. */
export const windowStorage = access<WindowKey>("session");
