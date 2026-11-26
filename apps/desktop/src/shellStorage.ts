/**
 * The only code that touches the browser's storage (a guard,
 * `no-direct-storage.test.ts`, holds every other file to it). What lives
 * here is the shell's own: what must be there before the daemon answers,
 * or belongs to this browser — its device id, and the token the host gave
 * it at pairing (the host origin's storage). The
 * window's own id lives only in memory, one per page load (`uiClientId.ts`).
 *
 * Nothing else is kept in the browser. A page's view is its **view
 * params** and its own memory its **history state** (its tab, which the
 * daemon keeps: `tabs/useTabSessions.ts`); what a page or the app remembers for the
 * person is **page memory** and **app preferences** (`prefs/remembered.ts`,
 * the project's database). See `.context/pages-and-tabs.md` "Page state".
 *
 * Every read and write is total: storage that's missing or refuses (a
 * test, private mode) reads as nothing and writes nothing.
 */

/** The shell's keys, and only these. */
export type ShellKey =
  | "oxplow.device"
  | "oxplow.hostToken";

function area(): Storage | null {
  try {
    return globalThis.localStorage;
  } catch {
    return null;
  }
}

function access<K extends string>() {
  return {
    getItem(key: K): string | null {
      try {
        return area()?.getItem(key) ?? null;
      } catch {
        return null;
      }
    },
    setItem(key: K, value: string): void {
      try {
        area()?.setItem(key, value);
      } catch {
        // Refused (quota, private mode): not kept.
      }
    },
    removeItem(key: K): void {
      try {
        area()?.removeItem(key);
      } catch {
        // Refused: nothing to do.
      }
    },
  };
}

/** The shell's keys, kept across restarts. */
export const shellStorage = access<ShellKey>();
