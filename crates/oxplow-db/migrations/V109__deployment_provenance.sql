-- Where each deployment came from: the primary checkout's HEAD when its
-- source lies inside the project (NULL for a source outside it), and
-- whether the source had uncommitted changes, untracked files included,
-- when it was deployed. `from_stream` was never written with a value once
-- an extension deploys only from the primary checkout.
ALTER TABLE extension_deployment DROP COLUMN from_stream;
ALTER TABLE extension_deployment ADD COLUMN "commit" TEXT;
ALTER TABLE extension_deployment ADD COLUMN dirty INTEGER NOT NULL DEFAULT 0;
