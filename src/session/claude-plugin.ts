import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { AGENT_GUIDE_FILENAME, buildAgentGuide } from "./agent-guide.js";
import {
  RUNTIME_SKILL_NAME,
  SKILL_FILE,
  SUBAGENT_PROTOCOL_SKILL_FILE,
  SUBAGENT_PROTOCOL_SKILL_NAME,
  WORK_NEXT_COMMAND_FILE,
  buildRuntimeSkill,
  buildSubagentProtocolSkill,
  buildWorkNextCommand,
} from "./agent-skills.js";
import {
  WIKI_CAPTURE_SKILL_NAME,
  WIKI_CAPTURE_SKILL_FILE,
  buildWikiCaptureSkill,
} from "./wiki-capture-skill.js";

export interface ElectronPluginOptions {
  /** Project dir. The plugin is written to
   *  <projectDir>/.oxplow/runtime/claude-plugin/. */
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
  /** Absolute path to the merged oxplow-runtime SKILL.md (filing + lifecycle + dispatch). */
  runtimeSkillPath: string;
  /** Back-compat alias for runtimeSkillPath — the three legacy skills
   *  now collapse into the single merged skill, so all three paths point
   *  at the same SKILL.md. Kept so older tests that checked each path
   *  independently still pass. */
  taskFilingSkillPath: string;
  /** Back-compat alias for runtimeSkillPath. See runtimeSkillPath. */
  taskLifecycleSkillPath: string;
  /** Back-compat alias for runtimeSkillPath. See runtimeSkillPath. */
  taskDispatchSkillPath: string;
  /** Absolute path to the subagent work-protocol SKILL.md. */
  subagentProtocolSkillPath: string;
  /** Absolute path to the wiki-capture SKILL.md. */
  wikiCaptureSkillPath: string;
  /** Absolute path to the /work-next slash command file. */
  workNextCommandPath: string;
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
  "OXPLOW_HOOK_TOKEN",
  "OXPLOW_STREAM_ID",
  "OXPLOW_THREAD_ID",
  "OXPLOW_PANE",
] as const;

/**
 * Writes a Claude Code plugin to <projectDir>/.oxplow/runtime/claude-plugin/
 * that registers http hooks pointing at the runtime's MCP hook endpoint.
 * Invoked via `claude --plugin-dir <absPath>` so no files land in the user's
 * worktree.
 *
 * Idempotent per (projectDir, hookUrl) — identity rides env-var-interpolated
 * headers, so re-writing is safe as long as the runtime's MCP port is stable.
 */
export function createElectronPlugin(opts: ElectronPluginOptions): ElectronPlugin {
  const pluginDir = join(opts.projectDir, ".oxplow", "runtime", "claude-plugin");
  const manifestDir = join(pluginDir, ".claude-plugin");
  const hooksDir = join(pluginDir, "hooks");
  mkdirSync(manifestDir, { recursive: true });
  mkdirSync(hooksDir, { recursive: true });

  const manifestPath = join(manifestDir, "plugin.json");
  writeFileSync(
    manifestPath,
    JSON.stringify(
      {
        name: "oxplow-runtime",
        version: "0.0.0",
        description: "Forwards Claude Code lifecycle hooks into the oxplow runtime.",
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

  // Model-invoked skills that fire on targeted triggers. Post-merge the
  // three legacy orchestrator-side skills (filing, lifecycle, dispatch)
  // collapse into one `oxplow-runtime` skill — the per-turn skill index
  // drops from three lines to one. The subagent-work-protocol skill
  // stays separate since it only triggers in subagent contexts.
  const writeSkill = (skillName: string, content: string): string => {
    const dir = join(pluginDir, "skills", skillName);
    mkdirSync(dir, { recursive: true });
    const path = join(dir, SKILL_FILE);
    writeFileSync(path, content, "utf8");
    return path;
  };
  const runtimeSkillPath = writeSkill(RUNTIME_SKILL_NAME, buildRuntimeSkill());
  // Back-compat aliases: all three legacy path fields point at the
  // merged skill so callers don't break.
  const taskFilingSkillPath = runtimeSkillPath;
  const taskLifecycleSkillPath = runtimeSkillPath;
  const taskDispatchSkillPath = runtimeSkillPath;

  // Standing dispatch protocol for subagents — loaded whenever an agent
  // (typically a general-purpose subagent the orchestrator launched) sees a
  // work-item id in its brief or touches the oxplow update/note tools. Keeping
  // this out of every orchestrator brief cuts per-dispatch brief size from
  // ~1000 tokens to ~150.
  const subagentProtocolDir = join(pluginDir, "skills", SUBAGENT_PROTOCOL_SKILL_NAME);
  mkdirSync(subagentProtocolDir, { recursive: true });
  const subagentProtocolSkillPath = join(subagentProtocolDir, SUBAGENT_PROTOCOL_SKILL_FILE);
  writeFileSync(subagentProtocolSkillPath, buildSubagentProtocolSkill(), "utf8");

  // Wiki-capture skill — fires when the agent uses the wiki MCP tools or
  // when the user prompt looks like a code-exploration question. Carries
  // the find-or-create flow + body conventions so a new note doesn't
  // fragment off an existing topic.
  const wikiCaptureDir = join(pluginDir, "skills", WIKI_CAPTURE_SKILL_NAME);
  mkdirSync(wikiCaptureDir, { recursive: true });
  const wikiCaptureSkillPath = join(wikiCaptureDir, WIKI_CAPTURE_SKILL_FILE);
  writeFileSync(wikiCaptureSkillPath, buildWikiCaptureSkill(), "utf8");

  // User-invoked slash commands. Shipped via the plugin so every project
  // running oxplow gets `/work-next` — replaces the old Stop-hook ready-
  // work directive. The repo-local `.claude/commands/` directory stays
  // scoped to oxplow-on-oxplow dogfooding.
  const commandsDir = join(pluginDir, "commands");
  mkdirSync(commandsDir, { recursive: true });
  const workNextCommandPath = join(commandsDir, WORK_NEXT_COMMAND_FILE);
  writeFileSync(workNextCommandPath, buildWorkNextCommand(), "utf8");

  return {
    pluginDir,
    hooksPath,
    manifestPath,
    agentGuidePath,
    runtimeSkillPath,
    taskFilingSkillPath,
    taskLifecycleSkillPath,
    taskDispatchSkillPath,
    subagentProtocolSkillPath,
    wikiCaptureSkillPath,
    workNextCommandPath,
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
        "Authorization": "Bearer $OXPLOW_HOOK_TOKEN",
        "X-Oxplow-Stream": "$OXPLOW_STREAM_ID",
        "X-Oxplow-Thread": "$OXPLOW_THREAD_ID",
        "X-Oxplow-Pane": "$OXPLOW_PANE",
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
