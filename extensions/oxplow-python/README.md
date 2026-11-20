# oxplow-python

What oxplow knows about Python, as a language pack that ships with oxplow and is off by default. Turn it on in a project that has Python files (`oxplow.extensions.oxplow-python.enabled: true`); oxplow suggests it when it sees them.

- `languages:` in `extension.yaml` declares each language: its file extensions, other names, its grammar (compiled into oxplow), and the Mason package of its language server.
- `languages/<id>/analysis.yaml` is what code analysis reads: function and decision node kinds, the parameter list, how visibility and doc comments are told, and the import, merge and duplication facets.
- `languages/<id>/symbols.scm` (and `symbols.yaml`) say what a file defines.
