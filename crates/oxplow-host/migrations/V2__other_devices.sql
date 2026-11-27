-- "Allow other devices", as the person saved it: one row, made when first
-- saved (none is off). What `serve`'s flags say for one run is never kept
-- here.
CREATE TABLE other_devices (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    allow INTEGER NOT NULL,
    address TEXT NOT NULL,
    port INTEGER,
    names TEXT NOT NULL,
    tls_cert TEXT,
    tls_key TEXT
);
