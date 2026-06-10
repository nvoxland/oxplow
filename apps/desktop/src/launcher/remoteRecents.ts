// Recent remote-daemon connections, persisted in localStorage (the
// launcher has no Services, and remote daemons are a per-machine
// concern — unlike RecentProjects, which the backend owns).
//
// Pure list logic over an injected storage so it unit-tests without a
// DOM. Most-recent first, deduped by base URL, capped.

const KEY = "oxplow.recentRemotes";
const CAP = 5;

export interface StringStorage {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
}

export interface RecentRemote {
  base: string;
  lastConnectedAt: number;
}

export function normalizeBase(input: string): string {
  let base = input.trim().replace(/\/+$/, "");
  if (base.length > 0 && !/^https?:\/\//.test(base)) base = `http://${base}`;
  return base;
}

export function loadRecentRemotes(storage: StringStorage): RecentRemote[] {
  try {
    const raw = storage.getItem(KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw) as unknown;
    if (!Array.isArray(parsed)) return [];
    return parsed.filter(
      (r): r is RecentRemote =>
        typeof r === "object" &&
        r !== null &&
        typeof (r as RecentRemote).base === "string" &&
        typeof (r as RecentRemote).lastConnectedAt === "number",
    );
  } catch {
    return [];
  }
}

export function rememberRemote(
  storage: StringStorage,
  base: string,
  now: number = Date.now(),
): RecentRemote[] {
  const cleaned = normalizeBase(base);
  const rest = loadRecentRemotes(storage).filter((r) => r.base !== cleaned);
  const next = [{ base: cleaned, lastConnectedAt: now }, ...rest].slice(0, CAP);
  storage.setItem(KEY, JSON.stringify(next));
  return next;
}

export function forgetRemote(storage: StringStorage, base: string): RecentRemote[] {
  const next = loadRecentRemotes(storage).filter((r) => r.base !== base);
  storage.setItem(KEY, JSON.stringify(next));
  return next;
}
