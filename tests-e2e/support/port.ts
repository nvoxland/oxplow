// Which port a checkout's `vite preview` serves on. Each checkout has its own,
// so runs in different worktrees neither wait for each other nor fail on a
// taken port, and it never changes, so a person can find it again.

const FIRST = 4200;
const COUNT = 800;

/** `dir`'s port, from its path; `E2E_PORT` in `env` overrides it. */
export function previewPort(dir: string, env: Record<string, string | undefined>): number {
  if (env.E2E_PORT) return Number(env.E2E_PORT);
  // FNV-1a: a short path's hash still spreads over the whole range.
  let hash = 0x811c9dc5;
  for (const char of dir) {
    hash ^= char.codePointAt(0)!;
    hash = Math.imul(hash, 0x01000193) >>> 0;
  }
  return FIRST + (hash % COUNT);
}
