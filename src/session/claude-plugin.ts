import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { AGENT_GUIDE_FILENAME, buildAgentGuide } from "./agent-guide.js";
import {
  SKILL_FILE,
  SUBAGENT_PROTOCOL_SKILL_FILE,
  SUBAGENT_PROTOCOL_SKILL_NAME,
  TASK_DISPATCH_SKILL_NAME,
  TASK_FILING_SKILL_NAME,
  TASK_LIFECYCLE_SKILL_NAME,
  buildSubagentProtocolSkill,
  buildTaskDispatchSkill,
  buildTaskFilingSkill,
  buildTaskLifecycleSkill,
} from "./agent-skills.js";

export interface ElectronPluginOptions {
  /** Project dir. The plugin is written to
   *  <projectDir>/.newde/runtime/claude-plugin/. */
  projectDir: string;
  /** Absolute URL the plugin's http hooks POST to. Event name is appended
   *  as a path segment. */
  hookUrl: string;
}

export interface ElectronPlugin {
  /** Absolute path to the plugin directory; pass to `claude --plugin-dir`. */
  pluginDir: string;
  /** Absolute path to hooks/hooks.json. */
  hooksPath: string;
  /** Absolute path to .claude-plugin/plugin.json. */
  manifestPath: string;
  /** Absolute path to the reference guide the agent can Read on demand. */
  agentGuidePath: string;
  /** Absolute path to the task-filing SKILL.md. */
  taskFilingSkillPath: string;
  /** Absolute path to the task-lifecycle SKILL.md. */
  taskLifecycleSkillPath: string;
  /** Absolute path to the task-dispatch SKILL.md. */
  taskDispatchSkillPath: string;
  /** Absolute path to the subagent work-protocol SKILL.md. */
  subagentProtocolSkillPath: string;
}

// SessionStart is registered but Claude Code silently drops HTTP hooks for
// it ("HTTP hooks are not supported for SessionStart" in its debug log). We
// learn the session id from the next hook that fires instead — see
// decideResumeUpdate in resume-tracker.ts. The registration stays in case a
// future Claude version starts delivering it.
export const HOOK_EVENTS = [
  "PreToolUse",
  "PostToolUse",
  "UserPromptSubmit",
  "SessionStart",
  "SessionEnd",
  "Stop",
  "Notification",
] as const;

const PLUGIN_ENV_VARS = [
  "NEWDE_HOOK_TOKEN",
  "NEWDE_STREAM_ID",
  "NEWDE_THREAD_ID",
  "NEWDE_PANE",
] as const;

/**
 * Writes a Claude Code plugin to <projectDir>/.newde/runtime/claude-plugin/
 * that registers http hooks pointing at the runtime's MCP hook endpoint.
 * Invoked via `claude --plugin-dir <absPath>` so no files land in the user's
 * worktree.
 *
 * Idempotent per (projectDir, hookUrl) — identity rides env-var-interpolated
 * headers, so re-writing is safe as long as the runtime's MCP port is stable.
 */
export function createElectronPlugin(opts: ElectronPluginOptions): ElectronPlugin {
  const pluginDir = join(opts.projectDir, ".newde", "runtime", "claude-plugin");
  const manifestDir = join(pluginDir, ".claude-plugin");
  const hooksDir = join(pluginDir, "hooks");
  mkdirSync(manifestDir, { recursive: true });
  mkdirSync(hooksDir, { recursive: true });

  const manifestPath = join(manifestDir, "plugin.json");
  writeFileSync(
    manifestPath,
    JSON.stringify(
      {
        name: "newde-runtime",
        version: "0.0.0",
        description: "Forwards Claude Code lifecycle hooks into the newde runtime.",
      },
      null,
      2,
    ) + "\n",
    "utf8",
  );

  const hooksPath = join(hooksDir, "hooks.json");
  writeFileSync(
    hooksPath,
    JSON.stringify(buildPluginHooks(opts.hookUrl), null, 2) + "\n",
    "utf8",
  );

  // Reference catalog the agent can Read on demand instead of carrying
  // it in the system prompt every turn.
  const agentGuidePath = join(pluginDir, AGENT_GUIDE_FILENAME);
  writeFileSync(agentGuidePath, buildAgentGuide(), "utf8");

  // Model-invoked skills that fire on targeted triggers so each invocation
  // loads ~1k of focused policy instead of the old ~4k monolith. Three
  // orchestrator-side skills (filing, lifecycle, dispatch) + one
  // subagent-side skill (protocol).
  const writeSkill = (skillName: string, content: string): string => {
    const dir = join(pluginDir, "skills", skillName);
    mkdirSync(dir, { recursive: true });
    const path = join(dir, SKILL_FILE);
    writeFileSync(path, content, "utf8");
    return path;
  };
  const taskFilingSkillPath = writeSkill(TASK_FILING_SKILL_NAME, buildTaskFilingSkill());
  const taskLifecycleSkillPath = writeSkill(TASK_LIFECYCLE_SKILL_NAME, buildTaskLifecycleSkill());
  const taskDispatchSkillPath = writeSkill(TASK_DISPATCH_SKILL_NAME, buildTaskDispatchSkill());

  // Standing dispatch protocol for subagents — loaded whenever an agent
  // (typically a general-purpose subagent the orchestrator launched) sees a
  // work-item id in its brief or touches the newde update/note tools. Keeping
  // this out of every orchestrator brief cuts per-dispatch brief size from
  // ~1000 tokens to ~150.
  const subagentProtocolDir = join(pluginDir, "skills", SUBAGENT_PROTOCOL_SKILL_NAME);
  mkdirSync(subagentProtocolDir, { recursive: true });
  const subagentProtocolSkillPath = join(subagentProtocolDir, SUBAGENT_PROTOCOL_SKILL_FILE);
  writeFileSync(subagentProtocolSkillPath, buildSubagentProtocolSkill(), "utf8");

  return {
    pluginDir,
    hooksPath,
    manifestPath,
    agentGuidePath,
    taskFilingSkillPath,
    taskLifecycleSkillPath,
    taskDispatchSkillPath,
    subagentProtocolSkillPath,
  };
}

function buildPluginHooks(hookUrl: string) {
  const hooks: Record<string, unknown> = {};
  for (const event of HOOK_EVENTS) {
    const entry = {
      type: "http" as const,
      url: `${hookUrl}/${event}`,
      timeout: 3,
      headers: {
        "Authorization": "Bearer $NEWDE_HOOK_TOKEN",
        "X-Newde-Stream": "$NEWDE_STREAM_ID",
        "X-Newde-Thread": "$NEWDE_THREAD_ID",
        "X-Newde-Pane": "$NEWDE_PANE",
      },
      allowedEnvVars: [...PLUGIN_ENV_VARS],
    };
    if (event === "PreToolUse" || event === "PostToolUse") {
      hooks[event] = [{ matcher: "*", hooks: [entry] }];
    } else {
      hooks[event] = [{ hooks: [entry] }];
    }
  }
  return { hooks };
}
