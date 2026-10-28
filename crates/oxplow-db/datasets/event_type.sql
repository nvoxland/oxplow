SELECT t.event_type, t.v, t.extension, t.summary, t.registered,
       t.v = (SELECT max(o.v) FROM source('event_type_contract') o
               WHERE o.event_type = t.event_type AND o.registered = 1) AS latest,
       t.schema_json, t.recorded_at
FROM source('event_type_contract') t
