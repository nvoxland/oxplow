//! The editor's syntax highlighting by file extension, for the files no
//! enabled language pack declares: the Monaco id a filesystem's `stat`
//! reports as `language` when no language claims the extension. A language
//! that declares its files (`languages: - { editor: { monaco } }`) always
//! wins over this table, and no oxplow analysis language relies on it: it
//! is Monaco's own id space (`apps/desktop/src/editor-language.ts` is the
//! desktop's copy, which draws the editor: a test below holds the two
//! equal), not the `Language` of `oxplow-code-metrics`, which names only
//! the languages a pack declares (`.context/language-support.md`).

/// Extension suffix → language id, first match wins (the order of the
/// desktop's table).
const EXTENSION_MAP: &[(&str, &str)] = &[
    (".tsx", "typescript"),
    (".ts", "typescript"),
    (".mts", "typescript"),
    (".cts", "typescript"),
    (".jsx", "javascript"),
    (".mjs", "javascript"),
    (".cjs", "javascript"),
    (".js", "javascript"),
    (".rs", "rust"),
    (".py", "python"),
    (".pyi", "python"),
    (".go", "go"),
    (".java", "java"),
    (".kt", "kotlin"),
    (".kts", "kotlin"),
    (".scala", "scala"),
    (".rb", "ruby"),
    (".php", "php"),
    (".cs", "csharp"),
    (".swift", "swift"),
    (".m", "objective-c"),
    (".mm", "objective-c"),
    (".cpp", "cpp"),
    (".cxx", "cpp"),
    (".cc", "cpp"),
    (".hpp", "cpp"),
    (".hxx", "cpp"),
    (".hh", "cpp"),
    (".c", "c"),
    (".h", "c"),
    (".sql", "sql"),
    (".lua", "lua"),
    (".dart", "dart"),
    (".r", "r"),
    (".jl", "julia"),
    (".clj", "clojure"),
    (".cljs", "clojure"),
    (".cljc", "clojure"),
    (".ex", "elixir"),
    (".exs", "elixir"),
    (".erl", "erlang"),
    (".hs", "haskell"),
    (".pl", "perl"),
    (".ps1", "powershell"),
    (".bat", "bat"),
    (".cmd", "bat"),
    (".ini", "ini"),
    (".toml", "ini"),
    (".dockerfile", "dockerfile"),
    (".graphql", "graphql"),
    (".gql", "graphql"),
    (".tex", "latex"),
    (".xml", "xml"),
    (".svg", "xml"),
    (".json", "json"),
    (".jsonc", "json"),
    (".json5", "json"),
    (".md", "markdown"),
    (".markdown", "markdown"),
    (".css", "css"),
    (".scss", "scss"),
    (".less", "less"),
    (".html", "html"),
    (".htm", "html"),
    (".vue", "html"),
    (".yml", "yaml"),
    (".yaml", "yaml"),
    (".sh", "shell"),
    (".bash", "shell"),
    (".zsh", "shell"),
    (".fish", "shell"),
];

/// The highlighting fallback for `path`, or `None` for an unrecognized
/// extension (the desktop draws that as `plaintext`).
pub fn highlighting_fallback_for_path(path: &str) -> Option<&'static str> {
    let lower = path.to_lowercase();
    EXTENSION_MAP
        .iter()
        .find(|(suffix, _)| lower.ends_with(suffix))
        .map(|(_, id)| *id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_extension_names_its_language_case_insensitively() {
        assert_eq!(highlighting_fallback_for_path("src/a.rs"), Some("rust"));
        assert_eq!(highlighting_fallback_for_path("A.TSX"), Some("typescript"));
        assert_eq!(highlighting_fallback_for_path("a.yaml"), Some("yaml"));
        assert_eq!(highlighting_fallback_for_path("Makefile"), None);
        assert_eq!(highlighting_fallback_for_path("a.bin"), None);
    }

    /// The desktop keeps its own copy of the table (Monaco is its own
    /// world); this keeps the two from drifting.
    #[test]
    fn the_desktop_table_is_this_table() {
        let ts = include_str!("../../../apps/desktop/src/editor-language.ts");
        let mut theirs = Vec::new();
        for line in ts.lines() {
            let line = line.trim();
            if let Some(rest) = line.strip_prefix("[\"") {
                let (suffix, rest) = rest.split_once("\", \"").expect("a pair");
                let (id, _) = rest.split_once('"').expect("a pair");
                theirs.push((suffix.to_string(), id.to_string()));
            }
        }
        let ours: Vec<(String, String)> = EXTENSION_MAP
            .iter()
            .map(|(s, i)| (s.to_string(), i.to_string()))
            .collect();
        assert_eq!(ours, theirs);
    }
}
