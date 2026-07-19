//! Tier-2 AST structural merge — parse + per-item 3-way merge
//! (tsk134, tsk135; wired into auto-resolve in tsk136; second-slice
//! languages added in tsk137).
//!
//! The language-neutral core of the AST merge designed in
//! `.context/smart-merge.md` (Tier 2), in two layers:
//!
//! 1. **Parse → items** ([`parse_top_level_items`], tsk134): parse a
//!    file's bytes with the right tree-sitter grammar and reduce it to
//!    its ordered top-level items, each carrying
//!    - a **stable identity key** for 3-way matching (an import is keyed
//!      by its normalized text; a named declaration by `kind + name`;
//!      anything unnamed falls back to `kind + normalized-text`), and
//!    - its **original byte span** (so reconstruction reuses exact
//!      source, including a declaration's attached leading doc-comments).
//! 2. **3-way merge** ([`merge_top_level`], tsk135): per-item classify
//!    (take-the-changed-side / take-agreement / refuse on overlap),
//!    deterministic ordering + verbatim reconstruction, and a re-parse
//!    guard that discards any reconstruction that doesn't parse cleanly.
//!
//! Wired into `auto_resolve_conflicts` as a Tier-1.5 fallback in tsk136
//! (see `smart_merge.rs`). The original feasibility spike lives at
//! `crates/oxplow-git/tests/ast_merge_spike.rs`.
//!
//! ## Safety
//!
//! [`parse_top_level_items`] returns `None` when the file isn't a
//! supported language *or* the parse produced any error/missing node —
//! we never operate on an untrusted tree. Callers treat `None` as "leave
//! git's conflict markers", preserving Tier-2's reduce-only invariant.
//!
//! ## Supported languages
//!
//! First slice (tsk134): rust, typescript, tsx, javascript, python, go.
//! Second slice (tsk137): java, c, cpp, clojure. C/C++ and Clojure use
//! bespoke key resolvers (see [`KeyStrategy`]); the rest share the generic
//! `name_fields` path.
//!
//! ## Identity-key fidelity
//!
//! Imports and the high-value named declarations (`fn`/function, `struct`,
//! `class`/`interface`/`enum`, `type` alias, `trait`, Java type decls, …)
//! get precise `kind+name` keys, which is what the flagship commutative
//! cases need (both sides add a different import / a different top-level
//! decl). Go methods are disambiguated by their receiver; C/C++ functions
//! by their declarator (name + signature, so overloads stay distinct);
//! Clojure forms by their head symbol + defined name.
//!
//! Declarations whose name sits under a declarator list and that take the
//! `Generic` strategy (Rust/TS `const`/`let`/`var`, Go `type`/`var`/`const`
//! groups) fall back to text identity. That merges correctly when one side
//! *adds* such an item (add is key-independent), but because a text key
//! changes when its body changes, two sides editing the *same* such item
//! differently are mis-classified as two independent adds rather than a
//! conflict — see the `text-identity divergent-edit` note in
//! `.context/smart-merge.md` (tracked as a separate hardening task).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::ops::Range;

use tree_sitter::{Language as TsLanguage, Node, Parser};

/// The canonical, workspace-wide language identity (tsk321). The AST merge
/// front half no longer declares its own enum — it reuses
/// `oxplow_code_metrics::Language` and keys its `MergeSpec` table off it via
/// [`merge_spec`], which returns `None` for the canonical variants the merge
/// tier doesn't support (e.g. `CSharp`). Re-exported so existing call sites
/// (`crate::ast_merge::Language`, `oxplow_git::MergeLanguage`) keep working.
pub use oxplow_code_metrics::Language;

/// Map a repo-relative (or any) path to its merge language by extension.
/// Resolves through the canonical `oxplow_code_metrics::language_for_path`,
/// then drops any language the merge front half doesn't support (no
/// [`MergeSpec`]), so e.g. `.cs` ⇒ `None`. `None` ⇒ unsupported, so the AST
/// tier is skipped and git's markers stand.
pub fn language_for_path(path: &str) -> Option<Language> {
    let lang = oxplow_code_metrics::language_for_path(path)?;
    merge_spec(lang).map(|_| lang)
}

/// Per-language table of the tree-sitter node names that drive identity.
/// Modeled on `oxplow-code-metrics`'s `LanguageSpec` kind tables, but
/// scoped to what top-level-item identity needs rather than complexity
/// metrics.
pub struct MergeSpec {
    /// Node kinds whose identity is their normalized text — imports/use.
    /// A `use`/`import` *is* what it brings into scope, so two textually
    /// distinct imports are distinct items regardless of order.
    pub import_kinds: &'static [&'static str],
    /// "See-through" wrapper kinds (e.g. TS `export_statement`, Python
    /// `decorated_definition`): identity is computed from the declaration
    /// they wrap, while the wrapper's own text/span is still kept so the
    /// `export `/decorator prefix travels with the item.
    pub transparent_kinds: &'static [&'static str],
    /// Field names tried (in order) to descend a transparent wrapper to
    /// the declaration it wraps.
    pub inner_decl_fields: &'static [&'static str],
    /// Field names tried (in order) to find a declaration's identifier.
    pub name_fields: &'static [&'static str],
    /// How a non-import item's identity key is computed. Most languages
    /// use [`KeyStrategy::Generic`] (the `name_fields` path); the C family
    /// and Clojure need bespoke resolvers. See [`KeyStrategy`].
    pub key_strategy: KeyStrategy,
    /// Loader for the bundled tree-sitter grammar.
    grammar: fn() -> TsLanguage,
}

/// Per-language identity-key resolution strategy for non-import items.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyStrategy {
    /// `transparent_kinds` see-through + `name_fields` lookup + Go-receiver
    /// disambiguation, falling back to `kind + normalized-text`. Used by
    /// rust/ts/tsx/js/python/go/java.
    Generic,
    /// C/C++: functions keyed by their declarator (name + signature, so
    /// overloads stay distinct), structs/unions/enums/classes/namespaces
    /// by name, typedefs and globals by their declarator's leaf
    /// identifier; `template_declaration` is seen through to the wrapped
    /// declaration. Everything else falls back to `kind + normalized-text`.
    CFamily,
    /// Clojure: top-level `(def…/defn…/ns …)` forms keyed by
    /// `head-symbol + defined-name`; any other form by `kind +
    /// normalized-text`.
    Clojure,
}

impl MergeSpec {
    fn tree_sitter_language(&self) -> TsLanguage {
        (self.grammar)()
    }
}

/// The [`MergeSpec`] for `lang`, or `None` for canonical languages the AST
/// merge front half does not support. Their variant exists in the shared
/// `Language` enum (so identity stays unified) but carries no merge spec —
/// `C#` is analysed for metrics but never structurally merged here.
pub fn merge_spec(lang: Language) -> Option<&'static MergeSpec> {
    Some(match lang {
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
        Language::CSharp => return None,
    })
}

/// A top-level declaration, reduced to what the structural merge needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// Stable identity for 3-way matching across base/ours/theirs.
    pub key: String,
    /// Verbatim source slice covering `byte_span` (includes any attached
    /// leading doc-comments). Reconstruction splices this back unchanged.
    pub text: String,
    /// Byte range in the original source. `text == src[byte_span]`.
    pub byte_span: Range<usize>,
}

/// Parse `src` as `lang` and return its ordered top-level items, or
/// `None` if the parse failed (any error/missing node) — never operate
/// on an untrusted tree.
pub fn parse_top_level_items(src: &str, lang: Language) -> Option<Vec<Item>> {
    let spec = merge_spec(lang)?;
    let mut parser = Parser::new();
    parser.set_language(&spec.tree_sitter_language()).ok()?;
    let tree = parser.parse(src, None)?;
    let root = tree.root_node();
    if root.has_error() {
        return None;
    }

    let mut items = Vec::new();
    let mut cursor = root.walk();
    // Track a contiguous run of leading comments to attach to the next
    // declaration (so a moved decl carries its doc comment).
    let mut comment_run: Option<(usize, usize)> = None; // (start_byte, end_byte)

    for child in root.children(&mut cursor) {
        if child.is_extra() {
            // Comment / whitespace at top level. Extend (or open) the run.
            let start = comment_run.map_or(child.start_byte(), |(s, _)| s);
            comment_run = Some((start, child.end_byte()));
            continue;
        }

        // Absorb the preceding comment run into this item's span iff it
        // is directly attached (no blank line between it and the decl).
        // A DETACHED run (blank line between) becomes its own item instead of
        // being dropped — a licence header or section divider is content, and
        // silently losing it while reporting a clean merge is the worst
        // outcome available (tsk159).
        let mut start = child.start_byte();
        if let Some((cstart, cend)) = comment_run {
            let gap = &src[cend..child.start_byte()];
            if gap.contains("\n\n") {
                push_comment_item(&mut items, src, cstart, cend);
            } else {
                start = cstart;
            }
        }
        comment_run = None;

        let byte_span = start..child.end_byte();
        let text = src[byte_span.clone()].to_string();
        let key = item_key(child, src, spec);
        items.push(Item {
            key,
            text,
            byte_span,
        });
    }

    // A run trailing the last declaration (or a comment-only file) attaches to
    // nothing, so it needs the same treatment.
    if let Some((cstart, cend)) = comment_run {
        push_comment_item(&mut items, src, cstart, cend);
    }

    Some(items)
}

/// Emit a detached comment run as a standalone [`Item`].
///
/// Keyed by normalized text, like imports: an identical header on all three
/// sides matches and survives once, while a header edited on one side reads as
/// a delete + add and lands in the conservative conflict path rather than being
/// silently rewritten. Whitespace-only runs are skipped — `is_extra()` covers
/// whitespace as well as comments, and a blank-line "item" is not content.
fn push_comment_item(items: &mut Vec<Item>, src: &str, start: usize, end: usize) {
    let text = &src[start..end];
    if text.trim().is_empty() {
        return;
    }
    items.push(Item {
        key: format!("{COMMENT_KEY_PREFIX}{}", normalize_ws(text)),
        text: text.to_string(),
        byte_span: start..end,
    });
}

/// Key prefix marking a standalone comment block (tsk159). `reconstruct` reads
/// it to restore the blank line that made the block detached in the first
/// place.
const COMMENT_KEY_PREFIX: &str = "comment::";

/// Compute an item's identity key per the spec's rules.
fn item_key(node: Node, src: &str, spec: &MergeSpec) -> String {
    let kind = node.kind();

    // Imports/use: identity is the normalized text (all strategies).
    if spec.import_kinds.contains(&kind) {
        return format!("import::{}", normalize_ws(&node_text(node, src)));
    }

    match spec.key_strategy {
        KeyStrategy::Generic => generic_item_key(node, src, spec),
        KeyStrategy::CFamily => c_family_item_key(node, src),
        KeyStrategy::Clojure => clojure_item_key(node, src),
    }
}

/// The default key: see-through wrappers + `name_fields` + Go-receiver
/// disambiguation, falling back to `kind + normalized-text`.
fn generic_item_key(node: Node, src: &str, spec: &MergeSpec) -> String {
    let kind = node.kind();

    // See through wrappers (export/decorated) to the inner declaration
    // for naming, but key off the *inner* kind+name.
    let named = if spec.transparent_kinds.contains(&kind) {
        inner_decl(node, spec).unwrap_or(node)
    } else {
        node
    };

    if let Some(name) = decl_name(named, src, spec) {
        let inner_kind = named.kind();
        // Disambiguate Go methods by receiver type — two methods can
        // share a name on different receivers.
        if let Some(recv) = named.child_by_field_name("receiver") {
            return format!(
                "{inner_kind}::{}::{name}",
                normalize_ws(&node_text(recv, src))
            );
        }
        return format!("{inner_kind}::{name}");
    }

    // Unnamed item (bare `impl`, const/var declarator lists, …): fall
    // back to kind + normalized text so identical blocks match and
    // divergent ones stay distinct.
    format!("{kind}::{}", normalize_ws(&node_text(node, src)))
}

/// C/C++ identity key. Functions key by their declarator (name +
/// signature, so overloads stay distinct); structs/unions/enums/classes/
/// namespaces by name; typedefs and globals by their declarator's leaf
/// identifier. `template_declaration` is seen through to the wrapped
/// declaration. Anything else → `kind + normalized-text`.
fn c_family_item_key(node: Node, src: &str) -> String {
    // See through a `template<…>` wrapper to the declaration it templates.
    let inner = if node.kind() == "template_declaration" {
        c_template_inner(node).unwrap_or(node)
    } else {
        node
    };
    let kind = inner.kind();
    match kind {
        // Name + signature: the declarator text covers both, and is stable
        // across body edits while keeping overloads distinct.
        "function_definition" => {
            if let Some(d) = inner.child_by_field_name("declarator") {
                return format!("function_definition::{}", normalize_ws(&node_text(d, src)));
            }
        }
        "struct_specifier"
        | "union_specifier"
        | "enum_specifier"
        | "class_specifier"
        | "namespace_definition" => {
            if let Some(n) = inner.child_by_field_name("name") {
                return format!("{kind}::{}", node_text(n, src));
            }
        }
        // typedef alias name / global variable name: the declarator's leaf
        // identifier (a value edit must not change identity).
        "type_definition" | "declaration" => {
            if let Some(name) = c_declarator_leaf_name(inner) {
                return format!("{kind}::{}", node_text(name, src));
            }
        }
        _ => {}
    }
    format!("{}::{}", node.kind(), normalize_ws(&node_text(node, src)))
}

/// First `function_definition`/`*_specifier`/`declaration` child of a C++
/// `template_declaration` (the declaration it templates).
fn c_template_inner(node: Node) -> Option<Node> {
    let mut cursor = node.walk();
    let found = node.children(&mut cursor).find(|c| {
        matches!(
            c.kind(),
            "function_definition"
                | "struct_specifier"
                | "union_specifier"
                | "enum_specifier"
                | "class_specifier"
                | "declaration"
        )
    });
    found
}

/// Descend a node's `declarator` field chain to its leaf identifier.
fn c_declarator_leaf_name(node: Node) -> Option<Node> {
    let mut cur = node.child_by_field_name("declarator")?;
    loop {
        match cur.kind() {
            "identifier" | "type_identifier" | "field_identifier" | "qualified_identifier" => {
                return Some(cur)
            }
            _ => cur = cur.child_by_field_name("declarator")?,
        }
    }
}

/// Clojure identity key. A top-level `(head name …)` form whose head is a
/// recognized `def…`/`ns` symbol keys by `head::name`; any other form by
/// `kind + normalized-text`. Two `defmethod`s on the same multifn collapse
/// to one key and conservatively bail (DuplicateKeys) — safe, not wrong.
fn clojure_item_key(node: Node, src: &str) -> String {
    if node.kind() == "list_lit" {
        let mut cursor = node.walk();
        let values: Vec<Node> = node.children_by_field_name("value", &mut cursor).collect();
        if let (Some(head), Some(name)) = (values.first(), values.get(1)) {
            let head_txt = node_text(*head, src);
            if CLOJURE_DEF_HEADS.contains(&head_txt.as_str()) {
                return format!("{head_txt}::{}", normalize_ws(&node_text(*name, src)));
            }
        }
    }
    format!("{}::{}", node.kind(), normalize_ws(&node_text(node, src)))
}

/// Clojure head symbols whose form defines a top-level named entity.
static CLOJURE_DEF_HEADS: &[&str] = &[
    "def",
    "defn",
    "defn-",
    "defmacro",
    "defmulti",
    "defmethod",
    "defprotocol",
    "defrecord",
    "deftype",
    "definterface",
    "defonce",
    "ns",
];

/// First child reachable through the spec's `inner_decl_fields`.
fn inner_decl<'a>(node: Node<'a>, spec: &MergeSpec) -> Option<Node<'a>> {
    spec.inner_decl_fields
        .iter()
        .find_map(|f| node.child_by_field_name(f))
}

/// A declaration's name via the spec's `name_fields`, if present.
fn decl_name(node: Node, src: &str, spec: &MergeSpec) -> Option<String> {
    spec.name_fields
        .iter()
        .find_map(|f| node.child_by_field_name(f))
        .map(|n| node_text(n, src))
}

fn node_text(node: Node, src: &str) -> String {
    node.utf8_text(src.as_bytes())
        .unwrap_or_default()
        .trim()
        .to_string()
}

/// Collapse interior whitespace runs to single spaces so trivial
/// reformatting (`use a::b ;` vs `use a::b;`) keys identically.
fn normalize_ws(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

// ---- Language specs ----

static RUST: MergeSpec = MergeSpec {
    import_kinds: &["use_declaration", "extern_crate_declaration"],
    transparent_kinds: &[],
    inner_decl_fields: &[],
    name_fields: &["name"],
    key_strategy: KeyStrategy::Generic,
    grammar: || tree_sitter_rust::LANGUAGE.into(),
};

// TypeScript / TSX / JavaScript share identity rules; only the grammar
// (and thus the top-level item kinds the parser yields) differs. A
// top-level `export ...` is an `export_statement` wrapping the real
// declaration under the `declaration` field.
static TS_IMPORT_KINDS: &[&str] = &["import_statement"];
static TS_TRANSPARENT_KINDS: &[&str] = &["export_statement"];
static TS_INNER_DECL_FIELDS: &[&str] = &["declaration"];

static TYPESCRIPT: MergeSpec = MergeSpec {
    import_kinds: TS_IMPORT_KINDS,
    transparent_kinds: TS_TRANSPARENT_KINDS,
    inner_decl_fields: TS_INNER_DECL_FIELDS,
    name_fields: &["name"],
    key_strategy: KeyStrategy::Generic,
    grammar: || tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
};

static TSX: MergeSpec = MergeSpec {
    import_kinds: TS_IMPORT_KINDS,
    transparent_kinds: TS_TRANSPARENT_KINDS,
    inner_decl_fields: TS_INNER_DECL_FIELDS,
    name_fields: &["name"],
    key_strategy: KeyStrategy::Generic,
    grammar: || tree_sitter_typescript::LANGUAGE_TSX.into(),
};

static JAVASCRIPT: MergeSpec = MergeSpec {
    import_kinds: TS_IMPORT_KINDS,
    transparent_kinds: TS_TRANSPARENT_KINDS,
    inner_decl_fields: TS_INNER_DECL_FIELDS,
    name_fields: &["name"],
    key_strategy: KeyStrategy::Generic,
    grammar: || tree_sitter_javascript::LANGUAGE.into(),
};

static PYTHON: MergeSpec = MergeSpec {
    import_kinds: &[
        "import_statement",
        "import_from_statement",
        "future_import_statement",
    ],
    // A decorated def wraps the real `function_definition`/`class_definition`
    // under the `definition` field; the decorators ride along in the text.
    transparent_kinds: &["decorated_definition"],
    inner_decl_fields: &["definition"],
    name_fields: &["name"],
    key_strategy: KeyStrategy::Generic,
    grammar: || tree_sitter_python::LANGUAGE.into(),
};

static GO: MergeSpec = MergeSpec {
    import_kinds: &["import_declaration"],
    transparent_kinds: &[],
    inner_decl_fields: &[],
    name_fields: &["name"],
    key_strategy: KeyStrategy::Generic,
    grammar: || tree_sitter_go::LANGUAGE.into(),
};

// ---- Second-slice languages (tsk137) ----

// Java top-level items are imports + type declarations (methods/fields are
// nested in a class body — intra-body merge is Tier-1's job, per the
// design). The generic `name_fields=["name"]` path keys each
// class/interface/enum/record by its identifier; `package_declaration`
// falls back to text.
static JAVA: MergeSpec = MergeSpec {
    import_kinds: &["import_declaration"],
    transparent_kinds: &[],
    inner_decl_fields: &[],
    name_fields: &["name"],
    key_strategy: KeyStrategy::Generic,
    grammar: || tree_sitter_java::LANGUAGE.into(),
};

// C / C++ use the bespoke `CFamily` key resolver (declarator-based names +
// signatures). `import_kinds` covers `#include`; everything else is keyed
// by `c_family_item_key`.
static C: MergeSpec = MergeSpec {
    import_kinds: &["preproc_include"],
    transparent_kinds: &[],
    inner_decl_fields: &[],
    name_fields: &[],
    key_strategy: KeyStrategy::CFamily,
    grammar: || tree_sitter_c::LANGUAGE.into(),
};

static CPP: MergeSpec = MergeSpec {
    import_kinds: &["preproc_include"],
    transparent_kinds: &[],
    inner_decl_fields: &[],
    name_fields: &[],
    key_strategy: KeyStrategy::CFamily,
    grammar: || tree_sitter_cpp::LANGUAGE.into(),
};

// Clojure flattens every parenthesized form to `list_lit`; the bespoke
// `Clojure` resolver reads the head symbol + defined name. There is no
// distinct import node (requires live in `ns`), so `import_kinds` is empty.
static CLOJURE: MergeSpec = MergeSpec {
    import_kinds: &[],
    transparent_kinds: &[],
    inner_decl_fields: &[],
    name_fields: &[],
    key_strategy: KeyStrategy::Clojure,
    grammar: || tree_sitter_clojure_orchard::LANGUAGE.into(),
};

// ===================================================================
// Per-item 3-way merge: classify + ordering/reconstruction + re-parse
// guard. Built on `parse_top_level_items`. Pure; no merge is wired into
// `auto_resolve_conflicts` yet (that's tsk136).
// ===================================================================

/// Outcome of a top-level structural 3-way merge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AstMerge {
    /// Every item was independently resolvable. The `String` is the
    /// reconstructed, re-parse-validated file content.
    Resolved(String),
    /// At least one item is a true semantic overlap (a divergent same-key
    /// edit, a delete-vs-edit, or an add/add with different text). The
    /// keys are the offending items, sorted. The caller leaves git's
    /// conflict markers — we only ever *reduce* the conflict count.
    Conflict(Vec<String>),
    /// We could not produce a result we trust. The caller also leaves
    /// git's markers. See [`BailReason`].
    Bail(BailReason),
}

/// Why a structural merge declined to produce a resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BailReason {
    /// One of base/ours/theirs failed to parse (error/missing node) — we
    /// never operate on an untrusted tree.
    SideParseFailed,
    /// A side had two top-level items with the same identity key, so the
    /// per-key 3-way classify would be ambiguous. Conservatively refuse.
    DuplicateKeys,
    /// The reconstructed source did not parse cleanly — discard it rather
    /// than emit a tree we can't trust.
    ReparseFailed,
}

/// Structural 3-way merge of the top-level items of `base`/`ours`/`theirs`
/// (all the same `lang`). Pure. See [`AstMerge`] for the outcomes.
///
/// Resolution rule (per the spike, lifted to production): per identity
/// key, take the side that changed, take agreement, and refuse the whole
/// file on any divergent same-key edit, delete-vs-edit, or add/add with
/// different text. Ordering is conservative and deterministic: base order
/// for surviving items, with each side's additions inserted after their
/// nearest surviving base anchor (ours before theirs on ties). The
/// reconstruction reuses each item's verbatim byte-span text (no reflow)
/// and is re-parsed as a guard before being returned.
pub fn merge_top_level(base: &str, ours: &str, theirs: &str, lang: Language) -> AstMerge {
    merge_with_reconstruct(base, ours, theirs, lang, reconstruct)
}

/// Inner pipeline parameterized on the reconstruction function so the
/// re-parse guard's discard path is directly testable (inject a
/// reconstructor that emits broken source and assert `Bail(ReparseFailed)`).
fn merge_with_reconstruct(
    base: &str,
    ours: &str,
    theirs: &str,
    lang: Language,
    reconstruct_fn: impl Fn(&[Item]) -> String,
) -> AstMerge {
    let (Some(b_items), Some(o_items), Some(t_items)) = (
        parse_top_level_items(base, lang),
        parse_top_level_items(ours, lang),
        parse_top_level_items(theirs, lang),
    ) else {
        return AstMerge::Bail(BailReason::SideParseFailed);
    };

    let (Some(bmap), Some(omap), Some(tmap)) = (
        index_by_key(&b_items),
        index_by_key(&o_items),
        index_by_key(&t_items),
    ) else {
        return AstMerge::Bail(BailReason::DuplicateKeys);
    };

    // Classify every key once. `resolved` holds the winning item per
    // surviving key; dropped keys are simply absent.
    let mut all_keys: Vec<&str> = bmap
        .keys()
        .chain(omap.keys())
        .chain(tmap.keys())
        .copied()
        .collect();
    all_keys.sort_unstable();
    all_keys.dedup();

    let mut resolved: BTreeMap<&str, &Item> = BTreeMap::new();
    let mut conflicts: Vec<String> = Vec::new();
    for &key in &all_keys {
        match classify(
            bmap.get(key).copied(),
            omap.get(key).copied(),
            tmap.get(key).copied(),
        ) {
            Ok(Some(item)) => {
                resolved.insert(key, item);
            }
            Ok(None) => {}
            Err(()) => conflicts.push(key.to_string()),
        }
    }
    if !conflicts.is_empty() {
        return AstMerge::Conflict(conflicts);
    }

    let merged = order_items(&b_items, &o_items, &t_items, &bmap, &omap, &resolved);
    let text = reconstruct_fn(&merged);

    // Re-parse guard: never hand back a tree we can't re-parse.
    if parse_top_level_items(&text, lang).is_none() {
        return AstMerge::Bail(BailReason::ReparseFailed);
    }
    AstMerge::Resolved(text)
}

/// Index items by identity key. `None` if a key repeats within one side
/// (ambiguous — the caller bails).
fn index_by_key(items: &[Item]) -> Option<BTreeMap<&str, &Item>> {
    let mut map = BTreeMap::new();
    for it in items {
        if map.insert(it.key.as_str(), it).is_some() {
            return None;
        }
    }
    Some(map)
}

/// Per-key 3-way classify. `Ok(Some)` ⇒ keep that item; `Ok(None)` ⇒
/// drop (deleted); `Err(())` ⇒ true overlap (refuse the file).
fn classify<'a>(
    b: Option<&'a Item>,
    o: Option<&'a Item>,
    t: Option<&'a Item>,
) -> Result<Option<&'a Item>, ()> {
    match (b, o, t) {
        // Present everywhere: classic 3-way on the verbatim item text.
        (Some(b), Some(o), Some(t)) => {
            if o.text == t.text {
                Ok(Some(o)) // both agree (incl. both-made-same-edit)
            } else if o.text == b.text {
                Ok(Some(t)) // only theirs changed
            } else if t.text == b.text {
                Ok(Some(o)) // only ours changed
            } else {
                Err(()) // divergent same-key edit
            }
        }
        // Deleted by one side, untouched by the other → drop.
        (Some(b), None, Some(t)) if t.text == b.text => Ok(None),
        (Some(b), Some(o), None) if o.text == b.text => Ok(None),
        // Delete vs edit — true overlap.
        (Some(_), None, Some(_)) | (Some(_), Some(_), None) => Err(()),
        // Deleted by both → drop.
        (Some(_), None, None) => Ok(None),
        // Added by exactly one side → the commutative win.
        (None, Some(o), None) => Ok(Some(o)),
        (None, None, Some(t)) => Ok(Some(t)),
        // Added by both: fine iff identical, else add/add overlap.
        (None, Some(o), Some(t)) => {
            if o.text == t.text {
                Ok(Some(o))
            } else {
                Err(())
            }
        }
        (None, None, None) => unreachable!("key came from some side"),
    }
}

/// Build the merged item list in a deterministic, conservative order:
/// base order for surviving base items, with each side's new items
/// inserted after their nearest surviving base anchor (ours before
/// theirs; items before any base anchor go to the front, ours first).
fn order_items<'a>(
    b_items: &'a [Item],
    o_items: &'a [Item],
    t_items: &'a [Item],
    bmap: &BTreeMap<&str, &Item>,
    omap: &BTreeMap<&str, &Item>,
    resolved: &BTreeMap<&str, &'a Item>,
) -> Vec<Item> {
    let base_keys: HashSet<&str> = bmap.keys().copied().collect();

    // Front bucket (no surviving base anchor precedes them) and
    // anchor → items inserted directly after that base key.
    let mut front: Vec<&Item> = Vec::new();
    let mut after: HashMap<&str, Vec<&Item>> = HashMap::new();

    let mut place = |side: &'a [Item], skip_both_add: bool| {
        let mut anchor: Option<&str> = None;
        for it in side {
            let k = it.key.as_str();
            if base_keys.contains(k) {
                // Only a *surviving* base item is a valid anchor.
                if resolved.contains_key(k) {
                    anchor = Some(k);
                }
                continue;
            }
            // An addition from this side. Skip both-add keys on the
            // theirs pass — ours already placed them.
            if skip_both_add && omap.contains_key(k) {
                continue;
            }
            if let Some(&item) = resolved.get(k) {
                match anchor {
                    Some(a) => after.entry(a).or_default().push(item),
                    None => front.push(item),
                }
            }
        }
    };
    place(o_items, false);
    place(t_items, true);

    let mut ordered: Vec<&Item> = Vec::new();
    ordered.extend(front);
    for it in b_items {
        let k = it.key.as_str();
        if let Some(&winner) = resolved.get(k) {
            ordered.push(winner);
            if let Some(extra) = after.get(k) {
                ordered.extend(extra.iter().copied());
            }
        }
    }

    ordered.into_iter().cloned().collect()
}

/// Reconstruct file text from ordered items. Each item's verbatim span
/// text is reused unchanged (no reflow); only the inter-item joins (a
/// single newline) are synthesized. Trailing newline included.
fn reconstruct(items: &[Item]) -> String {
    let mut out = String::new();
    for it in items {
        out.push_str(it.text.trim_end());
        out.push('\n');
        // A standalone comment block was detached by a blank line — that blank
        // line is what made it standalone, so put it back rather than gluing a
        // licence header onto the first declaration (tsk159).
        if it.key.starts_with(COMMENT_KEY_PREFIX) {
            out.push('\n');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Assert `src[item.byte_span] == item.text` for every item — the
    /// span is the authoritative source range.
    fn assert_spans_match(src: &str, items: &[Item]) {
        for it in items {
            assert_eq!(
                &src[it.byte_span.clone()],
                it.text,
                "span/text mismatch for {}",
                it.key
            );
        }
    }

    fn keys(items: &[Item]) -> Vec<&str> {
        items.iter().map(|i| i.key.as_str()).collect()
    }

    /// tsk159: a comment run separated from the next declaration by a blank
    /// line — a licence header, a section divider, a trailing note — belongs to
    /// no item, so `reconstruct` (which emits only `Item.text`) dropped it.
    /// The re-parse guard cannot catch this: source with the header removed
    /// parses perfectly.
    ///
    /// This falsified the documented invariant that Tier-2 "can only ever
    /// reduce the conflict count, never introduce new content" — it *removed*
    /// content, silently, in the most common file shape there is, and reported
    /// success.
    #[test]
    fn detached_comment_runs_survive_a_structural_merge() {
        let base = "// Copyright 2026 ACME Corp.\n\
                    // SPDX-License-Identifier: MIT\n\
                    \n\
                    use std::fmt;\n\
                    \n\
                    pub fn run() -> u32 {\n    1\n}\n\
                    \n\
                    // TODO: revisit the retry policy\n";
        let ours = base.replace("use std::fmt;", "use std::fmt;\nuse std::io;");
        let theirs = base.replace("use std::fmt;", "use std::fmt;\nuse std::net;");

        let AstMerge::Resolved(merged) = merge_top_level(base, &ours, &theirs, Language::Rust)
        else {
            panic!("both sides adding a different import must resolve structurally");
        };

        assert!(
            merged.contains("// Copyright 2026 ACME Corp."),
            "licence header dropped:\n{merged}"
        );
        assert!(
            merged.contains("// SPDX-License-Identifier: MIT"),
            "SPDX line dropped:\n{merged}"
        );
        assert!(
            merged.contains("// TODO: revisit the retry policy"),
            "trailing comment at EOF dropped:\n{merged}"
        );
        // The merge itself still has to work.
        assert!(merged.contains("use std::io;"), "{merged}");
        assert!(merged.contains("use std::net;"), "{merged}");
        assert!(merged.contains("pub fn run()"), "{merged}");
        // And the header must stay at the top, not float into the body.
        let header = merged.find("// Copyright").expect("header present");
        let first_decl = merged.find("use std::").expect("an import present");
        assert!(
            header < first_decl,
            "header must precede the declarations:\n{merged}"
        );
    }

    #[test]
    fn a_detached_comment_run_is_parsed_as_its_own_item() {
        let src = "// header\n\nuse std::fmt;\n\n// trailing\n";
        let items = parse_top_level_items(src, Language::Rust).expect("parses");
        assert_spans_match(src, &items);
        // header, the use, trailing — three items, in source order.
        assert_eq!(items.len(), 3, "got keys {:?}", keys(&items));
        assert!(items[0].text.contains("// header"));
        assert!(items[1].text.contains("use std::fmt;"));
        assert!(items[2].text.contains("// trailing"));
    }

    #[test]
    fn detached_comments_survive_in_python_too() {
        // The fix rides tree-sitter's `is_extra()`, so it should be
        // language-agnostic — but the original report named Python explicitly,
        // so pin it rather than assume.
        let base = "# Copyright 2026 ACME Corp.\n\nimport os\n\ndef run():\n    return 1\n";
        let ours = base.replace("import os", "import os\nimport sys");
        let theirs = base.replace("import os", "import os\nimport json");
        let AstMerge::Resolved(merged) = merge_top_level(base, &ours, &theirs, Language::Python)
        else {
            panic!("independent imports must resolve structurally");
        };
        assert!(
            merged.contains("# Copyright 2026 ACME Corp."),
            "header dropped:\n{merged}"
        );
        assert!(merged.contains("import sys"), "{merged}");
        assert!(merged.contains("import json"), "{merged}");
    }

    #[test]
    fn an_attached_doc_comment_still_rides_with_its_declaration() {
        // The existing behaviour must not regress: no blank line means the
        // comment belongs to the decl and moves with it.
        let src = "/// docs for run\npub fn run() {}\n";
        let items = parse_top_level_items(src, Language::Rust).expect("parses");
        assert_eq!(items.len(), 1, "got keys {:?}", keys(&items));
        assert!(items[0].text.contains("/// docs for run"));
        assert!(items[0].text.contains("pub fn run()"));
    }

    #[test]
    fn language_for_path_maps_supported_extensions() {
        assert_eq!(language_for_path("a/b.rs"), Some(Language::Rust));
        assert_eq!(language_for_path("x.ts"), Some(Language::TypeScript));
        assert_eq!(language_for_path("x.tsx"), Some(Language::Tsx));
        assert_eq!(language_for_path("x.js"), Some(Language::JavaScript));
        assert_eq!(language_for_path("x.mjs"), Some(Language::JavaScript));
        assert_eq!(language_for_path("x.jsx"), Some(Language::JavaScript));
        assert_eq!(language_for_path("x.py"), Some(Language::Python));
        assert_eq!(language_for_path("x.go"), Some(Language::Go));
        // Second-slice languages (tsk137).
        assert_eq!(language_for_path("x.java"), Some(Language::Java));
        assert_eq!(language_for_path("x.c"), Some(Language::C));
        assert_eq!(language_for_path("x.h"), Some(Language::C));
        assert_eq!(language_for_path("x.cpp"), Some(Language::Cpp));
        assert_eq!(language_for_path("x.hpp"), Some(Language::Cpp));
        assert_eq!(language_for_path("x.clj"), Some(Language::Clojure));
        assert_eq!(language_for_path("x.cljs"), Some(Language::Clojure));
        // Genuinely unsupported.
        assert_eq!(language_for_path("x.txt"), None);
        assert_eq!(language_for_path("noext"), None);
    }

    #[test]
    fn csharp_is_canonical_but_unsupported_for_merge() {
        // C# lives in the unified `Language` enum (tsk321) — it's analysed
        // for metrics — but the AST merge front half doesn't support it, so
        // it has no `MergeSpec`, `parse_top_level_items` bails gracefully,
        // and a `.cs` path is never picked up by the merge resolver.
        assert!(merge_spec(Language::CSharp).is_none());
        assert!(merge_spec(Language::Rust).is_some());
        assert!(parse_top_level_items("class C { void M() {} }", Language::CSharp).is_none());
        assert_eq!(language_for_path("x.cs"), None);
    }

    #[test]
    fn rust_extracts_top_level_items_with_keys() {
        let src = "use std::fmt;\nextern crate foo;\nfn a() {}\nstruct S;\nenum E { A }\n\
                   const K: u8 = 1;\ntrait T {}\nimpl S {}\n";
        let items = parse_top_level_items(src, Language::Rust).unwrap();
        assert_spans_match(src, &items);
        let k = keys(&items);
        assert!(k.contains(&"import::use std::fmt;"));
        assert!(k.contains(&"import::extern crate foo;"));
        assert!(k.contains(&"function_item::a"));
        assert!(k.contains(&"struct_item::S"));
        assert!(k.contains(&"enum_item::E"));
        assert!(k.contains(&"const_item::K"));
        assert!(k.contains(&"trait_item::T"));
        // Bare impl has no name field → kind + normalized text fallback.
        assert!(k.iter().any(|s| s.starts_with("impl_item::")));
    }

    #[test]
    fn rust_attaches_leading_doc_comment_to_following_item() {
        let src = "/// doc for a\nfn a() {}\n\n// detached\n\nfn b() {}\n";
        let items = parse_top_level_items(src, Language::Rust).unwrap();
        assert_spans_match(src, &items);
        let a = items.iter().find(|i| i.key == "function_item::a").unwrap();
        assert!(
            a.text.starts_with("/// doc for a"),
            "doc comment should be in the span: {:?}",
            a.text
        );
        // The blank-line-separated comment does NOT glue onto `b`.
        let b = items.iter().find(|i| i.key == "function_item::b").unwrap();
        assert!(
            b.text.starts_with("fn b"),
            "detached comment must not attach: {:?}",
            b.text
        );
    }

    #[test]
    fn rust_bails_on_parse_failure() {
        // Unbalanced brace → tree-sitter produces an ERROR node.
        assert!(parse_top_level_items("fn a() { ", Language::Rust).is_none());
    }

    #[test]
    fn typescript_imports_classes_and_exported_decls() {
        let src = "import { x } from 'm';\nfunction f() {}\nclass C {}\n\
                   interface I {}\ntype T = number;\nenum E { A }\n\
                   export function g() {}\nexport class D {}\nconst k = 1;\n";
        let items = parse_top_level_items(src, Language::TypeScript).unwrap();
        assert_spans_match(src, &items);
        let k = keys(&items);
        assert!(k.contains(&"import::import { x } from 'm';"));
        assert!(k.contains(&"function_declaration::f"));
        assert!(k.contains(&"class_declaration::C"));
        assert!(k.contains(&"interface_declaration::I"));
        assert!(k.contains(&"type_alias_declaration::T"));
        assert!(k.contains(&"enum_declaration::E"));
        // export wrapper is seen through to the inner decl identity.
        assert!(k.contains(&"function_declaration::g"));
        assert!(k.contains(&"class_declaration::D"));
        // `const` lands on the text fallback (declarator-named).
        assert!(k.iter().any(|s| s.starts_with("lexical_declaration::")));
    }

    #[test]
    fn tsx_parses_component_and_keeps_export_identity() {
        let src = "import React from 'react';\n\
                   export function App() { return <div>hi</div>; }\n";
        let items = parse_top_level_items(src, Language::Tsx).unwrap();
        assert_spans_match(src, &items);
        let k = keys(&items);
        assert!(k.contains(&"import::import React from 'react';"));
        assert!(k.contains(&"function_declaration::App"));
    }

    #[test]
    fn javascript_imports_and_functions() {
        let src = "import x from 'm';\nfunction f() {}\nclass C {}\nexport function g() {}\n";
        let items = parse_top_level_items(src, Language::JavaScript).unwrap();
        assert_spans_match(src, &items);
        let k = keys(&items);
        assert!(k.contains(&"import::import x from 'm';"));
        assert!(k.contains(&"function_declaration::f"));
        assert!(k.contains(&"class_declaration::C"));
        assert!(k.contains(&"function_declaration::g"));
    }

    #[test]
    fn python_imports_defs_classes_and_decorated() {
        let src = "import os\nfrom sys import path\n\
                   def f():\n    pass\nclass C:\n    pass\n\
                   @deco\ndef g():\n    pass\n";
        let items = parse_top_level_items(src, Language::Python).unwrap();
        assert_spans_match(src, &items);
        let k = keys(&items);
        assert!(k.contains(&"import::import os"));
        assert!(k.contains(&"import::from sys import path"));
        assert!(k.contains(&"function_definition::f"));
        assert!(k.contains(&"class_definition::C"));
        // decorated def: keyed by the wrapped function, decorator in text.
        assert!(k.contains(&"function_definition::g"));
        let g = items
            .iter()
            .find(|i| i.key == "function_definition::g")
            .unwrap();
        assert!(
            g.text.starts_with("@deco"),
            "decorator rides along: {:?}",
            g.text
        );
    }

    #[test]
    fn python_bails_on_parse_failure() {
        // A `def` with no body is a syntax error in Python.
        assert!(parse_top_level_items("def f(:\n", Language::Python).is_none());
    }

    #[test]
    fn go_package_imports_funcs_methods_and_types() {
        let src = "package main\n\
                   import \"fmt\"\n\
                   func F() {}\n\
                   func (s S) M() {}\n\
                   type S struct{}\n\
                   const K = 1\n";
        let items = parse_top_level_items(src, Language::Go).unwrap();
        assert_spans_match(src, &items);
        let k = keys(&items);
        assert!(k.contains(&"import::import \"fmt\""));
        assert!(k.contains(&"function_declaration::F"));
        // Method disambiguated by receiver.
        assert!(
            k.iter()
                .any(|s| s.starts_with("method_declaration::") && s.ends_with("::M")),
            "method keyed with receiver: {k:?}"
        );
        // package clause and grouped type/const land on text fallback.
        assert!(k.iter().any(|s| s.starts_with("package_clause::")));
        assert!(k.iter().any(|s| s.starts_with("type_declaration::")));
    }

    #[test]
    fn java_imports_and_type_declarations() {
        let src = "package p;\nimport java.util.List;\n\
                   class C { int x; void m(int a) {} }\n\
                   interface I {}\nenum E { A }\nrecord R(int a) {}\n";
        let items = parse_top_level_items(src, Language::Java).unwrap();
        assert_spans_match(src, &items);
        let k = keys(&items);
        assert!(k.contains(&"import::import java.util.List;"));
        assert!(k.contains(&"class_declaration::C"));
        assert!(k.contains(&"interface_declaration::I"));
        assert!(k.contains(&"enum_declaration::E"));
        assert!(k.contains(&"record_declaration::R"));
        // package clause has no name field → text fallback.
        assert!(k.iter().any(|s| s.starts_with("package_declaration::")));
    }

    #[test]
    fn c_includes_functions_structs_typedefs_globals() {
        let src = "#include <stdio.h>\nint g = 1;\ntypedef int myint;\n\
                   struct S { int a; };\nint main(void) { return 0; }\n\
                   void foo(int x) {}\n";
        let items = parse_top_level_items(src, Language::C).unwrap();
        assert_spans_match(src, &items);
        let k = keys(&items);
        assert!(k.contains(&"import::#include <stdio.h>"));
        assert!(k.contains(&"declaration::g"));
        assert!(k.contains(&"type_definition::myint"));
        assert!(k.contains(&"struct_specifier::S"));
        // Functions keyed by declarator (name + signature).
        assert!(k.contains(&"function_definition::main(void)"));
        assert!(k.contains(&"function_definition::foo(int x)"));
    }

    #[test]
    fn cpp_overloads_stay_distinct_and_template_seen_through() {
        let src = "#include <vector>\nnamespace ns { int v; }\n\
                   class C {};\nstruct S {};\n\
                   template<typename T> T id(T x) { return x; }\n\
                   void f(int a) {}\nvoid f(double a) {}\n";
        let items = parse_top_level_items(src, Language::Cpp).unwrap();
        assert_spans_match(src, &items);
        let k = keys(&items);
        assert!(k.contains(&"import::#include <vector>"));
        assert!(k.contains(&"namespace_definition::ns"));
        assert!(k.contains(&"class_specifier::C"));
        assert!(k.contains(&"struct_specifier::S"));
        // Overloads keyed by signature stay distinct.
        assert!(k.contains(&"function_definition::f(int a)"));
        assert!(k.contains(&"function_definition::f(double a)"));
        // template_declaration is seen through to the wrapped function.
        assert!(k.contains(&"function_definition::id(T x)"));
    }

    #[test]
    fn clojure_def_forms_keyed_by_head_and_name() {
        let src = "(ns my.app)\n(def x 1)\n(defn foo [a] a)\n\
                   (defn- bar [] 2)\n(defmacro m [] nil)\n";
        let items = parse_top_level_items(src, Language::Clojure).unwrap();
        assert_spans_match(src, &items);
        let k = keys(&items);
        assert!(k.contains(&"ns::my.app"));
        assert!(k.contains(&"def::x"));
        assert!(k.contains(&"defn::foo"));
        assert!(k.contains(&"defn-::bar"));
        assert!(k.contains(&"defmacro::m"));
    }

    #[test]
    fn java_both_add_classes_resolve() {
        let base = "import a.A;\nclass Base {}\n";
        let ours = "import a.A;\nclass Base {}\nclass Ours {}\n";
        let theirs = "import a.A;\nclass Base {}\nclass Theirs {}\n";
        let text = resolved_text(merge_top_level(base, ours, theirs, Language::Java));
        assert!(text.contains("class Ours"), "{text}");
        assert!(text.contains("class Theirs"), "{text}");
        assert!(parse_top_level_items(&text, Language::Java).is_some());
    }

    #[test]
    fn c_both_add_functions_resolve() {
        let base = "int shared(void) { return 0; }\n";
        let ours = "int shared(void) { return 0; }\nint ours(void) { return 1; }\n";
        let theirs = "int shared(void) { return 0; }\nint theirs(void) { return 2; }\n";
        let text = resolved_text(merge_top_level(base, ours, theirs, Language::C));
        assert!(text.contains("int ours"), "{text}");
        assert!(text.contains("int theirs"), "{text}");
        assert!(parse_top_level_items(&text, Language::C).is_some());
    }

    #[test]
    fn c_same_function_edited_differently_refuses() {
        let base = "int f(void) { return 1; }\n";
        let ours = "int f(void) { return 2; }\n";
        let theirs = "int f(void) { return 3; }\n";
        match merge_top_level(base, ours, theirs, Language::C) {
            AstMerge::Conflict(keys) => assert!(
                keys.iter()
                    .any(|k| k.starts_with("function_definition::f(void)")),
                "{keys:?}"
            ),
            other => panic!("must refuse divergent body edit, got {other:?}"),
        }
    }

    #[test]
    fn cpp_both_add_imports_resolve() {
        let base = "#include <a>\nint main() { return 0; }\n";
        let ours = "#include <a>\n#include <b>\nint main() { return 0; }\n";
        let theirs = "#include <a>\n#include <c>\nint main() { return 0; }\n";
        let text = resolved_text(merge_top_level(base, ours, theirs, Language::Cpp));
        assert!(text.contains("#include <b>"), "{text}");
        assert!(text.contains("#include <c>"), "{text}");
        assert!(parse_top_level_items(&text, Language::Cpp).is_some());
    }

    #[test]
    fn clojure_both_add_defns_resolve() {
        let base = "(ns app)\n(defn shared [] 0)\n";
        let ours = "(ns app)\n(defn shared [] 0)\n(defn ours [] 1)\n";
        let theirs = "(ns app)\n(defn shared [] 0)\n(defn theirs [] 2)\n";
        let text = resolved_text(merge_top_level(base, ours, theirs, Language::Clojure));
        assert!(text.contains("(defn ours"), "{text}");
        assert!(text.contains("(defn theirs"), "{text}");
        assert!(parse_top_level_items(&text, Language::Clojure).is_some());
    }

    #[test]
    fn clojure_same_defn_edited_differently_refuses() {
        let base = "(defn f [] 1)\n";
        let ours = "(defn f [] 2)\n";
        let theirs = "(defn f [] 3)\n";
        match merge_top_level(base, ours, theirs, Language::Clojure) {
            AstMerge::Conflict(keys) => {
                assert!(keys.iter().any(|k| k == "defn::f"), "{keys:?}")
            }
            other => panic!("must refuse, got {other:?}"),
        }
    }

    #[test]
    fn empty_source_yields_no_items() {
        assert!(parse_top_level_items("", Language::Rust)
            .unwrap()
            .is_empty());
        assert!(parse_top_level_items("\n\n", Language::Python)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn ordering_is_source_order() {
        let src = "fn first() {}\nfn second() {}\nfn third() {}\n";
        let items = parse_top_level_items(src, Language::Rust).unwrap();
        let k = keys(&items);
        assert_eq!(
            k,
            vec![
                "function_item::first",
                "function_item::second",
                "function_item::third"
            ]
        );
    }

    // ---- per-item 3-way merge ----

    fn resolved_text(m: AstMerge) -> String {
        match m {
            AstMerge::Resolved(s) => s,
            other => panic!("expected Resolved, got {other:?}"),
        }
    }

    #[test]
    fn rust_both_add_different_imports_resolves_in_order() {
        // The flagship Tier-2 case: two new `use`s, order-insensitive.
        let base = "use std::fmt;\n";
        let ours = "use std::collections::HashMap;\nuse std::fmt;\n";
        let theirs = "use std::fmt;\nuse std::io;\n";
        let text = resolved_text(merge_top_level(base, ours, theirs, Language::Rust));
        // ours-add (front) → base survivor → theirs-add (anchored after).
        assert_eq!(
            text,
            "use std::collections::HashMap;\nuse std::fmt;\nuse std::io;\n"
        );
    }

    #[test]
    fn rust_independently_added_fns_resolve() {
        let base = "fn shared() {}\n";
        let ours = "fn shared() {}\nfn from_ours() {}\n";
        let theirs = "fn shared() {}\nfn from_theirs() {}\n";
        let text = resolved_text(merge_top_level(base, ours, theirs, Language::Rust));
        assert!(text.contains("fn from_ours"), "{text}");
        assert!(text.contains("fn from_theirs"), "{text}");
        // Both new fns anchor after the surviving `shared`, ours first.
        let o = text.find("from_ours").unwrap();
        let t = text.find("from_theirs").unwrap();
        let s = text.find("fn shared").unwrap();
        assert!(
            s < o && o < t,
            "order ours-before-theirs after anchor: {text}"
        );
    }

    #[test]
    fn rust_body_edit_on_one_side_add_on_other_resolves() {
        let base = "fn a() { 1 }\n";
        let ours = "fn a() { 2 }\n"; // body edit
        let theirs = "fn a() { 1 }\nfn b() {}\n"; // new fn
        let text = resolved_text(merge_top_level(base, ours, theirs, Language::Rust));
        assert!(text.contains("fn a() { 2 }"), "edited body wins: {text}");
        assert!(text.contains("fn b()"), "{text}");
    }

    #[test]
    fn rust_both_delete_same_item_resolves() {
        let base = "fn a() {}\nfn b() {}\n";
        let ours = "fn b() {}\n";
        let theirs = "fn b() {}\n";
        let text = resolved_text(merge_top_level(base, ours, theirs, Language::Rust));
        assert!(!text.contains("fn a()"), "a deleted by both: {text}");
        assert!(text.contains("fn b()"), "{text}");
    }

    #[test]
    fn rust_same_item_edited_differently_refuses() {
        let base = "fn calc() { a() }\n";
        let ours = "fn calc() { b() }\n";
        let theirs = "fn calc() { c() }\n";
        match merge_top_level(base, ours, theirs, Language::Rust) {
            AstMerge::Conflict(keys) => {
                assert!(
                    keys.iter().any(|k| k.starts_with("function_item::calc")),
                    "{keys:?}"
                )
            }
            other => panic!("must refuse divergent same-item edit, got {other:?}"),
        }
    }

    #[test]
    fn rust_add_add_with_different_bodies_refuses() {
        let base = "fn keep() {}\n";
        let ours = "fn keep() {}\nfn n() { 1 }\n";
        let theirs = "fn keep() {}\nfn n() { 2 }\n";
        match merge_top_level(base, ours, theirs, Language::Rust) {
            AstMerge::Conflict(keys) => {
                assert!(
                    keys.iter().any(|k| k.starts_with("function_item::n")),
                    "{keys:?}"
                )
            }
            other => panic!("must refuse add/add divergence, got {other:?}"),
        }
    }

    #[test]
    fn rust_delete_vs_edit_refuses() {
        let base = "fn a() { 1 }\n";
        let ours = "\n"; // a deleted
        let theirs = "fn a() { 2 }\n"; // a edited
        match merge_top_level(base, ours, theirs, Language::Rust) {
            AstMerge::Conflict(keys) => {
                assert!(
                    keys.iter().any(|k| k.starts_with("function_item::a")),
                    "{keys:?}"
                )
            }
            other => panic!("must refuse delete-vs-edit, got {other:?}"),
        }
    }

    #[test]
    fn ordering_front_additions_are_deterministic_ours_then_theirs() {
        // Both sides add a fn *before* the surviving base item → both land
        // in the front bucket, ours first, deterministically.
        let base = "fn base() {}\n";
        let ours = "fn o_new() {}\nfn base() {}\n";
        let theirs = "fn t_new() {}\nfn base() {}\n";
        let text = resolved_text(merge_top_level(base, ours, theirs, Language::Rust));
        assert_eq!(text, "fn o_new() {}\nfn t_new() {}\nfn base() {}\n");
    }

    #[test]
    fn reparse_guard_discards_broken_reconstruction() {
        // Inject a reconstructor that emits un-parseable source; even
        // though the classify is clean, the guard must bail.
        let base = "use std::fmt;\n";
        let ours = "use std::io;\nuse std::fmt;\n";
        let theirs = "use std::fmt;\nuse std::cmp;\n";
        let out = merge_with_reconstruct(base, ours, theirs, Language::Rust, |_| {
            "fn broken( {\n".to_string()
        });
        assert_eq!(out, AstMerge::Bail(BailReason::ReparseFailed));
        // Sanity: the real reconstructor resolves the same inputs.
        assert!(matches!(
            merge_top_level(base, ours, theirs, Language::Rust),
            AstMerge::Resolved(_)
        ));
    }

    #[test]
    fn unparseable_side_bails() {
        let base = "fn a() {}\n";
        let ours = "fn a() { "; // unbalanced → parse error
        let theirs = "fn a() {}\nfn b() {}\n";
        assert_eq!(
            merge_top_level(base, ours, theirs, Language::Rust),
            AstMerge::Bail(BailReason::SideParseFailed)
        );
    }

    #[test]
    fn typescript_both_add_imports_resolve() {
        let base = "import a from 'a';\n";
        let ours = "import b from 'b';\nimport a from 'a';\n";
        let theirs = "import a from 'a';\nimport c from 'c';\n";
        let text = resolved_text(merge_top_level(base, ours, theirs, Language::TypeScript));
        assert_eq!(
            text,
            "import b from 'b';\nimport a from 'a';\nimport c from 'c';\n"
        );
    }

    #[test]
    fn javascript_body_edit_vs_add_resolves() {
        let base = "function a() { return 1; }\n";
        let ours = "function a() { return 2; }\n";
        let theirs = "function a() { return 1; }\nfunction b() {}\n";
        let text = resolved_text(merge_top_level(base, ours, theirs, Language::JavaScript));
        assert!(text.contains("return 2"), "edited body wins: {text}");
        assert!(text.contains("function b()"), "{text}");
    }

    #[test]
    fn python_both_add_functions_resolve() {
        let base = "def shared():\n    pass\n";
        let ours = "def shared():\n    pass\ndef o():\n    pass\n";
        let theirs = "def shared():\n    pass\ndef t():\n    pass\n";
        let text = resolved_text(merge_top_level(base, ours, theirs, Language::Python));
        assert!(text.contains("def o():"), "{text}");
        assert!(text.contains("def t():"), "{text}");
        // Reconstruction must itself be valid Python (re-parse guard passed).
        assert!(parse_top_level_items(&text, Language::Python).is_some());
    }

    #[test]
    fn python_same_def_edited_differently_refuses() {
        let base = "def f():\n    return 1\n";
        let ours = "def f():\n    return 2\n";
        let theirs = "def f():\n    return 3\n";
        match merge_top_level(base, ours, theirs, Language::Python) {
            AstMerge::Conflict(keys) => {
                assert!(
                    keys.iter().any(|k| k.starts_with("function_definition::f")),
                    "{keys:?}"
                )
            }
            other => panic!("must refuse, got {other:?}"),
        }
    }

    #[test]
    fn go_both_add_functions_resolve_keeping_package_first() {
        let base = "package main\nfunc S() {}\n";
        let ours = "package main\nfunc S() {}\nfunc O() {}\n";
        let theirs = "package main\nfunc S() {}\nfunc T() {}\n";
        let text = resolved_text(merge_top_level(base, ours, theirs, Language::Go));
        assert!(
            text.starts_with("package main"),
            "package stays first: {text}"
        );
        assert!(text.contains("func O()"), "{text}");
        assert!(text.contains("func T()"), "{text}");
        assert!(parse_top_level_items(&text, Language::Go).is_some());
    }
}
