//! Adding one entry to a top-level list of a YAML file as a text edit, so
//! the file's comments and formatting stay (`extension add` uses it on
//! `extension.yaml`).

/// Why an entry couldn't be added.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct ListEditError(pub String);

/// `yaml` with `entry_yaml` added at the end of the top-level list `block`
/// (the key is added, at the end of the file, when it is missing).
///
/// `entry_yaml` is one list item as it reads in a file whose lists are
/// indented two spaces: its first line starts `  - `, its comments and
/// continuation lines are indented to match. It is re-indented to the list
/// it joins. A block written as an empty flow list (`block: []`) becomes a
/// block list; a non-empty flow list is refused (rewriting it would lose its
/// formatting): the caller says to rewrite it as a block list.
///
/// Both texts are parsed afterwards: the result must be the original with
/// that one list grown by exactly that entry, or nothing is returned.
pub fn append_list_entry(
    yaml: &str,
    block: &str,
    entry_yaml: &str,
) -> Result<String, ListEditError> {
    let bad = |why: String| ListEditError(why);
    // The entry: one list item, dedented to column 0 for re-indenting.
    let entry_lines: Vec<&str> = entry_yaml.lines().collect();
    let first_item = entry_lines
        .iter()
        .position(|l| l.trim_start().starts_with("- "))
        .ok_or_else(|| bad("the entry is not a list item (`- …`)".into()))?;
    let base = entry_lines[first_item].len() - entry_lines[first_item].trim_start().len();
    let item_starts = entry_lines
        .iter()
        .filter(|l| l.len() - l.trim_start().len() == base && l.trim_start().starts_with("- "))
        .count();
    if item_starts != 1 {
        return Err(bad("the entry must be one entry".into()));
    }
    let entry: Vec<&str> = entry_lines
        .iter()
        .map(|l| match l.get(..base) {
            Some(lead) if lead.trim().is_empty() => &l[base..],
            _ => l.trim_start(),
        })
        .collect();
    let parsed_entry: serde_yaml::Value = serde_yaml::from_str(&format!("{}\n", entry.join("\n")))
        .map_err(|e| bad(format!("the entry doesn't parse: {e}")))?;
    let Some(item) = parsed_entry.as_sequence().and_then(|s| s.first()).cloned() else {
        return Err(bad("the entry is not a list item (`- …`)".into()));
    };

    let original: serde_yaml::Value =
        serde_yaml::from_str(yaml).map_err(|e| bad(format!("the file doesn't parse: {e}")))?;
    let key = serde_yaml::Value::String(block.to_string());
    let current = original.get(&key);
    let lines: Vec<&str> = yaml.split_inclusive('\n').collect();
    let at = lines.iter().position(|l| top_key(l) == Some(block));

    let render = |indent: &str| -> String {
        entry
            .iter()
            .map(|l| {
                if l.is_empty() {
                    "\n".to_string()
                } else {
                    format!("{indent}{l}\n")
                }
            })
            .collect()
    };
    let edited = match (at, current) {
        (None, _) => {
            let mut out = yaml.to_string();
            if !out.is_empty() && !out.ends_with('\n') {
                out.push('\n');
            }
            out.push_str(&format!("{block}:\n{}", render("  ")));
            out
        }
        (Some(at), Some(serde_yaml::Value::Sequence(items))) if lines[at].contains('[') => {
            if !items.is_empty() {
                return Err(bad(format!(
                    "`{block}` is a flow list (`[…]`): rewrite it as a block list (`- …` lines) first"
                )));
            }
            let mut out: String = lines[..at].concat();
            out.push_str(&format!("{block}:\n{}", render("  ")));
            out.push_str(&lines[at + 1..].concat());
            out
        }
        (Some(at), Some(serde_yaml::Value::Sequence(_))) => {
            // The block runs to the next top-level key; the entry goes after
            // its last line that holds content, ahead of blank lines and
            // comments that lead the next key.
            let next = lines[at + 1..]
                .iter()
                .position(|l| top_key(l).is_some())
                .map_or(lines.len(), |i| at + 1 + i);
            let last = (at + 1..next)
                .rev()
                .find(|&i| {
                    let t = lines[i].trim();
                    !t.is_empty() && !t.starts_with('#')
                })
                .unwrap_or(at);
            let indent = lines[at + 1..next]
                .iter()
                .find(|l| l.trim_start().starts_with("- "))
                .map(|l| " ".repeat(l.len() - l.trim_start().len()))
                .unwrap_or_else(|| "  ".to_string());
            let mut out: String = lines[..=last].concat();
            if !out.ends_with('\n') {
                out.push('\n');
            }
            out.push_str(&render(&indent));
            out.push_str(&lines[last + 1..].concat());
            out
        }
        (Some(_), Some(serde_yaml::Value::Null)) => {
            return Err(bad(format!(
                "`{block}` is empty (`{block}:`): remove it or give it entries"
            )))
        }
        (Some(_), _) => return Err(bad(format!("`{block}` is not a list"))),
    };

    // The edit grew that one list by that one entry, and changed nothing else.
    let mut expected = match original {
        serde_yaml::Value::Null => serde_yaml::Value::Mapping(serde_yaml::Mapping::new()),
        other => other,
    };
    let Some(map) = expected.as_mapping_mut() else {
        return Err(bad("the file is not a mapping".into()));
    };
    match map.get_mut(&key) {
        Some(serde_yaml::Value::Sequence(items)) => items.push(item),
        _ => {
            map.insert(key, serde_yaml::Value::Sequence(vec![item]));
        }
    }
    let after: serde_yaml::Value = serde_yaml::from_str(&edited)
        .map_err(|e| bad(format!("the edited file doesn't parse: {e}")))?;
    if after != expected {
        return Err(bad(format!(
            "`{block}` can't be edited in place (its YAML ties entries together, e.g. with anchors); edit the file by hand"
        )));
    }
    Ok(edited)
}

/// The top-level key a line starts, if it does: a `name:` at column 0 (not
/// a comment, list item or flow value).
fn top_key(line: &str) -> Option<&str> {
    let first = line.chars().next()?;
    if first.is_whitespace() || matches!(first, '#' | '-' | '{' | '[' | '\'' | '"') {
        return None;
    }
    let (name, _) = line.split_once(':')?;
    Some(name.trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ENTRY: &str = "  - id: b\n    note: new\n";

    #[test]
    fn appends_to_an_existing_list_and_keeps_comments() {
        let yaml = "name: x # the name\n# about jobs\njobs:\n  # the first\n  - id: a\n    note: old # keep\n\n# about the next key\nother: 1\n";
        assert_eq!(
            append_list_entry(yaml, "jobs", ENTRY).unwrap(),
            "name: x # the name\n# about jobs\njobs:\n  # the first\n  - id: a\n    note: old # keep\n  - id: b\n    note: new\n\n# about the next key\nother: 1\n"
        );
    }

    #[test]
    fn appends_to_the_last_block_of_a_file_without_a_final_newline() {
        let yaml = "jobs:\n  - id: a";
        assert_eq!(
            append_list_entry(yaml, "jobs", ENTRY).unwrap(),
            "jobs:\n  - id: a\n  - id: b\n    note: new\n"
        );
    }

    #[test]
    fn adds_a_missing_block_at_the_end() {
        let yaml = "name: x\n# trailing note\n";
        assert_eq!(
            append_list_entry(yaml, "jobs", ENTRY).unwrap(),
            "name: x\n# trailing note\njobs:\n  - id: b\n    note: new\n"
        );
    }

    #[test]
    fn follows_the_indentation_of_the_list_it_joins() {
        let yaml = "jobs:\n- id: a\n  note: old\nother: 1\n";
        assert_eq!(
            append_list_entry(yaml, "jobs", ENTRY).unwrap(),
            "jobs:\n- id: a\n  note: old\n- id: b\n  note: new\nother: 1\n"
        );
    }

    #[test]
    fn an_empty_flow_list_becomes_a_block_list_and_a_full_one_is_refused() {
        assert_eq!(
            append_list_entry("jobs: []\nother: 1\n", "jobs", ENTRY).unwrap(),
            "jobs:\n  - id: b\n    note: new\nother: 1\n"
        );
        let err = append_list_entry("jobs: [{ id: a }]\n", "jobs", ENTRY).unwrap_err();
        assert!(err.to_string().contains("block list"), "{err}");
    }

    /// A block scalar's last newline is part of its value: the entry is
    /// compared as the file will hold it.
    #[test]
    fn an_entry_with_a_block_scalar_is_added() {
        let entry = "  - id: b\n    query: |\n      SELECT 1\n";
        assert_eq!(
            append_list_entry("name: x\n", "advisories", entry).unwrap(),
            "name: x\nadvisories:\n  - id: b\n    query: |\n      SELECT 1\n"
        );
    }

    #[test]
    fn refuses_what_is_not_a_list_or_not_one_entry() {
        let err = append_list_entry("jobs: 3\n", "jobs", ENTRY).unwrap_err();
        assert!(err.to_string().contains("not a list"), "{err}");
        let two = "  - id: b\n  - id: c\n";
        let err = append_list_entry("jobs:\n  - id: a\n", "jobs", two).unwrap_err();
        assert!(err.to_string().contains("one entry"), "{err}");
    }
}
