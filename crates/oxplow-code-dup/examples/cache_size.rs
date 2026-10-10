//! How much a directory's source files take of the duplicate cache's
//! budget: `cargo run --release -p oxplow-code-dup --example cache_size -- <dir>`
//! (`.context/performance.md` "Duplicate detection cache").

use std::path::Path;

use oxplow_code_dup::{
    detect_duplicates_scoped_cached, CachedFile, DupCache, DupOptions, FileKey, CACHE_BUDGET_BYTES,
    RECORD_BYTES,
};

fn walk(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') || name == "target" || name == "node_modules" {
            continue;
        }
        if path.is_dir() {
            walk(&path, out);
        } else if oxplow_code_metrics::is_supported_path(&path) {
            out.push(path);
        }
    }
}

fn main() {
    let root = std::env::args().nth(1).unwrap_or_else(|| ".".into());
    let mut paths = Vec::new();
    walk(Path::new(&root), &mut paths);
    let files: Vec<CachedFile> = paths
        .iter()
        .map(|p| {
            let path = p.to_string_lossy().to_string();
            let read = p.clone();
            CachedFile {
                key: Some(FileKey {
                    space: "disk".into(),
                    id: path.clone(),
                }),
                path,
                text: Box::new(move || std::fs::read_to_string(read).ok()),
            }
        })
        .collect();
    let cache = DupCache::with_budget(usize::MAX / 2);
    let started = std::time::Instant::now();
    let (_, stats) = detect_duplicates_scoped_cached(
        files,
        &std::collections::BTreeSet::new(),
        DupOptions::default(),
        &cache,
    );
    let (records, bytes) = cache.held();
    println!(
        "{} files, {records} records ({:.1} per file), {:.1} MB of {:.0} MB budget ({RECORD_BYTES} B each), cold scan {:?}, {} misses",
        paths.len(),
        records as f64 / paths.len().max(1) as f64,
        bytes as f64 / 1048576.0,
        CACHE_BUDGET_BYTES as f64 / 1048576.0,
        started.elapsed(),
        stats.misses
    );
}
