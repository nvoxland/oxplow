-- Whether each extension is the team's: on the project's own list.
ALTER TABLE extension_listing ADD COLUMN shared INTEGER NOT NULL DEFAULT 0;
