# shelf

The owner of a shared model.

- Declares the open model `book` (title, author, about, a markdown body with pins).
- Declares the change event `shelf.book.changed` and the action point `shelf/book.new`.
- Has one read-only page, `books`, over `v_shelf_book`.
- Keeps no books. shelf-folder and shelf-db fill the model.
- Shows that a page can read rows of many stores and never write them.
