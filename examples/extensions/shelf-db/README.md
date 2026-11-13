# shelf-db

A store over the extension's own SQLite file.

- `storage: {sqlite: true}` and two migrations: `V1__book.sql`, and `V2__change_sequence.sql` (a `seq` change number on each row, and `deleted` tombstones).
- Fills `shelf/book` for the store `shelf-db`, with the field `rating`.
- A collector with triggers `start` and `{sqlite: own}` states a full snapshot on `start` (and on a first run), then only what changed: the rows after the cursor (the last `seq` seen) as `upsert`, the tombstones as `delete`. The write commands give their row the next `seq`; remove keeps a tombstone, so a delete is seen. This is the incremental form to copy for a store with a lot of rows.
- Commands `book.add` (at the action point `shelf/book.new`) and `book.remove` (on shelf-db items only). A store write leaves oxplow's records, so a run of it has no undo of its own: `book.remove` reads the book (`store.sql`) and returns an `inverse` beside its write, the `book.add` call that puts the book back (it revives the tombstone). The bus keeps it as the run's undo. Fixtures: `removing-a-book-returns-the-inverse-that-adds-it-back` (a dry run, `expect.inverse`) and `undoing-a-remove-puts-the-book-back` (a `when: { undo: … }` step).
- Reads with `store.sql`; writes the file by composing `oxplow.extension_store.write`.
