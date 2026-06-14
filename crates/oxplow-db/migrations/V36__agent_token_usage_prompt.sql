-- Per-turn user prompt capture (tsk143). The token-usage rows in
-- `agent_token_usage` already carry one row per agent turn (usage + model)
-- parsed from the transcript on Stop. This adds the human-authored prompt
-- text that OPENED that turn, so an effort review can show what was ASKED
-- next to what LANDED.
--
-- Pure hooks-only OBSERVATION: oxplow reads the prompt out of the same
-- transcript walk it already does — it never generates or sends a prompt.
-- Nullable: a turn can be recorded with no opening user prompt (assistant
-- continuation, or an agent kind whose transcript we don't parse for text).
-- Stored locally in the effort DB like every other effort artifact. See
-- `.context/agent-model.md` (token usage capture) + `.context/data-model.md`.

ALTER TABLE agent_token_usage ADD COLUMN prompt TEXT;
