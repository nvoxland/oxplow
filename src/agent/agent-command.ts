import type { AgentKind } from "../config/config.js";
import type { PaneKind, Stream } from "../persistence/stream-store.js";

export interface AgentCommandOptions {
  pluginDir?: string;
  allowedTools?: string[];
  appendSystemPrompt?: string;
  mcpConfig?: string;
  env?: Record<string, string>;
}

export function buildAgentCommand(
  agent: AgentKind,
  stream: Stream,
  pane: PaneKind,
  opts: AgentCommandOptions = {},
): string {
  const resumeSessionId = pane === "working"
    ? stream.resume.working_session_id
    : stream.resume.talking_session_id;
  return buildAgentCommandForSession(agent, stream.worktree_path, resumeSessionId, opts);
}

export function buildAgentCommandForSession(
  agent: AgentKind,
  cwd: string,
  resumeSessionId: string,
  opts: AgentCommandOptions = {},
): string {
  const envPrefix = buildEnvPrefix(opts.env);
  if (agent === "copilot") {
    return `sh -lc ${shellEscape(`cd ${shellEscape(cwd)} && ${envPrefix}exec copilot`)}`;
  }

  const pluginArg = opts.pluginDir ? ` --plugin-dir ${shellEscape(opts.pluginDir)}` : "";
  const allowedToolsArg = opts.allowedTools && opts.allowedTools.length > 0
    ? ` --allowedTools ${opts.allowedTools.map(shellEscape).join(" ")}`
    : "";
  const promptArg = opts.appendSystemPrompt
    ? ` --append-system-prompt ${shellEscape(opts.appendSystemPrompt)}`
    : "";
  const mcpArg = opts.mcpConfig
    ? ` --mcp-config ${shellEscape(opts.mcpConfig)} --strict-mcp-config`
    : "";
  const claudeBase = `claude${pluginArg}${allowedToolsArg}${promptArg}${mcpArg}`;
  const freshClaude = `${envPrefix}exec ${claudeBase}`;
  const command = resumeSessionId
    ? `${envPrefix}${claudeBase} --resume ${shellEscape(resumeSessionId)} || { echo '[oxplow] saved resume id was stale; starting a fresh Claude session' >&2; ${freshClaude}; }`
    : freshClaude;
  return `sh -lc ${shellEscape(`cd ${shellEscape(cwd)} && ${command}`)}`;
}

function buildEnvPrefix(env?: Record<string, string>): string {
  if (!env) return "";
  const parts: string[] = [];
  for (const [key, value] of Object.entries(env)) {
    // Keys come from this file / runtime — restrict to POSIX env-var charset.
    if (!/^[A-Z_][A-Z0-9_]*$/.test(key)) {
      throw new Error(`invalid env var name: ${key}`);
    }
    parts.push(`${key}=${shellEscape(value)}`);
  }
  return parts.length > 0 ? parts.join(" ") + " " : "";
}

export function shellEscape(s: string): string {
  return `'${s.replace(/'/g, `'\\''`)}'`;
}
