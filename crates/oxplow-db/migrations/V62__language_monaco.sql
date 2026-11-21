-- The editor's (Monaco's) id for the language, which its declaration gives
-- (`editor: { monaco }`); NULL when it declares none.
ALTER TABLE language_decl ADD COLUMN monaco TEXT;
