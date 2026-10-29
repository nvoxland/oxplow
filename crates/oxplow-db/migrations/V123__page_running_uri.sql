-- The file URI of the copy of a page that runs, pinned to its deployment.
-- Restated whole with each extension generation, as the rest of `page` is.
ALTER TABLE page ADD COLUMN running_uri TEXT;
