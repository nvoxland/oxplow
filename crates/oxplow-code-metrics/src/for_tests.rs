//! The languages oxplow's packs declare, read from their source folders:
//! for tests in this crate and the ones over it (a test has no running
//! publication). Not for production: it reads this checkout's
//! `extensions/` folder.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::spec::{LanguageDecl, LanguageSource, Languages};

fn extensions_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../extensions")
}

/// The `languages:` of one extension folder, with their files read.
pub fn sources_in(
    dir: &Path,
    extension: &str,
    shipped: bool,
) -> Result<Vec<LanguageSource>, String> {
    let manifest = std::fs::read_to_string(dir.join("extension.yaml"))
        .map_err(|e| format!("{}: {e}", dir.display()))?;
    let value: serde_yaml::Value =
        serde_yaml::from_str(&manifest).map_err(|e| format!("{}: {e}", dir.display()))?;
    let Some(list) = value.get("languages") else {
        return Ok(Vec::new());
    };
    let decls: Vec<LanguageDecl> = serde_yaml::from_value(list.clone())
        .map_err(|e| format!("{}: languages: {e}", dir.display()))?;
    let read = |rel: &str| std::fs::read(dir.join(rel)).ok();
    decls
        .into_iter()
        .map(|d| LanguageSource::read(extension, shipped, d, &read))
        .collect()
}

/// Every language the checkout's `extensions/` folders declare (the
/// shipped packs and foundation), built once. Panics on a problem: the
/// packs must build.
pub fn shipped() -> Languages {
    static SET: OnceLock<Languages> = OnceLock::new();
    SET.get_or_init(|| {
        // Every extension folder here that declares languages, by name.
        let mut names: Vec<String> = std::fs::read_dir(extensions_dir())
            .expect("the checkout's extensions folder")
            .filter_map(|e| e.ok())
            .filter(|e| e.path().join("extension.yaml").is_file())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        let mut sources = Vec::new();
        for name in &names {
            let dir = extensions_dir().join(name);
            sources.extend(sources_in(&dir, name, true).unwrap_or_else(|e| panic!("{e}")));
        }
        let (set, problems) = Languages::build(&sources);
        assert!(problems.is_empty(), "{problems:#?}");
        set
    })
    .clone()
}

/// One shipped language by id. Panics when no pack declares it.
pub fn lang(id: &str) -> crate::spec::Language {
    shipped()
        .by_name(id)
        .cloned()
        .unwrap_or_else(|| panic!("no shipped language `{id}`"))
}
