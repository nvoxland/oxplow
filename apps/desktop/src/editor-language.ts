/** The Monaco language id of a file path. A language an enabled pack
 *  declares (`editor: { monaco }`, `v_language.monaco`) always wins; the
 *  table below is only the highlighting fallback for extensions no pack
 *  declares (its ids match Monaco's bundled basic-languages
 *  registrations). `plaintext` for anything unrecognized. */
export function languageForPath(path: string | null): string {
  if (!path) return "plaintext";
  const lower = path.toLowerCase();
  for (const [suffix, id] of declared) {
    if (lower.endsWith(suffix)) return id;
  }
  for (const [suffix, id] of EXTENSION_MAP) {
    if (lower.endsWith(suffix)) return id;
  }
  return "plaintext";
}

/** Extension suffix (`.ex`) → Monaco id, from the languages the enabled
 *  extensions declare; loaded by `editorLanguages.ts`. */
let declared: ReadonlyArray<readonly [string, string]> = [];
const declaredListeners = new Set<() => void>();

/** What the declared languages are now: `rows` are `v_language`'s file
 *  extensions (no dot) and its Monaco id. */
export function setDeclaredEditorLanguages(rows: ReadonlyArray<{ extensions: string[]; monaco: string }>): void {
  const next: Array<readonly [string, string]> = [];
  for (const row of rows) {
    for (const ext of row.extensions) next.push([`.${ext.toLowerCase()}`, row.monaco] as const);
  }
  const same =
    next.length === declared.length && next.every(([s, id], i) => declared[i]![0] === s && declared[i]![1] === id);
  if (same) return;
  declared = next;
  for (const l of declaredListeners) l();
}

/** Called when the declared languages change (a pack turned on or off). */
export function onDeclaredEditorLanguagesChanged(listener: () => void): () => void {
  declaredListeners.add(listener);
  return () => declaredListeners.delete(listener);
}

const EXTENSION_MAP: ReadonlyArray<readonly [string, string]> = [
  [".tsx", "typescript"],
  [".ts", "typescript"],
  [".mts", "typescript"],
  [".cts", "typescript"],
  [".jsx", "javascript"],
  [".mjs", "javascript"],
  [".cjs", "javascript"],
  [".js", "javascript"],
  [".rs", "rust"],
  [".py", "python"],
  [".pyi", "python"],
  [".go", "go"],
  [".java", "java"],
  [".kt", "kotlin"],
  [".kts", "kotlin"],
  [".scala", "scala"],
  [".rb", "ruby"],
  [".php", "php"],
  [".cs", "csharp"],
  [".swift", "swift"],
  [".m", "objective-c"],
  [".mm", "objective-c"],
  [".cpp", "cpp"],
  [".cxx", "cpp"],
  [".cc", "cpp"],
  [".hpp", "cpp"],
  [".hxx", "cpp"],
  [".hh", "cpp"],
  [".c", "c"],
  [".h", "c"],
  [".sql", "sql"],
  [".lua", "lua"],
  [".dart", "dart"],
  [".r", "r"],
  [".jl", "julia"],
  [".clj", "clojure"],
  [".cljs", "clojure"],
  [".cljc", "clojure"],
  [".ex", "elixir"],
  [".exs", "elixir"],
  [".erl", "erlang"],
  [".hs", "haskell"],
  [".pl", "perl"],
  [".ps1", "powershell"],
  [".bat", "bat"],
  [".cmd", "bat"],
  [".ini", "ini"],
  [".toml", "ini"],
  [".dockerfile", "dockerfile"],
  [".graphql", "graphql"],
  [".gql", "graphql"],
  [".tex", "latex"],
  [".xml", "xml"],
  [".svg", "xml"],
  [".json", "json"],
  [".jsonc", "json"],
  [".json5", "json"],
  [".md", "markdown"],
  [".markdown", "markdown"],
  [".css", "css"],
  [".scss", "scss"],
  [".less", "less"],
  [".html", "html"],
  [".htm", "html"],
  [".vue", "html"],
  [".yml", "yaml"],
  [".yaml", "yaml"],
  [".sh", "shell"],
  [".bash", "shell"],
  [".zsh", "shell"],
  [".fish", "shell"],
];

/// Every distinct Monaco language id we map extensions to. The LSP
/// providers register once for each of these and no-op per call via
/// `hasLspServer` — see `editor-lsp-providers.ts`.
export function allKnownLanguageIds(): string[] {
  return [...new Set([...declared, ...EXTENSION_MAP].map(([, id]) => id))];
}

/// Whether a language can use LSP features is data-driven now: see
/// `hasLspServer` in `lsp-servers-store.ts` (any language with a
/// configured or installed server qualifies).
