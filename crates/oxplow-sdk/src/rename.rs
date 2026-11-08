//! `extension new --from <example> <name>`: copy an example under another
//! name. Its own references — dataset views `v_<ns>_…`, pages and other
//! kinds `<ns>/…`, commands, event types and view nodes `<ns>.…`, found by
//! the one rule (`oxplow_domain::contribution::own_references`) — and the
//! manifest's `name:` are rewritten; every other word is left alone.

use oxplow_domain::contribution::{extension_namespace, own_references, RefPart};

/// What a rename turns one extension's names into.
pub struct Rename {
    from_name: String,
    from_ns: String,
    to_name: String,
    to_ns: String,
}

impl Rename {
    /// `from` and `to` are extension names. A manifest that sets
    /// `namespace:` keeps it as written (`namespace_override`), so its
    /// references stay as they are then.
    pub fn new(from: &str, to: &str, namespace_override: Option<&str>) -> Rename {
        let from_ns = namespace_override
            .map(str::to_string)
            .unwrap_or_else(|| extension_namespace(from));
        let to_ns = namespace_override
            .map(str::to_string)
            .unwrap_or_else(|| extension_namespace(to));
        Rename {
            from_name: from.to_string(),
            from_ns,
            to_name: to.to_string(),
            to_ns,
        }
    }

    /// `text` with each own reference naming the new extension, the value
    /// of an `extension:` key (a page's param, a fixture's run) that is the
    /// old name, and of a `namespace:` key (a fixture's health row) that is
    /// the old namespace.
    pub fn text(&self, text: &str) -> String {
        let mut spans: Vec<(std::ops::Range<usize>, String)> = own_references(text, &self.from_ns)
            .into_iter()
            .map(|r| {
                let to = match r.part {
                    RefPart::Dataset => format!("v_{}_", self.to_ns),
                    RefPart::Page => format!("{}/", self.to_ns),
                    RefPart::Command => format!("{}.", self.to_ns),
                };
                (r.prefix, to)
            })
            .collect();
        spans.extend(key_values(
            text,
            "extension",
            &self.from_name,
            &self.to_name,
        ));
        spans.extend(key_values(text, "namespace", &self.from_ns, &self.to_ns));
        spans.sort_by_key(|(r, _)| r.start);
        let mut out = String::with_capacity(text.len());
        let mut at = 0;
        for (range, to) in spans {
            if range.start < at {
                continue;
            }
            out.push_str(&text[at..range.start]);
            out.push_str(&to);
            at = range.end;
        }
        out.push_str(&text[at..]);
        out
    }

    /// `extension.yaml` with its `name:` too.
    pub fn manifest(&self, text: &str) -> String {
        let renamed = self.text(text);
        let mut out = String::with_capacity(renamed.len());
        let mut done = false;
        for line in renamed.split_inclusive('\n') {
            let value = line.strip_prefix("name:").map(|v| v.trim());
            if !done && value.is_some_and(|v| v.trim_matches(['"', '\'']) == self.from_name) {
                out.push_str(&format!("name: {}\n", self.to_name));
                done = true;
            } else {
                out.push_str(line);
            }
        }
        out
    }
}

/// The values of YAML key `key` (block or flow style) that are `from`, each
/// with `to` in its place.
fn key_values(
    text: &str,
    key: &str,
    from: &str,
    to: &str,
) -> Vec<(std::ops::Range<usize>, String)> {
    let mut spans = Vec::new();
    if from == to {
        return spans;
    }
    for (at, _) in text.match_indices(key) {
        let before = text[..at].chars().next_back();
        if before.is_some_and(|c| c.is_alphanumeric() || "_-".contains(c)) {
            continue;
        }
        let Some(rest) = text[at + key.len()..].strip_prefix(':') else {
            continue;
        };
        let spaces = rest.len() - rest.trim_start_matches(' ').len();
        let value_at = text.len() - rest.len() + spaces;
        let quote = text[value_at..].starts_with('"');
        let start = value_at + usize::from(quote);
        if !text[start..].starts_with(from) {
            continue;
        }
        let end = start + from.len();
        let after = text[end..].chars().next();
        if after.is_some_and(|c| c.is_alphanumeric() || "_-./".contains(c)) {
            continue;
        }
        spans.push((start..end, to.to_string()));
    }
    spans
}

/// The `namespace:` a manifest sets, if it does (a top-level key).
pub fn namespace_override(manifest: &str) -> Option<String> {
    manifest.lines().find_map(|line| {
        let v = line.strip_prefix("namespace:")?.trim();
        let v = v.split(" #").next().unwrap_or(v).trim_matches(['"', '\'']);
        (!v.is_empty()).then(|| v.to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrites_own_references_and_leaves_other_words() {
        let r = Rename::new("triage", "my-triage", None);
        let text = "SELECT * FROM v_triage_x; page triage/summarize; run triage.todos.decide; wiki slug triage-summary; other-triage/x; end of triage.\n";
        assert_eq!(
            r.text(text),
            "SELECT * FROM v_my_triage_x; page my_triage/summarize; run my_triage.todos.decide; wiki slug triage-summary; other-triage/x; end of triage.\n"
        );
    }

    #[test]
    fn the_manifest_name_is_renamed_and_a_namespace_override_stays() {
        let manifest = "manifest: 2\nname: triage\nnamespace: tri\ndescription: name: triage\n";
        let ns = namespace_override(manifest);
        assert_eq!(ns.as_deref(), Some("tri"));
        let r = Rename::new("triage", "mine", ns.as_deref());
        assert_eq!(
            r.manifest(manifest),
            "manifest: 2\nname: mine\nnamespace: tri\ndescription: name: triage\n"
        );
        assert_eq!(r.text("tri.a.b tri/p v_tri_x"), "tri.a.b tri/p v_tri_x");
    }

    #[test]
    fn an_extension_key_with_the_old_name_as_its_value_is_renamed() {
        let r = Rename::new("triage", "mine", None);
        assert_eq!(
            r.text("{ extension: triage, other: triage }\nextension: \"triage\"\nextension: triage-x\n"),
            "{ extension: mine, other: triage }\nextension: \"mine\"\nextension: triage-x\n"
        );
        // Ids are the namespace's: the dashed name names nothing but itself.
        let r = Rename::new("task-views", "mine", None);
        assert_eq!(
            r.text("task-views.board task_views.a task_views/b namespace: task_views"),
            "task-views.board mine.a mine/b namespace: mine"
        );
    }
}
