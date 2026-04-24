export interface NoteRef {
  path: string;
}

// Path tokens: at least one slash, a single 1-6 char extension, no leading
// slash (to skip absolute paths / URLs), and at most one trailing `:Symbol`
// anchor that we strip.
const PATH_RE = /(?<![/\w])([a-zA-Z0-9_.-]+(?:\/[a-zA-Z0-9_.-]+)+\.[a-zA-Z0-9]{1,6})(?::[A-Za-z_][\w.]*)?/g;

const URL_RE = /\b[a-zA-Z][a-zA-Z0-9+.-]*:\/\/\S+/g;

export function parseNoteRefs(body: string): NoteRef[] {
  if (!body) return [];
  const stripped = body.replace(URL_RE, " ");
  const seen = new Set<string>();
  for (const match of stripped.matchAll(PATH_RE)) {
    const path = match[1];
    if (!path) continue;
    seen.add(path);
  }
  return Array.from(seen).map((path) => ({ path }));
}
