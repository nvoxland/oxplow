-- tsk780: what each asset's last recompute computed — a fingerprint of a
-- materialized model's SELECT. A clocked (`every:`) asset whose definition
-- changed is due at once rather than serving the old SQL's rows until its
-- clock comes round.
ALTER TABLE asset_state ADD COLUMN definition TEXT;
