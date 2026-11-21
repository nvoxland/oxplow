//! A language, as an extension declares it.
//!
//! Core names no language. Each language is a `languages:` entry of an
//! extension's manifest (oxplow's language packs, `oxplow-rust` and the
//! rest, ship them) with its data files: `analysis.yaml` (what this crate's
//! walkers need: function and decision node kinds, the parameter list,
//! which visibility and doc strategy, and the optional import, merge and
//! duplication facets) and `symbols.scm` / `symbols.yaml` (what a file
//! defines, [`crate::symbols`]). [`Languages`] is the set the enabled
//! extensions declare, built at each publication; a [`Language`] is a
//! cheap handle into it.
//!
//! Per-language *behavior* that data can't say (how Rust spells `pub`, how
//! Python marks a docstring) is a strategy here, chosen by name from
//! `analysis.yaml`. A language needing a new one is a core change.
//!
//! Decision points follow McCabe's classic definition: every `if`,
//! `else if`, `case`, `catch`, `for`, `while`, `&&`, `||`, plus ternary
//! expressions, as each language's `decisions.kinds` lists them.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, OnceLock};

use serde::{Deserialize, Serialize};
use tree_sitter::Language as TsLanguage;

use crate::language::UnitKind;
use crate::symbols::SymbolSpec;

/// How a function's visibility is decided. Each names what it looks at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VisibilityStrategy {
    /// A `visibility_modifier` child whose text starts with `pub` →
    /// public; a function item without one → private.
    VisibilityModifier,
    /// An `accessibility_modifier` child on a class member (`private`,
    /// `protected`, `public`; a `#name` is private); a class member
    /// without one → public; a top-level function → public only inside an
    /// `export_statement`.
    AccessibilityModifier,
    /// A `modifiers` child holding `private` / `protected` / `public`.
    ModifiersKeyword,
    /// The `access_specifier` before it in its `class_specifier` (default
    /// private) or `struct_specifier` (default public).
    AccessSpecifier,
    /// An uppercase first letter → public.
    CapitalizedName,
    /// A leading `_` → private.
    UnderscorePrefix,
    /// A `static` storage class → private (file scope).
    StaticStorage,
    /// A form whose head is `defn-`, or whose metadata says `:private` →
    /// private.
    FormMeta,
    /// No signal: every function is unknown.
    Unknown,
}

/// How a documented function is recognized.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "strategy", rename_all = "snake_case", deny_unknown_fields)]
pub enum DocStrategy {
    /// A comment right before the item (attributes, decorators and
    /// annotations between are skipped) whose text starts with one of
    /// `prefixes`: a plain explanatory `//` isn't a doc comment.
    PrecedingComment { prefixes: Vec<String> },
    /// The body's first statement is a string literal (a docstring).
    FirstStringStatement,
    /// A string literal child of the form before its parameter vector.
    FormString,
    /// No documentation convention: always undocumented.
    None,
}

/// How a file's imports are read (`oxplow-code-deps`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportStrategy {
    RustUse,
    EsImport,
    PythonImport,
    GoImport,
    JavaImport,
    CInclude,
    CppInclude,
    ClojureNs,
    CsharpUsing,
}

/// How a top-level item's identity is computed for a structural merge
/// (`oxplow-git`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MergeKey {
    /// `transparent_kinds` seen through, then `name_fields`; falls back to
    /// kind + normalized text.
    Named,
    /// A function by its declarator (name and signature, so overloads stay
    /// apart), a type or namespace by name, a typedef or global by its
    /// declarator's identifier.
    CDeclarator,
    /// A `(def… name …)` form by head and name.
    FormHead,
}

/// The node kinds of a grammar where every parenthesized form is one node
/// kind and a form is told apart by its head symbol.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Forms {
    /// The kind of every form (`list_lit`).
    pub list: String,
    /// The kind of a symbol (`sym_lit`).
    pub symbol: String,
    /// The kind of a symbol's name inside it (`sym_name`).
    pub name: String,
}

/// The structural-merge facet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MergeSpec {
    pub key: MergeKey,
    /// Kinds whose identity is their normalized text (imports).
    #[serde(default)]
    pub import_kinds: Vec<String>,
    /// Wrappers seen through to the declaration they wrap (`export_statement`).
    #[serde(default)]
    pub transparent_kinds: Vec<String>,
    /// Fields tried, in order, to reach a wrapper's declaration.
    #[serde(default)]
    pub inner_decl_fields: Vec<String>,
    /// Fields tried, in order, to find a declaration's name.
    #[serde(default)]
    pub name_fields: Vec<String>,
    /// With `key: form_head`: the heads of a form that defines a named
    /// top-level entity (`defn`, `ns`).
    #[serde(default)]
    pub def_heads: Vec<String>,
}

/// The duplicate detector's facet: what it skips whole.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Duplication {
    /// Node kinds whose subtree is noise (imports, package directives).
    #[serde(default)]
    pub skip_kinds: Vec<String>,
    /// Form heads whose form is skipped (with `forms`).
    #[serde(default)]
    pub skip_form_heads: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FunctionsFile {
    #[serde(default)]
    kinds: Vec<String>,
    #[serde(default)]
    param_list_fields: Vec<String>,
    #[serde(default)]
    parameter_kinds: Vec<String>,
    #[serde(default)]
    form_heads: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct DecisionsFile {
    #[serde(default)]
    kinds: Vec<String>,
    #[serde(default)]
    form_heads: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct VisibilityFile {
    strategy: VisibilityStrategy,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ImportsFile {
    strategy: ImportStrategy,
}

/// `analysis.yaml`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct AnalysisFile {
    functions: FunctionsFile,
    decisions: DecisionsFile,
    #[serde(default)]
    forms: Option<Forms>,
    visibility: VisibilityFile,
    doc: DocStrategy,
    #[serde(default)]
    imports: Option<ImportsFile>,
    #[serde(default)]
    duplication: Duplication,
    #[serde(default)]
    merge: Option<MergeSpec>,
}

/// What this crate's walkers know about a language's syntax: its
/// `analysis.yaml`.
#[derive(Debug, Clone)]
pub struct LanguageSpec {
    /// Node kinds of function-like syntax, closures included: the
    /// duplication detector's units. What a file *defines* (and so what has
    /// metrics) comes from its symbol query.
    pub function_kinds: Vec<String>,
    /// Fields tried, in order, for the parameter list.
    pub param_list_fields: Vec<String>,
    /// Kinds inside a parameter list that are one parameter each.
    pub parameter_kinds: Vec<String>,
    /// Kinds that add 1 to cyclomatic complexity.
    pub decision_kinds: Vec<String>,
    /// With `forms`: heads of a form that is a function.
    pub function_form_heads: Vec<String>,
    /// With `forms`: heads of a form that is a decision point.
    pub decision_form_heads: Vec<String>,
    /// For a grammar of generic forms (Clojure's `list_lit`).
    pub forms: Option<Forms>,
    pub visibility: VisibilityStrategy,
    pub doc: DocStrategy,
    pub imports: Option<ImportStrategy>,
    pub duplication: Duplication,
    pub merge: Option<MergeSpec>,
}

impl LanguageSpec {
    /// Parse `analysis.yaml`.
    pub fn parse(text: &str) -> Result<Self, String> {
        let f: AnalysisFile =
            serde_yaml::from_str(text).map_err(|e| format!("analysis.yaml: {e}"))?;
        if (!f.functions.form_heads.is_empty() || !f.decisions.form_heads.is_empty())
            && f.forms.is_none()
        {
            return Err(
                "analysis.yaml: `form_heads` need `forms:` (the list, symbol and name kinds)"
                    .into(),
            );
        }
        Ok(Self {
            function_kinds: f.functions.kinds,
            param_list_fields: f.functions.param_list_fields,
            parameter_kinds: f.functions.parameter_kinds,
            decision_kinds: f.decisions.kinds,
            function_form_heads: f.functions.form_heads,
            decision_form_heads: f.decisions.form_heads,
            forms: f.forms,
            visibility: f.visibility.strategy,
            doc: f.doc,
            imports: f.imports.map(|i| i.strategy),
            duplication: f.duplication,
            merge: f.merge,
        })
    }

    /// Every node kind the file names, for a check against the grammar.
    pub fn node_kinds(&self) -> Vec<&str> {
        let mut out: Vec<&str> = Vec::new();
        out.extend(self.function_kinds.iter().map(String::as_str));
        out.extend(self.parameter_kinds.iter().map(String::as_str));
        out.extend(self.decision_kinds.iter().map(String::as_str));
        out.extend(self.duplication.skip_kinds.iter().map(String::as_str));
        if let Some(f) = &self.forms {
            out.extend([f.list.as_str(), f.symbol.as_str(), f.name.as_str()]);
        }
        if let Some(m) = &self.merge {
            out.extend(m.import_kinds.iter().map(String::as_str));
            out.extend(m.transparent_kinds.iter().map(String::as_str));
        }
        out
    }
}

/// Where a language's grammar comes from: exactly one of `native` (a
/// grammar compiled into oxplow, by name, [`native_grammar`]; only for the
/// extensions that ship with oxplow) and `wasm` (a tree-sitter grammar
/// compiled to WebAssembly, a file of the extension).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GrammarDecl {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wasm: Option<String>,
    /// Where `wasm` was built from: only `oxplow extension grammar build`
    /// reads it; oxplow itself never fetches or compiles a grammar.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<GrammarSource>,
}

/// The tree-sitter grammar repository a `wasm` grammar is built from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GrammarSource {
    /// The git repository (`https://github.com/tree-sitter/tree-sitter-rust`).
    pub repository: String,
    /// The commit to build, as a full sha or a tag.
    pub rev: String,
    /// The folder of the repository that holds `src/parser.c`, when it
    /// isn't the root (a repository of several grammars).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// The parser ABI `tree-sitter generate` writes when the repository
    /// doesn't commit `src/parser.c` (default 14; tree-sitter 0.27 reads
    /// 13 to 15). Only `grammar build` reads it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub abi: Option<u32>,
}

/// The Mason suggestion for a language's server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LspDecl {
    pub package: String,
}

/// How the metric scaffold writes a sample function: `function` opens it
/// (`{name}` is its name), `branch` is repeated once per decision (`{n}` is
/// its number), `tail` closes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StarterDecl {
    pub function: String,
    pub branch: Vec<String>,
    pub tail: Vec<String>,
}

impl StarterDecl {
    /// A function named `name` with `branches` decisions, as lines.
    pub fn function(&self, name: &str, branches: usize) -> Vec<String> {
        let mut out = vec![self.function.replace("{name}", name)];
        for n in 1..=branches {
            out.extend(self.branch.iter().map(|l| l.replace("{n}", &n.to_string())));
        }
        out.extend(self.tail.iter().cloned());
        out
    }
}

/// One `languages:` entry of an extension's manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LanguageDecl {
    /// The stored name (`rust`): `symbol.language`, the `oxplow.language`
    /// dimension, what a script passes to `ast_query`.
    pub id: String,
    /// The name a person reads (`C++`).
    pub title: String,
    /// File extensions it claims, lowercase, no dot.
    #[serde(default)]
    pub extensions: Vec<String>,
    /// Other names scripts may use (`rs`, `c++`).
    #[serde(default)]
    pub aliases: Vec<String>,
    /// LSP `languageId`s that are this language (`typescriptreact`).
    #[serde(default)]
    pub lsp_ids: Vec<String>,
    /// Its grammar; none for a language oxplow only knows by its server.
    #[serde(default)]
    pub grammar: Option<GrammarDecl>,
    /// Its `analysis.yaml`, a file of the extension (with a grammar).
    #[serde(default)]
    pub analysis: Option<String>,
    /// Its `symbols.scm` (with a grammar); a `symbols.yaml` beside it is
    /// read too.
    #[serde(default)]
    pub symbols: Option<String>,
    #[serde(default)]
    pub lsp: Option<LspDecl>,
    /// The code units it gives beyond functions.
    #[serde(default)]
    pub units: Vec<UnitKind>,
    /// How the metric scaffold writes a sample function (none: no starter).
    #[serde(default)]
    pub starter: Option<StarterDecl>,
}

/// A language's declaration with its files' texts: what an extension's
/// loader reads, and what the host ships a script worker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LanguageSource {
    /// The extension that declares it.
    pub extension: String,
    /// Whether that extension ships with oxplow (may name a native grammar).
    pub shipped: bool,
    pub decl: LanguageDecl,
    #[serde(default)]
    pub analysis: Option<String>,
    #[serde(default)]
    pub symbols: Option<String>,
    #[serde(default)]
    pub symbols_yaml: Option<String>,
    /// Its WebAssembly grammar's bytes (`grammar: { wasm }`), base64 on the
    /// wire to a script worker.
    #[serde(default, with = "base64_bytes")]
    pub wasm: Option<Vec<u8>>,
}

mod base64_bytes {
    use base64::Engine as _;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &Option<Vec<u8>>, s: S) -> Result<S::Ok, S::Error> {
        match v {
            Some(b) => s.serialize_some(&base64::engine::general_purpose::STANDARD.encode(b)),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Vec<u8>>, D::Error> {
        Option::<String>::deserialize(d)?
            .map(|t| {
                base64::engine::general_purpose::STANDARD
                    .decode(t)
                    .map_err(serde::de::Error::custom)
            })
            .transpose()
    }
}

impl LanguageSource {
    /// Read a declaration's files through `read` (a path of the
    /// extension → its bytes). A file the declaration names but `read`
    /// doesn't find, or a text file that isn't UTF-8, is an error.
    pub fn read(
        extension: &str,
        shipped: bool,
        decl: LanguageDecl,
        read: &dyn Fn(&str) -> Option<Vec<u8>>,
    ) -> Result<Self, String> {
        let bytes = |path: &Option<String>| -> Result<Option<Vec<u8>>, String> {
            match path {
                None => Ok(None),
                Some(p) => read(p).map(Some).ok_or_else(|| {
                    format!(
                        "language `{}`: `{p}` isn't a file of the extension",
                        decl.id
                    )
                }),
            }
        };
        let text = |path: &Option<String>| -> Result<Option<String>, String> {
            bytes(path)?
                .map(|b| {
                    String::from_utf8(b).map_err(|_| {
                        format!(
                            "language `{}`: `{}` isn't UTF-8 text",
                            decl.id,
                            path.as_deref().unwrap_or_default()
                        )
                    })
                })
                .transpose()
        };
        let analysis = text(&decl.analysis)?;
        let symbols = text(&decl.symbols)?;
        let symbols_yaml = decl
            .symbols
            .as_deref()
            .and_then(|p| p.strip_suffix(".scm"))
            .and_then(|stem| read(&format!("{stem}.yaml")))
            .and_then(|b| String::from_utf8(b).ok());
        let wasm = bytes(&decl.grammar.as_ref().and_then(|g| g.wasm.clone()))?;
        Ok(Self {
            extension: extension.into(),
            shipped,
            decl,
            analysis,
            symbols,
            symbols_yaml,
            wasm,
        })
    }
}

/// The grammars compiled into oxplow, by the name a shipped language pack's
/// `grammar: { native: <name> }` gives. The one place in core that names
/// grammar crates.
pub fn native_grammar(name: &str) -> Option<TsLanguage> {
    Some(match name {
        "rust" => tree_sitter_rust::LANGUAGE.into(),
        "typescript" => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        "tsx" => tree_sitter_typescript::LANGUAGE_TSX.into(),
        "javascript" => tree_sitter_javascript::LANGUAGE.into(),
        "python" => tree_sitter_python::LANGUAGE.into(),
        "starlark" => tree_sitter_starlark::LANGUAGE.into(),
        "go" => tree_sitter_go::LANGUAGE.into(),
        "java" => tree_sitter_java::LANGUAGE.into(),
        "c" => tree_sitter_c::LANGUAGE.into(),
        "cpp" => tree_sitter_cpp::LANGUAGE.into(),
        "csharp" => tree_sitter_c_sharp::LANGUAGE.into(),
        _ => return None,
    })
}

/// One language, built: its declaration, its grammar and analysis (when it
/// has a grammar), its symbol query (compiled on first use).
pub struct LanguageDef {
    pub decl: LanguageDecl,
    pub extension: String,
    grammar: GrammarSlot,
    spec: Option<LanguageSpec>,
    symbols_src: Option<(String, Option<String>)>,
    symbols: OnceLock<Result<SymbolSpec, String>>,
    tag: u64,
    fingerprint: u64,
}

/// A language's grammar: a native one is at hand; a WebAssembly one is
/// compiled the first time something parses with it (and checked against the
/// analysis then), so a language set that is built often, or shipped to a
/// script worker that parses nothing of it, doesn't pay for it.
enum GrammarSlot {
    None,
    Native(TsLanguage),
    Wasm {
        path: String,
        bytes: Arc<[u8]>,
        /// The node kinds `analysis.yaml` names, for the check at load.
        node_kinds: Vec<String>,
        loaded: OnceLock<Result<TsLanguage, String>>,
    },
}

impl GrammarSlot {
    fn get(&self) -> Result<Option<&TsLanguage>, &str> {
        match self {
            GrammarSlot::None => Ok(None),
            GrammarSlot::Native(g) => Ok(Some(g)),
            GrammarSlot::Wasm {
                bytes,
                node_kinds,
                loaded,
                path,
            } => loaded
                .get_or_init(|| {
                    let g =
                        crate::grammar::load_wasm(bytes).map_err(|e| format!("`{path}`: {e}"))?;
                    let missing: Vec<&str> = node_kinds
                        .iter()
                        .map(String::as_str)
                        .filter(|k| {
                            g.id_for_node_kind(k, true) == 0 && g.id_for_node_kind(k, false) == 0
                        })
                        .collect();
                    if missing.is_empty() {
                        Ok(g)
                    } else {
                        Err(format!(
                            "analysis.yaml names node kinds its grammar doesn't have: {}",
                            missing.join(", ")
                        ))
                    }
                })
                .as_ref()
                .map(Some)
                .map_err(String::as_str),
        }
    }

    fn present(&self) -> bool {
        !matches!(self, GrammarSlot::None)
    }
}

impl std::fmt::Debug for LanguageDef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LanguageDef")
            .field("id", &self.decl.id)
            .field("extension", &self.extension)
            .finish()
    }
}

/// A language: a cheap handle, equal by id.
#[derive(Clone)]
pub struct Language(Arc<LanguageDef>);

impl std::fmt::Debug for Language {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Language({})", self.0.decl.id)
    }
}

impl PartialEq for Language {
    fn eq(&self, other: &Self) -> bool {
        self.0.decl.id == other.0.decl.id
    }
}
impl Eq for Language {}
impl Hash for Language {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.decl.id.hash(state)
    }
}

impl Language {
    /// Build one language from its source. A wrong file is an error.
    pub fn build(source: &LanguageSource) -> Result<Self, String> {
        let decl = source.decl.clone();
        let id = &decl.id;
        let grammar = match decl
            .grammar
            .as_ref()
            .map(|g| (g.native.as_deref(), g.wasm.as_deref()))
        {
            None => GrammarSlot::None,
            Some((None, None)) | Some((Some(_), Some(_))) => {
                return Err(format!(
                    "language `{id}`: `grammar:` names one of `native` and `wasm`"
                ))
            }
            Some((Some(name), None)) => {
                if !source.shipped {
                    return Err(format!(
                        "language `{id}`: `grammar: {{ native: {name} }}` is for the extensions that ship with oxplow; bring a WebAssembly grammar (`grammar: {{ wasm: <file> }}`)"
                    ));
                }
                GrammarSlot::Native(native_grammar(name).ok_or_else(|| {
                    format!("language `{id}`: oxplow has no native grammar `{name}`")
                })?)
            }
            Some((None, Some(path))) => {
                let bytes = source
                    .wasm
                    .as_deref()
                    .ok_or_else(|| format!("language `{id}`: its grammar `{path}` wasn't read"))?;
                // Cheap: the module's own export says it is a grammar. Loading
                // it is for the first parse (or `check`).
                crate::grammar::wasm_grammar_name(bytes)
                    .map_err(|e| format!("language `{id}`: `{path}`: {e}"))?;
                GrammarSlot::Wasm {
                    path: path.to_string(),
                    bytes: bytes.into(),
                    node_kinds: Vec::new(),
                    loaded: OnceLock::new(),
                }
            }
        };
        let (grammar, spec) = match (grammar, &source.analysis) {
            (g, Some(text)) if g.present() => {
                let spec =
                    LanguageSpec::parse(text).map_err(|e| format!("language `{id}`: {e}"))?;
                let g = match g {
                    GrammarSlot::Native(native) => {
                        let missing: Vec<&str> = spec
                            .node_kinds()
                            .into_iter()
                            .filter(|k| {
                                native.id_for_node_kind(k, true) == 0
                                    && native.id_for_node_kind(k, false) == 0
                            })
                            .collect();
                        if !missing.is_empty() {
                            return Err(format!(
                                "language `{id}`: analysis.yaml names node kinds its grammar doesn't have: {}",
                                missing.join(", ")
                            ));
                        }
                        GrammarSlot::Native(native)
                    }
                    GrammarSlot::Wasm {
                        path,
                        bytes,
                        loaded,
                        ..
                    } => GrammarSlot::Wasm {
                        path,
                        bytes,
                        node_kinds: spec.node_kinds().into_iter().map(String::from).collect(),
                        loaded,
                    },
                    GrammarSlot::None => GrammarSlot::None,
                };
                (g, Some(spec))
            }
            (g, None) if g.present() => {
                return Err(format!(
                    "language `{id}`: a grammar needs `analysis:` (its analysis.yaml)"
                ))
            }
            (_, Some(_)) => return Err(format!("language `{id}`: `analysis:` needs a `grammar:`")),
            (g, None) => (g, None),
        };
        let symbols_src = match (grammar.present(), &source.symbols) {
            (true, Some(scm)) => Some((scm.clone(), source.symbols_yaml.clone())),
            (false, Some(_)) => {
                return Err(format!("language `{id}`: `symbols:` needs a `grammar:`"))
            }
            _ => None,
        };
        // FNV-1a of the id: stable across builds and Rust versions (a
        // pinned test holds the detector's hashes).
        let tag = id.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |acc, b| {
            (acc ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
        });
        // FNV-1a over everything that decides what the language computes, so
        // a memo keyed by it is stale the moment a definition changes.
        let fingerprint = {
            let decl_text = serde_yaml::to_string(&source.decl).unwrap_or_default();
            [
                decl_text.as_str(),
                source.analysis.as_deref().unwrap_or(""),
                source.symbols.as_deref().unwrap_or(""),
                source.symbols_yaml.as_deref().unwrap_or(""),
            ]
            .iter()
            .map(|t| t.as_bytes())
            .chain(std::iter::once(source.wasm.as_deref().unwrap_or(&[])))
            .flat_map(|t| t.iter().copied().chain(std::iter::once(0xff)))
            .fold(0xcbf2_9ce4_8422_2325_u64, |acc, b| {
                (acc ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
            })
        };
        Ok(Language(Arc::new(LanguageDef {
            fingerprint,
            extension: source.extension.clone(),
            decl,
            grammar,
            spec,
            symbols_src,
            symbols: OnceLock::new(),
            tag,
        })))
    }

    /// Its stored name (`rust`).
    pub fn name(&self) -> &str {
        &self.0.decl.id
    }

    pub fn title(&self) -> &str {
        &self.0.decl.title
    }

    pub fn decl(&self) -> &LanguageDecl {
        &self.0.decl
    }

    /// The extension that declares it.
    pub fn extension(&self) -> &str {
        &self.0.extension
    }

    /// Its grammar, when it has one (a language known only by its server
    /// has none).
    pub fn grammar(&self) -> Option<&TsLanguage> {
        self.0.grammar.get().ok().flatten()
    }

    /// Why its grammar can't be used (a WebAssembly file tree-sitter
    /// refuses, node kinds `analysis.yaml` names that it lacks): loads it if
    /// it isn't yet.
    pub fn grammar_error(&self) -> Option<String> {
        self.0
            .grammar
            .get()
            .err()
            .map(|e| format!("language `{}`: {e}", self.name()))
    }

    /// Check what loading would find wrong: the grammar (loaded now) and
    /// the symbol query. For `extension check` and the load of an extension;
    /// a set built to parse isn't checked.
    pub fn check(&self) -> Result<(), String> {
        match self.grammar_error() {
            Some(e) => Err(e),
            None => self.check_symbols(),
        }
    }

    /// A parser set to its grammar (a wasm grammar's parser borrows this
    /// thread's store), or `None` when it has no grammar.
    pub fn parser(&self) -> Option<crate::LangParser> {
        crate::grammar::parser(self.grammar()?)
    }

    /// Whether oxplow can parse it.
    pub fn analyzable(&self) -> bool {
        self.0.spec.is_some()
    }

    /// Its analysis tables. Only an [`analyzable`](Self::analyzable)
    /// language has them; [`Languages::for_path`] returns no other.
    pub fn spec(&self) -> &LanguageSpec {
        self.0
            .spec
            .as_ref()
            .unwrap_or_else(|| panic!("language `{}` has no analysis", self.name()))
    }

    /// The tree-sitter grammar of an [`analyzable`](Self::analyzable)
    /// language.
    pub fn tree_sitter_language(&self) -> Option<TsLanguage> {
        self.grammar().cloned()
    }

    /// A hash of its id: the duplicate detector's salt, so token streams of
    /// two languages never collide.
    pub fn tag(&self) -> u64 {
        self.0.tag
    }

    /// A hash of its declaration and files: what a cache of its results is
    /// keyed by.
    pub fn fingerprint(&self) -> u64 {
        self.0.fingerprint
    }

    /// Its compiled symbol query, compiled on first use.
    pub(crate) fn symbol_spec(&self) -> Result<&SymbolSpec, String> {
        self.0
            .symbols
            .get_or_init(|| match (&self.0.symbols_src, self.grammar()) {
                (Some((scm, yaml)), Some(grammar)) => {
                    SymbolSpec::compile(grammar, scm, yaml.as_deref())
                }
                _ => Err("no symbol query".into()),
            })
            .as_ref()
            .map_err(|e| format!("{}: {e}", self.name()))
    }

    /// Compile its symbol query now, for a check.
    pub fn check_symbols(&self) -> Result<(), String> {
        if self.0.symbols_src.is_none() {
            return Ok(());
        }
        self.symbol_spec().map(|_| ())
    }
}

/// The languages the enabled extensions declare.
#[derive(Clone, Default)]
pub struct Languages {
    all: Vec<Language>,
    /// What it was built from (the languages that joined), to ship to a
    /// script worker, which builds its own.
    sources: Vec<LanguageSource>,
    by_extension: HashMap<String, usize>,
    by_name: HashMap<String, usize>,
    by_lsp_id: HashMap<String, usize>,
}

impl std::fmt::Debug for Languages {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list()
            .entries(self.all.iter().map(|l| l.name()))
            .finish()
    }
}

/// A language that couldn't join a set, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanguageProblem {
    pub extension: String,
    pub language: String,
    pub message: String,
}

impl Languages {
    /// Build the set from `sources`, in order. A language that doesn't
    /// build, or whose id, alias, LSP id or file extension an earlier one
    /// already has, is left out with a problem.
    pub fn build(sources: &[LanguageSource]) -> (Self, Vec<LanguageProblem>) {
        let mut out = Self::default();
        let mut problems = Vec::new();
        for source in sources {
            let problem = |message: String| LanguageProblem {
                extension: source.extension.clone(),
                language: source.decl.id.clone(),
                message,
            };
            let lang = match Language::build(source) {
                Ok(l) => l,
                Err(e) => {
                    problems.push(problem(e));
                    continue;
                }
            };
            let decl = lang.decl();
            let names: Vec<String> = std::iter::once(&decl.id)
                .chain(&decl.aliases)
                .map(|n| n.to_ascii_lowercase())
                .collect();
            let exts: Vec<String> = decl
                .extensions
                .iter()
                .map(|e| e.to_ascii_lowercase())
                .collect();
            let lsp: Vec<String> = decl
                .lsp_ids
                .iter()
                .map(|e| e.to_ascii_lowercase())
                .collect();
            let taken = names
                .iter()
                .find(|n| out.by_name.contains_key(*n))
                .map(|n| format!("the name `{n}`"))
                .or_else(|| {
                    exts.iter()
                        .find(|e| out.by_extension.contains_key(*e))
                        .map(|e| format!("files `.{e}`"))
                })
                .or_else(|| {
                    lsp.iter()
                        .find(|e| out.by_lsp_id.contains_key(*e))
                        .map(|e| format!("the LSP id `{e}`"))
                });
            if let Some(what) = taken {
                let other = out
                    .by_name
                    .get(&names[0])
                    .or_else(|| exts.iter().find_map(|e| out.by_extension.get(e)))
                    .map(|i| out.all[*i].extension().to_string())
                    .unwrap_or_default();
                problems.push(problem(format!(
                    "{what} already belongs to another language{}",
                    if other.is_empty() {
                        String::new()
                    } else {
                        format!(" (from `{other}`)")
                    }
                )));
                continue;
            }
            let i = out.all.len();
            for n in names {
                out.by_name.insert(n, i);
            }
            for e in exts {
                out.by_extension.insert(e, i);
            }
            for e in lsp {
                out.by_lsp_id.insert(e, i);
            }
            out.all.push(lang);
            out.sources.push(source.clone());
        }
        (out, problems)
    }

    /// A hash of every language's fingerprint, in order: what a result
    /// computed with the set is keyed by, so turning a pack on, off or
    /// editing one is new input.
    pub fn fingerprint(&self) -> u64 {
        self.all.iter().fold(0xcbf2_9ce4_8422_2325_u64, |acc, l| {
            l.fingerprint().to_le_bytes().iter().fold(acc, |a, b| {
                (a ^ u64::from(*b)).wrapping_mul(0x0000_0100_0000_01b3)
            })
        })
    }

    /// What the set was built from: [`Languages::build`] of these is the same
    /// set.
    pub fn sources(&self) -> &[LanguageSource] {
        &self.sources
    }

    /// Every language, in the order the set was built.
    pub fn all(&self) -> &[Language] {
        &self.all
    }

    /// The language a name (an id or alias, any case) names.
    pub fn by_name(&self, name: &str) -> Option<&Language> {
        self.by_name
            .get(&name.trim().to_ascii_lowercase())
            .map(|i| &self.all[*i])
    }

    /// The language an LSP `languageId` names: its declared LSP ids, then
    /// its names.
    pub fn by_lsp_id(&self, id: &str) -> Option<&Language> {
        let id = id.trim().to_ascii_lowercase();
        self.by_lsp_id
            .get(&id)
            .map(|i| &self.all[*i])
            .or_else(|| self.by_name(&id))
    }

    /// The language a file is, by its extension, whether or not oxplow can
    /// parse it.
    pub fn declared_for_path(&self, path: &str) -> Option<&Language> {
        let file = path.rsplit(['/', '\\']).next().unwrap_or(path);
        let (_, ext) = file.rsplit_once('.')?;
        self.by_extension
            .get(&ext.to_ascii_lowercase())
            .map(|i| &self.all[*i])
    }

    /// The language oxplow parses a file as: [`declared_for_path`](Self::declared_for_path)
    /// when it has a grammar.
    pub fn for_path(&self, path: &str) -> Option<&Language> {
        self.declared_for_path(path).filter(|l| l.analyzable())
    }

    /// Whether oxplow parses a file of this path.
    pub fn is_supported_path(&self, path: impl AsRef<std::path::Path>) -> bool {
        self.for_path(&path.as_ref().to_string_lossy()).is_some()
    }

    /// The Mason package suggested for an LSP id's server.
    pub fn mason_suggestion(&self, lsp_id: &str) -> Option<&str> {
        self.by_lsp_id(lsp_id)
            .and_then(|l| l.decl().lsp.as_ref())
            .map(|l| l.package.as_str())
    }

    /// The file extensions of the languages whose server is `package`
    /// (what an installed server matches files by).
    pub fn lsp_extensions(&self, lsp_id: &str) -> Vec<String> {
        self.by_lsp_id(lsp_id)
            .map(|l| l.decl().extensions.clone())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod lazy_grammar_tests {
    use super::*;

    /// The Clojure pack's own files, read as an extension would hand them.
    fn clojure(skip_kind: &str) -> LanguageSource {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../extensions/oxplow-clojure");
        let manifest: serde_yaml::Value =
            serde_yaml::from_str(&std::fs::read_to_string(root.join("extension.yaml")).unwrap())
                .unwrap();
        let decl: LanguageDecl = serde_yaml::from_value(manifest["languages"][0].clone()).unwrap();
        let analysis = decl.analysis.clone().unwrap();
        LanguageSource::read("oxplow-clojure", false, decl, &|rel| {
            let bytes = std::fs::read(root.join(rel)).ok()?;
            Some(if rel == analysis && !skip_kind.is_empty() {
                String::from_utf8(bytes)
                    .unwrap()
                    .replace("skip_kinds: []", &format!("skip_kinds: [{skip_kind}]"))
                    .into_bytes()
            } else {
                bytes
            })
        })
        .unwrap()
    }

    /// Building a language doesn't compile its WebAssembly grammar: the first
    /// parse does, and a set built often (or shipped to a script worker that
    /// parses nothing of it) doesn't pay for it.
    #[test]
    fn a_wasm_grammar_is_loaded_on_first_use() {
        let before = crate::grammar::loaded_count();
        let lang = Language::build(&clojure("")).unwrap();
        assert_eq!(crate::grammar::loaded_count(), before, "built, not loaded");
        assert!(lang.parser().is_some());
        assert_eq!(crate::grammar::loaded_count(), before + 1);
        assert!(lang.check().is_ok());
    }

    /// A node kind the analysis names that the grammar lacks is found when
    /// the grammar loads (`check`, `extension check`), not at every build.
    #[test]
    fn analysis_naming_a_missing_node_kind_fails_the_check() {
        let lang = Language::build(&clojure("no_such_node")).unwrap();
        let e = lang.check().unwrap_err();
        assert!(
            e.contains("node kinds its grammar doesn't have: no_such_node"),
            "{e}"
        );
        assert!(lang.parser().is_none(), "such a grammar isn't used");
    }
}
