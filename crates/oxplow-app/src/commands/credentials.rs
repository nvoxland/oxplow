//! The values an extension keeps in the OS keychain, set as commands:
//! `oxplow.collector.set_credential` (a credential some collector
//! declares), `oxplow.extension.set_secret` and `revoke_secret` (a host
//! secret a provider's credential refers to). Secrets are a kind of value,
//! not a kind of actor: written and never read back, kept out of every
//! record (`value` is `writeOnly`, commands.md "Secrets"), masked where a
//! person types one. What gates who may write one is the harness's and the
//! person's permissions, not a floor here. Reading is presence only: `v_collector_credential`
//! and `v_extension_secret`.
//! `External`: the keychain is outside oxplow's database.

use std::sync::Arc;

use oxplow_domain::extension_secret::SecretRef;
use oxplow_domain::CommandError;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

use super::ops::Op;
use super::util::{failed, invalid, parse, schema, ServicesSlot};
use super::{Handler, HandlerOutput, Invocation};
use crate::collector_runner::{self, Collectors};
use crate::events::OxplowEvent;

pub const SET_CREDENTIAL: &str = "oxplow.collector.set_credential";
pub const SET_SECRET: &str = "oxplow.extension.set_secret";
pub const REVOKE_SECRET: &str = "oxplow.extension.revoke_secret";

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetCredentialInput {
    /// The collector that declares the credential: its URI
    /// (`oxplow:oxplow/collector?collector=<extension>/<id>`) or its
    /// `<extension>/<id>`.
    pub collector: String,
    /// The credential's name, as the collector declares it.
    pub name: String,
    /// Its value, kept in the OS keychain and never read back. Left out or
    /// blank, the credential is cleared. A secret (`writeOnly`).
    #[serde(default)]
    #[schemars(extend("writeOnly" = true))]
    pub value: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetSecretInput {
    /// The extension whose provider's credential refers to the secret.
    pub extension: String,
    /// The secret's reference (`v_extension_secret.reference`); left out, a
    /// fresh one is made.
    #[serde(default)]
    pub reference: Option<SecretRef>,
    /// The value, kept in the OS keychain and never read back. A secret
    /// (`writeOnly`).
    #[schemars(extend("writeOnly" = true))]
    pub value: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RevokeSecretInput {
    /// The extension the secret belongs to.
    pub extension: String,
    /// The secret's reference (`v_extension_secret.reference`).
    pub reference: SecretRef,
}

/// The namespace a collector is named in, from its URI or `<namespace>/<id>`.
fn namespace_of(collector: &str) -> Result<String, CommandError> {
    let id = oxplow_domain::uri::oxplow_uri::OxplowUri::parse(collector)
        .ok()
        .and_then(|u| u.param("collector").map(str::to_string))
        .unwrap_or_else(|| collector.to_string());
    id.split_once('/')
        .map(|(extension, _)| extension.to_string())
        .filter(|e| !e.is_empty())
        .ok_or_else(|| {
            invalid(
                "/collector",
                "name a collector as `<namespace>/<id>` or its URI",
            )
        })
}

pub fn ops(services: ServicesSlot) -> Vec<Op> {
    let set_credential = {
        let services = services.clone();
        Op::new(
            "credentials.write",
            "set_credential",
            schema::<SetCredentialInput>(),
            false,
            Handler::External(Arc::new(move |_: Invocation, input| {
                let services = services.clone();
                Box::pin(async move {
                    let input: SetCredentialInput = parse(input)?;
                    let svc = services.get()?;
                    let namespace = namespace_of(&input.collector)?;
                    let root = svc.worktrees.resolve(None).await.into_local_path();
                    let given = input.value.as_deref().is_some_and(|v| !v.trim().is_empty());
                    collector_runner::set_credential(
                        &Collectors::of(&svc, &root),
                        &namespace,
                        &input.name,
                        input.value.as_deref(),
                    )
                    .map_err(|e| match e {
                        oxplow_domain::DomainError::Invalid(m) => invalid("/name", m),
                        oxplow_domain::DomainError::NotFound => invalid(
                            "/collector",
                            format!("no extension in namespace `{namespace}`"),
                        ),
                        other => failed(other),
                    })?;
                    let events = svc.events.clone();
                    let name = input.name.clone();
                    Ok(HandlerOutput {
                        result: json!({ "collector": input.collector, "name": input.name, "set": given }),
                        after_commit: Some(Box::new(move || {
                            events.emit(OxplowEvent::CredentialChanged {
                                instance: namespace,
                                name,
                                sign_in: None,
                                error: None,
                            })
                        })),
                        ..HandlerOutput::default()
                    })
                })
            })),
        )
    };
    let set_secret = {
        let services = services.clone();
        Op::new(
            "credentials.write",
            "set_secret",
            schema::<SetSecretInput>(),
            false,
            Handler::External(Arc::new(move |_: Invocation, input| {
                let services = services.clone();
                Box::pin(async move {
                    let input: SetSecretInput = parse(input)?;
                    let svc = services.get()?;
                    let reference = svc
                        .extension_secrets
                        .save(&input.extension, input.reference, &input.value)
                        .await
                        .map_err(|e| match e {
                            oxplow_domain::DomainError::Invalid(m) => invalid("/value", m),
                            other => failed(other),
                        })?;
                    secret_changed(&svc, &input.extension, &reference).await;
                    let events = svc.events.clone();
                    let extension = input.extension.clone();
                    Ok(HandlerOutput {
                        result: json!({ "extension": input.extension, "reference": reference }),
                        after_commit: Some(Box::new(move || {
                            events.emit(OxplowEvent::CredentialChanged {
                                instance: extension,
                                name: String::new(),
                                sign_in: None,
                                error: None,
                            })
                        })),
                        ..HandlerOutput::default()
                    })
                })
            })),
        )
    };
    let revoke_secret = Op::new(
        "credentials.write",
        "revoke_secret",
        schema::<RevokeSecretInput>(),
        false,
        Handler::External(Arc::new(move |_: Invocation, input| {
            let services = services.clone();
            Box::pin(async move {
                let input: RevokeSecretInput = parse(input)?;
                let svc = services.get()?;
                svc.extension_secrets
                    .revoke(&input.extension, &input.reference)
                    .map_err(failed)?;
                secret_changed(&svc, &input.extension, &input.reference).await;
                let events = svc.events.clone();
                let extension = input.extension.clone();
                Ok(HandlerOutput {
                    result: json!({ "extension": input.extension, "reference": input.reference }),
                    after_commit: Some(Box::new(move || {
                        events.emit(OxplowEvent::CredentialChanged {
                            instance: extension,
                            name: String::new(),
                            sign_in: None,
                            error: None,
                        })
                    })),
                    ..HandlerOutput::default()
                })
            })
        })),
    );
    vec![set_credential, set_secret, revoke_secret]
}

/// Tell the providers running on the secret that it moved (a rotated token
/// restarts what holds it). The keychain change is already made; this
/// cannot be dropped with the request.
async fn secret_changed(svc: &crate::Services, extension: &str, reference: &SecretRef) {
    let providers = svc.providers.clone();
    let (extension, reference) = (extension.to_string(), reference.clone());
    let _ = tokio::spawn(async move {
        providers
            .extension_secret_changed(&extension, &reference)
            .await;
    })
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_domain::extension_secret::SecretRef;
    use oxplow_domain::Actor;

    /// A project extension whose collector declares the credential `TOKEN`,
    /// loaded by the fixture's catalog.
    fn extension(root: &std::path::Path) {
        let ext = root.join(".oxplow/extensions/acme");
        std::fs::create_dir_all(&ext).unwrap();
        std::fs::write(
            ext.join("extension.yaml"),
            "manifest: 2\nname: acme\nintent:\n  purpose: test\npermissions: [process.run, secrets:TOKEN]\ncollectors:\n  - id: s\n    runtime: exec\n    entry: sync.sh\n    credentials: [TOKEN]\n    entities:\n      - { name: row, key: id, columns: { id: int } }\n",
        )
        .unwrap();
        std::fs::write(
            ext.join("sync.sh"),
            "#!/bin/sh\necho '{\"entities\":{\"row\":[]}}'\n",
        )
        .unwrap();
    }

    async fn run(
        fx: &crate::test_fixtures::EffortFixture,
        actor: &Actor,
        name: &str,
        input: serde_json::Value,
    ) -> Result<crate::commands::CommandOutcome, CommandError> {
        fx.svc.commands.run(actor, name, input, true).await
    }

    /// A collector's credential is set and cleared as commands: the value
    /// goes to the keychain, `v_collector_credential` says whether one is
    /// set (never the value), the run's record and result leave the value
    /// out, and an agent may run it too — a secret is a value, not an
    /// actor's privilege.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_collector_credential_is_set_and_cleared_and_never_recorded() {
        let dir = tempfile::tempdir().unwrap();
        crate::test_fixtures::init_git_repo(dir.path());
        extension(dir.path());
        let fx = crate::test_fixtures::services_with_effort_in(dir).await;
        crate::test_fixtures::activate(&fx.svc).await;
        let collector = "oxplow:oxplow/collector?collector=acme/s";
        let set = || async {
            fx.svc
                .sql
                .query_sql(
                    "SELECT is_set FROM v_collector_credential WHERE name = 'TOKEN'",
                    vec![],
                    None,
                )
                .await
                .unwrap()
                .rows
        };
        let out = run(
            &fx,
            &Actor::Human,
            SET_CREDENTIAL,
            json!({ "collector": collector, "name": "TOKEN", "value": "sk-very-secret" }),
        )
        .await
        .unwrap();
        assert!(!out.result.to_string().contains("sk-very-secret"));
        assert_eq!(set().await[0][0], oxplow_db::SqlCell::Int(1));

        // Nowhere the audit or the event log holds it.
        let audit = fx
            .svc
            .commands
            .audit_store()
            .list_recent(1_000)
            .await
            .unwrap();
        assert!(
            !format!("{audit:?}").contains("sk-very-secret"),
            "{audit:?}"
        );

        // An agent may; a blank value clears it.
        let agent = Actor::Agent {
            session_id: None,
            thread_id: Some(fx.thread),
            stream_id: None,
        };
        run(
            &fx,
            &agent,
            SET_CREDENTIAL,
            json!({ "collector": "acme/s", "name": "TOKEN" }),
        )
        .await
        .unwrap();
        assert_eq!(set().await[0][0], oxplow_db::SqlCell::Int(0));

        // A credential nothing declares, or an unknown extension, is refused.
        for (input, field) in [
            (
                json!({ "collector": "acme/s", "name": "OTHER", "value": "x" }),
                "/name",
            ),
            (
                json!({ "collector": "nope/s", "name": "TOKEN", "value": "x" }),
                "/collector",
            ),
            (
                json!({ "collector": "no-slash", "name": "TOKEN", "value": "x" }),
                "/collector",
            ),
        ] {
            let err = run(&fx, &Actor::Human, SET_CREDENTIAL, input)
                .await
                .unwrap_err();
            assert!(
                matches!(&err, CommandError::Invalid { field: Some(f), .. } if f == field),
                "{err:?}"
            );
        }
    }

    /// A project extension whose provider is a remote MCP server reached
    /// with a host secret as its bearer (`reference`).
    fn secret_extension(root: &std::path::Path, reference: &SecretRef) {
        let ext = root.join(".oxplow/extensions/tracker");
        std::fs::create_dir_all(ext.join("mcp")).unwrap();
        let id = serde_json::to_value(reference).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string();
        std::fs::write(
            ext.join("extension.yaml"),
            format!(
                "manifest: 2\nname: tracker\nintent:\n  purpose: notes over MCP\n  examples: [{{ name: a }}]\npermissions: [process.run, network:mcp.example.com, secrets:OXPLOW_MCP_BEARER]\nproviders:\n  - id: notes\n    capability: work_items\n    network: [mcp.example.com]\n    adapter:\n      mcp: {{ url: \"https://mcp.example.com/mcp\", auth: OXPLOW_MCP_BEARER }}\n      mapping: mcp/x.star\n      tools: mcp/tools.json\n    credentials:\n      - {{ name: OXPLOW_MCP_BEARER, secretRef: {{ schemaVersion: 1, id: \"{id}\" }} }}\n    declarations: provider.json\n"
            ),
        )
        .unwrap();
        std::fs::write(ext.join("mcp/x.star"), "def transform(x):\n    return {}\n").unwrap();
        std::fs::write(
            ext.join("mcp/tools.json"),
            r#"[{ "name": "list_items", "description": "List.", "inputSchema": { "type": "object" } }]"#,
        )
        .unwrap();
        std::fs::write(
            ext.join("provider.json"),
            serde_json::to_string_pretty(&oxplow_provider_fake::declarations()).unwrap(),
        )
        .unwrap();
    }

    /// A provider's host secret is set, shown as set (never its value) and
    /// revoked as commands, by a person or an agent; the value is in no
    /// record.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_provider_secret_is_set_listed_and_revoked_and_never_recorded() {
        let reference = SecretRef::generate();
        let dir = tempfile::tempdir().unwrap();
        crate::test_fixtures::init_git_repo(dir.path());
        secret_extension(dir.path(), &reference);
        let fx = crate::test_fixtures::services_with_effort_in(dir).await;
        crate::test_fixtures::activate(&fx.svc).await;
        let rows = || async {
            fx.svc
                .sql
                .query_sql(
                    "SELECT extension, provider, name, reference, is_set FROM v_extension_secret",
                    vec![],
                    None,
                )
                .await
                .unwrap()
                .rows
        };
        let listed = rows().await;
        assert_eq!(listed.len(), 1, "{listed:?}");
        assert_eq!(listed[0][0], oxplow_db::SqlCell::Text("tracker".into()));
        assert_eq!(
            listed[0][2],
            oxplow_db::SqlCell::Text("OXPLOW_MCP_BEARER".into())
        );
        assert_eq!(listed[0][4], oxplow_db::SqlCell::Int(0));
        let wire: serde_json::Value = match &listed[0][3] {
            oxplow_db::SqlCell::Text(t) => serde_json::from_str(t).unwrap(),
            other => panic!("{other:?}"),
        };
        assert_eq!(wire, serde_json::to_value(&reference).unwrap());

        let agent = Actor::Agent {
            session_id: None,
            thread_id: Some(fx.thread),
            stream_id: None,
        };
        let out = run(
            &fx,
            &agent,
            SET_SECRET,
            json!({ "extension": "tracker", "reference": wire, "value": "bearer-very-secret" }),
        )
        .await
        .unwrap();
        assert!(!out.result.to_string().contains("bearer-very-secret"));
        assert_eq!(rows().await[0][4], oxplow_db::SqlCell::Int(1));

        let audit = fx
            .svc
            .commands
            .audit_store()
            .list_recent(1_000)
            .await
            .unwrap();
        assert!(!format!("{audit:?}").contains("bearer-very-secret"));

        run(
            &fx,
            &Actor::Human,
            REVOKE_SECRET,
            json!({ "extension": "tracker", "reference": wire }),
        )
        .await
        .unwrap();
        assert_eq!(rows().await[0][4], oxplow_db::SqlCell::Int(0));
    }
}
