-- Attribution is observed, not declared (.context/work-tracking.md). An
-- effort's files are claimed (an edit tool named them) or observed (they
-- changed during one of the thread's turns and nobody else claimed them),
-- recorded as the work happens; a run is the effort's whose tool call
-- caused it (metric_capture.effort_id). The close-time residue, the paths
-- an agent disowned and the run-claim ledger go: nothing is reconciled at
-- close any more.
ALTER TABLE effort_file ADD COLUMN source TEXT NOT NULL DEFAULT 'claimed'
    CHECK (source IN ('claimed', 'observed'));
INSERT OR IGNORE INTO effort_file (effort_id, path, change_kind, source)
    SELECT effort_id, path, 'updated', 'observed' FROM effort_unattributed_file;
DROP TABLE effort_unattributed_file;
DROP TABLE effort_acknowledged_path;
DROP TABLE effort_attribution;

-- Evidence signatures move from ledger claims to the effort's runs and
-- files (effort_evidence_store.rs SIG, the same expression): restamp them
-- so no effort reads as stale and recomputes at once.
UPDATE effort_evidence_state
   SET attribution_sig = (
       SELECT (SELECT count(*) || '|' || coalesce(max(c.id), '')
                 FROM metric_capture c
                WHERE c.effort_id = e.id AND c.trigger = 'on-report')
              || '|' || (SELECT count(*) FROM effort_file f WHERE f.effort_id = e.id)
         FROM effort e WHERE e.id = effort_evidence_state.effort_id);
