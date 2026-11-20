//! The programming languages the enabled extensions declare
//! (`languages:`, `.context/language-support.md`), as one set: what code
//! analysis, symbols, imports, duplication, merges, the script builtins
//! and the language-server suggestions parse and name files with. Core
//! names no language; a project's set is the packs it turned on.
//!
//! Built from the published extensions at each publication and swapped
//! whole (`extension_activation`, beside the model registry), so a reader
//! holds one consistent set for as long as it works.

use std::sync::{Arc, RwLock};

use oxplow_code_metrics::{LanguageProblem, Languages};

use crate::extension::extensions::Extension;

/// The order a set is built in: foundation, the extensions that ship with
/// oxplow in their order, then the rest by name. The first to claim a name
/// or a file extension keeps it.
pub fn of(extensions: &[Extension]) -> (Languages, Vec<LanguageProblem>) {
    let mut enabled: Vec<&Extension> = extensions
        .iter()
        .filter(|e| e.enabled && !e.languages.is_empty())
        .collect();
    let rank = |e: &Extension| {
        crate::bundled_extensions::BUNDLED
            .iter()
            .position(|b| b.name == e.name)
            .unwrap_or(usize::MAX)
    };
    enabled.sort_by(|a, b| rank(a).cmp(&rank(b)).then_with(|| a.name.cmp(&b.name)));
    let sources: Vec<_> = enabled
        .into_iter()
        .flat_map(|e| e.languages.iter().cloned())
        .collect();
    Languages::build(&sources)
}

/// Every language the extensions that ship with oxplow declare, enabled or
/// not, read from their embedded files: what the SDK offers offline (the
/// metric scaffold's starters) when no project is running.
pub fn shipped() -> Languages {
    let mut sources = Vec::new();
    for b in crate::bundled_extensions::BUNDLED {
        let Some(manifest) = b.file("extension.yaml") else {
            continue;
        };
        let Some(list) = serde_yaml::from_str::<serde_yaml::Value>(manifest)
            .ok()
            .and_then(|v| v.get("languages").cloned())
        else {
            continue;
        };
        let decls: Vec<oxplow_code_metrics::LanguageDecl> =
            serde_yaml::from_value(list).unwrap_or_default();
        for decl in decls {
            if let Ok(source) =
                oxplow_code_metrics::LanguageSource::read(b.name, true, decl, &|rel| {
                    b.file(rel).map(str::to_string)
                })
            {
                sources.push(source);
            }
        }
    }
    Languages::build(&sources).0
}

/// The set the app reads, swapped whole when what runs changes.
#[derive(Clone, Default)]
pub struct LanguagesHandle(Arc<RwLock<Arc<Languages>>>);

impl LanguagesHandle {
    pub fn current(&self) -> Arc<Languages> {
        self.0.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// oxplow's shipped packs' languages, for a test.
    #[cfg(test)]
    pub fn shipped() -> Self {
        let handle = Self::default();
        handle.swap(oxplow_code_metrics::for_tests::shipped());
        handle
    }

    pub fn swap(&self, languages: Languages) {
        *self.0.write().unwrap_or_else(|e| e.into_inner()) = Arc::new(languages);
    }
}
