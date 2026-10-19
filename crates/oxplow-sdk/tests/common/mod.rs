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
    let off = oxplow_config::extension_settings(root).personal.disable;
    for name in names.into_iter().filter(|n| !off.contains(n)) {
        oxplow_app::extension_deploy::deploy(svc, &name, root, None, true)
            .await
            .unwrap_or_else(|e| panic!("deploy `{name}`: {e}"));
    }
}

/// Boot oxplow over `root` and deploy the sources there.
pub async fn booted(root: &Path) -> Arc<oxplow_app::Services> {
    let svc = oxplow_app::Services::in_memory(root)
        .await
        .expect("oxplow boots over the project");
    deploy_sources(&svc, root).await;
    svc
}
