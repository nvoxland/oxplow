-- A Codex session is Codex's own TUI in a terminal, on a thread oxplow
-- opens and observes on Codex's daemon, no longer oxplow's chat view
-- (.context/agent-model.md "Codex: its TUI on a thread oxplow observes").
UPDATE agent_session SET kind = 'terminal' WHERE harness = 'codex' AND kind = 'chat';
