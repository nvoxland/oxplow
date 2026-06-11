/**
 * Hand-curated mapping from Monaco language id to a Mason-registry
 * package name. Used by the editor's "Install language server"
 * affordance when no LSP is configured for the file's language.
 *
 * Keep this list short — the goal is to cover the obvious defaults,
 * not every Mason package. Users who want something off this list
 * can install via a future "All packages" picker.
 */
export const SUGGESTIONS: Record<string, string> = {
  rust: "rust-analyzer",
  go: "gopls",
  typescript: "typescript-language-server",
  javascript: "typescript-language-server",
  typescriptreact: "typescript-language-server",
  javascriptreact: "typescript-language-server",
  python: "pyright",
  lua: "lua-language-server",
  c: "clangd",
  cpp: "clangd",
  json: "json-lsp",
  yaml: "yaml-language-server",
  html: "html-lsp",
  css: "css-lsp",
  bash: "bash-language-server",
  shell: "bash-language-server",
  ruby: "ruby-lsp",
  zig: "zls",
};

export function getSuggestedLspPackage(languageId: string): string | null {
  return SUGGESTIONS[languageId] ?? null;
}
