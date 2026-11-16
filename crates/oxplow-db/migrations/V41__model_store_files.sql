-- A store that keeps its rows as files (`fills.files`) says where: the root
-- (`project`, `agent:claude`), the folder in it and the file suffix, so a
-- reader can tell a row's own file from any other (the `[[` list offers the
-- row, not its file). Restated with each publication.
ALTER TABLE model_store_decl ADD COLUMN files_root TEXT;
ALTER TABLE model_store_decl ADD COLUMN files_folder TEXT;
ALTER TABLE model_store_decl ADD COLUMN files_suffix TEXT;
