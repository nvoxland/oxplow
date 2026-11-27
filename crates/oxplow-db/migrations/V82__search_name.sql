-- The search index's name index: what a person types for an entry (a
-- file's name, a page's title, a command's label), as trigrams, so a word
-- inside a name (`View` in `MarkdownView.tsx`) is found. Its rowid is the
-- entry's, as `search_fts`'s is.
CREATE VIRTUAL TABLE search_name USING fts5(name, tokenize = 'trigram');

-- Every entry is written again with its name: forget the versions, so the
-- next restate of each kind rewrites all of its entries. Until then an
-- entry is named by its title.
INSERT INTO search_name (rowid, name) SELECT rowid, title FROM search_fts;
UPDATE search_entry SET version = NULL;
