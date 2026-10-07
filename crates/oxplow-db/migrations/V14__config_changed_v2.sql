-- config.changed carries the layer it changed (project.yaml or a person's
-- personal.yaml): every logged v1 change was to the project's.
UPDATE event_log
   SET payload = json_set(payload, '$.layer', 'project'), v = 2
 WHERE type = 'config.changed' AND v = 1;
