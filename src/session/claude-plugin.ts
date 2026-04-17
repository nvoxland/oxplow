import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";

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
}

const HOOK_EVENTS = [
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
  "NEWDE_BATCH_ID",
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

  return { pluginDir, hooksPath, manifestPath };
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
        "X-Newde-Batch": "$NEWDE_BATCH_ID",
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
