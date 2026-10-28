//! Keys an `extension.yaml` no longer has, and what took their place: the
//! whole migration story for a private extension. A manifest with one is
//! refused like any unknown key, and the refusal names the new spelling
//! (`hint_unknown`), so `oxplow extension check` says what to write.

/// An old key and how to write what it said now.
pub const RENAMED: &[(&str, &str)] = &[
    (
        "slots",
        "`ui.mounts` places a page — `ui.slots: [{ slot: X, page: P, at: POS }]` is \
         `ui.mounts: [{ page: P, at: X, position: POS }]`",
    ),
    (
        "replacements",
        "`ui.mounts` places a page — `ui.replacements: [{ target: T, page: P, version: V }]` \
         is `ui.mounts: [{ page: P, at: T, version: V }]`",
    ),
    (
        "permissions",
        "`permissions:` is a list of scopes, in a script's `needs:` grammar — `{ sql: read, files: \
         read, ai: true, process: true, agent: true, network: [h], secrets: [S], commands: [p] }` \
         is `[sql.read, files.read, ai.compute, process.run, agent.write, network:h, secrets:S, \
         commands:p]`",
    ),
    (
        "models",
        "write `datasets:` — `models: [{ name, columns: [..] }]` is \
         `datasets: [{ name, fields: [..] }]`, each one's SQL still in `models/<name>.sql`",
    ),
    (
        "columns",
        "write `fields:` — a dataset's `columns: [{ name, type, doc }]` is \
         `fields: [{ name, type, doc }]`",
    ),
    (
    "trigger",
    "write `on:` on the entry itself — `trigger: { on: [x], where: {…}, settle: 30s }` is \
     `on: [x]` with `where:` and `settle:` beside it, `trigger: { every: 15m }` is \
     `on: { every: 15m }`, `trigger: { on_run: test }` is `on: { run: test }`, \
     `trigger: manual` is `on: manual`",
    ),
];

/// `message` (a parser's) with the new spelling added when it refuses a
/// key in [`RENAMED`] — as unknown (``unknown field `trigger`, expected
/// …`` becomes ``unknown key `trigger` — write `on:` …``) or as the map it
/// used to be (``permissions: invalid type: map, expected a sequence``).
/// Any other message is kept.
pub fn hint_unknown(message: &str) -> String {
    for (old, hint) in RENAMED {
        for needle in [
            format!("unknown field `{old}`"),
            format!("{old}: invalid type: map"),
        ] {
            if let Some(at) = message.find(&needle) {
                let rest = &message[at + needle.len()..];
                let tail = rest.find(" at line ").map_or("", |i| &rest[i..]);
                return format!("{}unknown key `{old}` — {hint}{tail}", &message[..at]);
            }
        }
    }
    message.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An old key's refusal says what to write instead, keeping where it
    /// is; any other message passes through.
    #[test]
    fn an_old_key_is_unknown_with_its_new_spelling() {
        let hinted = hint_unknown(
            "collectors[0]: unknown field `trigger`, expected one of `id`, `on` at line 4 column 5",
        );
        assert!(
            hinted.starts_with("collectors[0]: unknown key `trigger` — write `on:`"),
            "{hinted}"
        );
        assert!(hinted.ends_with(" at line 4 column 5"), "{hinted}");
        assert!(!hinted.contains("expected one of"), "{hinted}");
        let old_map =
            hint_unknown("permissions: invalid type: map, expected a sequence at line 3 column 14");
        assert!(
            old_map.starts_with("unknown key `permissions` — `permissions:` is a list of scopes"),
            "{old_map}"
        );
        let dataset = hint_unknown(
            "datasets: datasets[0]: unknown field `columns`, expected one of `name`, `fields` at line 7 column 5",
        );
        assert!(
            dataset.starts_with("datasets: datasets[0]: unknown key `columns` — write `fields:`"),
            "{dataset}"
        );
        assert!(
            hint_unknown("unknown field `models`, expected one of `datasets`")
                .starts_with("unknown key `models` — write `datasets:`"),
        );
        assert_eq!(
            hint_unknown("unknown field `bogus`, expected `id`"),
            "unknown field `bogus`, expected `id`"
        );
    }
}
