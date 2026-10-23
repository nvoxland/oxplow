/**
 * Turning a failure-disabled contribution back on (P7.C1–C3,
 * `.context/extensions.md` -> "Health, disable and repair"): three
 * failures in a row disable a provider instance, collector or effect until
 * a person runs `oxplow.contribution.enable`. Settings -> Extensions lists
 * them in the `extensions-health` lens; Settings -> Data turns a collector
 * back on from here.
 */
import { personCommands, type PersonCommands } from "./personCommands.js";

/** The contribution `enableAgain` names. */
export interface ContributionHealth {
  /** The extension. */
  extension: string;
  /** Its provider's or collector's id. */
  contribution: string;
  /** `provider`, `collector` or `effect`. */
  kind: string;
}

/** Enable Again: `oxplow.contribution.enable` as the person. */
export async function enableAgain(h: ContributionHealth, commands: PersonCommands = personCommands): Promise<boolean> {
  return !!(await commands.run(`Enable ${h.extension}/${h.contribution}`, "oxplow.contribution.enable", {
    extension: h.extension,
    kind: h.kind,
    contribution: h.contribution,
  }));
}

/** Enable All Disabled: `oxplow.contribution.enable_all` as the person —
 *  every failure-disabled contribution. */
export async function enableAllAgain(commands: PersonCommands = personCommands): Promise<boolean> {
  return !!(await commands.run("Enable all disabled contributions", "oxplow.contribution.enable_all", {}));
}
