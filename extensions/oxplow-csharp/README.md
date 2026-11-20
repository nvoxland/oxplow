# oxplow-csharp

What oxplow knows about C#, as a language pack that ships with oxplow and is off by default. Turn it on in a project that has C# files (`oxplow.extensions.oxplow-csharp.enabled: true`); oxplow suggests it when it sees them.

- `languages:` in `extension.yaml` declares each language: its file extensions, other names, its grammar (compiled into oxplow), and the Mason package of its language server.
- `languages/<id>/analysis.yaml` is what code analysis reads: function and decision node kinds, the parameter list, how visibility and doc comments are told, and the import, merge and duplication facets.
- `languages/<id>/symbols.scm` (and `symbols.yaml`) say what a file defines.
- `file_kinds:` names the files that are a dependency manifest, a lockfile or lint config.
- `collectors:` holds the idiom metrics (empty `catch` blocks, blocking calls on async code), on while the pack is on.
- `skills/csharp-setup` tells the agent how to make `dotnet test` write the JUnit and cobertura reports oxplow reads.
- `run_patterns:` marks the pack's test and analysis commands as runs.
