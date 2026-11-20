# oxplow-typescript

What oxplow knows about TypeScript and JavaScript, as a language pack that ships with oxplow and is off by default. Turn it on in a project that has TypeScript and JavaScript files (`oxplow.extensions.oxplow-typescript.enabled: true`); oxplow suggests it when it sees them.

- `languages:` in `extension.yaml` declares each language: its file extensions, other names, its grammar (compiled into oxplow), and the Mason package of its language server.
- `languages/<id>/analysis.yaml` is what code analysis reads: function and decision node kinds, the parameter list, how visibility and doc comments are told, and the import, merge and duplication facets.
- `languages/<id>/symbols.scm` (and `symbols.yaml`) say what a file defines.
- `file_kinds:` names the files that are a dependency manifest, a lockfile, lint config or a test harness (including golden snapshots).
- `collectors:` holds the idiom metrics (`any`, non-null assertions, `console.*`, `@ts-ignore`), on while the pack is on, and a report collector for ESLint (`oxplow:eslint`, path in the `typescript.eslintReport` setting, default `target/eslint.json`).
- `skills/typescript-setup` tells the agent how to make the stack write the JUnit, lcov and ESLint reports oxplow reads.
- `run_patterns:` marks the pack's test and analysis commands as runs.
