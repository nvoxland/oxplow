-- One row per glob of every enabled extension's `file_kinds:` and of the
-- project's `oxplow.files.kinds`.
SELECT kind, glob, label, extension
FROM source('file_kind_rule_decl')
