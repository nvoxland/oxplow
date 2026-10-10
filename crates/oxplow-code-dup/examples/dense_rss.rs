//! Peak memory of a duplicate detection over a clone-dense tree, shaped as
//! the `duplicate_lines` collector reads it: every block becomes a JSON
//! object, as the `duplicate_blocks` builtin builds them.
//!
//! ```sh
//! cargo build --release -p oxplow-code-dup --example dense_rss
//! /usr/bin/time -l target/release/examples/dense_rss 12 60   # macOS; -v on Linux
//! ```
//!
//! Arguments: directories, then files per directory; each file holds the
//! same function (`.context/performance.md` "Duplicate detection on
//! clone-dense trees").

use oxplow_code_dup::{detect_duplicates, DupOptions};

const BODY: &str = r#"
fn helper(items: Vec<i32>) -> Vec<i32> {
    let mut out = Vec::new();
    for item in items {
        if item > 0 {
            out.push(item * 2);
        } else if item < 0 {
            out.push(item * -1);
        } else {
            out.push(0);
        }
    }
    out
}
"#;

fn main() {
    let arg = |i: usize, default: usize| {
        std::env::args()
            .nth(i)
            .and_then(|a| a.parse().ok())
            .unwrap_or(default)
    };
    let (dirs, per_dir) = (arg(1, 12), arg(2, 60));
    let files: Vec<(String, String)> = (0..dirs)
        .flat_map(|d| {
            (0..per_dir).map(move |f| (format!("src/d{d:02}/m{f:03}.rs"), BODY.to_string()))
        })
        .collect();
    let started = std::time::Instant::now();
    let blocks = detect_duplicates(files, DupOptions::default());
    let values: Vec<serde_json::Value> = blocks
        .iter()
        .map(|b| {
            serde_json::json!({
                "a_path": b.a_path, "a_start_line": b.a_start_line, "a_end_line": b.a_end_line,
                "b_path": b.b_path, "b_start_line": b.b_start_line, "b_end_line": b.b_end_line,
                "line_count": b.line_count,
            })
        })
        .collect();
    println!(
        "{} files, {} blocks, {} values, {:?}",
        dirs * per_dir,
        blocks.len(),
        values.len(),
        started.elapsed()
    );
}
