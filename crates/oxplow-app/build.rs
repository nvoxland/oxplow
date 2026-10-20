//! oxplow's own extensions, compiled in (`src/bundled_extensions.rs`).
//!
//! Each one's files are whatever its folder `extensions/<name>/` holds —
//! dot entries and the folders its `build.exclude` names left out — written
//! to `$OUT_DIR/bundled_files.rs` as `include_bytes!` arrays, so adding a
//! file is adding it to the folder. One whose manifest declares `build:`
//! (`run`, `output`) is built here first, so its output ships in the binary
//! without being committed: building oxplow runs its command (`/bin/sh
//! -c`, in its folder — `bun` for the Timeline).
//!
//! Cargo reruns this when an input changes: the manifest, every input file,
//! and the input folders — but only folders that hold nothing skipped. Cargo
//! checks a folder it is given recursively, so a folder holding the build's
//! output (or `node_modules`, or a dot entry a tool rewrites) would make
//! every cargo command find this crate dirty again after the build step
//! writes there. Such a folder is watched through its files and its clean
//! subfolders instead. A new top-level file (or one in a folder like that)
//! is therefore noticed only once something else changes: touch the
//! extension's `extension.yaml`.
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// The extensions that ship with oxplow, by folder name.
const BUNDLED: &[&str] = &["oxplow-foundation", "oxplow-bundled"];

fn main() {
    let manifest_dir =
        PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR"));
    let extensions = manifest_dir.join("../../extensions");
    let mut out = String::new();
    for name in BUNDLED {
        let dir = extensions.join(name);
        let manifest = dir.join("extension.yaml");
        let text = std::fs::read_to_string(&manifest)
            .unwrap_or_else(|e| panic!("{}: {e}", manifest.display()));
        let build = Build::of(&text);
        let mut skip: Vec<String> = build.exclude.clone();
        if let Some(output) = &build.output {
            skip.push(output.clone());
        }
        // The inputs, tracked; then the build, which writes the output.
        let inputs = walk(&dir, &skip);
        println!("cargo:rerun-if-changed={}", manifest.display());
        for (_, path) in &inputs {
            println!("cargo:rerun-if-changed={}", path.display());
        }
        for folder in folders(&dir, &skip) {
            println!("cargo:rerun-if-changed={}", folder.display());
        }
        if let Some(run) = &build.run {
            let status = std::process::Command::new("/bin/sh")
                .arg("-c")
                .arg(run)
                .current_dir(&dir)
                .status()
                .unwrap_or_else(|e| {
                    panic!("building extensions/{name}: couldn't run `{run}`: {e}")
                });
            if !status.success() {
                panic!(
                    "building extensions/{name}: `{run}` failed ({status}); building oxplow \
                     builds its own extensions, which needs the tools their build uses (bun)"
                );
            }
        }
        // Every file it ships: the inputs and what the build wrote.
        let shipped = walk(&dir, &build.exclude);
        let ident = name.to_uppercase().replace('-', "_");
        writeln!(
            out,
            "/// `extensions/{name}/`'s files, as `(path inside it, bytes)`.\npub const {ident}: &[(&str, &[u8])] = &["
        )
        .expect("writing to a String");
        for (rel, path) in shipped {
            writeln!(
                out,
                "    ({rel:?}, include_bytes!({:?}) as &[u8]),",
                path.display().to_string()
            )
            .expect("writing to a String");
        }
        out.push_str("];\n");
    }
    let dest = PathBuf::from(std::env::var("OUT_DIR").expect("cargo sets OUT_DIR"))
        .join("bundled_files.rs");
    std::fs::write(dest, out).expect("write bundled_files.rs");
}

/// A manifest's `build:`, as far as packaging cares.
#[derive(Default)]
struct Build {
    run: Option<String>,
    output: Option<String>,
    exclude: Vec<String>,
}

impl Build {
    fn of(text: &str) -> Build {
        let Ok(manifest) = serde_yaml::from_str::<serde_yaml::Value>(text) else {
            return Build::default();
        };
        let Some(build) = manifest.get("build") else {
            return Build::default();
        };
        let field = |key: &str| build.get(key).and_then(|v| v.as_str()).map(str::to_string);
        Build {
            run: field("run"),
            output: field("output").map(|o| o.trim_end_matches('/').to_string()),
            exclude: build
                .get("exclude")
                .and_then(|v| v.as_sequence())
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|v| v.as_str().map(|s| s.trim_end_matches('/').to_string()))
                        .collect()
                })
                .unwrap_or_default(),
        }
    }
}

/// Every file under `dir` as `(relative path, absolute path)`, sorted,
/// leaving out dot entries and the `skip` folders (relative paths).
fn walk(dir: &Path, skip: &[String]) -> Vec<(String, PathBuf)> {
    fn go(dir: &Path, prefix: &str, skip: &[String], out: &mut Vec<(String, PathBuf)>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
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
            if skip.contains(&rel) {
                continue;
            }
            let path = entry.path();
            if path.is_dir() {
                go(&path, &rel, skip, out);
            } else {
                out.push((rel, path));
            }
        }
    }
    let mut out = Vec::new();
    go(dir, "", skip, &mut out);
    out.sort();
    out
}

/// The folders to watch for a file added or removed: the topmost folders
/// under `dir` that hold nothing skipped (no dot entry, no `skip` path, at
/// any depth), since cargo checks a watched folder recursively. A folder
/// that does hold one is not watched itself; its subfolders are looked at
/// in turn, and its own files are watched one by one (`walk`).
fn folders(dir: &Path, skip: &[String]) -> Vec<PathBuf> {
    fn holds_skipped(dir: &Path, rel: &str, skip: &[String]) -> bool {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return false;
        };
        entries.flatten().any(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let child = if rel.is_empty() {
                name.clone()
            } else {
                format!("{rel}/{name}")
            };
            name.starts_with('.')
                || skip.contains(&child)
                || (entry.path().is_dir() && holds_skipped(&entry.path(), &child, skip))
        })
    }
    fn go(dir: &Path, rel: &str, skip: &[String], out: &mut Vec<PathBuf>) {
        if !holds_skipped(dir, rel, skip) {
            out.push(dir.to_path_buf());
            return;
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let child = if rel.is_empty() {
                name.clone()
            } else {
                format!("{rel}/{name}")
            };
            if entry.path().is_dir() && !name.starts_with('.') && !skip.contains(&child) {
                go(&entry.path(), &child, skip, out);
            }
        }
    }
    let mut out = Vec::new();
    go(dir, "", skip, &mut out);
    out
}
