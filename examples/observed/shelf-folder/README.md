# shelf-folder

A store over markdown files in `shelf/` of the project.

- Fills the model `shelf/book` for the store `shelf-folder`.
- One collector, triggers `start` and `{files: "shelf/*.md"}`. Start states a snapshot. A file change upserts or deletes.
- Uses `files:`, `files.read`, `files.write` and `model.write` only.
- A command at the action point `shelf/book.new` writes a file.
- A body that cites `[[file:src/a.rs]]` gets a pinned edge, and goes stale when that file changes.
