//! The per-language plugin registry — one cohesive bundle per language
//! (tsk322, epic tsk320).
//!
//! `LanguagePlugin` is the single place that answers *"what does oxplow know
//! about language X?"*: its identity, file extensions, the curated LSP server
//! suggestion, and the code units it exposes. It is the source of truth for
//! the cross-cutting facets that used to be scattered across crates — the
//! Mason LSP suggestion map (was in `oxplow-app`), the extension→language
//! table (was a second match here), and the unit kinds a language declares.
//!
//! The static-analysis node tables (`LanguageSpec`: function/decision/
//! container kinds) stay in [`crate::spec`] and are reached via
//! [`LanguagePlugin::analysis_spec`]. The Tier-2 AST merge tables
//! (`MergeSpec`) stay in `oxplow-git` (merge-specific) and the idiom-metric
//! scripts stay in `oxplow-collect-plugin` (they need the Starlark runtime) —
//! both key off this same `Language` enum, so identity stays unified.
//!
//! **Compiled-in.** Language support ships with oxplow (decision tsk323): no
//! dynamic grammar loading. Adding a language is a single, well-isolated
//! change — one entry in this registry plus its `spec.rs` node tables (and,
//! if it should merge, an `oxplow-git` `MergeSpec`). See
//! `.context/language-plugins.md`.

use crate::spec::{language_from_lsp_id, Language, LanguageSpec};

/// A kind of code "unit" a language exposes, beyond the always-present
/// function — for the list-units navigation surface and the `metric_subject`
/// roll-up hierarchy (exercised in tsk325). Declared per language here; the
/// extraction wiring consumes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnitKind {
    /// A function / method / closure (every language has these).
    Function,
    /// A class-like type: class, struct, record, interface, impl block,
    /// trait, enum.
    Class,
    /// A namespace / module / `mod` / Clojure `ns`.
    Module,
    /// A package (Go package, Java package) — a path-level grouping.
    Package,
}

impl UnitKind {
    /// Stable lowercase tag for serialization / display.
    pub fn as_str(&self) -> &'static str {
        match self {
            UnitKind::Function => "function",
            UnitKind::Class => "class",
            UnitKind::Module => "module",
            UnitKind::Package => "package",
        }
    }
}

/// Everything oxplow knows about one language, in one place.
#[derive(Debug, Clone, Copy)]
pub struct LanguagePlugin {
    /// Canonical identity (the single workspace-wide `Language`).
    pub language: Language,
    /// Human-facing name (e.g. `"C++"`, `"C#"`).
    pub display_name: &'static str,
    /// File extensions (lowercase, no leading dot) this language claims.
    /// The single source of truth for [`language_for_path`].
    pub extensions: &'static [&'static str],
    /// Curated Mason-registry package suggestion for this language's LSP
    /// server, if oxplow ships one. `None` ⇒ no built-in suggestion.
    pub lsp_mason_package: Option<&'static str>,
    /// Code unit kinds this language exposes (always includes
    /// [`UnitKind::Function`]).
    pub unit_kinds: &'static [UnitKind],
}

impl LanguagePlugin {
    /// The static-analysis node tables for this language (complexity /
    /// container / decision-point kinds). Lives in [`crate::spec`].
    pub fn analysis_spec(&self) -> &'static LanguageSpec {
        self.language.spec()
    }
}

/// The bundled language registry — one entry per [`Language`] variant.
static REGISTRY: &[LanguagePlugin] = &[
    LanguagePlugin {
        language: Language::Rust,
        display_name: "Rust",
        extensions: &["rs"],
        lsp_mason_package: Some("rust-analyzer"),
        unit_kinds: &[UnitKind::Function, UnitKind::Class, UnitKind::Module],
    },
    LanguagePlugin {
        language: Language::TypeScript,
        display_name: "TypeScript",
        extensions: &["ts"],
        lsp_mason_package: Some("typescript-language-server"),
        unit_kinds: &[UnitKind::Function, UnitKind::Class, UnitKind::Module],
    },
    LanguagePlugin {
        language: Language::Tsx,
        display_name: "TSX",
        extensions: &["tsx"],
        lsp_mason_package: Some("typescript-language-server"),
        unit_kinds: &[UnitKind::Function, UnitKind::Class, UnitKind::Module],
    },
    LanguagePlugin {
        language: Language::JavaScript,
        display_name: "JavaScript",
        extensions: &["js", "mjs", "cjs", "jsx"],
        lsp_mason_package: Some("typescript-language-server"),
        unit_kinds: &[UnitKind::Function, UnitKind::Class],
    },
    LanguagePlugin {
        language: Language::Python,
        display_name: "Python",
        extensions: &["py"],
        lsp_mason_package: Some("pyright"),
        unit_kinds: &[UnitKind::Function, UnitKind::Class, UnitKind::Module],
    },
    LanguagePlugin {
        language: Language::Go,
        display_name: "Go",
        extensions: &["go"],
        lsp_mason_package: Some("gopls"),
        unit_kinds: &[UnitKind::Function, UnitKind::Package],
    },
    LanguagePlugin {
        language: Language::Java,
        display_name: "Java",
        extensions: &["java"],
        lsp_mason_package: None,
        unit_kinds: &[UnitKind::Function, UnitKind::Class, UnitKind::Package],
    },
    LanguagePlugin {
        language: Language::C,
        display_name: "C",
        extensions: &["c", "h"],
        lsp_mason_package: Some("clangd"),
        unit_kinds: &[UnitKind::Function],
    },
    LanguagePlugin {
        language: Language::Cpp,
        display_name: "C++",
        extensions: &["cc", "cxx", "cpp", "hpp", "hxx"],
        lsp_mason_package: Some("clangd"),
        unit_kinds: &[UnitKind::Function, UnitKind::Class, UnitKind::Module],
    },
    LanguagePlugin {
        language: Language::Clojure,
        display_name: "Clojure",
        extensions: &["clj", "cljs", "cljc"],
        lsp_mason_package: None,
        unit_kinds: &[UnitKind::Function, UnitKind::Module],
    },
    LanguagePlugin {
        language: Language::CSharp,
        display_name: "C#",
        extensions: &["cs"],
        lsp_mason_package: None,
        unit_kinds: &[UnitKind::Function, UnitKind::Class, UnitKind::Module],
    },
];

/// The bundled language registry — one [`LanguagePlugin`] per supported
/// language.
pub fn registry() -> &'static [LanguagePlugin] {
    REGISTRY
}

/// The plugin bundle for a given language. Every `Language` variant has
/// exactly one registry entry (guarded by a test), so this never returns
/// `None` for a valid variant.
pub fn for_language(language: Language) -> &'static LanguagePlugin {
    registry()
        .iter()
        .find(|p| p.language == language)
        .expect("every Language variant has a registry entry")
}

/// Resolve a path to its language by file extension, via the registry's
/// `extensions` (the single source of truth). Case-insensitive on the
/// extension. `None` ⇒ unsupported.
pub fn language_for_path(path: &str) -> Option<Language> {
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())?
        .to_ascii_lowercase();
    registry()
        .iter()
        .find(|p| p.extensions.contains(&ext.as_str()))
        .map(|p| p.language)
}

/// The curated Mason package suggestion for an LSP `languageId`. Resolves
/// analysis languages through the registry (the bundle's `lsp_mason_package`)
/// and falls back to a small table of **LSP-only** languages oxplow doesn't
/// statically analyse (no `Language` variant). The single source of truth
/// behind `oxplow-app`'s `mason_suggestion` and the renderer mirror
/// `apps/desktop/src/lspSuggestions.ts` (keep that mirror in sync by hand).
pub fn mason_suggestion(language_id: &str) -> Option<&'static str> {
    if let Some(lang) = language_from_lsp_id(language_id) {
        if let Some(pkg) = for_language(lang).lsp_mason_package {
            return Some(pkg);
        }
    }
    // LSP-only languages — no static analysis grammar, so no registry entry.
    match language_id.trim().to_ascii_lowercase().as_str() {
        "lua" => Some("lua-language-server"),
        "json" => Some("json-lsp"),
        "yaml" => Some("yaml-language-server"),
        "html" => Some("html-lsp"),
        "css" => Some("css-lsp"),
        "bash" | "shell" => Some("bash-language-server"),
        "ruby" => Some("ruby-lsp"),
        "zig" => Some("zls"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: &[Language] = &[
        Language::Rust,
        Language::TypeScript,
        Language::Tsx,
        Language::JavaScript,
        Language::Python,
        Language::Go,
        Language::Java,
        Language::C,
        Language::Cpp,
        Language::Clojure,
        Language::CSharp,
    ];

    #[test]
    fn registry_has_exactly_one_entry_per_language() {
        assert_eq!(registry().len(), ALL.len());
        for lang in ALL {
            let matches = registry().iter().filter(|p| p.language == *lang).count();
            assert_eq!(matches, 1, "expected one registry entry for {lang:?}");
            // Every language exposes functions, and resolves to its spec.
            let plugin = for_language(*lang);
            assert!(plugin.unit_kinds.contains(&UnitKind::Function));
            let _ = plugin.analysis_spec();
        }
    }

    #[test]
    fn language_for_path_resolves_via_registry_extensions() {
        assert_eq!(language_for_path("a/b.rs"), Some(Language::Rust));
        assert_eq!(language_for_path("x.ts"), Some(Language::TypeScript));
        assert_eq!(language_for_path("x.tsx"), Some(Language::Tsx));
        assert_eq!(language_for_path("x.js"), Some(Language::JavaScript));
        assert_eq!(language_for_path("x.mjs"), Some(Language::JavaScript));
        assert_eq!(language_for_path("x.cjs"), Some(Language::JavaScript));
        assert_eq!(language_for_path("x.jsx"), Some(Language::JavaScript));
        assert_eq!(language_for_path("x.py"), Some(Language::Python));
        assert_eq!(language_for_path("x.go"), Some(Language::Go));
        assert_eq!(language_for_path("x.java"), Some(Language::Java));
        assert_eq!(language_for_path("x.c"), Some(Language::C));
        assert_eq!(language_for_path("x.h"), Some(Language::C));
        assert_eq!(language_for_path("x.CPP"), Some(Language::Cpp)); // case-insensitive
        assert_eq!(language_for_path("x.hpp"), Some(Language::Cpp));
        assert_eq!(language_for_path("x.clj"), Some(Language::Clojure));
        assert_eq!(language_for_path("x.cs"), Some(Language::CSharp));
        assert_eq!(language_for_path("x.txt"), None);
        assert_eq!(language_for_path("noext"), None);
    }

    /// Parity guard: the registry-backed `mason_suggestion` must reproduce
    /// the exact mapping that lived in `oxplow-app::lsp_sessions` before the
    /// registry became the source of truth.
    #[test]
    fn mason_suggestion_matches_legacy_map() {
        assert_eq!(mason_suggestion("rust"), Some("rust-analyzer"));
        assert_eq!(mason_suggestion("go"), Some("gopls"));
        for id in [
            "typescript",
            "javascript",
            "typescriptreact",
            "javascriptreact",
        ] {
            assert_eq!(mason_suggestion(id), Some("typescript-language-server"));
        }
        assert_eq!(mason_suggestion("python"), Some("pyright"));
        assert_eq!(mason_suggestion("lua"), Some("lua-language-server"));
        assert_eq!(mason_suggestion("c"), Some("clangd"));
        assert_eq!(mason_suggestion("cpp"), Some("clangd"));
        assert_eq!(mason_suggestion("json"), Some("json-lsp"));
        assert_eq!(mason_suggestion("yaml"), Some("yaml-language-server"));
        assert_eq!(mason_suggestion("html"), Some("html-lsp"));
        assert_eq!(mason_suggestion("css"), Some("css-lsp"));
        assert_eq!(mason_suggestion("bash"), Some("bash-language-server"));
        assert_eq!(mason_suggestion("shell"), Some("bash-language-server"));
        assert_eq!(mason_suggestion("ruby"), Some("ruby-lsp"));
        assert_eq!(mason_suggestion("zig"), Some("zls"));
        // Analysis languages with no curated server (unchanged: no suggestion).
        assert_eq!(mason_suggestion("java"), None);
        assert_eq!(mason_suggestion("clojure"), None);
        assert_eq!(mason_suggestion("csharp"), None);
        assert_eq!(mason_suggestion("unknown-lang"), None);
    }

    /// Drift guard: the renderer mirror `apps/desktop/src/lspSuggestions.ts`
    /// is kept in sync with [`mason_suggestion`] (the single source of truth)
    /// by hand. Parse its `SUGGESTIONS` map and assert every entry reproduces
    /// what `mason_suggestion` returns, so a stale TS value fails the build
    /// instead of silently shipping a wrong "Install language server" hint.
    #[test]
    fn lsp_suggestions_ts_mirror_matches_mason_suggestion() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../apps/desktop/src/lspSuggestions.ts"
        );
        let src = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {path}: {e}"));

        // Walk the lines inside the `SUGGESTIONS` object literal, parsing the
        // `key: "value",` entries.
        let mut in_map = false;
        let mut parsed = 0usize;
        for line in src.lines() {
            let t = line.trim();
            if t.starts_with("export const SUGGESTIONS") {
                in_map = true;
                continue;
            }
            if !in_map {
                continue;
            }
            if t.starts_with("};") {
                break;
            }
            // `key: "value",` — key is a bare or quoted ident.
            let Some((raw_key, rest)) = t.split_once(':') else {
                continue;
            };
            let key = raw_key.trim().trim_matches('"');
            let Some(open) = rest.find('"') else { continue };
            let after = &rest[open + 1..];
            let Some(close) = after.find('"') else {
                continue;
            };
            let value = &after[..close];
            assert_eq!(
                mason_suggestion(key),
                Some(value),
                "lspSuggestions.ts drift: {key:?} → {value:?} but mason_suggestion gives {:?}",
                mason_suggestion(key),
            );
            parsed += 1;
        }
        // Guard against a broken parser passing vacuously.
        assert!(
            parsed >= 15,
            "parsed only {parsed} mirror entries — parser likely broken",
        );
    }
}
