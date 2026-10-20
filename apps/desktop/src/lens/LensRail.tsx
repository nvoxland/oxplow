import type { SqlCell } from "../api.js";
import { Facts } from "../kit/Facts.js";
import { humanize } from "../kit/humanize.js";
import { isDefault, ParamList } from "../kit/ParamControls.js";
import { RailGroup } from "../kit/RailGroup.js";
import { fileRef, settingsRef } from "../tabs/pageRefs.js";
import type { TabRef } from "../tabs/tabState.js";
import type { Lens } from "../tauri-bridge/generated/bindings.js";

/** The row a click picked for a selection param: its value and what the
 *  row showed, labelled as its columns are. */
export interface PickedRow {
  value: SqlCell;
  facts: { label: string; text: string }[];
}

export interface LensRailProps {
  lens: Lens;
  values: Record<string, SqlCell>;
  /** The rows picked for its selection params, by param. */
  picked: Record<string, PickedRow>;
  /** Its source changed after the deployment that runs. */
  outOfDate: boolean;
  onApply(name: string, value: SqlCell): void;
  onOpenPage(ref: TabRef): void;
}

/**
 * A lens page's rail, the one place its controls are: Filters (its filter
 * params as rows), Selection (what a row click picked, never an input),
 * and About this page (its extension and the source file to edit).
 */
export function LensRail({ lens, values, picked, outOfDate, onApply, onOpenPage }: LensRailProps) {
  const filters = lens.params.filter((p) => p.role !== "selection");
  const selections = lens.params.filter((p) => p.role === "selection");
  const changed = filters.filter((p) => !isDefault(p, values[p.name] ?? null));
  const reset = changed.length > 0 ? { id: "lens-rail-reset", label: "Reset", title: "Put every filter back to its default", run: () => changed.forEach((p) => onApply(p.name, p.default ?? null)) } : undefined;
  const { origin } = lens;
  return (
    <>
      {filters.length > 0 ? (
        <RailGroup title="Filters" action={reset} testId="lens-rail-filters">
          <ParamList params={filters} values={values} onApply={onApply} />
        </RailGroup>
      ) : null}
      {selections.length > 0 ? (
        <RailGroup title="Selection" testId="lens-rail-selection">
          {selections.map((p) => {
            const value = values[p.name] ?? null;
            const label = p.label ?? humanize(p.name);
            if (value === null) {
              return (
                <p key={p.name} className="ox-rail-line">
                  Click a row to select it.
                </p>
              );
            }
            const row = picked[p.name];
            const facts = row && String(row.value) === String(value) ? row.facts : [{ label, text: String(value) }];
            return (
              <div key={p.name} data-testid={`lens-selection-${p.name}`}>
                <Facts facts={facts.map((f, i) => ({ key: String(i), label: f.label, value: f.text }))} />
                <button type="button" className="ox-link ox-params-more" data-testid={`lens-selection-${p.name}-clear`} onClick={() => onApply(p.name, null)}>
                  Clear
                </button>
              </div>
            );
          })}
        </RailGroup>
      ) : null}
      <RailGroup title="About this page" testId="lens-rail-about">
        <Facts
          facts={[
            {
              key: "extension",
              label: "Extension",
              value: (
                <button type="button" className="ox-link ox-params-more" data-testid="lens-about-extension" onClick={() => onOpenPage(settingsRef("extensions"))}>
                  {origin.extension}
                </button>
              ),
            },
            {
              key: "source",
              label: "Source",
              value: origin.source ? (
                <button type="button" className="ox-link ox-params-more ox-mono" data-testid="lens-about-source" onClick={() => onOpenPage(fileRef(origin.source!))}>
                  {origin.source}
                </button>
              ) : origin.bundled ? (
                "Ships with oxplow"
              ) : (
                "Not on this machine"
              ),
            },
          ]}
        />
        {outOfDate ? (
          <p className="ox-rail-line" data-testid="lens-about-out-of-date">
            The source changed since it was deployed; deploy it to see the change.
          </p>
        ) : null}
      </RailGroup>
    </>
  );
}
