/**
 * Which lens, if any, stands in for a core sub-component right now
 * (P9.A1, `.context/extensions.md` → "Replacements"). An extension
 * declares `ui.replacements: [{ target, lens }]`; it renders only while
 * the target's capability's **active provider** is that extension's —
 * `v_capability_provider.active`, which changes with `activeProviders`
 * and no reload, so the choice is made here, not at load — and while the
 * person hasn't listed the target in `replacementsOff`. There is never a
 * second candidate: one provider is active.
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
  | { state: "replaced"; replacement: UiReplacement; run: LensRun }
  /** The replacement couldn't load: oxplow's own, and why. */
  | { state: "failed"; replacement: UiReplacement; message: string };

/** What a person calls each replaceable component. */
export const REPLACEABLE_LABELS: Readonly<Record<string, string>> = {
  "work_item.board": "board",
};

/** The targets the person turned replacements off for (`replacementsOff`). */
async function readReplacementsOff(): Promise<string[]> {
  const settings = await effectiveConfig();
  const value = settings.find((s) => s.key === "replacementsOff")?.value;
  return Array.isArray(value) ? value.map(String) : [];
}

export function useReplacement(target: string, props: Record<string, SqlCell>, streamId: string | null): Replacement {
  const exts = useExtensions(streamId);
  const candidates = (exts ?? [])
    .filter((e) => e.enabled)
    .flatMap((e) => e.ui.replacements)
    .filter((r) => r.target === target);
  const capability = candidates[0]?.capability ?? null;

  // The capability's active provider's extension (`null`: oxplow's own),
  // and the targets turned off; `undefined` while not read.
  const [activeExtension, setActiveExtension] = useState<string | null | undefined>(undefined);
  const [providerReads, setProviderReads] = useState<Reads>(NO_READS);
  const [off, setOff] = useState<string[] | undefined>(undefined);
  const readProviders = useCallback(async () => {
    if (capability === null) return;
    try {
      const out = await readCapabilityProviders(capability);
      setActiveExtension(out.providers.find((p) => p.active)?.extension ?? null);
      setProviderReads(out.reads);
    } catch {
      // Unknown who's active: oxplow's own is always right to show.
      setActiveExtension(null);
    }
  }, [capability]);
  const readOff = useCallback(async () => {
    try {
      setOff(await readReplacementsOff());
    } catch {
      setOff([]);
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

  const chosen =
    activeExtension === undefined || off === undefined || activeExtension === null || off.includes(target)
      ? null
      : (candidates.find((c) => c.extension === activeExtension) ?? null);
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

  // Usage is the evidence a kind needs to be promoted: once per replacement shown.
  const shown = ran !== null && ran.run !== null && ran.key === runKey ? ran.id : null;
  useEffect(() => {
    if (shown !== null) void recordUsage({ kind: "replacement", key: shown, streamId }).catch(() => {});
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [shown]);

  if (exts === null) return { state: "pending" };
  if (candidates.length === 0) return { state: "core" };
  if (activeExtension === undefined || off === undefined) return { state: "pending" };
  if (chosen === null) return { state: "core" };
  if (lens === null) return { state: "failed", replacement: chosen, message: `its lens ${chosen.lensId} didn't load` };
  if (ran === null || ran.key !== runKey) return { state: "pending" };
  if (ran.run === null) return { state: "failed", replacement: chosen, message: ran.error ?? "it failed" };
  return { state: "replaced", replacement: chosen, run: ran.run };
}
