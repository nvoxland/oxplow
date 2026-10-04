-- tsk889: what each effort's stored evidence was computed from — its run
-- and file claims as a signature (how many, and the newest's time) — so a
-- claim that moves after the effort closed recomputes it. Every existing
-- effort is current as of now.
CREATE TABLE effort_evidence_state (
    effort_id       INTEGER PRIMARY KEY REFERENCES effort(id) ON DELETE CASCADE,
    attribution_sig TEXT NOT NULL,
    refreshed_at    TEXT NOT NULL
) STRICT;

INSERT INTO effort_evidence_state (effort_id, attribution_sig, refreshed_at)
SELECT e.id,
       (SELECT count(*) || '|' || coalesce(max(a.recorded_at), '')
          FROM effort_attribution a WHERE a.effort_id = e.id),
       strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
  FROM effort e;
