//! P10: a signed-in bearer, end to end — the person signs in at the
//! stand-in authorization server (`oxplow-oauth-sim`), the redirect
//! caught by the shell's listener and handed to the core, a by-url MCP
//! instance runs on that token, the server stops taking it, and the next
//! call renews once and lands.

#![allow(clippy::unwrap_used)]

use std::path::Path;
use std::sync::Arc;

use oxplow_app::events::OxplowEvent;
use oxplow_app::providers::registry::InstanceState;
use oxplow_app::providers::SignInCompletion;
use oxplow_domain::Actor;
use oxplow_oauth_sim::OAuthSim;
use serde_json::json;

const INSTANCE: &str = "notes/notes";

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// The notes fixture with its server reached at the stand-in's `/mcp`, its
/// bearer `NOTES_TOKEN` a credential the person signs in for there.
fn write_extension(project: &Path, sim: &OAuthSim) {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/notes");
    let ext = project.join("oxplow/extensions/notes");
    copy_dir(&fixture, &ext);
    let manifest = std::fs::read_to_string(ext.join("extension.yaml")).unwrap();
    let by_command = "      mcp: { command: [bin/notes-server] }\n";
    assert!(manifest.contains(by_command));
    std::fs::write(
        ext.join("extension.yaml"),
        manifest.replace(
            by_command,
            &format!("      mcp: {{ url: \"{}\", auth: NOTES_TOKEN }}\n", sim.mcp_url),
        ) + &format!(
            "    credentials:\n      - name: NOTES_TOKEN\n        oauth:\n          authorize_url: {}\n          token_url: {}\n          client_id: oxplow-test\n    network: [127.0.0.1]\n",
            sim.authorize_url, sim.token_url
        ),
    )
    .unwrap();
}

async fn create(svc: &oxplow_app::Services, title: &str) -> Result<(), String> {
    svc.commands
        .run(
            &Actor::Human,
            "work_item.create",
            json!({ "provider": "notes", "title": title }),
            false,
        )
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[tokio::test(flavor = "multi_thread")]
async fn a_signed_in_bearer_renews_once_when_its_server_stops_taking_it() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path();
    oxplow_app::vcs::GitProvider
        .init_repository(project)
        .await
        .unwrap();
    let sim = OAuthSim::start().await;
    write_extension(project, &sim);
    let svc = oxplow_app::Services::in_memory_on_machine(
        project,
        project.join(".oxplow/global-config"),
        Arc::new(oxplow_ai::secrets::MemorySecrets::default()),
        oxplow_app::providers::host::process_env(),
    )
    .unwrap();

    // The person approves the provider (where it signs in is part of it)
    // and enables the instance.
    let hosted = svc
        .extension_catalog
        .get(project)
        .iter()
        .find(|e| e.name == "notes")
        .cloned()
        .unwrap();
    assert_eq!(hosted.errors, Vec::<String>::new());
    let spec = hosted.providers[0].clone();
    let program = oxplow_app::exec_consent::provider_program(&hosted, &spec);
    let version = program.hash(project).unwrap();
    oxplow_app::exec_consent::approve_program(
        &svc.approvals,
        project,
        &oxplow_app::config_service::read_config(&svc.config),
        std::slice::from_ref(&hosted),
        oxplow_app::exec_consent::ProgramKind::Provider,
        &program.name,
        &version,
    )
    .unwrap();
    svc.config.write().unwrap().extension_instances.insert(
        INSTANCE.into(),
        oxplow_config::ExtensionInstanceConfig {
            enabled: true,
            config: json!({}),
            sync_minutes: None,
            provider: None,
        },
    );

    // Sign in as the desktop does: the shell listens for the redirect,
    // the person's browser opens the page and comes back to it, and the
    // shell hands the redirect to the core and answers the browser.
    let mut ui = svc.events.subscribe_ui();
    let mut listener = oxplow_app::oauth_redirect::RedirectListener::bind(0)
        .await
        .unwrap();
    let page = svc
        .providers
        .begin_sign_in(INSTANCE, "NOTES_TOKEN", listener.port())
        .await
        .unwrap();
    let browser = tokio::spawn(async move { reqwest::get(&page).await.unwrap().status() });
    let redirect = listener.next().await.unwrap();
    let completion = svc
        .providers
        .complete_sign_in(INSTANCE, "NOTES_TOKEN", &redirect.target)
        .await
        .unwrap();
    assert_eq!(completion, SignInCompletion::SignedIn);
    redirect.answer(true, "Signed in.").await;
    assert!(browser.await.unwrap().is_success());
    let signed_in = async {
        loop {
            if let Ok(OxplowEvent::CredentialChanged { name, error, .. }) = ui.recv().await {
                if name == "NOTES_TOKEN" {
                    return error;
                }
            }
        }
    };
    let error = tokio::time::timeout(std::time::Duration::from_secs(20), signed_in)
        .await
        .expect("the sign-in finishes");
    assert_eq!(error, None);
    svc.providers.reconcile().await;
    assert_eq!(
        svc.providers.health(INSTANCE).map(|h| h.state),
        Some(InstanceState::Ready)
    );
    create(&svc, "First").await.unwrap();
    assert_eq!(sim.grants(), vec!["authorization_code"]);

    // The server stops taking the token the instance holds: the next call
    // is refused `Auth` naming NOTES_TOKEN, renewed once, and lands.
    sim.expire();
    create(&svc, "Second").await.unwrap();
    assert_eq!(sim.grants(), vec!["authorization_code", "refresh_token"]);
    let health = svc.providers.health(INSTANCE).unwrap();
    assert_eq!(
        (health.state, health.consecutive_failures),
        (InstanceState::Ready, 0)
    );
    svc.providers.stop(INSTANCE).await;
}
