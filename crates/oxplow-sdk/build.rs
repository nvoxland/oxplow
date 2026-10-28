//! The example extensions (`examples/extensions/<name>/`), compiled into a
//! release build so `oxplow extension new --from <example>` works from an
//! installed app (`src/examples.rs`). A debug build reads the folders at run
//! time instead, so this script does nothing there and editing an example
//! never recompiles this crate.
//!
//! Each example's files are whatever its folder holds, dot entries left out,
//! written to `$OUT_DIR/example_files.rs` as `include_bytes!` arrays.
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let manifest_dir =
        PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR"));
    let examples = manifest_dir.join("../../examples/extensions");
    let dest = PathBuf::from(std::env::var("OUT_DIR").expect("cargo sets OUT_DIR"))
        .join("example_files.rs");
    if std::env::var_os("CARGO_CFG_DEBUG_ASSERTIONS").is_some() {
        std::fs::write(dest, "").expect("write example_files.rs");
        return;
    }
    // Cargo checks a watched folder recursively: a new file or example is seen.
    println!("cargo:rerun-if-changed={}", examples.display());
    let mut names: Vec<String> = std::fs::read_dir(&examples)
        .unwrap_or_else(|e| panic!("{}: {e}", examples.display()))
        .flatten()
        .filter(|e| e.path().join("extension.yaml").is_file())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    let mut out = String::from("pub static ALL: &[super::Example] = &[\n");
    for name in &names {
        writeln!(out, "    super::Example {{ name: {name:?}, files: &[").expect("string write");
        for (rel, path) in walk(&examples.join(name)) {
            writeln!(
                out,
                "        ({rel:?}, include_bytes!({:?}) as &[u8]),",
                path.display().to_string()
            )
            .expect("string write");
        }
        out.push_str("    ] },\n");
    }
    out.push_str("];\n");
    std::fs::write(dest, out).expect("write example_files.rs");
}

/// Every file under `dir` as `(relative path, absolute path)`, sorted, dot
/// entries left out.
fn walk(dir: &Path) -> Vec<(String, PathBuf)> {
    fn go(dir: &Path, prefix: &str, out: &mut Vec<(String, PathBuf)>) {
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
            let path = entry.path();
            if path.is_dir() {
                go(&path, &rel, out);
            } else {
                out.push((rel, path));
            }
        }
    }
    let mut out = Vec::new();
    go(dir, "", &mut out);
    out.sort();
    out
}
