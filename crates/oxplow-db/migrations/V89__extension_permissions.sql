-- What each extension declares it may reach beyond the baseline (its
-- manifest's `permissions:`), as JSON; `{}` is the baseline.
ALTER TABLE extension_listing ADD COLUMN permissions_json TEXT NOT NULL DEFAULT '{}';
