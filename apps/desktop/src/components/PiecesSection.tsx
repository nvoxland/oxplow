/// Settings → Pieces: for each capability a project chooses an
/// implementation of (the work list, the effort policy, snapshots), its
/// choices as `v_capability_provider` lists them, which is active and
/// why, the project's default, and a person's own choice ("Just for me",
/// `.oxplow/personal.yaml`). `activeProviders` is a person-only key, so the
/// click is the confirmation `oxplow.config.set` asks for. Choosing "none" says
/// what it turns off. See `.context/work-tracking.md` "Swappable pieces".
///
/// Usability contract (.context/usability.md): a choice applies at once,
/// failures go to opErrorsStore.

import type { CSSProperties } from "react";
import { useCallback, useEffect, useState } from "react";

import { effectiveConfig, listExtensions, querySql, runCommand, subscribeOxplowEvents } from "../api.js";
import { NO_READS, useRerunOnChange } from "../lens/lensRerun.js";
import type { Extension, Reads } from "../tauri-bridge/generated/bindings.js";
import { chosenNote, nextChoices, offWithout, piecesFromResult, type Piece } from "./piecesModel.js";
import { recordOpError } from "./opErrorsStore.js";

const QUERY =
  "SELECT capability, provider, extension, features, active, title, source, available, chosen_by, " +
  "capability_title, choosable, optional FROM v_capability_provider ORDER BY capability, provider";

type Layer = "project" | "personal";

const asChoices = (value: unknown): Record<string, string> =>
  Object.fromEntries(
    Object.entries((value ?? {}) as Record<string, unknown>).filter((e): e is [string, string] => typeof e[1] === "string"),
  );

export function PiecesSection() {
  const [pieces, setPieces] = useState<Piece[] | null>(null);
  const [reads, setReads] = useState<Reads>(NO_READS);
  const [chosen, setChosen] = useState<Record<Layer, Record<string, string>>>({ project: {}, personal: {} });
  const [extensions, setExtensions] = useState<Extension[]>([]);

  const load = useCallback(async () => {
    try {
      const [res, settings, exts] = await Promise.all([querySql(QUERY, [], 500), effectiveConfig(), listExtensions()]);
      setPieces(piecesFromResult(res));
      setReads(res.reads);
      setExtensions(exts);
      setChosen({
        project: asChoices(settings.find((s) => s.key === "activeProviders" && s.origin === "project")?.value),
        personal: asChoices(settings.find((s) => s.key === "personal.activeProviders")?.value),
      });
    } catch (e) {
      recordOpError({ label: "Read the pieces", message: e instanceof Error ? e.message : String(e) });
      setPieces([]);
    }
  }, []);

  useEffect(() => {
    void load();
    return subscribeOxplowEvents((event) => {
      if (event.kind === "configChanged") void load();
    });
  }, [load]);
  useRerunOnChange(reads, () => void load());

  async function choose(layer: Layer, capability: string, id: string | null) {
    const next = nextChoices(chosen[layer], capability, id);
    const where = layer === "personal" ? { layer: "personal" } : {};
    try {
      if (next === null) {
        await runCommand("oxplow.config.unset", { key: "activeProviders", ...where }, true);
      } else {
        await runCommand("oxplow.config.set", { key: "activeProviders", value: next, ...where }, true);
      }
      setChosen({ ...chosen, [layer]: next ?? {} });
    } catch (e) {
      recordOpError({ label: "Choose an implementation", message: e instanceof Error ? e.message : String(e) });
    }
  }

  if (pieces === null) return <div style={mutedStyle}>Loading…</div>;
  return (
    <div data-testid="pieces-section" style={{ display: "flex", flexDirection: "column", gap: 16 }}>
      {pieces.map((p) => {
        const active = p.choices.find((c) => c.id === p.active);
        const off = p.optional ? offWithout(p.capability, extensions) : [];
        return (
          <section key={p.capability} data-testid={`pieces-${p.capability}`}>
            <h4 style={{ margin: "0 0 4px" }}>{p.title}</h4>
            <div data-testid={`pieces-${p.capability}-status`} style={mutedStyle}>
              {active?.title ?? p.active} — {chosenNote(p)}
            </div>
            <fieldset style={fieldsetStyle}>
              <legend style={mutedStyle}>The project's choice</legend>
              <label style={rowStyle}>
                <input
                  type="radio"
                  name={`pieces-${p.capability}-project`}
                  data-testid={`pieces-${p.capability}-project-default`}
                  checked={chosen.project[p.capability] === undefined}
                  onChange={() => void choose("project", p.capability, null)}
                />
                The default
              </label>
              {p.choices.map((c) => (
                <label key={c.id} style={rowStyle}>
                  <input
                    type="radio"
                    name={`pieces-${p.capability}-project`}
                    data-testid={`pieces-${p.capability}-project-${c.id}`}
                    checked={chosen.project[p.capability] === c.id}
                    onChange={() => void choose("project", p.capability, c.id)}
                  />
                  {c.title}
                  {c.features.length > 0 ? <span style={mutedStyle}>· {c.features.join(", ")}</span> : null}
                  {c.source === "none" && off.length > 0 ? (
                    <span style={mutedStyle} data-testid={`pieces-${p.capability}-off`}>
                      · turns off {off.join(", ")}
                    </span>
                  ) : null}
                </label>
              ))}
            </fieldset>
            <label style={rowStyle}>
              <span style={mutedStyle}>Just for me</span>
              <select
                data-testid={`pieces-${p.capability}-personal`}
                value={chosen.personal[p.capability] ?? ""}
                onChange={(e) => void choose("personal", p.capability, e.target.value === "" ? null : e.target.value)}
              >
                <option value="">The same as the project</option>
                {p.choices.map((c) => (
                  <option key={c.id} value={c.id}>
                    {c.title}
                  </option>
                ))}
              </select>
            </label>
          </section>
        );
      })}
    </div>
  );
}

const mutedStyle: CSSProperties = { fontSize: "var(--text-xs)", color: "var(--text-secondary)" };
const rowStyle: CSSProperties = { display: "flex", gap: 6, alignItems: "center" };
const fieldsetStyle: CSSProperties = {
  border: "1px solid var(--border-subtle)",
  borderRadius: 4,
  padding: "6px 10px",
  margin: "6px 0",
};
