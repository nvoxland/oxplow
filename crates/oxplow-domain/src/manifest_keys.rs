//! Keys an `extension.yaml` or a page file no longer has, and what took
//! their place: the whole migration story for a private extension. A
//! manifest (`RENAMED`, `hint_unknown`) or a page (`PAGE_RENAMED`,
//! `hint_unknown_page`) with one is refused like any unknown key, and the
//! refusal names the new spelling, so `oxplow extension check` says what to
//! write. Pages have their own table: a manifest's `columns` and a page's
//! `columns` are different keys.

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
         `datasets: [{ name, fields: [..] }]`, each one's SQL moves to `datasets/<name>.sql`",
    ),
    (
        "columns",
        "write `fields:` — a dataset's `columns: [{ name, type, doc }]` is \
         `fields: [{ name, type, doc }]`",
    ),
    (
        "sourceMeasure",
        "a metric's `measure:` names the measure and its one `expression:` says how its facts \
         combine — `sourceMeasure: M, aggregation: sum` is `measure: M, expression: \"SUM(value)\"`",
    ),
    (
        "filter",
        "a metric's filter is part of its `expression:` — `filter: { minValue: 11 }` is \
         `expression: \"COUNT(*) FILTER (WHERE value >= 11)\"`, `filter: { dimEq: [k, v] }` is \
         `FILTER (WHERE dim('k') = 'v')`, `filter: { severity: s }` is `FILTER (WHERE severity = 's')`",
    ),
    (
        "formula",
        "a derived metric is an `expression:` — `formula: { op: div, left: a, right: b }` is \
         `expression: \"METRIC('a') / METRIC('b')\"` (`add` is `+`, `sub` `-`, `mul` `*`)",
    ),
    (
    "trigger",
    "write `on:` on the entry itself — `trigger: { on: [x], where: {…}, settle: 30s }` is \
     `on: [x]` with `where:` and `settle:` beside it, `trigger: { every: 15m }` is \
     `on: { every: 15m }`, `trigger: { on_run: test }` is `on: { run: test }`, \
     `trigger: manual` is `on: manual`",
    ),
];

/// An old top-level key of a page file and how to write what it said now.
pub const PAGE_RENAMED: &[(&str, &str)] = &[(
    "data",
    "write `loaders:` — a page's `data: { name: {...} }` is `loaders: { name: {...} }`",
)];

/// [`hint_unknown`] for a page file's parser message.
pub fn hint_unknown_page(message: &str) -> String {
    hint_with(PAGE_RENAMED, message)
}

/// `message` (a parser's) with the new spelling added when it refuses a
/// key in [`RENAMED`] — as unknown (``unknown field `trigger`, expected
/// …`` becomes ``unknown key `trigger` — write `on:` …``) or as the map it
/// used to be (``permissions: invalid type: map, expected a sequence``).
/// Any other message is kept.
pub fn hint_unknown(message: &str) -> String {
    hint_with(RENAMED, message)
}

fn hint_with(table: &[(&str, &str)], message: &str) -> String {
    for (old, hint) in table {
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

    /// The metric keys that became `measure:` and `expression:` say so, with
    /// the equivalent expression.
    #[test]
    fn the_old_metric_keys_say_what_an_expression_is() {
        for (old, says) in [
            ("sourceMeasure", "measure: M, expression:"),
            ("filter", "FILTER (WHERE value >= 11)"),
            ("formula", "METRIC('a') / METRIC('b')"),
        ] {
            let hinted = hint_unknown(&format!(
                "metrics[0]: unknown field `{old}`, expected one of `key`, `expression` at line 3 column 5"
            ));
            assert!(
                hinted.starts_with(&format!("metrics[0]: unknown key `{old}` — ")),
                "{hinted}"
            );
            assert!(hinted.contains(says), "{old}: {hinted}");
        }
    }

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
