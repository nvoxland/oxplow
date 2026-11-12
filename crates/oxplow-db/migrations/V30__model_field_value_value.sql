-- Assignee-style lookups read a field by its value (model, namespace, name, value).
CREATE INDEX model_field_value_value ON model_field_value (model, namespace, name, value);
