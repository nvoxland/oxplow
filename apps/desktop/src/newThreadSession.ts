/** A new thread opens the session picker; starting an agent requires a person's choice. */
import { newThreadTabs } from "./tabs/sessionTabs.js";

export async function newThreadOpening(): Promise<ReturnType<typeof newThreadTabs>> {
  return newThreadTabs();
}
