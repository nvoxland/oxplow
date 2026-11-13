# shelf-folder

A store over markdown files in `shelf/` of the project.

- Fills the model `shelf/book` for the store `shelf-folder`.
- One collector, triggers `start` and `{files: "shelf/*.md"}`. Start states a snapshot. A file change upserts or deletes.
- The collector only reads (`files.list`, `files.read`) and returns the `models` output. The host applies it.
- The command writes by composing `oxplow.extension_files.write`.
- The collector declares a placeholder entity `seen`: a collector must declare an entity or a fact today, even when it only fills a model.
- A command at the action point `shelf/book.new` writes a file.
- A body that cites `[[file:src/a.rs]]` gets a pinned edge, and goes stale when that file changes.
