//! `extensions.check` operations: look at an extension before it ships —
//! validate a folder (`oxplow.extension.validate`), preview it in a
//! throwaway oxplow (`oxplow.extension.preview`). Neither changes a record.

use std::path::Path;
use std::sync::{Arc, OnceLock};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use oxplow_domain::CommandError;

use super::super::ops::Op;
use super::super::util::{failed, invalid, ServicesSlot};
use super::ids::{check_optional_stream, read_op, result_of};
use super::stream_or_actors;

/// Why a check of an extension didn't produce a report.
pub enum CheckError {
    /// No such extension in that stream's worktree.
    NotFound,
    Failed(String),
}

/// The checks that live in `oxplow-sdk`, which depends on this crate: the
/// host that links both installs the implementation once
/// (`Services::extension_checks`; `oxplow_mcp::install_extension_checks`).
#[async_trait::async_trait]
pub trait ExtensionChecks: Send + Sync {
    /// The `oxplow extension check` report for `name` under `root`.
    async fn validate(
        &self,
        svc: &crate::Services,
        root: &Path,
        name: &str,
    ) -> Result<Value, CheckError>;
    /// What `oxplow extension test` reports for `name` under `root`, in a
    /// throwaway oxplow with no environment.
    async fn preview(&self, root: &Path, name: &str) -> Result<Value, CheckError>;
}

/// Where the installed [`ExtensionChecks`] sits.
#[derive(Clone, Default)]
pub struct ExtensionChecksSlot(Arc<OnceLock<Arc<dyn ExtensionChecks>>>);

impl ExtensionChecksSlot {
    pub fn install(&self, checks: Arc<dyn ExtensionChecks>) {
        let _ = self.0.set(checks);
    }

    fn get(&self) -> Result<Arc<dyn ExtensionChecks>, CommandError> {
        self.0.get().cloned().ok_or_else(|| CommandError::Failed {
            message: "this oxplow host doesn't run extension checks".into(),
        })
    }
}

fn check_failed(name: &str, e: CheckError) -> CommandError {
    match e {
        CheckError::NotFound => not_found(name),
        CheckError::Failed(m) => failed(m),
    }
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ValidateExtensionParams {
    /// Extension folder name under `.oxplow/extensions/`.
    pub name: String,
    /// Stream whose worktree to read from; omit for your own.
    pub stream_id: Option<String>,
}

fn not_found(name: &str) -> CommandError {
    invalid(
        "/name",
        format!("no extension `{name}` under .oxplow/extensions/ in that stream"),
    )
}

pub fn ops(services: ServicesSlot) -> Vec<Op> {
    vec![
        read_op(
            "extensions.check",
            "validate",
            services.clone(),
            |svc, actor, p: ValidateExtensionParams| async move {
                check_optional_stream(
                    "oxplow_validate_extension",
                    "/stream_id",
                    p.stream_id.as_deref(),
                )?;
                let stream = stream_or_actors(&svc, &actor, p.stream_id.clone()).await?;
                let root = svc
                    .worktrees
                    .resolve(stream.as_deref())
                    .await
                    .into_local_path();
                let report = svc
                    .extension_checks
                    .get()?
                    .validate(&svc, &root, &p.name)
                    .await
                    .map_err(|e| check_failed(&p.name, e))?;
                result_of(report)
            },
        ),
        read_op(
            "extensions.check",
            "preview",
            services.clone(),
            |svc, actor, p: ValidateExtensionParams| async move {
                check_optional_stream(
                    "oxplow_preview_extension",
                    "/stream_id",
                    p.stream_id.as_deref(),
                )?;
                let stream = stream_or_actors(&svc, &actor, p.stream_id.clone()).await?;
                let root = svc
                    .worktrees
                    .resolve(stream.as_deref())
                    .await
                    .into_local_path();
                // An agent's preview: the extension's programs see no
                // environment (no person's credentials), and model calls
                // answer from the fixtures.
                let report = svc
                    .extension_checks
                    .get()?
                    .preview(&root, &p.name)
                    .await
                    .map_err(|e| check_failed(&p.name, e))?;
                result_of(report)
            },
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_domain::Actor;
    use serde_json::json;

    /// Stands in for `oxplow-sdk`'s checks: knows one extension, `mine`,
    /// by finding its folder under the root it was given.
    struct Folder;

    #[async_trait::async_trait]
    impl ExtensionChecks for Folder {
        async fn validate(
            &self,
            _: &crate::Services,
            root: &Path,
            name: &str,
        ) -> Result<Value, CheckError> {
            root.join(".oxplow/extensions")
                .join(name)
                .exists()
                .then(|| json!({ "ok": true, "root": root.to_string_lossy() }))
                .ok_or(CheckError::NotFound)
        }
        async fn preview(&self, root: &Path, name: &str) -> Result<Value, CheckError> {
            self.validate_in(root, name)
        }
    }

    impl Folder {
        fn validate_in(&self, root: &Path, name: &str) -> Result<Value, CheckError> {
            root.join(".oxplow/extensions")
                .join(name)
                .exists()
                .then(|| json!({ "ok": true }))
                .ok_or(CheckError::NotFound)
        }
    }

    /// An extension is validated from the calling stream's worktree
    /// (an agent in a worktree sees what it just wrote there); an unknown
    /// one and a review naming neither source are refused as the caller's.
    #[tokio::test]
    async fn extensions_are_checked_in_the_callers_stream() {
        use oxplow_domain::stores::StreamStore as _;
        let dir = tempfile::tempdir().unwrap();
        crate::test_fixtures::init_git_repo(dir.path());
        let svc = crate::Services::in_memory(dir.path()).await.unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let ext = elsewhere.path().join(".oxplow/extensions/mine");
        std::fs::create_dir_all(&ext).unwrap();
        std::fs::write(
            ext.join("extension.yaml"),
            "manifest: 2\nname: mine\nintent:\n  purpose: test\n",
        )
        .unwrap();
        let mut other = svc.stream_store.list().await.unwrap().pop().unwrap();
        other.id = oxplow_domain::StreamId::placeholder();
        other.title = "other".into();
        other.branch = "other".into();
        other.kind = oxplow_domain::StreamKind::Worktree;
        other.worktree_path = elsewhere.path().to_string_lossy().into_owned();
        let other_id = svc.stream_store.upsert(&other).await.unwrap();
        svc.extension_checks.install(Arc::new(Folder));
        let in_other = Actor::Agent {
            session_id: None,
            thread_id: None,
            stream_id: Some(other_id),
        };
        let report = svc
            .commands
            .run(
                &in_other,
                "oxplow.extension.validate",
                json!({ "name": "mine" }),
                false,
            )
            .await
            .unwrap();
        assert!(report.audit_id.is_none());
        // It ran in the calling stream's worktree, not the primary's.
        assert_eq!(
            report.result["root"],
            json!(elsewhere.path().to_string_lossy()),
            "{}",
            report.result
        );

        let err = svc
            .commands
            .run(
                &in_other,
                "oxplow.extension.validate",
                json!({ "name": "nope" }),
                false,
            )
            .await
            .unwrap_err();
        assert!(
            matches!(&err, CommandError::Invalid { message, .. } if message.contains("no extension `nope`")),
            "{err:?}"
        );
    }
}
