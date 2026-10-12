/**
 * Which lens, if any, stands in for a core sub-component right now
 * (P9.A1, `.context/extensions.md` → "Replacements"). An extension
 * declares `ui.replacements: [{ target, lens }]`. The person says whose
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

import { effectiveConfig, recordUsage, runLens, subscribeOxplowEvents, type LensRun, type SqlCell } from "../api.js";
import { useExtensions } from "../extensionsStore.js";
import type { UiReplacement } from "../tauri-bridge/generated/bindings.js";
import { readCapabilityProviders } from "../workItems.js";
import { childParams } from "./lensModel.js";
import { NO_READS, unionReads, useRerunOnChange } from "./lensRerun.js";
import type { Reads } from "../tauri-bridge/generated/bindings.js";

export type Replacement =
  /** Not known yet: show nothing, so the wrong component never flashes. */
  | { state: "pending" }
  /** Oxplow's own component. */
  | { state: "core" }
  | {
      state: "replaced";
      replacement: UiReplacement;
      run: LensRun;
      /** Its custom component couldn't be shown, and why: from here it is
       *  `failed`, and oxplow's own shows (tsk855). */
      fail(reason: string): void;
      /** Its custom component said `ready`: it is shown. */
      shown(): void;
    }
  /** The replacement couldn't load: oxplow's own, and why. */
  | { state: "failed"; replacement: UiReplacement; message: string };

/** The `replacements` value that keeps oxplow's own component. */
export const OXPLOWS_OWN = "oxplow";

/** What the person said, by target: an extension, or `"oxplow"`. */
async function readChoices(): Promise<Record<string, string>> {
  const settings = await effectiveConfig();
  const chosen = settings.find((s) => s.key === "replacements")?.value;
  return chosen !== null && typeof chosen === "object" && !Array.isArray(chosen)
    ? Object.fromEntries(Object.entries(chosen as Record<string, unknown>).map(([k, v]) => [k, String(v)]))
    : {};
}

export function useReplacement(
  target: string,
  props: Record<string, SqlCell>,
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
  const lens = chosen === null ? null : ((exts ?? []).flatMap((e) => e.lenses).find((l) => l.id === chosen.lensId) ?? null);

  const [ran, setRan] = useState<{ id: string; key: string; run: LensRun | null; error: string | null } | null>(null);
  const propsKey = JSON.stringify(props);
  const runKey = chosen === null ? null : `${chosen.id}|${propsKey}|${streamId ?? ""}`;
  const seq = useRef(0);
  const run = useCallback(async () => {
    if (chosen === null || lens === null || runKey === null) return;
    const mine = ++seq.current;
    try {
      const out = await runLens(chosen.lensId, childParams(lens, JSON.parse(propsKey) as Record<string, SqlCell>), streamId);
      if (seq.current === mine) setRan({ id: chosen.id, key: runKey, run: out, error: null });
    } catch (e) {
      if (seq.current === mine) {
        setRan({ id: chosen.id, key: runKey, run: null, error: e instanceof Error ? e.message : String(e) });
      }
    }
    // `runKey` stands in for `chosen`, `props` and the stream.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [runKey, lens?.id]);
  useEffect(() => {
    seq.current += 1;
    void run();
  }, [run]);
  useRerunOnChange(unionReads([providerReads, ran?.run?.result.reads]), () => {
    void readProviders();
    void run();
  });

  // A custom component that couldn't be shown, for the run it failed on.
  const [failure, setFailure] = useState<{ key: string; message: string } | null>(null);
  const failedHere = failure !== null && failure.key === runKey;

  // Usage is the evidence a kind needs to be promoted: once per replacement
  // actually shown — a kit lens when it ran, a custom one when its
  // component says `ready` (one that never loads isn't shown).
  const record = useCallback(
    (id: string) => void recordUsage({ kind: "replacement", key: id, streamId }).catch(() => {}),
    [streamId],
  );
  const ranOk = ran !== null && ran.run !== null && ran.key === runKey ? ran : null;
  const shownNow = ranOk !== null && ranOk.run?.lens.viz !== "custom" && !failedHere ? ranOk.id : null;
  useEffect(() => {
    if (shownNow !== null) record(shownNow);
  }, [shownNow, record]);

  if (exts === null) return { state: "pending" };
  if (candidates.length === 0) return { state: "core" };
  if (!known) return { state: "pending" };
  if (chosen === null) return { state: "core" };
  if (lens === null) return { state: "failed", replacement: chosen, message: `its lens ${chosen.lensId} didn't load` };
  if (ran === null || ran.key !== runKey) return { state: "pending" };
  if (ran.run === null) return { state: "failed", replacement: chosen, message: ran.error ?? "it failed" };
  if (failedHere) return { state: "failed", replacement: chosen, message: failure.message };
  const key = runKey;
  const id = chosen.id;
  return {
    state: "replaced",
    replacement: chosen,
    run: ran.run,
    fail: (message) => setFailure({ key, message }),
    shown: () => record(id),
  };
}
