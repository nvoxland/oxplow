//! Helpers that scenario tests of an extension's life (deploy, disable,
//! rollback, the work list switching) share with the tests of one
//! transition: what once sat private in those files.

#![cfg(test)]

use std::path::{Path, PathBuf};

/// The extension the fake provider is declared in.
pub(crate) const EXT: &str = "tracker";

/// The fake provider's binary, built beside this test binary (the
/// workspace build builds every crate's bins).
pub(crate) fn fake_bin() -> PathBuf {
    let exe = std::env::current_exe().expect("test exe");
    let dir = exe
        .parent()
        .and_then(Path::parent)
        .expect("target/<profile>/deps");
    let bin = dir.join("oxplow-provider-fake");
    assert!(
        bin.is_file(),
        "{} is missing; build it with `cargo build -p oxplow-provider-fake`",
        bin.display()
    );
    bin
}

/// A private extension declaring the fake as its work-items provider,
/// its entry a script running the fake with `hooks`.
pub(crate) fn write_extension(project: &Path, hooks: &str) {
    let dir = project.join(".oxplow/extensions").join(EXT);
    std::fs::create_dir_all(dir.join("bin")).unwrap();
    std::fs::write(
        dir.join("extension.yaml"),
        "manifest: 2\nname: tracker\nintent:\n  purpose: the fake tracker\n  examples: [{ name: a }]\ncommands:\n  - name: item.estimate\n    summary: Set a work item's points.\n    provider: fake\n    op: estimate\n    ui: { label: \"Estimate in Fake…\", group: Tracker, about: oxplow/work_item, input: { uri: \"{{uri}}\", points: 3 } }\npermissions: [process.run]\nproviders:\n  - id: fake\n    capability: work_items\n    entry: bin/provider\n    declarations: provider.json\n",
    )
    .unwrap();
    let script = dir.join("bin/provider");
    // Its service's state outlives its process, as a real service's does
    // (each instance its own file): what a restart re-sends against.
    let state = project.join(".oxplow");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nOXPLOW_FAKE_HOOKS='{hooks}' OXPLOW_FAKE_STATE=\"{}/fake-state-$OXPLOW_PROVIDER_ID.json\" exec '{}' \"$@\"\n",
            state.display(),
            fake_bin().display()
        ),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    // `plain-writes` declares no `idempotent_writes`: what is approved.
    let declared = if hooks.split(',').any(|h| h.trim() == "plain-writes") {
        oxplow_provider_fake::plain_declarations()
    } else {
        oxplow_provider_fake::declarations()
    };
    std::fs::write(
        dir.join("provider.json"),
        serde_json::to_string_pretty(&declared).unwrap(),
    )
    .unwrap();
}

/// Whether `view` can be read by SQL, as a user's query would.
pub(crate) async fn readable(svc: &crate::Services, view: &str) -> bool {
    svc.sql
        .query_sql(&format!("SELECT uri FROM {view}"), vec![], None)
        .await
        .is_ok()
}

/// What activation said against `extension`, one line each; empty when
/// nothing is held back.
pub(crate) fn problem(svc: &crate::Services, extension: &str) -> String {
    svc.extension_activation
        .problems()
        .get(extension)
        .map(|lines| lines.join("\n"))
        .unwrap_or_default()
}

/// Page `id` run as a person: an error when it isn't there (withheld)
/// or a loader of it failed.
pub(crate) async fn page(
    svc: &crate::Services,
    id: &str,
) -> Result<crate::pages::runner::PageRun, String> {
    use crate::pages::runner::{PageRunner, RunRequest, RunTarget};
    let run = PageRunner::of(svc)
        .run(RunRequest {
            target: RunTarget::Page(id.to_string()),
            params: Default::default(),
            ctx: Default::default(),
            viewer: oxplow_domain::Actor::Human,
        })
        .await
        .map_err(|e| e.to_string())?;
    match run.loaders.values().find_map(|l| l.error.clone()) {
        Some(error) => Err(error),
        None => Ok(run),
    }
}

/// Log an `effort.claim_verified` for `effort` with `evidence` and return
/// it as stored: an event an `on:` collector or effect can react to.
pub(crate) async fn log_claim(
    svc: &crate::Services,
    effort: oxplow_domain::EffortId,
    evidence: &str,
) -> oxplow_domain::StoredEvent {
    use oxplow_domain::events::Anchors;
    use oxplow_domain::Envelope;
    let env = Envelope::typed::<oxplow_domain::events::schema::EffortClaimVerified>(
        "human",
        &oxplow_domain::events::schema::EffortClaimVerifiedV2 {
            claim: 1,
            effort: None,
            evidence: Some(evidence.into()),
        },
    )
    .with_anchors(Anchors {
        effort_id: Some(effort),
        ..Anchors::default()
    });
    let id = env.id.clone();
    svc.event_log_store.append(env).await.unwrap();
    svc.event_log_store.get(id).await.unwrap().unwrap()
}

/// `CONFIGURATION` at version 2: `limit` was `max` at version 1.
pub(crate) const CONFIGURATION_V2: &str = "configuration:
  version: 2
  schema:
    type: object
    required: [limit]
    additionalProperties: false
    properties:
      limit: { type: integer, minimum: 1, maximum: 100 }
      label: { type: string }
  defaults: { limit: 10 }
";

/// `services_with_effort` with the pump's reactors a running oxplow has for
/// the life of an extension: its `on:` collectors, its effects and what
/// follows a changed setting. Boot registers them; in-memory services have
/// none, so a scenario that logs an event or sets a config key asks for them.
pub(crate) async fn booted() -> crate::test_fixtures::EffortFixture {
    let fx = crate::test_fixtures::services_with_effort().await;
    crate::capabilities::register(&fx.svc);
    crate::collector_triggers::register(&fx.svc);
    crate::effect_triggers::register(&fx.svc);
    crate::config_reactors::register(&fx.svc);
    fx
}
