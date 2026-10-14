//! Symbol extraction — what a file defines, and the id oxplow names each
//! definition by (`.context/language-support.md` "Symbols").
//!
//! Each language describes its definitions as **data**: a tree-sitter
//! query (`languages/<lang>/symbols.scm`) and an optional spec
//! (`languages/<lang>/symbols.yaml`). Nothing here is per-language code,
//! so a language contributed later brings the same two files.
//!
//! The query's captures:
//!
//! | capture | meaning |
//! |---|---|
//! | `@definition.<kind>` | the definition (its extent; a callable's metrics run on it); `<kind>` is a [`SymbolKind`] |
//! | `@name` | its name (several spell one name in source order); a name spelled with `::` (C++ `geo::area`) puts the leading parts in its container |
//! | `@scope` | a container named outside the nesting (Go's receiver type) |
//! | `@parameters` | the parameter list whose types make the signature (overloading languages) |
//! | `@signature` | a node whose text is the signature as written (a Clojure dispatch value) |
//! | `@container` + `@container.name` (+ `@container.trait`) | a node that only nests definitions (a Rust `impl`) |
//! | `@_…` | used by predicates only |
//!
//! A definition's id is its containers' names and its own joined by
//! `::`, its signature in parentheses when it has one, and `~n` for the
//! nth definition with that same id in the file (in position order).

use std::collections::HashMap;
use std::sync::OnceLock;

use serde::Deserialize;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Node, Query, QueryCursor, Tree};

use crate::spec::Language;

/// What a definition is — one vocabulary for every language.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SymbolKind {
    Function,
    Method,
    Constructor,
    Class,
    Struct,
    Enum,
    EnumMember,
    Interface,
    Trait,
    Type,
    Field,
    Constant,
    Variable,
    Module,
    Namespace,
    Macro,
}

impl SymbolKind {
    pub const ALL: [SymbolKind; 16] = [
        SymbolKind::Function,
        SymbolKind::Method,
        SymbolKind::Constructor,
        SymbolKind::Class,
        SymbolKind::Struct,
        SymbolKind::Enum,
        SymbolKind::EnumMember,
        SymbolKind::Interface,
        SymbolKind::Trait,
        SymbolKind::Type,
        SymbolKind::Field,
        SymbolKind::Constant,
        SymbolKind::Variable,
        SymbolKind::Module,
        SymbolKind::Namespace,
        SymbolKind::Macro,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            SymbolKind::Function => "function",
            SymbolKind::Method => "method",
            SymbolKind::Constructor => "constructor",
            SymbolKind::Class => "class",
            SymbolKind::Struct => "struct",
            SymbolKind::Enum => "enum",
            SymbolKind::EnumMember => "enum_member",
            SymbolKind::Interface => "interface",
            SymbolKind::Trait => "trait",
            SymbolKind::Type => "type",
            SymbolKind::Field => "field",
            SymbolKind::Constant => "constant",
            SymbolKind::Variable => "variable",
            SymbolKind::Module => "module",
            SymbolKind::Namespace => "namespace",
            SymbolKind::Macro => "macro",
        }
    }

    pub fn parse(text: &str) -> Option<SymbolKind> {
        Self::ALL.into_iter().find(|k| k.as_str() == text)
    }

    /// Something that runs: it has complexity, length and parameters.
    pub fn is_callable(&self) -> bool {
        matches!(
            self,
            SymbolKind::Function | SymbolKind::Method | SymbolKind::Constructor
        )
    }

    /// A type: a function directly inside one is a method.
    pub fn is_type(&self) -> bool {
        matches!(
            self,
            SymbolKind::Class
                | SymbolKind::Struct
                | SymbolKind::Enum
                | SymbolKind::Interface
                | SymbolKind::Trait
                | SymbolKind::Type
        )
    }
}

/// One definition in a file. Lines and columns are 1-based; the end is
/// exclusive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Definition {
    pub kind: SymbolKind,
    /// Its own name.
    pub name: String,
    /// The names of what it's inside, outermost first.
    pub container: Vec<String>,
    /// Its parameter types (or dispatch value) as written, without the
    /// parentheses; only in languages that tell overloads apart by them.
    pub signature: Option<String>,
    /// What oxplow names it by within its file: `Parser::parse(String,int)`,
    /// `main~2`. The symbol's ref is `symbol:<path>/<id>`.
    pub id: String,
    pub name_line: u32,
    pub name_col: u32,
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
}

/// How a language's parameters become a signature.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct SignatureSpec {
    /// Fields of a parameter node holding its name; a composite (a C++
    /// declarator) loses only its identifier, keeping `&` / `*`.
    #[serde(default)]
    name_fields: Vec<String>,
    /// Child kinds holding the name when no field does (Java's varargs
    /// `variable_declarator`).
    #[serde(default)]
    name_kinds: Vec<String>,
}

/// `symbols.yaml`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct SymbolsFile {
    /// How a container naming a trait is spelled, from `{type}` and
    /// `{trait}` (Rust: `<{type} as {trait}>`).
    #[serde(default)]
    trait_container: Option<String>,
    #[serde(default)]
    signature: SignatureSpec,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    Definition(SymbolKind),
    Name,
    Scope,
    Parameters,
    Signature,
    Container,
    ContainerName,
    ContainerTrait,
    Predicate,
}

/// A language's compiled symbol query and spec.
pub struct SymbolSpec {
    query: Query,
    roles: Vec<Role>,
    file: SymbolsFile,
}

impl SymbolSpec {
    /// Compile `query` against `language`'s grammar, with `yaml` as its
    /// `symbols.yaml`. Refuses a capture outside the vocabulary.
    pub fn compile(language: Language, query: &str, yaml: Option<&str>) -> Result<Self, String> {
        let query = Query::new(&language.tree_sitter_language(), query)
            .map_err(|e| format!("symbols.scm: {e}"))?;
        let roles = query
            .capture_names()
            .iter()
            .map(|name| role(name))
            .collect::<Result<Vec<_>, _>>()?;
        let file = match yaml {
            Some(text) => serde_yaml::from_str(text).map_err(|e| format!("symbols.yaml: {e}"))?,
            None => SymbolsFile::default(),
        };
        Ok(SymbolSpec { query, roles, file })
    }
}

fn role(name: &str) -> Result<Role, String> {
    if let Some(kind) = name.strip_prefix("definition.") {
        return SymbolKind::parse(kind)
            .map(Role::Definition)
            .ok_or_else(|| {
                format!(
                    "symbols.scm: `{name}` isn't a symbol kind (one of {})",
                    SymbolKind::ALL.map(|k| k.as_str()).join(", ")
                )
            });
    }
    match name {
        "name" => Ok(Role::Name),
        "scope" => Ok(Role::Scope),
        "parameters" => Ok(Role::Parameters),
        "signature" => Ok(Role::Signature),
        "container" => Ok(Role::Container),
        "container.name" => Ok(Role::ContainerName),
        "container.trait" => Ok(Role::ContainerTrait),
        n if n.starts_with('_') => Ok(Role::Predicate),
        n => Err(format!(
            "symbols.scm: `{n}` isn't a symbol capture (definition.<kind>, name, scope, \
             parameters, signature, container, container.name, container.trait, or _… for \
             predicates)"
        )),
    }
}

/// The built-in `symbols.scm` / `symbols.yaml` of `language`.
fn builtin(language: Language) -> (&'static str, Option<&'static str>) {
    match language {
        Language::Rust => (
            include_str!("../languages/rust/symbols.scm"),
            Some(include_str!("../languages/rust/symbols.yaml")),
        ),
        Language::TypeScript | Language::Tsx => {
            (include_str!("../languages/typescript/symbols.scm"), None)
        }
        Language::JavaScript => (include_str!("../languages/javascript/symbols.scm"), None),
        Language::Python => (include_str!("../languages/python/symbols.scm"), None),
        Language::Go => (include_str!("../languages/go/symbols.scm"), None),
        Language::Java => (
            include_str!("../languages/java/symbols.scm"),
            Some(include_str!("../languages/java/symbols.yaml")),
        ),
        Language::C => (include_str!("../languages/c/symbols.scm"), None),
        Language::Cpp => (
            include_str!("../languages/cpp/symbols.scm"),
            Some(include_str!("../languages/cpp/symbols.yaml")),
        ),
        Language::Clojure => (include_str!("../languages/clojure/symbols.scm"), None),
        Language::CSharp => (
            include_str!("../languages/csharp/symbols.scm"),
            Some(include_str!("../languages/csharp/symbols.yaml")),
        ),
    }
}

/// `language`'s compiled built-in spec (compiled once).
pub(crate) fn symbol_spec(language: Language) -> Result<&'static SymbolSpec, String> {
    static SPECS: OnceLock<HashMap<Language, Result<SymbolSpec, String>>> = OnceLock::new();
    let specs = SPECS.get_or_init(|| {
        crate::registry()
            .iter()
            .map(|s| {
                let (query, yaml) = builtin(s.language);
                (s.language, SymbolSpec::compile(s.language, query, yaml))
            })
            .collect()
    });
    match specs.get(&language) {
        Some(Ok(spec)) => Ok(spec),
        Some(Err(e)) => Err(format!("{}: {e}", language.name())),
        None => Err(format!("{}: no symbol spec", language.name())),
    }
}

/// The definitions in `source`, a file at `path`, in source order. Empty
/// for a file of no supported language or one that doesn't parse.
pub fn extract(path: &str, source: &str) -> Vec<Definition> {
    let Some(language) = crate::language_for_path(path) else {
        return Vec::new();
    };
    extract_with_language(source, language)
}

/// [`extract`] with the language given.
pub fn extract_with_language(source: &str, language: Language) -> Vec<Definition> {
    let Some(tree) = crate::parse(source, language) else {
        return Vec::new();
    };
    extract_tree(&tree, source.as_bytes(), language)
        .into_iter()
        .map(|(d, _)| d)
        .collect()
}

/// One captured definition or nesting-only container, before nesting.
struct Raw<'t> {
    node: Node<'t>,
    pattern: usize,
    /// `None`: a container that only nests.
    kind: Option<SymbolKind>,
    name: String,
    name_node: Option<Node<'t>>,
    scope: Vec<String>,
    signature: Option<String>,
}

/// The definitions in a parsed tree with their nodes, in source order.
pub(crate) fn extract_tree<'t>(
    tree: &'t Tree,
    src: &[u8],
    language: Language,
) -> Vec<(Definition, Node<'t>)> {
    let spec = match symbol_spec(language) {
        Ok(spec) => spec,
        Err(_) => return Vec::new(),
    };
    let mut raws: Vec<Raw<'t>> = Vec::new();
    let mut by_node: HashMap<usize, usize> = HashMap::new();
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&spec.query, tree.root_node(), src);
    while let Some(m) = matches.next() {
        let Some(raw) = raw_of(spec, m, src) else {
            continue;
        };
        // One node matched by several patterns is the first pattern's.
        match by_node.get(&raw.node.id()) {
            Some(&i) if raws[i].pattern <= raw.pattern => {}
            Some(&i) => raws[i] = raw,
            None => {
                by_node.insert(raw.node.id(), raws.len());
                raws.push(raw);
            }
        }
    }
    raws.sort_by_key(|r| {
        (
            r.node.start_byte(),
            std::cmp::Reverse(r.node.end_byte()),
            r.pattern,
        )
    });

    // Nest by containment: each entry's parent is the innermost earlier
    // entry whose range holds it.
    struct Open {
        end: usize,
        path: Vec<String>,
        kind: Option<SymbolKind>,
    }
    let mut stack: Vec<Open> = Vec::new();
    let mut out: Vec<(Definition, Node<'t>)> = Vec::new();
    let mut seen: HashMap<String, u32> = HashMap::new();
    for raw in raws {
        let (start, end) = (raw.node.start_byte(), raw.node.end_byte());
        while stack.last().is_some_and(|o| o.end < end || o.end <= start) {
            stack.pop();
        }
        let parent = stack.last();
        let mut container = parent.map(|o| o.path.clone()).unwrap_or_default();
        let parent_nests_methods = parent.is_some_and(|o| o.kind.is_none_or(|k| k.is_type()));
        let scoped = !raw.scope.is_empty();
        container.extend(raw.scope);
        let mut path = container.clone();
        path.push(raw.name.clone());
        stack.push(Open {
            end,
            path,
            kind: raw.kind,
        });
        let Some(mut kind) = raw.kind else {
            continue;
        };
        if kind == SymbolKind::Function && (parent_nests_methods || scoped) {
            kind = SymbolKind::Method;
        }
        let mut qualified = container
            .iter()
            .chain([&raw.name])
            .cloned()
            .collect::<Vec<_>>()
            .join("::");
        if let Some(sig) = &raw.signature {
            qualified = format!("{qualified}({sig})");
        }
        let nth = seen.entry(qualified.clone()).or_insert(0);
        *nth += 1;
        let id = match *nth {
            1 => qualified,
            n => format!("{qualified}~{n}"),
        };
        let at = raw.name_node.unwrap_or(raw.node).start_position();
        let (s, e) = (raw.node.start_position(), raw.node.end_position());
        out.push((
            Definition {
                kind,
                name: raw.name,
                container,
                signature: raw.signature,
                id,
                name_line: at.row as u32 + 1,
                name_col: at.column as u32 + 1,
                start_line: s.row as u32 + 1,
                start_col: s.column as u32 + 1,
                end_line: e.row as u32 + 1,
                end_col: e.column as u32 + 1,
            },
            raw.node,
        ));
    }
    out
}

fn raw_of<'t>(
    spec: &SymbolSpec,
    m: &tree_sitter::QueryMatch<'_, 't>,
    src: &[u8],
) -> Option<Raw<'t>> {
    let mut def: Option<(Node<'t>, Option<SymbolKind>)> = None;
    let (mut names, mut scope, mut params, mut signature) = (Vec::new(), Vec::new(), None, None);
    let (mut container_name, mut container_trait) = (None, None);
    for cap in m.captures() {
        let node = cap.node;
        match spec.roles[cap.index as usize] {
            Role::Definition(kind) => def = Some((node, Some(kind))),
            Role::Container => def = Some((node, None)),
            Role::Name => names.push(node),
            Role::Scope => scope.push(type_name(text(node, src))),
            Role::Parameters => params = Some(node),
            Role::Signature => signature = Some(normalize(text(node, src))),
            Role::ContainerName => container_name = Some(type_name(text(node, src))),
            Role::ContainerTrait => container_trait = Some(normalize(text(node, src))),
            Role::Predicate => {}
        }
    }
    let (node, kind) = def?;
    let pattern = m.pattern_index;
    if kind.is_none() {
        let ty = container_name?;
        let name = match (container_trait, &spec.file.trait_container) {
            (Some(tr), Some(template)) => template.replace("{type}", &ty).replace("{trait}", &tr),
            _ => ty,
        };
        return Some(Raw {
            node,
            pattern,
            kind,
            name,
            name_node: None,
            scope,
            signature: None,
        });
    }
    // Several `@name` captures spell one name in source order (C#'s
    // destructor: the `~` token, then the identifier).
    names.sort_by_key(|n| n.start_byte());
    let name_node = *names.first()?;
    let spelled = normalize(&names.iter().map(|n| text(*n, src)).collect::<String>());
    let mut parts: Vec<String> = spelled.split("::").map(|p| p.trim().to_string()).collect();
    let own = parts.pop().filter(|n| !n.is_empty())?;
    scope.extend(parts.iter().map(|p| type_name(p)).filter(|p| !p.is_empty()));
    let signature = signature.or_else(|| params.map(|p| parameter_types(p, src, &spec.file)));
    Some(Raw {
        node,
        pattern,
        kind,
        name: own,
        name_node: Some(name_node),
        scope,
        signature,
    })
}

fn text<'s>(node: Node<'_>, src: &'s [u8]) -> &'s str {
    node.utf8_text(src).unwrap_or_default()
}

/// Whitespace runs as one space, trimmed.
fn normalize(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A type as a container names it: no reference, pointer or generic
/// arguments (`&'a Stack<T>` → `Stack`).
fn type_name(s: &str) -> String {
    let s = normalize(s);
    let s = s.trim_start_matches(['&', '*']);
    let s = s.strip_prefix("'").map_or(s, |rest| {
        rest.split_once(' ').map_or(rest, |(_, after)| after)
    });
    let s = s.strip_prefix("mut ").unwrap_or(s);
    let cut = s.find(['<', '[']).unwrap_or(s.len());
    s[..cut].trim().to_string()
}

/// The parameter list's types: each parameter as written, less its name
/// and anything from a `=` on (a default), joined by `,`.
fn parameter_types(params: Node<'_>, src: &[u8], file: &SymbolsFile) -> String {
    let sig = &file.signature;
    let mut types = Vec::new();
    let mut cursor = params.walk();
    for param in params.named_children(&mut cursor) {
        if param.kind().contains("comment") {
            continue;
        }
        let mut cut: Vec<(usize, usize)> = Vec::new();
        let named = sig
            .name_fields
            .iter()
            .find_map(|f| param.child_by_field_name(f))
            .or_else(|| {
                let mut c = param.walk();
                let found = param
                    .named_children(&mut c)
                    .find(|n| sig.name_kinds.iter().any(|k| k == n.kind()));
                found
            });
        if let Some(id) = named.and_then(identifier_in) {
            cut.push((id.start_byte(), id.end_byte()));
        }
        cut.sort();
        let (from, to) = (param.start_byte(), param.end_byte());
        let mut kept = String::new();
        let mut at = from;
        for (s, e) in cut {
            if s >= at {
                kept.push_str(std::str::from_utf8(&src[at..s]).unwrap_or_default());
                at = e;
            }
        }
        kept.push_str(std::str::from_utf8(&src[at..to]).unwrap_or_default());
        let kept = normalize(&kept);
        // A default value (`int n = 0`) is never part of the type.
        let kept = kept
            .split_once('=')
            .map_or(kept.as_str(), |(ty, _)| ty)
            .trim();
        // `Shape &` reads as `Shape&`: no space before a trailing sigil.
        let kept = kept.replace(" &", "&").replace(" *", "*");
        types.push(kept);
    }
    types.join(",")
}

/// `node` itself when it is an identifier, else its first identifier.
fn identifier_in(node: Node<'_>) -> Option<Node<'_>> {
    if matches!(node.kind(), "identifier" | "field_identifier") {
        return Some(node);
    }
    let mut cursor = node.walk();
    let children: Vec<_> = node.named_children(&mut cursor).collect();
    children.into_iter().find_map(identifier_in)
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_c;
#[cfg(test)]
mod tests_clojure;
#[cfg(test)]
mod tests_cpp;
#[cfg(test)]
mod tests_csharp;
#[cfg(test)]
mod tests_go;
#[cfg(test)]
mod tests_javascript;
#[cfg(test)]
mod tests_typescript;
