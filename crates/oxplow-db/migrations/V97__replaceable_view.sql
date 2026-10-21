-- Each replaceable target's view: the host page's view params its
-- replacement shares (JSON array of names), restated with the rest.
ALTER TABLE replaceable ADD COLUMN view TEXT NOT NULL DEFAULT '[]';
