//! The running vocabulary follows the primary worktree's extensions
//! (P8.D3, `.context/refs.md` "There is no process-wide registry"): core
//! event types and ref kinds, plus what each enabled private extension
//! declares (`event_types:`). A pass rebuilds the whole vocabulary and
//! swaps it into `Services.vocabulary`; writers already in a transaction
//! keep the snapshot they took.
//!
//! A pass runs at boot and on the extension catalog's signal, settled
//! like `extension_models`, and does nothing when the declarations didn't
//! change. What it refuses is that extension's health (an error in the
//! list), never a boot failure:
//! - a declaration the registry refuses (`register_declared`);
//! - a schema that differs from the one recorded at that `type@v`
//!   (`event_type_contract`): a new shape is a new version;
//! - two extensions whose names make one namespace (`acme-pr`,
//!   `acme_pr`): neither registers.
//!
//! The pass restates `event_type_contract` (read as `v_event_type`) in
//! the same transaction that checks it.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use oxplow_db::event_retention::{restate_declared_tx, DeclaredRetention};
use oxplow_db::event_type_store::{recorded_schema_tx, restate_tx, EventTypeRow};
use oxplow_db::ref_kind_store::RefKindRow;
use oxplow_db::Database;
use oxplow_domain::events::schema::{plugin_namespace, EventSchemaRegistry};
use oxplow_domain::refs::kind::{core_kinds, KindLifecycle, KindRegistry, KindSpec};
use oxplow_domain::vocabulary::{Vocabulary, VocabularyHandle};
use oxplow_domain::DomainError;
use tokio::sync::Mutex;

use crate::extension_catalog::ExtensionCatalog;
use crate::extension_event_types::EventTypes;
use crate::extension_ref_kinds::RefKindDecl;
use crate::extensions::Extension;

/// A burst of file events is one pass.
const SETTLE: Duration = Duration::from_millis(250);

#[derive(Default)]
struct State {
    fingerprint: Option<u64>,
    errors: BTreeMap<String, Vec<String>>,
}

pub struct VocabularyService {
    db: Database,
    catalog: Arc<ExtensionCatalog>,
    /// The primary worktree, whose extensions declare the vocabulary.
    root: PathBuf,
    vocabulary: VocabularyHandle,
    state: Mutex<State>,
}

/// Each enabled extension's `event_types:` and `ref_kinds:`, by name.
type Declared = Vec<(String, EventTypes, Vec<RefKindDecl>)>;

impl VocabularyService {
    pub fn new(
        db: Database,
        catalog: Arc<ExtensionCatalog>,
        root: PathBuf,
        vocabulary: VocabularyHandle,
    ) -> Self {
        Self {
            db,
            catalog,
            root,
            vocabulary,
            state: Mutex::new(State::default()),
        }
    }

    /// Rebuild and swap the vocabulary if what extensions declare changed.
    /// Passes are serialized.
    pub async fn sync(&self) -> Result<(), DomainError> {
        let mut state = self.state.lock().await;
        // Every enabled extension: one that declares nothing restates its
        // retention back to the default.
        let declared: Declared = self
            .catalog
            .get(&self.root)
            .iter()
            .filter(|e| e.enabled)
            .map(|e| (e.name.clone(), e.event_types.clone(), e.ref_kinds.clone()))
            .collect();
        let fingerprint = {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            serde_json::to_string(&declared)
                .map_err(|e| DomainError::Invalid(e.to_string()))?
                .hash(&mut h);
            h.finish()
        };
        if state.fingerprint == Some(fingerprint) {
            return Ok(());
        }
        let now = oxplow_domain::Timestamp::now().to_string();
        let (vocabulary, errors) = self
            .db
            .transaction(move |tx| build_tx(tx, &declared, &now))
            .await?;
        for (extension, errs) in &errors {
            for e in errs {
                tracing::warn!(extension, error = %e, "extension event type not registered");
            }
        }
        self.vocabulary.swap(vocabulary);
        state.errors = errors;
        state.fingerprint = Some(fingerprint);
        Ok(())
    }

    /// `extensions` with what each one's declarations were refused for
    /// added to its `errors` — for the primary worktree's list.
    pub async fn with_health(&self, root: &Path, mut extensions: Vec<Extension>) -> Vec<Extension> {
        if root != self.root {
            return extensions;
        }
        let state = self.state.lock().await;
        for ext in &mut extensions {
            if let Some(errs) = state.errors.get(&ext.name) {
                ext.errors.extend(errs.iter().cloned());
            }
        }
        extensions
    }

    /// Build now, then again whenever the primary worktree's extensions
    /// may have changed, for the life of the process.
    pub fn spawn(self: Arc<Self>, mut changes: tokio::sync::broadcast::Receiver<()>) {
        tokio::spawn(async move {
            if let Err(error) = self.sync().await {
                tracing::warn!(%error, "the vocabulary didn't build at boot");
            }
            loop {
                match changes.recv().await {
                    Ok(()) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                }
                tokio::time::sleep(SETTLE).await;
                while changes.try_recv().is_ok() {}
                if let Err(error) = self.sync().await {
                    tracing::warn!(%error, "the vocabulary didn't build");
                }
            }
        });
    }
}

/// The vocabulary `declared` makes, checked against and restated into
/// `event_type_contract`, and each extension's refusals.
fn build_tx(
    tx: &rusqlite::Connection,
    declared: &Declared,
    now: &str,
) -> Result<(Vocabulary, BTreeMap<String, Vec<String>>), DomainError> {
    let mut events = EventSchemaRegistry::core();
    let mut errors: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let declares = |t: &EventTypes| !t.types.is_empty() || t.retention.is_some();
    let mut by_namespace: HashMap<String, Vec<&str>> = HashMap::new();
    for (extension, _, _) in declared.iter().filter(|(_, t, _)| declares(t)) {
        by_namespace
            .entry(plugin_namespace(extension))
            .or_default()
            .push(extension);
    }
    let mut retention = Vec::new();
    for (extension, types, _) in declared {
        let namespace = plugin_namespace(extension);
        let others: Vec<&str> = by_namespace
            .get(&namespace)
            .into_iter()
            .flatten()
            .copied()
            .filter(|o| o != extension)
            .collect();
        if declares(types) && !others.is_empty() {
            errors.entry(extension.clone()).or_default().push(format!(
                "event types: `{namespace}.*` is also {}'s namespace; rename one extension",
                others
                    .iter()
                    .map(|o| format!("`{o}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
            continue;
        }
        // A namesake that declares nothing doesn't speak for a namespace a
        // declaring extension holds: it would drop that one's window (tsk796).
        if !declares(types) && by_namespace.contains_key(&namespace) {
            continue;
        }
        retention.push(DeclaredRetention {
            namespace,
            extension: extension.clone(),
            window: types
                .retention
                .map(|r| (r.payload_days.into(), r.content_days.into())),
        });
        for d in &types.types {
            let refused = match recorded_schema_tx(tx, &d.event_type, d.v)? {
                Some(recorded) if recorded != d.schema => Some(format!(
                    "{}: the schema of `{}@{}` changed since that version was recorded; declare \
                     the new shape as v{} with an upcast",
                    d.declared_at,
                    d.event_type,
                    d.v,
                    d.v + 1
                )),
                _ => events
                    .register_declared(extension, d.declared())
                    .err()
                    .map(|e| format!("{}: {e}", d.declared_at)),
            };
            if let Some(e) = refused {
                errors.entry(extension.clone()).or_default().push(e);
            }
        }
    }
    restate_declared_tx(tx, &retention, now)?;
    let rows: Vec<EventTypeRow> = events
        .versions()
        .into_iter()
        .map(|(event_type, v)| EventTypeRow {
            extension: events.owner(&event_type, v).flatten().map(str::to_string),
            schema: events.schema(&event_type, v).cloned().unwrap_or_default(),
            summary: events.summary(&event_type, v).map(str::to_string),
            event_type,
            v,
        })
        .collect();
    restate_tx(tx, &rows, now)?;
    let kinds = register_kinds(declared, &mut errors);
    oxplow_db::ref_kind_store::restate_tx(tx, &kind_rows(&kinds, declared))?;
    Ok((Vocabulary::new(events, kinds), errors))
}

/// Core's kinds plus every extension's that doesn't collide: a kind or
/// `wikilink:` prefix two extensions both use (one's kind as the other's
/// prefix too) is an error on each, and neither registers it; one core
/// holds is that extension's error.
fn register_kinds(declared: &Declared, errors: &mut BTreeMap<String, Vec<String>>) -> KindRegistry {
    let mut kinds = core_kinds();
    let mut users: HashMap<&str, BTreeSet<&str>> = HashMap::new();
    for (extension, _, decls) in declared {
        for d in decls {
            for name in std::iter::once(d.kind.as_str()).chain(d.wikilink.as_deref()) {
                users.entry(name).or_default().insert(extension);
            }
        }
    }
    for (extension, _, decls) in declared {
        for d in decls {
            let shared: BTreeSet<&str> = std::iter::once(d.kind.as_str())
                .chain(d.wikilink.as_deref())
                .flat_map(|name| users[name].iter().copied())
                .filter(|o| o != extension)
                .collect();
            let refused = if !shared.is_empty() {
                Some(format!(
                    "{}: ref kind `{}` collides with {}'s ref kinds; rename one",
                    d.declared_at,
                    d.kind,
                    shared
                        .iter()
                        .map(|o| format!("`{o}`"))
                        .collect::<Vec<_>>()
                        .join(", ")
                ))
            } else {
                let spec = KindSpec::new(&d.kind, &d.id_pattern).map(|s| {
                    let s = s.lifecycle(KindLifecycle::Experimental);
                    match &d.wikilink {
                        Some(w) => s.wikilink_prefix(w),
                        None => s,
                    }
                });
                spec.and_then(|s| kinds.register(s))
                    .err()
                    .map(|e| format!("{}: {e}", d.declared_at))
            };
            if let Some(e) = refused {
                errors.entry(extension.clone()).or_default().push(e);
            }
        }
    }
    kinds
}

/// `v_ref_kind`'s rows: every registered kind, an extension's with how
/// to show it.
fn kind_rows(kinds: &KindRegistry, declared: &Declared) -> Vec<RefKindRow> {
    kinds
        .kinds()
        .map(|k| {
            let decl = declared
                .iter()
                .flat_map(|(_, _, d)| d)
                .find(|d| d.kind == k.kind);
            RefKindRow {
                kind: k.kind.clone(),
                extension: decl.map(|d| d.extension.clone()),
                label: decl.map(|d| d.label.clone()),
                id_pattern: k.id_regex.to_string(),
                revisioned: k.revisioned,
                wikilinks: k.wikilink_prefixes.clone(),
                resolve: decl.map(|d| d.resolve.clone()),
                page: decl.map(|d| d.page.clone()),
                icon: decl.map(|d| d.icon.clone()),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_db::event_log_store::SqliteEventLogStore;
    use oxplow_db::SqlCell;
    use oxplow_domain::Envelope;
    use serde_json::json;

    fn write(root: &Path, rel: &str, body: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    const MANIFEST: &str = "manifest: 2\nname: acme-pr\nsharing: private\nintent: { purpose: PRs., origin: null, examples: [] }\nevent_types:\n  types:\n    - type: acme_pr.merged\n      v: 1\n      schema: merged.json\n      summary: A pull request merged.\n";

    fn merged(number: i64) -> Envelope {
        Envelope::new("acme_pr.merged", 1, "test", json!({ "number": number })).unwrap()
    }

    fn schema(required: &str) -> String {
        format!(
            r#"{{"type": "object", "required": ["{required}"], "properties": {{"{required}": {{"type": "integer"}}}}}}"#
        )
    }

    /// The whole life of a declared type: it appends once registered, a
    /// changed schema at the same version is refused (and named in the
    /// extension's errors), and once the extension is gone appends fail
    /// while the rows already logged still read.
    #[tokio::test]
    async fn a_declared_type_registers_keeps_its_contract_and_outlives_its_extension() {
        let f = crate::test_fixtures::services_with_effort().await;
        let svc = &f.svc;
        let root = svc.layout.project_dir.clone();
        let log = SqliteEventLogStore::new(svc.db.clone(), svc.vocabulary.clone());
        assert!(log.append(merged(1)).await.is_err());

        write(&root, "oxplow/extensions/acme-pr/extension.yaml", MANIFEST);
        write(
            &root,
            "oxplow/extensions/acme-pr/merged.json",
            &schema("number"),
        );
        svc.vocabulary_service.sync().await.unwrap();
        log.append(merged(12)).await.unwrap();
        let listed = svc
            .sql
            .query_sql(
                "SELECT extension, summary, registered, latest FROM v_event_type \
                 WHERE event_type = 'acme_pr.merged'",
                vec![],
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            listed.rows,
            vec![vec![
                SqlCell::Text("acme-pr".into()),
                SqlCell::Text("A pull request merged.".into()),
                SqlCell::Int(1),
                SqlCell::Int(1)
            ]]
        );

        // The same version, a new shape: refused; the old one is gone too.
        write(
            &root,
            "oxplow/extensions/acme-pr/merged.json",
            &schema("pr"),
        );
        svc.vocabulary_service.sync().await.unwrap();
        let errors = svc
            .listed_extensions(&root)
            .await
            .into_iter()
            .find(|e| e.name == "acme-pr")
            .unwrap()
            .errors;
        assert!(
            errors.iter().any(|e| e.contains("extension.yaml:7")
                && e.contains("changed since that version was recorded")),
            "{errors:?}"
        );
        assert!(log.append(merged(13)).await.is_err());

        // Removed: no appends, the logged row still reads, the type stays
        // listed unregistered.
        std::fs::remove_dir_all(root.join("oxplow/extensions/acme-pr")).unwrap();
        svc.vocabulary_service.sync().await.unwrap();
        assert!(log.append(merged(14)).await.is_err());
        let rows = log.read_after(0, 10_000).await.unwrap();
        assert!(rows.iter().any(
            |r| r.envelope.event_type == "acme_pr.merged" && r.envelope.payload["number"] == 12
        ));
        let registered = svc
            .sql
            .query_sql(
                "SELECT registered FROM v_event_type WHERE event_type = 'acme_pr.merged'",
                vec![],
                None,
            )
            .await
            .unwrap();
        assert_eq!(registered.rows, vec![vec![SqlCell::Int(0)]]);
    }

    /// P8.D5: a declared retention window is recorded for the namespace
    /// and stays after the extension is removed.
    #[tokio::test]
    async fn a_declared_retention_window_outlives_its_extension() {
        let f = crate::test_fixtures::services_with_effort().await;
        let svc = &f.svc;
        let root = svc.layout.project_dir.clone();
        write(
            &root,
            "oxplow/extensions/acme-pr/extension.yaml",
            &MANIFEST.replace(
                "event_types:\n",
                "event_types:\n  retention: { payload_days: 7, content_days: 3 }\n",
            ),
        );
        write(
            &root,
            "oxplow/extensions/acme-pr/merged.json",
            &schema("number"),
        );
        let window = || async {
            svc.db
                .read(|tx| {
                    tx.query_row(
                        "SELECT payload_days, content_days FROM plugin_event_retention \
                         WHERE namespace = 'acme_pr'",
                        [],
                        |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
                    )
                    .map_err(oxplow_db::map_sql_err)
                })
                .await
                .unwrap()
        };
        svc.vocabulary_service.sync().await.unwrap();
        assert_eq!(window().await, (7, 3));
        std::fs::remove_dir_all(root.join("oxplow/extensions/acme-pr")).unwrap();
        svc.vocabulary_service.sync().await.unwrap();
        assert_eq!(window().await, (7, 3));
    }

    /// P8.D6: a declared ref kind links (`[[pr:12]]` → `acme_pr:12`) and
    /// is listed while its extension is installed, and is unrecognized
    /// once it's gone.
    #[tokio::test]
    async fn a_declared_ref_kind_links_while_its_extension_is_installed() {
        use crate::extension_ref_kinds::tests::{write_acme, MANIFEST as ACME};
        let f = crate::test_fixtures::services_with_effort().await;
        let svc = &f.svc;
        let root = svc.layout.project_dir.clone();
        let link = || {
            oxplow_domain::refs::canonical_wikilink(&svc.vocabulary.current().kinds, "pr:12")
                .map(|r| r.to_string())
        };
        assert_eq!(link(), None);
        write_acme(&root, ACME);
        svc.vocabulary_service.sync().await.unwrap();
        assert_eq!(link().as_deref(), Some("acme_pr:12"));
        let listed = svc
            .sql
            .query_sql(
                "SELECT extension, page, icon FROM v_ref_kind WHERE kind = 'acme_pr'",
                vec![],
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            listed.rows,
            vec![vec![
                SqlCell::Text("acme".into()),
                SqlCell::Text("page:ext.acme.pr".into()),
                SqlCell::Text("git-pull-request".into())
            ]]
        );
        std::fs::remove_dir_all(root.join("oxplow/extensions/acme")).unwrap();
        svc.vocabulary_service.sync().await.unwrap();
        assert_eq!(link(), None);
    }

    /// Two extensions using one `wikilink:` prefix: an error on each, and
    /// neither's kind registers.
    #[tokio::test]
    async fn a_ref_kind_collision_is_an_error_on_both_extensions() {
        use crate::extension_ref_kinds::tests::{write_acme, MANIFEST as ACME};
        let f = crate::test_fixtures::services_with_effort().await;
        let svc = &f.svc;
        let root = svc.layout.project_dir.clone();
        write_acme(&root, ACME);
        let beta = tempfile::tempdir().unwrap();
        write_acme(
            beta.path(),
            &ACME
                .replace("name: acme", "name: beta")
                .replace("kind: acme_pr", "kind: beta_pr"),
        );
        std::fs::rename(
            beta.path().join("oxplow/extensions/acme"),
            root.join("oxplow/extensions/beta"),
        )
        .unwrap();
        svc.vocabulary_service.sync().await.unwrap();
        let listed = svc.listed_extensions(&root).await;
        for (name, other) in [("acme", "beta"), ("beta", "acme")] {
            let errors = listed
                .iter()
                .find(|e| e.name == name)
                .unwrap()
                .errors
                .join("\n");
            assert!(
                errors.contains(&format!("collides with `{other}`")),
                "{name}: {errors}"
            );
        }
        let kinds = &svc.vocabulary.current().kinds;
        assert!(kinds.get("acme_pr").is_none() && kinds.get("beta_pr").is_none());
    }

    /// tsk796: a namesake that declares nothing (`acme_pr` beside
    /// `acme-pr`) doesn't take the declaring one's retention window away.
    #[tokio::test]
    async fn a_namesake_that_declares_nothing_keeps_the_others_window() {
        let f = crate::test_fixtures::services_with_effort().await;
        let svc = &f.svc;
        let root = svc.layout.project_dir.clone();
        write(
            &root,
            "oxplow/extensions/acme-pr/extension.yaml",
            &MANIFEST.replace(
                "event_types:\n",
                "event_types:\n  retention: { payload_days: 7, content_days: 3 }\n",
            ),
        );
        write(
            &root,
            "oxplow/extensions/acme-pr/merged.json",
            &schema("number"),
        );
        write(
            &root,
            "oxplow/extensions/acme_pr/extension.yaml",
            "manifest: 2\nname: acme_pr\nsharing: private\nintent: { purpose: Other., origin: null, examples: [] }\n",
        );
        svc.vocabulary_service.sync().await.unwrap();
        let window = svc
            .db
            .read(|tx| {
                tx.query_row(
                    "SELECT payload_days, content_days FROM plugin_event_retention \
                     WHERE namespace = 'acme_pr'",
                    [],
                    |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
                )
                .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap();
        assert_eq!(window, (7, 3));
    }

    /// A collector may follow its own extension's declared types, never
    /// another extension's.
    #[test]
    fn a_collector_follows_its_own_extensions_types() {
        let dir = tempfile::tempdir().unwrap();
        let collector = |on: &str| {
            format!(
                "{MANIFEST}collectors:\n  - {{ id: tally, runtime: starlark, entry: tally.star, trigger: {{ on: [{on}] }}, entities: [{{ name: tally, key: id, columns: {{ id: int }} }}] }}\n"
            )
        };
        write(
            dir.path(),
            "oxplow/extensions/acme-pr/merged.json",
            &schema("number"),
        );
        write(
            dir.path(),
            "oxplow/extensions/acme-pr/tally.star",
            "def transform(x):\n    return []\n",
        );
        write(
            dir.path(),
            "oxplow/extensions/acme-pr/extension.yaml",
            &collector("acme_pr.merged"),
        );
        let loaded = crate::extensions::load_extensions(dir.path());
        let ext = loaded.iter().find(|e| e.name == "acme-pr").unwrap();
        assert!(ext.errors.is_empty(), "{:?}", ext.errors);
        assert_eq!(ext.collectors.len(), 1);
        write(
            dir.path(),
            "oxplow/extensions/acme-pr/extension.yaml",
            &collector("other_ext.thing"),
        );
        let loaded = crate::extensions::load_extensions(dir.path());
        let ext = loaded.iter().find(|e| e.name == "acme-pr").unwrap();
        assert!(
            ext.errors
                .iter()
                .any(|e| e.contains("isn't a registered event type")),
            "{:?}",
            ext.errors
        );
    }

    #[tokio::test]
    async fn two_extensions_sharing_a_namespace_both_fail() {
        let f = crate::test_fixtures::services_with_effort().await;
        let svc = &f.svc;
        let root = svc.layout.project_dir.clone();
        write(&root, "oxplow/extensions/acme-pr/extension.yaml", MANIFEST);
        write(
            &root,
            "oxplow/extensions/acme-pr/merged.json",
            &schema("number"),
        );
        write(
            &root,
            "oxplow/extensions/acme_pr/extension.yaml",
            &MANIFEST.replace("name: acme-pr", "name: acme_pr"),
        );
        write(
            &root,
            "oxplow/extensions/acme_pr/merged.json",
            &schema("number"),
        );
        svc.vocabulary_service.sync().await.unwrap();
        let listed = svc.listed_extensions(&root).await;
        for name in ["acme-pr", "acme_pr"] {
            let errors = &listed.iter().find(|e| e.name == name).unwrap().errors;
            assert!(
                errors
                    .iter()
                    .any(|e| e.contains("also") && e.contains("namespace")),
                "{name}: {errors:?}"
            );
        }
        assert!(!svc.vocabulary.current().is_registered("acme_pr.merged", 1));
    }
}
