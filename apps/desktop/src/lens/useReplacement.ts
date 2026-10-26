/**
 * Which page, if any, stands in for a core sub-component right now (P9.A1,
 * `.context/extensions.md` → "Replacements"). An extension declares
 * `ui.replacements: [{ target, page }]`; its host (`Replaceable`) runs the
 * page with the target's props. The person says whose
 * shows, by target (`replacements: { <target>: <extension> | "oxplow" }`);
 * `"oxplow"` keeps oxplow's own wherever.
 *
 * A bound target (a work item's state control: a provider's own states)
 * is given the provider of the thing it shows (`provider`, the item's,
 * tsk918): only that provider's extension's replacement shows. Which
 * extension brings a provider is read from `v_capability_provider` here,
 * not at load, so installing or switching one needs no reload.
 *
 * An unbound target (the Board, the Tasks list, the detail editor, the
 * history graph) has none: the person's chosen extension shows, or with
 * no choice the only extension replacing it; several and no choice keeps
 * oxplow's own.
 */
import { useCallback, useEffect, useRef, useState } from "react";

import { recordUsage, subscribeOxplowEvents } from "../api.js";
import { settingValue } from "../configValues.js";
import { useExtensions } from "../extensionsStore.js";
import type { UiReplacement } from "../tauri-bridge/generated/bindings.js";
import { readCapabilityProviders } from "../workItems.js";
import { NO_READS, useRerunOnChange } from "./lensRerun.js";
import type { Reads } from "../tauri-bridge/generated/bindings.js";

export type Replacement =
  /** Not known yet: show nothing, so the wrong component never flashes. */
  | { state: "pending" }
  /** Oxplow's own component. */
  | { state: "core" }
  /** A page stands in: its host runs it with the target's props. */
  | { state: "page"; replacement: UiReplacement; page: string }
  /** The replacement couldn't load: oxplow's own, and why. */
  | { state: "failed"; replacement: UiReplacement; message: string };

/** The `replacements` value that keeps oxplow's own component. */
export const OXPLOWS_OWN = "oxplow";

/** What the person said, by target: an extension, or `"oxplow"`. */
async function readChoices(): Promise<Record<string, string>> {
  const chosen = await settingValue("replacements");
  return chosen !== null && typeof chosen === "object" && !Array.isArray(chosen)
    ? Object.fromEntries(Object.entries(chosen as Record<string, unknown>).map(([k, v]) => [k, String(v)]))
    : {};
}

export function useReplacement(
  target: string,
  streamId: string | null,
  /** The provider whose thing the component shows (a bound target's
   *  decider; an unbound target takes none). */
  provider: string | null = null,
): Replacement {
  const exts = useExtensions();
  const candidates = (exts ?? [])
    .filter((e) => e.enabled)
    .flatMap((e) => e.ui.replacements)
    .filter((r) => r.target === target);
  const capability = candidates[0]?.capability ?? null;
  const bound = candidates[0]?.boundToProvider ?? true;

  // The deciding provider's extension (`null`: one no extension brings),
  // and the person's choices; `undefined` while not read. An unbound
  // target reads no provider.
  const [owner, setOwner] = useState<string | null | undefined>(undefined);
  const [providerReads, setProviderReads] = useState<Reads>(NO_READS);
  const [choices, setChoices] = useState<Record<string, string> | undefined>(undefined);
  // Reads overlap (one per change): the newest one asked decides, however
  // they resolve (tsk857).
  const providersSeq = useRef(0);
  const readProviders = useCallback(async () => {
    if (capability === null || !bound || provider === null) {
      setOwner(null);
      return;
    }
    const mine = ++providersSeq.current;
    try {
      const out = await readCapabilityProviders(capability);
      if (providersSeq.current !== mine) return;
      const deciding = out.providers.find((p) => p.provider === provider);
      setOwner(deciding?.extension ?? null);
      setProviderReads(out.reads);
    } catch {
      // Unknown whose it is: oxplow's own is always right to show.
      if (providersSeq.current === mine) setOwner(null);
    }
  }, [capability, provider, bound]);
  const readOff = useCallback(async () => {
    try {
      setChoices(await readChoices());
    } catch {
      setChoices({});
    }
  }, []);
  useEffect(() => {
    if (capability === null) return;
    void readProviders();
    void readOff();
    return subscribeOxplowEvents((event) => {
      if (event.kind === "configChanged") void readOff();
    });
  }, [capability, readProviders, readOff]);

  // "oxplow": oxplow's own. Bound: the deciding provider's extension.
  // Unbound: the person's choice, or the only candidate.
  const said = choices?.[target];
  const decider =
    said === OXPLOWS_OWN
      ? null
      : bound
        ? owner
        : (said ?? (candidates.length === 1 ? candidates[0].extension : null));
  const known = choices !== undefined && (!bound || owner !== undefined);
  const chosen =
    !known || decider === null || decider === undefined ? null : (candidates.find((c) => c.extension === decider) ?? null);
  const page = chosen?.page ?? null;
  useRerunOnChange(providerReads, () => void readProviders());

  // Usage is the evidence a kind needs to be promoted: once per
  // replacement actually shown.
  const record = useCallback(
    (id: string) => void recordUsage({ kind: "replacement", key: id, streamId }).catch(() => {}),
    [streamId],
  );
  const shownNow = known && chosen !== null && page ? chosen.id : null;
  useEffect(() => {
    if (shownNow !== null) record(shownNow);
  }, [shownNow, record]);

  if (exts === null) return { state: "pending" };
  if (candidates.length === 0) return { state: "core" };
  if (!known) return { state: "pending" };
  if (chosen === null) return { state: "core" };
  if (page) return { state: "page", replacement: chosen, page };
  return { state: "failed", replacement: chosen, message: "it names no page" };
}
