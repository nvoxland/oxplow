-- P6.G1: the person's left-nav layout in this project — which panels
-- show, in what order, collapsed or not. Local state (never the repo);
-- a panel the table doesn't name shows expanded after the ones it does.
CREATE TABLE panel_layout (
    panel TEXT PRIMARY KEY,
    position INTEGER NOT NULL,
    hidden INTEGER NOT NULL DEFAULT 0 CHECK (hidden IN (0, 1)),
    collapsed INTEGER NOT NULL DEFAULT 0 CHECK (collapsed IN (0, 1))
);
