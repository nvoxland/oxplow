-- Code-quality scans used to pile up: every change analysis stored a new
-- duplication scan for its scope and kept the older ones, so a long-lived
-- project held hundreds of thousands of findings (and one `page_ref` edge
-- each) nobody reads — readers want a scope's latest scan only. A scan now
-- replaces its scope's older ones as it finishes
-- (`SqliteCodeQualityStore::finish_scan_with_findings`); this prunes what
-- piled up before that: every finished scan older than its scope's latest
-- done one, with its findings and their edges. A running scan stays.
-- (Explicit deletes: foreign keys may be off while migrations run.)
CREATE TEMP TABLE superseded_scan AS
  SELECT o.id FROM code_quality_scan o
  WHERE o.status <> 'running'
    AND o.id < (SELECT max(n.id) FROM code_quality_scan n
                WHERE n.tool = o.tool AND n.scope = o.scope AND n.status = 'done');

DELETE FROM page_ref
  WHERE source_kind = 'finding'
    AND source_id IN (SELECT CAST(f.id AS TEXT) FROM code_quality_finding f
                      WHERE f.scan_id IN (SELECT id FROM superseded_scan));
DELETE FROM code_quality_finding WHERE scan_id IN (SELECT id FROM superseded_scan);
DELETE FROM code_quality_scan WHERE id IN (SELECT id FROM superseded_scan);
DROP TABLE superseded_scan;
