//! The v1 → v2 `extension.yaml` migration (`.context/extensions.md`).
//!
//! Textual on purpose: a YAML round trip through a parser would drop
//! comments and restyle every line, and the bytes of a `collectors`
//! (v1 `sources`) or `advisories` node are what a person's consent
//! covers. So the migration only **inserts lines at the top** and
//! **renames two top-level keys**; everything below is byte-identical.
//! The loader runs it in memory on a v1 manifest (then reads the result
//! as v2), and `oxplow plugin migrate` writes it to the file — one
//! conversion, two callers.

use super::manifest_v2::key_line;

/// A v1 manifest as a v2 one. Idempotent: text that already has a
/// `manifest:` key comes back unchanged.
///
/// - `manifest: 2` is prepended;
/// - `sharing: private` and an `intent` skeleton (its `purpose` from
///   `description`) are inserted after the header (`name`,
///   `description`); the agent fills in `origin` and `examples`;
/// - `sources:` becomes `collectors:` and `slots:` becomes `slot_mounts:`.
pub fn migrate_v1_to_v2(text: &str) -> String {
    if key_line(text, "manifest").is_some() {
        return text.to_string();
    }
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    // The header ends after the last of `name:` / `description:` — but a
    // multi-line description (`description: >-` or `|`) continues on
    // indented lines, which stay with it.
    let header_end = {
        let mut end = 0;
        for (i, line) in lines.iter().enumerate() {
            let top_level =
                !line.starts_with(' ') && !line.starts_with('#') && !line.trim().is_empty();
            if top_level {
                if line.starts_with("name:") || line.starts_with("description:") {
                    end = i + 1;
                } else if end > 0 {
                    break;
                }
            } else if end > 0 && end == i {
                // An indented continuation of the header key just above.
                end = i + 1;
            }
        }
        end
    };
    let description = description_of(text);
    let purpose = yaml_scalar(match &description {
        Some(d) if !d.trim().is_empty() => d.trim(),
        _ => "TODO: what this extension is for (one sentence)",
    });
    let inserted = format!(
        "sharing: private\nintent:\n  purpose: {purpose}\n  origin: null\n  examples: []\n"
    );
    let mut out = String::with_capacity(text.len() + 96);
    out.push_str("manifest: 2\n");
    for (i, line) in lines.iter().enumerate() {
        if i == header_end {
            out.push_str(&inserted);
        }
        out.push_str(&rename_key(line));
    }
    if header_end == lines.len() {
        out.push_str(&inserted);
    }
    out
}

/// `sources:` → `collectors:` and `slots:` → `slot_mounts:`, top level only.
fn rename_key(line: &str) -> String {
    for (from, to) in [("sources:", "collectors:"), ("slots:", "slot_mounts:")] {
        if let Some(rest) = line.strip_prefix(from) {
            return format!("{to}{rest}");
        }
    }
    line.to_string()
}

/// The manifest's `description`, read by a parser (a scalar in any style).
fn description_of(text: &str) -> Option<String> {
    #[derive(serde::Deserialize)]
    struct Header {
        #[serde(default)]
        description: Option<String>,
    }
    serde_yaml::from_str::<Header>(text)
        .ok()
        .and_then(|h| h.description)
}

/// A string as a single-line YAML scalar (double-quoted when it needs it).
fn yaml_scalar(s: &str) -> String {
    let plain_ok = !s.contains([
        '\n', ':', '#', '"', '\'', '{', '}', '[', ']', '&', '*', '!', '|', '>', '%', '@', '`',
    ]) && !s.starts_with(['-', '?'])
        && s.trim() == s;
    if plain_ok {
        s.to_string()
    } else {
        format!(
            "\"{}\"",
            s.replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('\n', " ")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extensions::manifest_v2::ManifestV2;

    const V1: &str = "name: review\ndescription: Review helpers\nsources:\n  - id: prs\n    runtime: exec\n    entry: sync.sh\n    entities: []\n# the mounts\nslots:\n  - { slot: rail, lens: waiting }\nadvisories:\n  - id: a\n    on: prompt\n    query: SELECT 'x' AS message\n";

    #[test]
    fn inserts_the_header_renames_two_keys_and_leaves_the_body_bytes_alone() {
        let out = migrate_v1_to_v2(V1);
        let expected = "manifest: 2\nname: review\ndescription: Review helpers\nsharing: private\nintent:\n  purpose: Review helpers\n  origin: null\n  examples: []\ncollectors:\n  - id: prs\n    runtime: exec\n    entry: sync.sh\n    entities: []\n# the mounts\nslot_mounts:\n  - { slot: rail, lens: waiting }\nadvisories:\n  - id: a\n    on: prompt\n    query: SELECT 'x' AS message\n";
        assert_eq!(out, expected);
        // Below the inserted block, only the two key lines differ.
        let body_v1: Vec<&str> = V1.lines().skip(2).collect();
        let body_v2: Vec<&str> = out.lines().skip(8).collect();
        assert_eq!(body_v1.len(), body_v2.len());
        for (a, b) in body_v1.iter().zip(&body_v2) {
            if *a == "sources:" {
                assert_eq!(*b, "collectors:");
            } else if *a == "slots:" {
                assert_eq!(*b, "slot_mounts:");
            } else {
                assert_eq!(a, b);
            }
        }
        let parsed: ManifestV2 = serde_yaml::from_str(&out).unwrap();
        assert_eq!(parsed.manifest, 2);
        assert_eq!(parsed.intent.unwrap().purpose, "Review helpers");
        assert!(parsed.collectors.is_some());
        assert_eq!(parsed.slot_mounts.len(), 1);
        assert_eq!(parsed.advisories.len(), 1);
    }

    #[test]
    fn is_idempotent() {
        let once = migrate_v1_to_v2(V1);
        assert_eq!(migrate_v1_to_v2(&once), once);
    }

    #[test]
    fn handles_a_bare_name_a_multiline_description_and_awkward_text() {
        let out = migrate_v1_to_v2("name: x\n");
        assert_eq!(
            out,
            "manifest: 2\nname: x\nsharing: private\nintent:\n  purpose: \"TODO: what this extension is for (one sentence)\"\n  origin: null\n  examples: []\n"
        );
        let _: ManifestV2 = serde_yaml::from_str(&out).unwrap();
        let out =
            migrate_v1_to_v2("name: x\ndescription: >-\n  Two lines: yes\n  really\nslots: []\n");
        assert!(out.contains("  really\nsharing: private\n"), "{out}");
        assert!(
            out.contains("  purpose: \"Two lines: yes really\"\n"),
            "{out}"
        );
        assert!(out.ends_with("slot_mounts: []\n"), "{out}");
        let parsed: ManifestV2 = serde_yaml::from_str(&out).unwrap();
        assert_eq!(parsed.intent.unwrap().purpose, "Two lines: yes really");
    }
}
