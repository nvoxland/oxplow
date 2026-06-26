//! Per-language tree-sitter node-name tables.
//!
//! Each `LanguageSpec` says:
//! - which AST node kinds are "function-like" (so we count them as
//!   metric subjects),
//! - which child field name holds the function name,
//! - which field holds the parameter list and which child kinds count
//!   as one parameter,
//! - which AST node kinds count as a "decision point" for cyclomatic
//!   complexity (one branch per occurrence inside the function body).
//!
//! Decision-point sets follow McCabe's classic definition: every
//! `if`, `else if`, `case`, `catch`, `for`, `while`, `&&`, `||`, plus
//! ternary expressions.

use tree_sitter::Language as TsLanguage;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Language {
    Rust,
    TypeScript,
    Tsx,
    JavaScript,
    Python,
    Go,
    Java,
    C,
    Cpp,
    /// Clojure / ClojureScript / cljc — share the same grammar and
    /// idioms. tree-sitter-clojure represents every parenthesized
    /// form as a generic `list_lit`, so function and
    /// decision-point detection use the head-symbol-text matchers
    /// (`function_form_heads`, `decision_form_heads`) on
    /// `LanguageSpec` rather than the static-`node.kind()` arrays
    /// that suffice for the other ten languages.
    Clojure,
    /// C#. Standard `node.kind()`-driven detection like the other
    /// non-Clojure languages.
    CSharp,
}

pub struct LanguageSpec {
    /// AST node kinds that represent a function/method/closure body.
    pub function_kinds: &'static [&'static str],
    /// Field names to try (in order) for the function's identifier.
    pub name_fields: &'static [&'static str],
    /// Field names to try (in order) for the parameter list.
    pub param_list_fields: &'static [&'static str],
    /// AST node kinds inside a parameter list that represent one parameter.
    pub parameter_kinds: &'static [&'static str],
    /// AST node kinds that increment cyclomatic complexity by 1.
    pub decision_kinds: &'static [&'static str],
    /// AST node kinds that act as named containers for the
    /// hierarchical "where does this function live" path
    /// (class, impl, module, namespace, etc.).
    pub container_kinds: &'static [&'static str],
    /// Field names tried (in order) on a container node to locate
    /// its identifier. `name` covers most languages; Rust's
    /// `impl_item` exposes the type via `type` instead, so we try
    /// multiple fields.
    pub container_name_fields: &'static [&'static str],
    /// Strategy used to classify a function's visibility (public /
    /// private / unknown). The strategy is language-specific because
    /// each language encodes "private" differently — modifier nodes
    /// (Java/TS/Rust), name conventions (Python `_foo`, Go
    /// capitalization), or scope-based (C `static`, TS top-level
    /// `export`).
    pub visibility: VisibilityStrategy,
    /// Form-head matchers for grammars whose AST collapses every
    /// parenthesized form to a single generic node kind (Clojure's
    /// `list_lit`). When non-empty, a node whose kind is `list_lit`
    /// AND whose first symbol child's text is in this list is
    /// treated as a function in addition to anything in
    /// `function_kinds`. Most languages leave this empty.
    pub function_form_heads: &'static [&'static str],
    /// Same shape as `function_form_heads` but for decision points
    /// (cyclomatic complexity counter).
    pub decision_form_heads: &'static [&'static str],
    /// Loader for the bundled tree-sitter grammar.
    grammar: fn() -> TsLanguage,
}

#[derive(Debug, Clone, Copy)]
pub enum VisibilityStrategy {
    /// Look for a `visibility_modifier` child whose text starts with
    /// `pub`. Present → public; absent → private.
    RustModifier,
    /// Look for an `accessibility_modifier` child on class/method
    /// nodes (`private`, `protected`, `public`). Present → mapped
    /// directly. Absent on a method inside a class → public (TS
    /// default). Absent on a top-level function → check enclosing
    /// `export_statement` ancestor.
    TsClassModifier,
    /// Java-style: look for an `identifier`-typed `modifiers` child
    /// containing `private`/`protected`/`public`.
    JavaModifier,
    /// Walk up to the nearest `class_specifier` and look at the
    /// preceding `access_specifier` to decide. C++ default is
    /// private inside a `class`, public inside a `struct`.
    CppAccessSpecifier,
    /// Capitalization of the identifier first letter. Uppercase →
    /// public; lowercase → private.
    GoCapitalization,
    /// Leading-underscore convention. Function name starts with `_`
    /// → private.
    PythonUnderscore,
    /// `static` storage class on the function definition → private
    /// (file-scope). Anything else → public.
    CStatic,
    /// All functions reported as Unknown — used for languages where
    /// no clean signal exists.
    Unknown,
    /// Clojure: head sym `defn-` ⇒ Private. A `meta_lit` /
    /// `old_meta_lit` child whose text contains `:private` ⇒
    /// Private. Otherwise Public.
    ClojureForm,
}

impl LanguageSpec {
    pub fn tree_sitter_language(&self) -> TsLanguage {
        (self.grammar)()
    }
}

impl Language {
    pub fn spec(&self) -> &'static LanguageSpec {
        match self {
            Language::Rust => &RUST,
            Language::TypeScript => &TYPESCRIPT,
            Language::Tsx => &TSX,
            Language::JavaScript => &JAVASCRIPT,
            Language::Python => &PYTHON,
            Language::Go => &GO,
            Language::Java => &JAVA,
            Language::C => &C,
            Language::Cpp => &CPP,
            Language::Clojure => &CLOJURE,
            Language::CSharp => &CSHARP,
        }
    }

    /// Convenience for callers (e.g. `oxplow-code-dup`) that need the
    /// raw tree-sitter grammar without going through `LanguageSpec`.
    pub fn tree_sitter_language(&self) -> TsLanguage {
        self.spec().tree_sitter_language()
    }

    /// Canonical lowercase name — the inverse of [`language_from_name`]
    /// (round-trips), and the value `source_files()` tags each file with so
    /// scripts can pass it straight back to `code_metrics`/`markers`.
    pub fn name(&self) -> &'static str {
        match self {
            Language::Rust => "rust",
            Language::TypeScript => "typescript",
            Language::Tsx => "tsx",
            Language::JavaScript => "javascript",
            Language::Python => "python",
            Language::Go => "go",
            Language::Java => "java",
            Language::C => "c",
            Language::Cpp => "cpp",
            Language::Clojure => "clojure",
            Language::CSharp => "csharp",
        }
    }

    /// Stable u8 tag used as a hash salt so cross-language token
    /// streams can never collide.
    pub fn tag(&self) -> u8 {
        match self {
            Language::Rust => 1,
            Language::TypeScript => 2,
            Language::Tsx => 3,
            Language::JavaScript => 4,
            Language::Python => 5,
            Language::Go => 6,
            Language::Java => 7,
            Language::C => 8,
            Language::Cpp => 9,
            Language::Clojure => 10,
            Language::CSharp => 11,
        }
    }
}

/// Resolve a language by name (the string a `metrics:` plugin passes to
/// `ast_query`). Accepts common aliases; case-insensitive.
pub fn language_from_name(name: &str) -> Option<Language> {
    Some(match name.trim().to_ascii_lowercase().as_str() {
        "rust" | "rs" => Language::Rust,
        "typescript" | "ts" => Language::TypeScript,
        "tsx" => Language::Tsx,
        "javascript" | "js" | "jsx" => Language::JavaScript,
        "python" | "py" => Language::Python,
        "go" | "golang" => Language::Go,
        "java" => Language::Java,
        "c" => Language::C,
        "cpp" | "c++" | "cxx" | "cc" => Language::Cpp,
        "clojure" | "clj" | "cljs" | "cljc" => Language::Clojure,
        "csharp" | "c#" | "cs" => Language::CSharp,
        _ => return None,
    })
}

// Path-extension → language resolution lives in [`crate::plugin`] now: the
// per-language `LanguagePlugin::extensions` is the single source of truth
// (`plugin::language_for_path`). See `.context/language-plugins.md`.

/// Resolve an LSP `languageId` (the string a language server / editor
/// uses to label a buffer, e.g. `"typescriptreact"`) to the canonical
/// analysis [`Language`].
///
/// This is the **single bridge** between the LSP namespace — free strings
/// keyed per session in `oxplow-app/src/lsp_sessions.rs` — and the
/// static-analysis enum. LSP carries a few ids that [`language_from_name`]
/// doesn't (`typescriptreact`/`javascriptreact`); everything else delegates
/// to it. An LSP language with no analysis grammar (e.g. `lua`, `json`,
/// `yaml`) resolves to `None`, which callers read as "no tree-sitter
/// analysis for this buffer" — LSP features still work independently.
pub fn language_from_lsp_id(language_id: &str) -> Option<Language> {
    match language_id.trim().to_ascii_lowercase().as_str() {
        "typescriptreact" => Some(Language::Tsx),
        "javascriptreact" => Some(Language::JavaScript),
        other => language_from_name(other),
    }
}

// ---- Rust ----

static RUST: LanguageSpec = LanguageSpec {
    function_kinds: &["function_item", "closure_expression"],
    name_fields: &["name"],
    param_list_fields: &["parameters"],
    parameter_kinds: &["parameter", "self_parameter"],
    decision_kinds: &[
        "if_expression",
        // `else_clause` deliberately excluded — McCabe counts only
        // condition-bearing branches, so an `if/else if/else` chain
        // adds 2 (the two ifs), not 4.
        "match_arm",
        "while_expression",
        "for_expression",
        "loop_expression",
        "try_expression",
        // boolean operators (&&, ||) are tokens inside binary_expression,
        // not their own nodes — skipping for simplicity.
    ],
    container_kinds: &["impl_item", "trait_item", "mod_item"],
    container_name_fields: &["name", "type"],
    visibility: VisibilityStrategy::RustModifier,
    function_form_heads: &[],
    decision_form_heads: &[],
    grammar: || tree_sitter_rust::LANGUAGE.into(),
};

// ---- TypeScript / TSX / JavaScript ----

static TYPESCRIPT: LanguageSpec = LanguageSpec {
    function_kinds: JS_FUNCTION_KINDS,
    name_fields: JS_NAME_FIELDS,
    param_list_fields: JS_PARAM_FIELDS,
    parameter_kinds: JS_PARAM_KINDS,
    decision_kinds: JS_DECISION_KINDS,
    container_kinds: TS_CONTAINER_KINDS,
    container_name_fields: JS_NAME_FIELDS,
    visibility: VisibilityStrategy::TsClassModifier,
    function_form_heads: &[],
    decision_form_heads: &[],
    grammar: || tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
};

static TSX: LanguageSpec = LanguageSpec {
    function_kinds: JS_FUNCTION_KINDS,
    name_fields: JS_NAME_FIELDS,
    param_list_fields: JS_PARAM_FIELDS,
    parameter_kinds: JS_PARAM_KINDS,
    decision_kinds: JS_DECISION_KINDS,
    container_kinds: TS_CONTAINER_KINDS,
    container_name_fields: JS_NAME_FIELDS,
    visibility: VisibilityStrategy::TsClassModifier,
    function_form_heads: &[],
    decision_form_heads: &[],
    grammar: || tree_sitter_typescript::LANGUAGE_TSX.into(),
};

static JAVASCRIPT: LanguageSpec = LanguageSpec {
    function_kinds: JS_FUNCTION_KINDS,
    name_fields: JS_NAME_FIELDS,
    param_list_fields: JS_PARAM_FIELDS,
    parameter_kinds: JS_PARAM_KINDS,
    decision_kinds: JS_DECISION_KINDS,
    container_kinds: JS_CONTAINER_KINDS,
    container_name_fields: JS_NAME_FIELDS,
    visibility: VisibilityStrategy::TsClassModifier,
    function_form_heads: &[],
    decision_form_heads: &[],
    grammar: || tree_sitter_javascript::LANGUAGE.into(),
};

static TS_CONTAINER_KINDS: &[&str] = &[
    "class_declaration",
    "class",
    "abstract_class_declaration",
    "interface_declaration",
    "internal_module",
    "module",
    "namespace_declaration",
    "enum_declaration",
];
static JS_CONTAINER_KINDS: &[&str] = &["class_declaration", "class"];

static JS_FUNCTION_KINDS: &[&str] = &[
    "function_declaration",
    "function_expression",
    "arrow_function",
    "method_definition",
    "generator_function",
    "generator_function_declaration",
    "function_signature",
];
static JS_NAME_FIELDS: &[&str] = &["name"];
static JS_PARAM_FIELDS: &[&str] = &["parameters"];
static JS_PARAM_KINDS: &[&str] = &[
    "required_parameter",
    "optional_parameter",
    "rest_parameter",
    "identifier",
    "assignment_pattern",
    "object_pattern",
    "array_pattern",
];
static JS_DECISION_KINDS: &[&str] = &[
    "if_statement",
    // `else_clause` deliberately excluded for the same reason as Rust.
    "switch_case",
    "switch_default",
    "for_statement",
    "for_in_statement",
    "while_statement",
    "do_statement",
    "catch_clause",
    "ternary_expression",
];

// ---- Python ----

static PYTHON: LanguageSpec = LanguageSpec {
    function_kinds: &["function_definition", "lambda"],
    name_fields: &["name"],
    param_list_fields: &["parameters"],
    parameter_kinds: &[
        "identifier",
        "default_parameter",
        "typed_parameter",
        "typed_default_parameter",
        "list_splat_pattern",
        "dictionary_splat_pattern",
        "tuple_pattern",
    ],
    decision_kinds: &[
        "if_statement",
        "elif_clause",
        "for_statement",
        "while_statement",
        "try_statement",
        "except_clause",
        "with_statement",
        "conditional_expression",
        "boolean_operator",
        "match_statement",
        "case_clause",
    ],
    container_kinds: &["class_definition"],
    container_name_fields: &["name"],
    visibility: VisibilityStrategy::PythonUnderscore,
    function_form_heads: &[],
    decision_form_heads: &[],
    grammar: || tree_sitter_python::LANGUAGE.into(),
};

// ---- Go ----

static GO: LanguageSpec = LanguageSpec {
    function_kinds: &["function_declaration", "method_declaration", "func_literal"],
    name_fields: &["name"],
    param_list_fields: &["parameters"],
    parameter_kinds: &["parameter_declaration", "variadic_parameter_declaration"],
    decision_kinds: &[
        "if_statement",
        "for_statement",
        "type_switch_statement",
        "expression_switch_statement",
        "type_case",
        "expression_case",
        "select_statement",
        "communication_case",
    ],
    // Go has no class-like containers; the package is implicit at
    // the file level so there's nothing meaningful to attach.
    container_kinds: &[],
    container_name_fields: &["name"],
    visibility: VisibilityStrategy::GoCapitalization,
    function_form_heads: &[],
    decision_form_heads: &[],
    grammar: || tree_sitter_go::LANGUAGE.into(),
};

// ---- Java ----

static JAVA: LanguageSpec = LanguageSpec {
    function_kinds: &[
        "method_declaration",
        "constructor_declaration",
        "lambda_expression",
    ],
    name_fields: &["name"],
    param_list_fields: &["parameters"],
    parameter_kinds: &["formal_parameter", "spread_parameter"],
    decision_kinds: &[
        "if_statement",
        "switch_label",
        "switch_block_statement_group",
        "for_statement",
        "enhanced_for_statement",
        "while_statement",
        "do_statement",
        "catch_clause",
        "ternary_expression",
    ],
    container_kinds: &[
        "class_declaration",
        "interface_declaration",
        "enum_declaration",
        "record_declaration",
        "annotation_type_declaration",
    ],
    container_name_fields: &["name"],
    visibility: VisibilityStrategy::JavaModifier,
    function_form_heads: &[],
    decision_form_heads: &[],
    grammar: || tree_sitter_java::LANGUAGE.into(),
};

// ---- C ----

static C: LanguageSpec = LanguageSpec {
    function_kinds: &["function_definition"],
    name_fields: &["declarator"],
    param_list_fields: &["parameters"],
    parameter_kinds: &["parameter_declaration"],
    decision_kinds: &[
        "if_statement",
        "case_statement",
        "for_statement",
        "while_statement",
        "do_statement",
        "conditional_expression",
    ],
    // C has no class-like containers — top-level functions only.
    container_kinds: &[],
    container_name_fields: &["name"],
    visibility: VisibilityStrategy::CStatic,
    function_form_heads: &[],
    decision_form_heads: &[],
    grammar: || tree_sitter_c::LANGUAGE.into(),
};

// ---- C++ ----

static CPP: LanguageSpec = LanguageSpec {
    function_kinds: &["function_definition", "lambda_expression"],
    name_fields: &["declarator"],
    param_list_fields: &["parameters"],
    parameter_kinds: &["parameter_declaration", "optional_parameter_declaration"],
    decision_kinds: &[
        "if_statement",
        "case_statement",
        "for_statement",
        "for_range_loop",
        "while_statement",
        "do_statement",
        "catch_clause",
        "conditional_expression",
    ],
    container_kinds: &[
        "class_specifier",
        "struct_specifier",
        "namespace_definition",
    ],
    container_name_fields: &["name"],
    visibility: VisibilityStrategy::CppAccessSpecifier,
    function_form_heads: &[],
    decision_form_heads: &[],
    grammar: || tree_sitter_cpp::LANGUAGE.into(),
};

// ---- Clojure / ClojureScript / cljc ----
//
// tree-sitter-clojure flattens every parenthesized form to a
// generic `list_lit`, so `function_kinds` / `decision_kinds` are
// empty — detection runs through `function_form_heads` /
// `decision_form_heads` on the head symbol's text instead.

static CLOJURE: LanguageSpec = LanguageSpec {
    function_kinds: &[],
    name_fields: &[],
    param_list_fields: &[],
    parameter_kinds: &[],
    decision_kinds: &[],
    container_kinds: &[],
    container_name_fields: &[],
    visibility: VisibilityStrategy::ClojureForm,
    function_form_heads: &[
        "defn",
        "defn-",
        "defmacro",
        "defmethod",
        "definline",
        "fn",
        "fn*",
        "defprotocol",
    ],
    decision_form_heads: &[
        "if",
        "if-not",
        "if-let",
        "if-some",
        "when",
        "when-not",
        "when-let",
        "when-some",
        "cond",
        "condp",
        "case",
        "and",
        "or",
        "try",
    ],
    grammar: || tree_sitter_clojure_orchard::LANGUAGE.into(),
};

// ---- C# ----

static CSHARP: LanguageSpec = LanguageSpec {
    function_kinds: &[
        "method_declaration",
        "constructor_declaration",
        "destructor_declaration",
        "operator_declaration",
        "local_function_statement",
        "lambda_expression",
    ],
    name_fields: &["name"],
    param_list_fields: &["parameters"],
    parameter_kinds: &["parameter"],
    decision_kinds: &[
        "if_statement",
        // `else` is not condition-bearing — excluded (cf. Rust/JS).
        "for_statement",
        "for_each_statement",
        "while_statement",
        "do_statement",
        "case_switch_label",
        "case_pattern_switch_label",
        "switch_expression_arm",
        "catch_clause",
        "conditional_expression",
        "when_clause",
    ],
    container_kinds: &[
        "class_declaration",
        "struct_declaration",
        "interface_declaration",
        "record_declaration",
        "enum_declaration",
        "namespace_declaration",
        "file_scoped_namespace_declaration",
    ],
    container_name_fields: &["name"],
    // C# encodes visibility via `modifier` child nodes; no clean
    // single-signal strategy in the shared enum, so functions report
    // Unknown (none of the bundled C# metrics filter on visibility).
    visibility: VisibilityStrategy::Unknown,
    function_form_heads: &[],
    decision_form_heads: &[],
    grammar: || tree_sitter_c_sharp::LANGUAGE.into(),
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lsp_id_bridge_maps_react_variants_and_core_ids() {
        // LSP-only ids that language_from_name doesn't carry.
        assert!(matches!(
            language_from_lsp_id("typescriptreact"),
            Some(Language::Tsx)
        ));
        assert!(matches!(
            language_from_lsp_id("javascriptreact"),
            Some(Language::JavaScript)
        ));
        // Mixed case / whitespace tolerated.
        assert!(matches!(
            language_from_lsp_id("  TypeScriptReact "),
            Some(Language::Tsx)
        ));
        // Core ids delegate to language_from_name.
        assert!(matches!(language_from_lsp_id("rust"), Some(Language::Rust)));
        assert!(matches!(
            language_from_lsp_id("python"),
            Some(Language::Python)
        ));
        // An LSP language with no analysis grammar resolves to None.
        assert_eq!(language_from_lsp_id("lua"), None);
        assert_eq!(language_from_lsp_id("yaml"), None);
        assert_eq!(language_from_lsp_id("unknown-lang"), None);
    }

    #[test]
    fn lsp_id_bridge_round_trips_canonical_names() {
        // Every canonical language's own name() must resolve back through
        // the LSP bridge, so the LSP and analysis namespaces agree on the
        // core set.
        for lang in [
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
        ] {
            assert!(
                language_from_lsp_id(lang.name()).is_some(),
                "lsp bridge dropped canonical name {}",
                lang.name()
            );
        }
    }
}
