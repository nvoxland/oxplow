-- A reviewer's verdict on a claim oxplow read in an agent's answers:
-- `verified` (`oxplow.effort.verify_claim`) or `dismissed` (the agent
-- didn't claim it, or didn't mean it). Reading the effort again replaces
-- only the claims nobody reviewed.
ALTER TABLE claim ADD COLUMN review TEXT CHECK (review IN ('verified', 'dismissed'));
UPDATE claim SET review = 'verified' WHERE evidence_ref = 'reviewer';
