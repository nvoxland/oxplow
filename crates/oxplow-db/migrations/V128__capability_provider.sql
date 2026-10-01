-- P6b: each capability's providers and the feature flags they declare
-- (a provider's `initialize`, core's own), so the UI can hide what a
-- provider can't do. Restated from what runs: core's at boot, an
-- external provider's while its instance runs. `active` is the hook for
-- choosing a capability's active provider (P7); every row is 1 today.
CREATE TABLE capability_provider (
    capability TEXT NOT NULL,
    provider TEXT NOT NULL,
    extension TEXT,
    features_json TEXT NOT NULL DEFAULT '{}',
    active INTEGER NOT NULL DEFAULT 1 CHECK (active IN (0, 1)),
    PRIMARY KEY (capability, provider)
);
