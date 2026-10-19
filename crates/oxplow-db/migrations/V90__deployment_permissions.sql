-- What each deployment declares it may reach (its manifest's
-- `permissions:`), as JSON: what a person approved when it was deployed.
ALTER TABLE extension_deployment ADD COLUMN permissions_json TEXT NOT NULL DEFAULT '{}';
-- What an undeployed extension waits on a person for: the permissions it
-- would add, one line each, as a JSON array.
ALTER TABLE extension_listing ADD COLUMN pending_json TEXT NOT NULL DEFAULT '[]';
