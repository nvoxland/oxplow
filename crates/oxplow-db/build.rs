//! Build inputs the macros can't track themselves.
//!
//! - The migrations dir: `refinery::embed_migrations!` is a proc macro with
//!   no cargo dependency tracking of its own, so without this a
//!   MIGRATION-ONLY change (pure SQL, no `.rs` touched — e.g. V64's dim
//!   promotion) rebuilds nothing: cargo reuses the cached oxplow-db and
//!   every downstream binary silently ships without the new migration.
//! - The core models (`models/`): every file there is embedded into
//!   `$OUT_DIR/core_models.rs` as `(file name, contents)`, so adding a
//!   model is adding a file.
use std::fmt::Write as _;

fn main() {
    println!("cargo:rerun-if-changed=migrations");
    println!("cargo:rerun-if-changed=models");
    let dir = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR")).join("models");
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(&dir)
        .expect("models dir")
        .map(|e| e.expect("models entry").path())
        .filter(|p| p.is_file())
        .collect();
    files.sort();
    let mut out = String::from("/// Every file under `models/`, as `(file name, contents)`.\npub const FILES: &[(&str, &str)] = &[\n");
    for f in files {
        let name = f.file_name().expect("a file has a name").to_string_lossy().into_owned();
        writeln!(
            out,
            "    ({name:?}, include_str!({:?})),",
            f.display().to_string()
        )
        .expect("writing to a String");
    }
    out.push_str("];\n");
    let dest = std::path::Path::new(&std::env::var("OUT_DIR").expect("cargo sets OUT_DIR")).join("core_models.rs");
    std::fs::write(dest, out).expect("write core_models.rs");
}
