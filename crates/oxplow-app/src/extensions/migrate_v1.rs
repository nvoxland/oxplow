//! The v1 → v2 `extension.yaml` migration (`.context/extensions.md`).
//!
//! Textual on purpose: a YAML round trip through a parser would drop
//! comments and restyle every line, and the bytes of a `collectors`
//! (v1 `sources`) or `advisories` node are what a person's consent
//! covers. So the migration only **inserts lines at the top**, **renames
//! `sources:`** and **moves `slots:` under `ui:`** (its block indented two
//! spaces, each v1 slot name renamed); every other line is byte-identical.
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
/// - `sources:` becomes `collectors:`, and `slots:` becomes `ui:` /
///   `  slots:` with its block (comments and blank lines included)
///   indented under it and each v1 slot name renamed (`RENAMED_SLOTS`).
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
    let mut in_slots = false;
    for (i, line) in lines.iter().enumerate() {
        if i == header_end {
            out.push_str(&inserted);
        }
        let top_level = !line.starts_with([' ', '-', '#']) && !line.trim().is_empty();
        if top_level {
            in_slots = false;
        }
        if let Some(rest) = line.strip_prefix("slots:") {
            out.push_str("ui:\n  slots:");
            out.push_str(rest);
            in_slots = true;
        } else if in_slots && !line.trim().is_empty() {
            out.push_str("  ");
            out.push_str(&rename_slots(line));
        } else {
            out.push_str(&rename_key(line));
        }
    }
    if header_end == lines.len() {
        out.push_str(&inserted);
    }
    out
}

/// Each `slot: <v1 name>` on a line of the slots block (flow or block
/// style, bare or quoted) as its v2 name (`RENAMED_SLOTS`); anything else
/// is left for the loader to report.
fn rename_slots(line: &str) -> String {
    const KEY: &str = "slot:";
    let mut out = String::with_capacity(line.len() + 16);
    let mut rest = line;
    while let Some(at) = rest.find(KEY) {
        let is_key = rest[..at]
            .chars()
            .next_back()
            .is_none_or(|c| matches!(c, ' ' | '{' | ',' | '-'));
        let (head, tail) = rest.split_at(at + KEY.len());
        out.push_str(head);
        rest = tail;
        if !is_key {
            continue;
        }
        let spaces = rest.len() - rest.trim_start_matches(' ').len();
        let (lead, value) = rest.split_at(spaces);
        out.push_str(lead);
        let quote = value.chars().next().filter(|c| matches!(c, '"' | '\''));
        let name_at = quote.map_or(0, char::len_utf8);
        let name_len = value[name_at..]
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
            .unwrap_or(value.len() - name_at);
        let name = &value[name_at..name_at + name_len];
        let after = &value[name_at + name_len..];
        let closed = match quote {
            Some(q) => after.starts_with(q),
            None => after.is_empty() || after.starts_with([' ', ',', '}', '#', '\n', '\r']),
        };
        match super::RENAMED_SLOTS.iter().find(|(old, _)| *old == name) {
            Some((_, new)) if closed && !name.is_empty() => {
                out.push_str(&value[..name_at]);
                out.push_str(new);
                rest = after;
            }
            _ => rest = value,
        }
    }
    out.push_str(rest);
    out
}

/// `sources:` → `collectors:`, top level only.
fn rename_key(line: &str) -> String {
    match line.strip_prefix("sources:") {
        Some(rest) => format!("collectors:{rest}"),
        None => line.to_string(),
    }
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
    fn inserts_the_header_renames_sources_moves_slots_under_ui_and_leaves_the_rest_alone() {
        let out = migrate_v1_to_v2(V1);
        let expected = "manifest: 2\nname: review\ndescription: Review helpers\nsharing: private\nintent:\n  purpose: Review helpers\n  origin: null\n  examples: []\ncollectors:\n  - id: prs\n    runtime: exec\n    entry: sync.sh\n    entities: []\n# the mounts\nui:\n  slots:\n    - { slot: rail, lens: waiting }\nadvisories:\n  - id: a\n    on: prompt\n    query: SELECT 'x' AS message\n";
        assert_eq!(out, expected);
        let parsed: ManifestV2 = serde_yaml::from_str(&out).unwrap();
        assert_eq!(parsed.manifest, 2);
        assert_eq!(parsed.intent.unwrap().purpose, "Review helpers");
        assert!(parsed.collectors.is_some());
        assert_eq!(parsed.ui.slots.len(), 1);
        assert_eq!(parsed.advisories.len(), 1);
    }

    /// A slots block's comments and blank lines move with it; a
    /// column-0 sequence is indented too; the next key ends it; v1 slot
    /// names become their v2 names.
    #[test]
    fn the_slots_block_moves_whole() {
        let out = migrate_v1_to_v2(
            "name: x\nslots:\n# review\n- { slot: commit, lens: a }\n\n- { slot: thread, lens: b }\nadvisories: []\n",
        );
        assert!(
            out.ends_with(
                "ui:\n  slots:\n  # review\n  - { slot: vcs.commit.details, lens: a }\n\n  - { slot: thread.plan.header, lens: b }\nadvisories: []\n"
            ),
            "{out}"
        );
        let parsed: ManifestV2 = serde_yaml::from_str(&out).unwrap();
        assert_eq!(parsed.ui.slots.len(), 2);
    }

    /// Every v1 slot name is renamed, in a flow or block entry, bare or
    /// quoted; a name that isn't one (or a `slot:` outside the block)
    /// is left for the loader to report.
    #[test]
    fn v1_slot_names_become_their_v2_names() {
        let out = migrate_v1_to_v2(
            "name: x\nslots:\n  - slot: task-detail\n    lens: a\n  - { slot: \"uncommitted\", lens: b }\n  - { lens: c, slot: 'effort-review' }\n  - { slot: settings }\n  - { slot: rail, lens: d }\n  - { slot: commitx, lens: e }\nnote: { slot: commit }\n",
        );
        assert!(
            out.ends_with(
                "ui:\n  slots:\n    - slot: work_item.detail.body\n      lens: a\n    - { slot: \"vcs.status.details\", lens: b }\n    - { lens: c, slot: 'effort.review.details' }\n    - { slot: settings.section }\n    - { slot: rail, lens: d }\n    - { slot: commitx, lens: e }\nnote: { slot: commit }\n"
            ),
            "{out}"
        );
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
        assert!(out.ends_with("ui:\n  slots: []\n"), "{out}");
        let parsed: ManifestV2 = serde_yaml::from_str(&out).unwrap();
        assert_eq!(parsed.intent.unwrap().purpose, "Two lines: yes really");
    }
}
