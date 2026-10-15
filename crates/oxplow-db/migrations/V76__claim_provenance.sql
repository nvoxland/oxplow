-- Where a claim came from: the agent stated it in `oxplow.effort.report`
-- (`recorded`) or a model read it in the effort's answers (`inferred`).
-- Each source replaces only its own unreviewed claims. A claim the agent
-- recorded itself carried its session; one a model read never does.
ALTER TABLE claim ADD COLUMN provenance TEXT NOT NULL DEFAULT 'recorded'
  CHECK (provenance IN ('recorded', 'inferred'));
UPDATE claim SET provenance = 'inferred' WHERE agent_session_id IS NULL;
