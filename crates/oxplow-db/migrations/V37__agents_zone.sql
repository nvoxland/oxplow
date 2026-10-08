-- The zone holding the agent harnesses, the agent text and the control
-- plane is `agents` (`.oxplow/project.yaml` `zones:`); "plugin" names only
-- a Claude Code or opencode plugin. The changes already analyzed carry
-- the new name too.
UPDATE change_file SET zone = 'agents' WHERE zone = 'plugin';
UPDATE change_import SET from_zone = 'agents' WHERE from_zone = 'plugin';
UPDATE change_import SET to_zone = 'agents' WHERE to_zone = 'plugin';
