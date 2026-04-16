import type { AgentKind } from "../config/config.js";
import type { PaneKind, Stream } from "../persistence/stream-store.js";

export function buildAgentCommand(
  agent: AgentKind,
  stream: Stream,
  pane: PaneKind,
  settingsPath?: string,
  appendSystemPrompt?: string,
  mcpConfig?: string,
): string {
  const resumeSessionId = pane === "working"
    ? stream.resume.working_session_id
    : stream.resume.talking_session_id;
  return buildAgentCommandForSession(agent, stream.worktree_path, resumeSessionId, settingsPath, appendSystemPrompt, mcpConfig);
}

export function buildAgentCommandForSession(
  agent: AgentKind,
  cwd: string,
  resumeSessionId: string,
  settingsPath?: string,
  appendSystemPrompt?: string,
  mcpConfig?: string,
): string {
  if (agent === "copilot") {
    return `sh -lc ${shellEscape(`cd ${shellEscape(cwd)} && exec copilot`)}`;
  }

  if (!settingsPath) {
    throw new Error("Claude agent requires a settingsPath");
  }
  const promptArg = appendSystemPrompt
    ? ` --append-system-prompt ${shellEscape(appendSystemPrompt)}`
    : "";
  const mcpArg = mcpConfig
    ? ` --mcp-config ${shellEscape(mcpConfig)} --strict-mcp-config`
    : "";
  const claudeBase = `claude --settings ${shellEscape(settingsPath)}${promptArg}${mcpArg}`;
  const freshClaude = `exec ${claudeBase}`;
  const command = resumeSessionId
    ? `${claudeBase} --resume ${shellEscape(resumeSessionId)} || { echo '[newde] saved resume id was stale; starting a fresh Claude session' >&2; ${freshClaude}; }`
    : freshClaude;
  return `sh -lc ${shellEscape(`cd ${shellEscape(cwd)} && ${command}`)}`;
}

export function shellEscape(s: string): string {
  return `'${s.replace(/'/g, `'\\''`)}'`;
}
