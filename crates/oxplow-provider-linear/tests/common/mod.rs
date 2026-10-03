//! What the kit and live suites share: the example extension, installed
//! into a throwaway repo with the built provider as its entry.

#![allow(clippy::unwrap_used)]

use std::path::{Path, PathBuf};

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

/// `examples/extensions/linear` in this repo.
pub fn example_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/extensions/linear")
}

/// A git repo at `root` with the example extension in it, its entry
/// running the provider this build produced: the extension's folder.
pub async fn install_example(root: &Path) -> PathBuf {
    oxplow_app::vcs::GitProvider
        .init_repository(root)
        .await
        .unwrap();
    let ext = root.join("oxplow/extensions/linear");
    copy_dir(&example_dir(), &ext);
    let entry = ext.join("bin/oxplow-provider-linear");
    std::fs::write(
        &entry,
        format!(
            "#!/bin/sh\nexec '{}' \"$@\"\n",
            env!("CARGO_BIN_EXE_oxplow-provider-linear")
        ),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&entry, std::fs::Permissions::from_mode(0o755)).unwrap();
    ext
}

/// Rewrite one of the extension's files: `from` (which must be there)
/// becomes `to`.
pub fn rewrite(ext: &Path, file: &str, from: &str, to: &str) {
    let path = ext.join(file);
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains(from), "{file} has no `{from}`");
    std::fs::write(&path, text.replace(from, to)).unwrap();
}

/// The environment a `plugin test` run of the example sees: this key, and
/// `LINEAR_API_URL` only when `url` is given (Linear's own otherwise) —
/// whatever this process holds for either. Each run names its own, so
/// tests running at once never meet (no `set_var`: the kit and the live
/// suite share a binary's threads).
pub fn linear_env(key: &str, url: Option<&str>) -> oxplow_app::providers::host::HostEnv {
    let key = key.to_string();
    let url = url.map(str::to_string);
    std::sync::Arc::new(move |name| match name {
        "LINEAR_API_KEY" => Some(key.clone()),
        "LINEAR_API_URL" => url.clone(),
        _ => std::env::var(name).ok(),
    })
}
