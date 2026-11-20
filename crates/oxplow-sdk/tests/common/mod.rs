//! Shared by the integration tests: a real oxplow over a project whose
//! extension sources were just written. Only deployed extensions run, so
//! what the test wrote is deployed, as an agent that wrote it would.

#![allow(dead_code)]

use std::path::Path;
use std::sync::Arc;

/// Deploy every extension source under `root`'s `.oxplow/extensions/` (an
/// unchanged one is already live, a disabled one is left) and activate.
pub async fn deploy_sources(svc: &Arc<oxplow_app::Services>, root: &Path) {
    let mut names: Vec<String> = std::fs::read_dir(root.join(".oxplow/extensions"))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    names.sort();
    // One the person turned off stays off (deploying it would turn it on).
    let settings = oxplow_config::extension_settings(root);
    for name in names.into_iter().filter(|n| !settings.turned_off(n)) {
        oxplow_app::extension::extension_deploy::deploy(svc, &name, true)
            .await
            .unwrap_or_else(|e| panic!("deploy `{name}`: {e}"));
    }
}

/// Turn on, in `root`'s project settings, each shipped extension a source
/// there `requires:` (a language pack a metric reads with), as a person
/// does before deploying it.
pub fn enable_required_shipped(root: &Path) {
    use std::io::Write as _;
    let mut wanted: Vec<String> = Vec::new();
    for entry in std::fs::read_dir(root.join(".oxplow/extensions"))
        .into_iter()
        .flatten()
        .flatten()
    {
        let Ok(text) = std::fs::read_to_string(entry.path().join("extension.yaml")) else {
            continue;
        };
        let Ok(value) = serde_yaml::from_str::<serde_yaml::Value>(&text) else {
            continue;
        };
        for name in value
            .get("requires")
            .and_then(|r| r.as_sequence())
            .into_iter()
            .flatten()
            .filter_map(|v| v.as_str())
        {
            if oxplow_app::bundled_extensions::find(name).is_some_and(|b| !b.required)
                && !wanted.iter().any(|w| w == name)
            {
                wanted.push(name.to_string());
            }
        }
    }
    if wanted.is_empty() {
        return;
    }
    let path = root.join(".oxplow/project.yaml");
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .expect("project settings");
    for name in wanted {
        writeln!(f, "\noxplow.extensions.{name}.enabled: true").expect("written");
    }
}

/// Boot oxplow over `root` and deploy the sources there.
pub async fn booted(root: &Path) -> Arc<oxplow_app::Services> {
    enable_required_shipped(root);
    let svc = oxplow_app::Services::in_memory(root)
        .await
        .expect("oxplow boots over the project");
    deploy_sources(&svc, root).await;
    svc
}
