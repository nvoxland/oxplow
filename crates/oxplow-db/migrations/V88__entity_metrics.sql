-- Entity metrics and entity dimensions (tsk322). ADD COLUMN only: rebuilding
-- metric_spec / dimension would cascade into facts (the V54 lesson).
--
-- metric_spec.entity_json = {view, where?, time?, value?, aggregation} for a
-- metric computed over a v_* view instead of a measure's facts.
-- dimension.entity_json = {view, expr, join?} for a dimension that slices
-- entity metrics over the same view.
ALTER TABLE metric_spec ADD COLUMN entity_json TEXT;
ALTER TABLE dimension ADD COLUMN entity_json TEXT;
