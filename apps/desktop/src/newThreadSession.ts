/**
 * What a new thread starts with — the person's own `newThreadSession`
 * (`.oxplow/personal.yaml`): `ask` (the session picker, the default),
 * `none`, or an agent (`<harness>` / `<harness>:<acp agent>`). Settings →
 * Agents and the picker's Remember This set it.
 */
import { effectiveConfig, listAgentHarnesses, runCommand } from "./api.js";
import { newThreadTabs } from "./tabs/sessionTabs.js";

const KEY = "newThreadSession";

/** The person's choice; `ask` when they haven't made one. */
export async function readNewThreadSession(): Promise<string> {
  const value = (await effectiveConfig()).find((s) => s.key === `personal.${KEY}`)?.value;
  return typeof value === "string" && value !== "" ? value : "ask";
}

/** Make `choice` what new threads start with (`ask` clears it). */
export async function saveNewThreadSession(choice: string): Promise<void> {
  if (choice === "ask") {
    await runCommand("oxplow.config.unset", { key: KEY, layer: "personal" }, true);
  } else {
    await runCommand("oxplow.config.set", { key: KEY, value: choice, layer: "personal" }, true);
  }
}

/** An agent choice's value, as the picker and the setting name it. */
export function agentChoiceValue(harness: string, acpAgent: string | null): string {
  return acpAgent ? `${harness}:${acpAgent}` : harness;
}

/** What a new thread opens with now: the setting against the harnesses as
 *  they are (read fresh — the app's list is empty until it loads). */
export async function newThreadOpening(): Promise<ReturnType<typeof newThreadTabs>> {
  const [setting, harnesses] = await Promise.all([readNewThreadSession(), listAgentHarnesses()]);
  return newThreadTabs(setting, harnesses);
}
