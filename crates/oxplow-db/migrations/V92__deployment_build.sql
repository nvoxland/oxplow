-- The command a deployment was built with (its manifest's `build.run`):
-- what a person approved when it was deployed. NULL when it has no build.
ALTER TABLE extension_deployment ADD COLUMN build_run TEXT;
