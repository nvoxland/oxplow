/** Maps a file path's extension to a Monaco language id. The set of
 *  ids matches Monaco's bundled basic-languages registrations — adding
 *  an entry here is enough to light up syntax highlighting for that
 *  extension. Falls back to `plaintext` for anything unrecognized. */
export function languageForPath(path: string | null): string {
  if (!path) return "plaintext";
  const lower = path.toLowerCase();
  for (const [suffix, id] of EXTENSION_MAP) {
    if (lower.endsWith(suffix)) return id;
  }
  return "plaintext";
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
  return [...new Set(EXTENSION_MAP.map(([, id]) => id))];
}

/// Whether a language can use LSP features is data-driven now: see
/// `hasLspServer` in `lsp-servers-store.ts` (any language with a
/// configured or installed server qualifies).
