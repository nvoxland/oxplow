//! The one rule for a path an extension's manifest names relative to its
//! own folder (a command's `script:`, a component's bundle, a provider's
//! entry).

use std::path::{Component, Path};

/// Whether `rel` is a non-empty relative path made only of normal
/// segments: no leading `/`, no `..`, and no leading `./`. (A `.` inside
/// the path, as in `a/./b`, is dropped by `Path` and so accepted.)
///
/// Lexical only. It does not look at the disk, so it says nothing about
/// symlinks or the canonical location; that containment check stays at
/// file access.
pub fn is_extension_relative(rel: &str) -> bool {
    !rel.is_empty()
        && Path::new(rel)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
}

#[cfg(test)]
mod tests {
    use super::is_extension_relative;

    #[test]
    fn accepts_plain_relative_paths() {
        for ok in [
            "a",
            "scripts/run.star",
            "a/b/c.txt",
            "dir/",
            "a//b",
            "a/./b",
        ] {
            assert!(is_extension_relative(ok), "{ok}");
        }
    }

    #[test]
    fn refuses_empty_absolute_and_escaping_paths() {
        for bad in [
            "",
            "/",
            "/etc/passwd",
            "..",
            "../x",
            "a/../b",
            "a/..",
            ".",
            "./a",
        ] {
            assert!(!is_extension_relative(bad), "{bad}");
        }
    }
}
