-- P7.C2: a disabled contribution's repair work item. `repair_item` is the
-- item's ref (`work_item:<provider>:<id>`) the `plugin.repair` consumer
-- filed for it; `repair_seq` the last `plugin.disabled` it handled, so a
-- redelivered event files or comments nothing twice. `v_plugin_health`
-- shows the item only while it's open.
ALTER TABLE plugin_health ADD COLUMN repair_item TEXT;
ALTER TABLE plugin_health ADD COLUMN repair_seq INTEGER;
