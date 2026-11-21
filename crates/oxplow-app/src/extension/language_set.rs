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
    shipped_once().as_ref().clone()
}

/// [`shipped`], built once per process: the embedded files never change.
pub fn shipped_once() -> Arc<Languages> {
    static SET: std::sync::OnceLock<Arc<Languages>> = std::sync::OnceLock::new();
    SET.get_or_init(|| Arc::new(build_shipped())).clone()
}

fn build_shipped() -> Languages {
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
                    b.bytes(rel).map(<[u8]>::to_vec)
                })
            {
                sources.push(source);
            }
        }
    }
    Languages::build(&sources).0
}

/// The project settings that turn on every shipped language pack (an
/// extension that ships with oxplow, may be turned off, and declares
/// languages), as `project.yaml` lines: for a test that parses code.
#[doc(hidden)]
pub fn every_pack_on_yaml() -> String {
    crate::bundled_extensions::BUNDLED
        .iter()
        .filter(|b| {
            !b.required
                && b.file("extension.yaml")
                    .is_some_and(|m| m.lines().any(|l| l.starts_with("languages:")))
        })
        .map(|b| format!("oxplow.extensions.{}.enabled: true\n", b.name))
        .collect()
}

/// The set the app reads, swapped whole when what runs changes.
#[derive(Clone, Default)]
pub struct LanguagesHandle(
    Arc<RwLock<Arc<Languages>>>,
    Arc<std::sync::atomic::AtomicBool>,
);

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

    /// [`swap`](Self::swap), saying whether what is indexed by the set needs
    /// restating: its first publication, or a set whose fingerprint moved.
    pub fn publish(&self, languages: Languages) -> bool {
        let moved = self.current().fingerprint() != languages.fingerprint();
        let first = !self.1.swap(true, std::sync::atomic::Ordering::SeqCst);
        self.swap(languages);
        first || moved
    }
}

#[cfg(test)]
mod tests {
    /// A language whose file extension an earlier extension's language holds
    /// holds the later extension back, naming the earlier; the set keeps the
    /// earlier's, and the later's other contributions stay inactive.
    #[tokio::test]
    async fn a_language_conflict_holds_the_later_extension() {
        let fx = crate::test_fixtures::services_with_thread().await;
        let svc = &fx.svc;
        let dir = svc.layout.project_dir.join(".oxplow/extensions/zeta-rust");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("extension.yaml"),
            "manifest: 2\nname: zeta-rust\nintent: { purpose: t, origin: null, examples: [] }\nlanguages:\n  - { id: zeta, title: Zeta, extensions: [rs] }\n",
        )
        .unwrap();
        let problems = crate::test_fixtures::activate_with_problems(svc).await;
        let held = problems
            .get("zeta-rust")
            .expect("the later extension is held");
        let conflict = held
            .iter()
            .find(|d| d.code == "language.conflict")
            .unwrap_or_else(|| panic!("{held:?}"));
        assert!(
            conflict.message.contains("files `.rs`") && conflict.message.contains("oxplow-rust"),
            "{}",
            conflict.message
        );
        let set = svc.languages.current();
        assert_eq!(
            set.for_path("src/a.rs").map(|l| l.name().to_string()),
            Some("rust".to_string()),
            "the earlier keeps `.rs`"
        );
        assert!(set.by_name("zeta").is_none());
    }

    /// `v_language` lists every language every loaded extension declares,
    /// on or off, with what oxplow knows about each.
    #[tokio::test]
    async fn v_language_lists_every_declared_language_on_or_off() {
        let fx = crate::test_fixtures::services_with_thread().await;
        let svc = &fx.svc;
        svc.commands
            .run(
                &oxplow_domain::Actor::Human,
                "oxplow.extension.disable",
                serde_json::json!({ "name": "oxplow-go" }),
                true,
            )
            .await
            .unwrap();
        let rows = |sql: &str| {
            let sql = sql.to_string();
            async move {
                serde_json::to_value(svc.sql.query_sql(&sql, vec![], None).await.unwrap().rows)
                    .unwrap()
            }
        };
        assert_eq!(
            rows("SELECT id, extension, enabled, analyzable, lsp_package FROM v_language WHERE id IN ('go', 'rust', 'lua') ORDER BY id")
                .await,
            serde_json::json!([
                ["go", "oxplow-go", 0, 1, "gopls"],
                ["lua", "oxplow-foundation", 1, 0, "lua-language-server"],
                ["rust", "oxplow-rust", 1, 1, "rust-analyzer"]
            ])
        );
        assert_eq!(
            rows("SELECT extensions, aliases, lsp_ids, facets FROM v_language WHERE id = 'tsx'")
                .await,
            serde_json::json!([[
                "[\"tsx\"]",
                "[]",
                "[\"typescriptreact\"]",
                "{\"imports\":1,\"merge\":1,\"duplication\":1,\"starter\":0}"
            ]])
        );
    }
}
