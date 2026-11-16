//! Finding a binary this build made. The app ships its sidecars beside its
//! executable; in development they are built into `target/<profile>/`,
//! while a test binary runs from below it — `target/<profile>/deps/`, or
//! with newer cargo `target/<profile>/build/<pkg>/<hash>/out/`. So a
//! binary is looked for beside the executable, then in each folder above.

use std::path::{Path, PathBuf};

/// The first file `name` in `dir` or a folder above it.
pub fn beside_or_above(dir: &Path, name: &str) -> Option<PathBuf> {
    dir.ancestors()
        .map(|d| d.join(name))
        .find(|candidate| candidate.is_file())
}

/// [`beside_or_above`] the running executable's folder.
pub fn built(name: &str) -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    beside_or_above(exe.parent()?, name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_binary_is_found_beside_an_executable_or_above_a_test_binary() {
        let target = tempfile::tempdir().unwrap();
        let profile = target.path().join("debug");
        let deps = profile.join("deps");
        let out = profile.join("build/oxplow-app/abc123/out");
        std::fs::create_dir_all(&deps).unwrap();
        std::fs::create_dir_all(&out).unwrap();
        std::fs::write(profile.join("tool"), "").unwrap();
        for dir in [&profile, &deps, &out] {
            assert_eq!(beside_or_above(dir, "tool"), Some(profile.join("tool")));
        }
        assert_eq!(beside_or_above(&out, "missing"), None);
    }
}
