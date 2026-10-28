-- Guidance for an agent reading the semantic layer: an optional
-- `ai_context` block (JSON) on a model, a metric and a dimension. A
-- model's columns' blocks are one JSON object keyed by column name.
ALTER TABLE model ADD COLUMN ai_context_json TEXT CHECK (ai_context_json IS NULL OR json_valid(ai_context_json));
ALTER TABLE model ADD COLUMN column_ai_context_json TEXT CHECK (column_ai_context_json IS NULL OR json_valid(column_ai_context_json));
ALTER TABLE metric_spec ADD COLUMN ai_context_json TEXT CHECK (ai_context_json IS NULL OR json_valid(ai_context_json));
ALTER TABLE dimension ADD COLUMN ai_context_json TEXT CHECK (ai_context_json IS NULL OR json_valid(ai_context_json));
