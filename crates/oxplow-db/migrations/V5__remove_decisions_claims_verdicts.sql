-- Decisions, claims and review verdicts are removed, and their data with
-- them: nothing is kept or converted. The tables, the events that recorded
-- them (and any dead letter of those events), and the verdicts' events.
DELETE FROM event_dead_letter
 WHERE event_seq IN (SELECT seq FROM event_log WHERE type IN (
   'effort.claim_verified', 'effort.claim_dismissed',
   'effort.decision_reviewed', 'effort.decisions_inferred',
   'oxplow.review.accepted', 'oxplow.review.changes_requested'));
DELETE FROM event_log
 WHERE type IN (
   'effort.claim_verified', 'effort.claim_dismissed',
   'effort.decision_reviewed', 'effort.decisions_inferred',
   'oxplow.review.accepted', 'oxplow.review.changes_requested');
DROP TABLE claim;
DROP TABLE decision;
