# shelf-score

A field added to books of any store.

- Declares `score` (number) on `shelf/book` with no stores (`fields:`).
- One command, `book.score`, composes `oxplow.model.set_fields`. It writes only the namespace `shelf_score`.
- A store's reconcile keeps the score of a book that is still there.
- The score goes when the book goes.
- `v_model_field_decl` lists the fields of all extensions side by side.
