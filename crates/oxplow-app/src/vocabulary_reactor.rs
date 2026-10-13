//! The running vocabulary follows the primary worktree's extensions
//! (P8.D3, `.context/refs.md` "There is no process-wide registry"): core
//! event types and ref kinds, plus what each enabled extension declares
//! (`event_types:`, `ref_kinds:` — both stable). A pass rebuilds the whole vocabulary and
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
//! - a reaction (an effect's or collector's `on:`) to another extension's
//!   type that no enabled extension registers (P9.D1) — the subscriber's
//!   error, gone once the type's owner is installed and its declaration
//!   accepted.
//!
//! The pass restates `event_type_contract` (read as `v_event_type`) in
//! the same transaction that checks it.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use oxplow_db::event_retention::{restate_declared_tx, DeclaredRetention};
use oxplow_db::event_type_store::{recorded_schema_tx, restate_tx, EventTypeRow};
use oxplow_db::ref_kind_store::RefKindRow;
use oxplow_domain::events::schema::{extension_namespace, EventSchemaRegistry};
use oxplow_domain::refs::kind::{core_kinds, KindRegistry};
use oxplow_domain::vocabulary::Vocabulary;
use oxplow_domain::DomainError;

use crate::extension_catalog::CatalogSnapshot;
use crate::extension_event_types::EventTypes;
use crate::extension_ref_kinds::RefKindDecl;
use crate::extensions::Extension;

/// What the generation needs of the host to build a vocabulary: the
/// active work list's id recognizer, asked at each build.
pub struct VocabularyService {
    /// The active work list's id recognizer (its id and `id_pattern`), if
    /// it declares one: the vocabulary's way of reading `tsk42` in text.
    work_item_ids: WorkItemIds,
}

/// The active work list's id recognizer, asked at each build.
pub type WorkItemIds = Arc<dyn Fn() -> Option<(String, String)> + Send + Sync>;

/// What one enabled extension brings to the vocabulary.
#[derive(serde::Serialize)]
struct Declares {
    extension: String,
    /// Its `event_types:`.
    types: EventTypes,
    /// Its `ref_kinds:`.
    kinds: Vec<RefKindDecl>,
    /// Its reactions to other extensions' types.
    subscriptions: Vec<crate::extensions::ForeignSubscription>,
}

type Declared = Vec<Declares>;

pub type VocabularyErrors = BTreeMap<String, Vec<String>>;

/// Frozen declarations for one candidate catalog. Preparing these does not
/// change either the database or the running vocabulary.
pub struct PreparedVocabulary {
    generation: String,
    declared: Declared,
    work_item_ids: Option<(String, String)>,
}

impl PreparedVocabulary {
    /// Freeze one candidate's enabled declarations with a host-selected work
    /// item recognizer. This permits capability and vocabulary candidates to
    /// use the same planned selection without changing either live registry.
    pub fn prepare(
        snapshot: &CatalogSnapshot,
        work_item_ids: Option<(String, String)>,
    ) -> Result<Self, VocabularyErrors> {
        let errors: VocabularyErrors = snapshot
            .extensions()
            .iter()
            .filter(|extension| extension.enabled && !extension.errors.is_empty())
            .map(|extension| (extension.name.clone(), extension.errors.clone()))
            .collect();
        if !errors.is_empty() {
            return Err(errors);
        }
        Ok(Self {
            generation: snapshot.generation().to_string(),
            declared: declarations(snapshot.extensions()),
            work_item_ids,
        })
    }

    pub fn generation(&self) -> &str {
        &self.generation
    }

    /// Validate and restate the complete candidate inside the caller's write
    /// transaction. Any diagnostic restores this helper's database writes.
    /// Propagate rejection to roll back other candidate registrations too.
    /// The returned vocabulary must only be published after the outer commit;
    /// this helper neither swaps a handle nor coordinates runtime readers.
    pub fn replace_tx(
        &self,
        tx: &rusqlite::Transaction<'_>,
        now: oxplow_domain::Timestamp,
    ) -> Result<Vocabulary, oxplow_db::TxError<VocabularyErrors>> {
        tx.execute_batch("SAVEPOINT candidate_extension_vocabulary")
            .map_err(oxplow_db::map_sql_err)?;
        let result = build_tx(tx, &self.declared, &now.to_string());
        match result {
            Ok((mut vocabulary, mut errors)) => {
                // A bad recognizer rejects the candidate instead of
                // silently dropping recognition.
                match self.work_item_ids.as_ref() {
                    Some((provider, pattern)) => match vocabulary
                        .kinds
                        .clone()
                        .with_work_item_ids(provider, pattern)
                    {
                        Ok(kinds) => vocabulary.kinds = kinds,
                        Err(error) => errors
                            .entry("<host>".to_string())
                            .or_default()
                            .push(format!("work item id recognizer: {error}")),
                    },
                    None => vocabulary.kinds = vocabulary.kinds.without_work_item_ids(),
                }
                errors.retain(|_, messages| !messages.is_empty());
                if errors.is_empty() {
                    tx.execute_batch("RELEASE candidate_extension_vocabulary")
                        .map_err(oxplow_db::map_sql_err)?;
                    Ok(vocabulary)
                } else {
                    rollback_candidate(tx)?;
                    Err(oxplow_db::TxError::Aborted(errors))
                }
            }
            Err(error) => {
                rollback_candidate(tx)?;
                Err(oxplow_db::TxError::Storage(error))
            }
        }
    }
}

fn rollback_candidate(tx: &rusqlite::Transaction<'_>) -> Result<(), DomainError> {
    tx.execute_batch(
        "ROLLBACK TO candidate_extension_vocabulary; RELEASE candidate_extension_vocabulary",
    )
    .map_err(oxplow_db::map_sql_err)
}

fn declarations(extensions: &[Extension]) -> Declared {
    extensions
        .iter()
        .filter(|extension| extension.enabled)
        .map(|extension| Declares {
            extension: extension.name.clone(),
            types: extension.event_types.clone(),
            kinds: extension.ref_kinds.clone(),
            subscriptions: extension.subscriptions.clone(),
        })
        .collect()
}

impl VocabularyService {
    pub fn new(work_item_ids: WorkItemIds) -> Self {
        Self { work_item_ids }
    }

    /// Capture declarations and the current host recognizer from one catalog
    /// snapshot. Coordinated capability changes should instead supply their
    /// planned recognizer to PreparedVocabulary::prepare.
    pub fn prepare(
        &self,
        snapshot: &CatalogSnapshot,
    ) -> Result<PreparedVocabulary, VocabularyErrors> {
        PreparedVocabulary::prepare(snapshot, (self.work_item_ids)())
    }
}

/// `kinds` recognizing the active work list's ids (`ids`), or none.
pub fn with_work_item_ids(kinds: KindRegistry, ids: Option<&(String, String)>) -> KindRegistry {
    match ids {
        Some((provider, pattern)) => kinds
            .clone()
            .with_work_item_ids(provider, pattern)
            .unwrap_or_else(|e| {
                tracing::warn!(provider, error = %e, "the work list's id pattern doesn't compile");
                kinds.without_work_item_ids()
            }),
        None => kinds.without_work_item_ids(),
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
    let mut retention = Vec::new();
    for Declares {
        extension, types, ..
    } in declared
    {
        let namespace = extension_namespace(extension);
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
    // A reaction to another extension's type holds only while someone
    // registers it: the name is the dependency.
    for Declares {
        extension,
        subscriptions,
        ..
    } in declared
    {
        for s in subscriptions {
            if events.latest(&s.event_type).is_none() {
                errors.entry(extension.clone()).or_default().push(format!(
                    "{}: {} reacts to `{}`, which no enabled extension registers: install or \
                     enable the extension that declares it (or see that one's errors, if its \
                     declaration was refused)",
                    s.declared_at, s.by, s.event_type
                ));
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
    oxplow_db::replaceable_store::restate_tx(tx)?;
    Ok((Vocabulary::new(events, kinds), errors))
}

/// Core's kinds plus every extension's. A kind two extensions both
/// declare was settled at load (`extensions::descriptors::check_catalog`:
/// the more specific namespace's, else the first, keeps it); a `wikilink:`
/// prefix another extension also uses (as its prefix, or as a kind) is an
/// error on each and costs only the sugar — the namespaced kind still
/// registers, so installing one extension never unlinks another's refs
/// (P10, K2). A kind or prefix core holds is that extension's error.
fn register_kinds(declared: &Declared, errors: &mut BTreeMap<String, Vec<String>>) -> KindRegistry {
    let mut kinds = core_kinds();
    // Who declares each name as a kind, and as a prefix.
    let mut as_kind: HashMap<&str, BTreeSet<&str>> = HashMap::new();
    let mut as_prefix: HashMap<&str, BTreeSet<&str>> = HashMap::new();
    for Declares {
        extension,
        kinds: decls,
        ..
    } in declared
    {
        for d in decls {
            as_kind.entry(&d.kind).or_default().insert(extension);
            if let Some(w) = &d.wikilink {
                as_prefix.entry(w).or_default().insert(extension);
            }
        }
    }
    let others = |name: &str, of: &str| -> BTreeSet<&str> {
        as_kind
            .get(name)
            .into_iter()
            .chain(as_prefix.get(name))
            .flatten()
            .copied()
            .filter(|o| *o != of)
            .collect()
    };
    let named = |set: &BTreeSet<&str>| {
        set.iter()
            .map(|o| format!("`{o}`"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    for Declares {
        extension,
        kinds: decls,
        ..
    } in declared
    {
        for d in decls {
            let mut problems = Vec::new();
            let mut decl = d.clone();
            if let Some(w) = &d.wikilink {
                let sharing = others(w, extension);
                if !sharing.is_empty() {
                    problems.push(format!(
                        "{}: `wikilink: {w}` is also {}'s, so `[[{w}:…]]` links neither; ref \
                         kind `{}` is still linked as `[[{}:…]]` — rename one prefix",
                        d.declared_at,
                        named(&sharing),
                        d.kind,
                        d.kind
                    ));
                    decl.wikilink = None;
                }
            }
            if let Err(e) =
                crate::extension_ref_kinds::kind_spec(&decl).and_then(|spec| kinds.register(spec))
            {
                problems.push(format!("{}: {e}", d.declared_at));
            }
            if !problems.is_empty() {
                errors
                    .entry(extension.clone())
                    .or_default()
                    .extend(problems);
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
                .flat_map(|d| &d.kinds)
                .find(|d| d.kind == k.kind);
            RefKindRow {
                kind: k.kind.clone(),
                extension: decl.map(|d| d.extension.clone()),
                label: decl.map(|d| d.label.clone()),
                searchable: decl.and_then(|d| d.searchable.clone()),
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
    use std::path::Path;

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
        crate::test_fixtures::activate_with_problems(svc).await;
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

        // The same version, a new shape: refused (named in the errors).
        write(
            &root,
            "oxplow/extensions/acme-pr/merged.json",
            &schema("pr"),
        );
        crate::test_fixtures::activate_with_problems(svc).await;
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
        // The edit isn't adopted: the last working version stays, so the
        // contract recorded at that version still holds.
        log.append(merged(13)).await.unwrap();
        assert!(log
            .append(Envelope::new("acme_pr.merged", 1, "test", json!({ "pr": 13 })).unwrap())
            .await
            .is_err());

        // Removed: no appends, the logged row still reads, the type stays
        // listed unregistered.
        std::fs::remove_dir_all(root.join("oxplow/extensions/acme-pr")).unwrap();
        crate::test_fixtures::activate_with_problems(svc).await;
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
                        "SELECT payload_days, content_days FROM extension_event_retention \
                         WHERE namespace = 'acme_pr'",
                        [],
                        |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
                    )
                    .map_err(oxplow_db::map_sql_err)
                })
                .await
                .unwrap()
        };
        crate::test_fixtures::activate_with_problems(svc).await;
        assert_eq!(window().await, (7, 3));
        std::fs::remove_dir_all(root.join("oxplow/extensions/acme-pr")).unwrap();
        crate::test_fixtures::activate_with_problems(svc).await;
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
        crate::test_fixtures::activate_with_problems(svc).await;
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
                SqlCell::Text("page:acme/pr".into()),
                SqlCell::Text("git-pull-request".into())
            ]]
        );
        std::fs::remove_dir_all(root.join("oxplow/extensions/acme")).unwrap();
        crate::test_fixtures::activate_with_problems(svc).await;
        assert_eq!(link(), None);
    }

    /// P10 (K2): two extensions using one `wikilink:` prefix lose the
    /// sugar — an error on each, and `[[pr:…]]` links neither — but each
    /// namespaced kind still registers, so installing one extension never
    /// unlinks another's refs.
    #[tokio::test]
    async fn a_shared_wikilink_prefix_costs_the_sugar_not_the_kind() {
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
        crate::test_fixtures::activate_with_problems(svc).await;
        let listed = svc.listed_extensions(&root).await;
        for (name, other) in [("acme", "beta"), ("beta", "acme")] {
            let errors = listed
                .iter()
                .find(|e| e.name == name)
                .unwrap()
                .errors
                .join("\n");
            assert!(
                errors.contains("`wikilink: pr`") && errors.contains(&format!("`{other}`")),
                "{name}: {errors}"
            );
        }
        // The error holds each extension back whole (an extension with a
        // load error doesn't activate), so neither kind is registered and
        // nothing links.
        let kinds = &svc.vocabulary.current().kinds;
        assert!(kinds.get("acme_pr").is_none() && kinds.get("beta_pr").is_none());
        let link =
            |l: &str| oxplow_domain::refs::canonical_wikilink(kinds, l).map(|r| r.to_string());
        assert_eq!(link("pr:12"), None, "the sugar links neither");
        assert_eq!(link("acme_pr:12"), None);
        assert_eq!(link("beta_pr:12"), None);
    }

    /// tsk933: namespaces are string prefixes, so `acme`'s (`acme_`) holds
    /// `acme-pr`'s (`acme_pr_`). A kind in both is the more specific
    /// namespace's: `acme-pr` keeps `acme_pr_x`, and `acme`'s declaration of
    /// it is refused, naming whose namespace it is — never both lost.
    #[tokio::test]
    async fn a_kind_in_another_extensions_namespace_is_theirs() {
        use crate::extension_ref_kinds::tests::{write_acme, MANIFEST as ACME};
        let f = crate::test_fixtures::services_with_effort().await;
        let svc = &f.svc;
        let root = svc.layout.project_dir.clone();
        write_acme(
            &root,
            &ACME
                .replace("kind: acme_pr", "kind: acme_pr_x")
                .replace("wikilink: pr", "wikilink: ax"),
        );
        let other = tempfile::tempdir().unwrap();
        write_acme(
            other.path(),
            &ACME
                .replace("name: acme", "name: acme-pr")
                .replace("kind: acme_pr", "kind: acme_pr_x")
                .replace("wikilink: pr", "wikilink: px"),
        );
        std::fs::rename(
            other.path().join("oxplow/extensions/acme"),
            root.join("oxplow/extensions/acme-pr"),
        )
        .unwrap();
        crate::test_fixtures::activate_with_problems(svc).await;
        let listed = svc.listed_extensions(&root).await;
        let errors = |name: &str| {
            listed
                .iter()
                .find(|e| e.name == name)
                .unwrap()
                .errors
                .join("\n")
        };
        assert!(
            errors("acme").contains("`acme_pr_x`") && errors("acme").contains("`acme-pr`"),
            "{}",
            errors("acme")
        );
        assert!(
            !errors("acme-pr").contains("acme_pr_x"),
            "{}",
            errors("acme-pr")
        );
        let kinds = &svc.vocabulary.current().kinds;
        assert!(kinds.get("acme_pr_x").is_some(), "acme-pr keeps its kind");
        let link =
            |l: &str| oxplow_domain::refs::canonical_wikilink(kinds, l).map(|r| r.to_string());
        assert_eq!(link("px:1").as_deref(), Some("acme_pr_x:1"));
        assert_eq!(link("ax:1"), None, "acme's refused kind brings no sugar");
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
        crate::test_fixtures::activate_with_problems(svc).await;
        let window = svc
            .db
            .read(|tx| {
                tx.query_row(
                    "SELECT payload_days, content_days FROM extension_event_retention \
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
        // One in its own namespace that it doesn't declare is no type.
        write(
            dir.path(),
            "oxplow/extensions/acme-pr/extension.yaml",
            &collector("acme_pr.closed"),
        );
        let loaded = crate::extensions::load_extensions(dir.path());
        let ext = loaded.iter().find(|e| e.name == "acme-pr").unwrap();
        assert!(
            ext.errors
                .iter()
                .any(|e| e.contains("`acme_pr.closed` isn't a registered event type")),
            "{:?}",
            ext.errors
        );
        // Another extension's is a subscription (P9.D1): a warning here.
        write(
            dir.path(),
            "oxplow/extensions/acme-pr/extension.yaml",
            &collector("other_ext.thing"),
        );
        let loaded = crate::extensions::load_extensions(dir.path());
        let ext = loaded.iter().find(|e| e.name == "acme-pr").unwrap();
        assert!(ext.errors.is_empty(), "{:?}", ext.errors);
        assert_eq!(ext.subscriptions.len(), 1);
        assert!(
            ext.warnings
                .iter()
                .any(|w| w.contains("collector `tally` reacts to `other_ext.thing`")),
            "{:?}",
            ext.warnings
        );
    }

    /// `acme_pr` would share `acme-pr`'s namespace, but an extension's name
    /// is lowercase letters, digits and single dashes: the second is refused
    /// at its name, so one namespace never has two owners.
    #[tokio::test]
    async fn a_name_that_would_share_a_namespace_is_refused_at_the_name() {
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
            &MANIFEST
                .replace("name: acme-pr", "name: acme_pr")
                .replace("acme_pr.merged", "acme_pr.closed")
                .replace("merged.json", "closed.json"),
        );
        write(
            &root,
            "oxplow/extensions/acme_pr/closed.json",
            &schema("number"),
        );
        crate::test_fixtures::activate_with_problems(svc).await;
        let listed = svc.listed_extensions(&root).await;
        let errors = |name: &str| {
            listed
                .iter()
                .find(|e| e.name == name)
                .unwrap()
                .errors
                .clone()
        };
        assert!(errors("acme-pr").is_empty(), "{:?}", errors("acme-pr"));
        assert!(
            errors("acme_pr")
                .iter()
                .any(|e| e.contains("isn't an extension name")),
            "{:?}",
            errors("acme_pr")
        );
        assert!(svc.vocabulary.current().is_registered("acme_pr.merged", 1));
        assert!(!svc.vocabulary.current().is_registered("acme_pr.closed", 1));
    }

    /// `beta`: an effect and a collector that react to `acme_pr.merged`,
    /// a type of another extension's.
    const SUBSCRIBER: &str = "manifest: 2\nname: beta\nsharing: private\nintent: { purpose: Follows merges., origin: null, examples: [] }\neffects:\n  - id: note\n    summary: Note a merge.\n    on: [acme_pr.merged]\n    entry: note.star\ncollectors:\n  - id: pull\n    runtime: exec\n    entry: pull.sh\n    trigger: { on: [acme_pr.merged] }\n    entities:\n      - { name: pr, key: n, columns: { n: int } }\n";

    async fn listed(svc: &crate::Services, name: &str) -> Extension {
        svc.listed_extensions(&svc.layout.project_dir)
            .await
            .into_iter()
            .find(|e| e.name == name)
            .unwrap()
    }

    /// P9.D1: an effect and a collector may react to another extension's
    /// event type. The type's name is the dependency: at load it is a
    /// warning (the owner may not be here yet); while no enabled extension
    /// registers it, it is the subscriber's error — gone once the owner is
    /// installed, back if the owner's declaration is refused.
    #[tokio::test]
    async fn a_subscription_to_another_extensions_type_resolves_when_its_owner_registers_it() {
        let f = crate::test_fixtures::services_with_effort().await;
        let svc = &f.svc;
        let root = svc.layout.project_dir.clone();
        write(&root, "oxplow/extensions/beta/extension.yaml", SUBSCRIBER);
        write(
            &root,
            "oxplow/extensions/beta/note.star",
            "def transform(x):\n    return {\"skip\": \"nothing to do\"}\n",
        );
        write(&root, "oxplow/extensions/beta/pull.sh", "#!/bin/sh\n");

        // Loading alone (what `oxplow extension check` sees): no error, and a
        // warning for each subscription.
        let loaded = crate::extensions::load_extensions(&root)
            .into_iter()
            .find(|e| e.name == "beta")
            .unwrap();
        assert_eq!(loaded.errors, Vec::<String>::new());
        assert_eq!((loaded.effects.len(), loaded.collectors.len()), (1, 1));
        let foreign: Vec<&String> = loaded
            .warnings
            .iter()
            .filter(|w| w.contains("`acme_pr.merged`") && w.contains("another extension's"))
            .collect();
        assert_eq!(foreign.len(), 2, "{:?}", loaded.warnings);
        assert!(
            foreign.iter().any(|w| w.contains("effect `note`"))
                && foreign.iter().any(|w| w.contains("collector `pull`")),
            "{foreign:?}"
        );

        // Nobody registers it: the subscriber's error, for each.
        crate::test_fixtures::activate_with_problems(svc).await;
        let unresolved = |errors: &[String]| {
            errors
                .iter()
                .filter(|e| {
                    e.contains("`acme_pr.merged`") && e.contains("no enabled extension registers")
                })
                .count()
        };
        let errors = listed(svc, "beta").await.errors;
        assert_eq!(unresolved(&errors), 2, "{errors:?}");
        assert!(
            errors
                .iter()
                .filter(|e| e.contains("no enabled extension registers"))
                .all(|e| e.contains("extension.yaml:")),
            "{errors:?}"
        );

        // Its owner arrives: resolved.
        write(&root, "oxplow/extensions/acme-pr/extension.yaml", MANIFEST);
        write(
            &root,
            "oxplow/extensions/acme-pr/merged.json",
            &schema("number"),
        );
        crate::test_fixtures::activate_with_problems(svc).await;
        assert_eq!(listed(svc, "beta").await.errors, Vec::<String>::new());

        // The owner's declaration is refused (a new shape at a recorded
        // version): the type isn't registered, and the subscriber says so.
        write(
            &root,
            "oxplow/extensions/acme-pr/merged.json",
            &schema("pr"),
        );
        crate::test_fixtures::activate_with_problems(svc).await;
        assert_eq!(unresolved(&listed(svc, "beta").await.errors), 2);

        // The owner goes: unresolved again.
        std::fs::remove_dir_all(root.join("oxplow/extensions/acme-pr")).unwrap();
        crate::test_fixtures::activate_with_problems(svc).await;
        assert_eq!(unresolved(&listed(svc, "beta").await.errors), 2);
    }

    /// P9.D1: what isn't another extension's type is still refused at
    /// load — a core namespace's type that doesn't exist, one in the
    /// extension's own namespace it doesn't declare, a name that isn't a
    /// type's.
    #[tokio::test]
    async fn a_subscription_to_no_type_at_all_is_refused_at_load() {
        let d = tempfile::tempdir().unwrap();
        for bad in ["work_item.nope", "beta.nope", "nope"] {
            write(
                d.path(),
                "oxplow/extensions/beta/extension.yaml",
                &SUBSCRIBER.replace("acme_pr.merged", bad),
            );
            write(
                d.path(),
                "oxplow/extensions/beta/note.star",
                "def transform(x):\n    return {\"skip\": \"no\"}\n",
            );
            write(d.path(), "oxplow/extensions/beta/pull.sh", "#!/bin/sh\n");
            let loaded = crate::extensions::load_extensions(d.path())
                .into_iter()
                .find(|e| e.name == "beta")
                .unwrap();
            let refused = loaded
                .errors
                .iter()
                .filter(|e| e.contains(&format!("`{bad}` isn't a registered event type")))
                .count();
            assert_eq!(refused, 2, "{bad}: {:?}", loaded.errors);
            assert!(
                loaded.effects.is_empty() && loaded.collectors.is_empty(),
                "{bad}"
            );
        }
    }
}
