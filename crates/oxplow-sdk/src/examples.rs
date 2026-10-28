//! The example extensions oxplow ships (`examples/extensions/<name>/`):
//! compiled into a release build, read from the folder by a debug build —
//! the way `oxplow-app`'s bundled extensions are. `oxplow extension new
//! --from <example>` copies one into a project.

/// One example: its name (its folder's) and its files as
/// `(path inside the folder, bytes)`.
pub struct Example {
    pub name: &'static str,
    pub files: &'static [(&'static str, &'static [u8])],
}

#[cfg(not(debug_assertions))]
mod generated {
    include!(concat!(env!("OUT_DIR"), "/example_files.rs"));
}

#[cfg(debug_assertions)]
mod generated {
    use super::Example;
    use std::sync::OnceLock;

    /// Every example, read from `examples/extensions/` the first time they
    /// are asked for and kept for the life of the process.
    pub fn all() -> &'static [Example] {
        static ALL: OnceLock<&'static [Example]> = OnceLock::new();
        ALL.get_or_init(|| {
            let dir =
                std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/extensions");
            let mut names: Vec<String> = std::fs::read_dir(&dir)
                .unwrap_or_else(|e| panic!("reading {}: {e}", dir.display()))
                .flatten()
                .filter(|e| e.path().join("extension.yaml").is_file())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            let examples: Vec<Example> = names
                .into_iter()
                .map(|name| {
                    let files = read(&dir.join(&name), "");
                    Example {
                        name: Box::leak(name.into_boxed_str()),
                        files: Box::leak(files.into_boxed_slice()),
                    }
                })
                .collect();
            Box::leak(examples.into_boxed_slice())
        })
    }

    fn read(dir: &std::path::Path, prefix: &str) -> Vec<(&'static str, &'static [u8])> {
        let mut out = Vec::new();
        let Ok(entries) = std::fs::read_dir(dir) else {
            return out;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let rel = if prefix.is_empty() {
                name
            } else {
                format!("{prefix}/{name}")
            };
            let path = entry.path();
            if path.is_dir() {
                out.extend(read(&path, &rel));
            } else if let Ok(bytes) = std::fs::read(&path) {
                out.push((
                    &*Box::leak(rel.into_boxed_str()),
                    &*Box::leak(bytes.into_boxed_slice()),
                ));
            }
        }
        out.sort();
        out
    }
}

/// Every example, by name.
pub fn all() -> &'static [Example] {
    #[cfg(debug_assertions)]
    {
        generated::all()
    }
    #[cfg(not(debug_assertions))]
    {
        generated::ALL
    }
}

/// The example called `name`.
pub fn find(name: &str) -> Option<&'static Example> {
    all().iter().find(|e| e.name == name)
}

/// The one-line `description:` of an example's manifest ("" if it has none).
pub fn describe(example: &Example) -> String {
    example
        .files
        .iter()
        .find(|(rel, _)| *rel == "extension.yaml")
        .and_then(|(_, bytes)| serde_yaml::from_slice::<serde_yaml::Value>(bytes).ok())
        .and_then(|manifest| manifest.get("description")?.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// The names as a sentence fragment: "`a`, `b` or `c`".
pub fn names() -> String {
    let names: Vec<String> = all().iter().map(|e| format!("`{}`", e.name)).collect();
    match names.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("{} or {last}", rest.join(", ")),
        _ => names.join(""),
    }
}
