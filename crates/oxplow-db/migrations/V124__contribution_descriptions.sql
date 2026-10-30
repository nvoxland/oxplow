-- A one-sentence description of the contributions that had none: a
-- dimension, a resource, a capability implementation and a collector
-- (`description:` in the manifest). NULL when the declaration says nothing.
ALTER TABLE dimension ADD COLUMN description TEXT;
ALTER TABLE resource ADD COLUMN description TEXT;
ALTER TABLE capability_provider ADD COLUMN description TEXT;
ALTER TABLE contribution ADD COLUMN description TEXT;
