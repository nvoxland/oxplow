use crate::{diff_edges, extract_imports};

#[test]
fn unsupported_extension_yields_empty() {
    assert!(extract_imports(
        &oxplow_code_metrics::for_tests::shipped(),
        "a.md",
        "import foo from 'bar';"
    )
    .is_empty());
}

#[test]
fn diff_added_and_removed() {
    let before = r#"
use std::fs;
use std::io;
"#;
    let after = r#"
use std::fs;
use std::path::Path;
"#;
    let b = extract_imports(&oxplow_code_metrics::for_tests::shipped(), "a.rs", before);
    let a = extract_imports(&oxplow_code_metrics::for_tests::shipped(), "a.rs", after);
    let (added, removed) = diff_edges(&b, &a);
    let added_mods: Vec<&str> = added.iter().map(|e| e.module.as_str()).collect();
    let removed_mods: Vec<&str> = removed.iter().map(|e| e.module.as_str()).collect();
    assert_eq!(added_mods, vec!["std::path::Path"]);
    assert_eq!(removed_mods, vec!["std::io"]);
}
