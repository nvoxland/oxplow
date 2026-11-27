-- A page visit records the device it was opened on. Rows from before
-- devices keep NULL: they count as every device's history.
ALTER TABLE page_visit ADD COLUMN device_id TEXT;
