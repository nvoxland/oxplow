-- P9.D3: a plugin ref kind may be searchable — `searchable` names the
-- view (`ref`, `title`, `body`) whose rows the site-wide search indexes
-- under the kind. NULL: its refs resolve and open, but aren't found by
-- search. Core kinds are indexed by core (`search.index`) and leave it NULL.
ALTER TABLE ref_kind ADD COLUMN searchable TEXT;
