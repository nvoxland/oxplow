//! The code units a language gives. What a language *is* lives in its
//! extension's `languages:` entry ([`crate::spec::LanguageDecl`]); oxplow's
//! own are its language packs (`extensions/oxplow-<pack>/`).
//! `.context/language-support.md` has the map.

use serde::{Deserialize, Serialize};

/// A kind of code "unit" a language exposes, beyond the always-present
/// function: for the list-units navigation surface and the `metric_subject`
/// roll-up hierarchy. Declared per language (`units:`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum UnitKind {
    /// A function / method / closure (every language has these).
    Function,
    /// A class-like type: class, struct, record, interface, impl block,
    /// trait, enum.
    Class,
    /// A namespace / module / `mod` / Clojure `ns`.
    Module,
    /// A package (Go package, Java package): a path-level grouping.
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
