# shelf-db

A store over the extension's own SQLite file.

- `storage: {sqlite: true}` and one migration, `V1__book.sql`.
- Fills `shelf/book` for the store `shelf-db`, with the field `rating`.
- A collector with triggers `start` and `{sqlite: own}` states a snapshot and returns a cursor (the max rowid).
- Commands `book.add` (at the action point `shelf/book.new`) and `book.remove` (on shelf-db items only).
- Uses `store.sql` and `model.write` only.
