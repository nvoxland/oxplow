# oxplow-c

What oxplow knows about C and C++, as a language pack that ships with oxplow and is off by default. Turn it on in a project that has C and C++ files (`oxplow.extensions.oxplow-c.enabled: true`); oxplow suggests it when it sees them.

- `languages:` in `extension.yaml` declares each language: its file extensions, other names, its grammar (compiled into oxplow), and the Mason package of its language server.
- `languages/<id>/analysis.yaml` is what code analysis reads: function and decision node kinds, the parameter list, how visibility and doc comments are told, and the import, merge and duplication facets.
- `languages/<id>/symbols.scm` (and `symbols.yaml`) say what a file defines.
- `file_kinds:` names the files of this stack that are not source: dependency manifests, lockfiles, lint configs and test harness files.
- `run_patterns:` lists the commands that count as a test or an analysis run for this stack.
- `skills:` has `setup-c-reports`, which tells the agent how to make this stack write the JUnit and coverage reports oxplow reads. No report collector is declared: the linters here have no bundled report parser.
