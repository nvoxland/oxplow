-- What each tool call does in oxplow's own vocabulary
-- (`oxplow_domain::agent::tool::ToolKind`): `read`, `edit`, `shell`,
-- `subagent`, `ask`, `plan`, `search`, `fetch`, `mcp`, `other`. `tool`
-- stays the harness's own name for it. The views over tool calls
-- (`v_struggle`, `v_context_read`, a turn's writing calls) read the kind,
-- so they hold for every harness.
--
-- Existing rows are read from their names: Claude Code's, which every
-- transport mapped onto before (ACP calls included), and Codex's, which
-- arrived as they were.
ALTER TABLE agent_tool_call ADD COLUMN kind TEXT NOT NULL DEFAULT 'other';

UPDATE agent_tool_call SET kind = CASE
    WHEN tool IN ('Write', 'Edit', 'MultiEdit', 'NotebookEdit', 'apply_patch') THEN 'edit'
    WHEN tool IN ('Bash', 'shell', 'exec_command', 'local_shell') THEN 'shell'
    WHEN tool IN ('Task', 'Agent') THEN 'subagent'
    WHEN tool = 'AskUserQuestion' THEN 'ask'
    WHEN tool = 'ExitPlanMode' THEN 'plan'
    WHEN tool = 'Read' THEN 'read'
    WHEN tool IN ('Grep', 'Glob', 'List') THEN 'search'
    WHEN tool IN ('WebFetch', 'WebSearch') THEN 'fetch'
    WHEN tool LIKE 'mcp\_\_%' ESCAPE '\' THEN 'mcp'
    ELSE 'other'
END;
