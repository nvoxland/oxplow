/// What every page shows the same way: the rail's Selection (what a row
/// click picked, never an input), and About this page (what it is for,
/// its extension, the source file to edit, whether that source changed
/// since it was deployed), which its ☰ opens.
import { useEffect, useState } from "react";

import { querySql, type SqlCell } from "../api.js";
import { Facts } from "../kit/Facts.js";
import { humanize } from "../kit/humanize.js";
import { RailGroup } from "../kit/RailGroup.js";
import { fileRef, settingsRef } from "../tabs/pageRefs.js";
import type { TabRef } from "../tabs/tabState.js";
import type { PageParam, PageProvenance } from "../tauri-bridge/generated/bindings.js";

/** The row a click picked for a selection param: its value and what the
 *  row showed, labelled as its columns are. */
export interface PickedRow {
  value: SqlCell;
  facts: { label: string; text: string }[];
}

export function RailSelection({
  params,
  values,
  picked,
  onApply,
  testPrefix,
}: {
  /** The page's selection params. */
  params: PageParam[];
  values: Record<string, SqlCell>;
  picked: Record<string, PickedRow>;
  onApply(name: string, value: SqlCell): void;
  /** The test ids' prefix. */
  testPrefix: string;
}) {
  if (params.length === 0) return null;
  return (
    <RailGroup title="Selection" testId={`${testPrefix}-rail-selection`}>
      {params.map((p) => {
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
          <div key={p.name} data-testid={`${testPrefix}-selection-${p.name}`}>
            <Facts facts={facts.map((f, i) => ({ key: String(i), label: f.label, value: f.text }))} />
            <button type="button" className="ox-link ox-params-more" data-testid={`${testPrefix}-selection-${p.name}-clear`} onClick={() => onApply(p.name, null)}>
              Clear
            </button>
          </div>
        );
      })}
    </RailGroup>
  );
}

export function PageAbout({
  description,
  origin,
  outOfDate,
  onOpenPage,
  testPrefix,
}: {
  /** What the page is for (its `description`). */
  description: string;
  origin: PageProvenance;
  /** Its source changed after the deployment that runs. */
  outOfDate: boolean;
  onOpenPage?(ref: TabRef): void;
  testPrefix: string;
}) {
  return (
    <div data-testid={`${testPrefix}-about`}>
      {description ? <p className="ox-rail-line">{description}</p> : null}
      <Facts
        facts={[
          {
            key: "extension",
            label: "Extension",
            value: onOpenPage ? (
              <button type="button" className="ox-link ox-params-more" data-testid={`${testPrefix}-about-extension`} onClick={() => onOpenPage(settingsRef("extensions"))}>
                {origin.extension}
              </button>
            ) : (
              origin.extension
            ),
          },
          {
            key: "source",
            label: "Source",
            value:
              origin.source && onOpenPage ? (
                <button type="button" className="ox-link ox-params-more ox-mono" data-testid={`${testPrefix}-about-source`} onClick={() => onOpenPage(fileRef(origin.source!))}>
                  {origin.source}
                </button>
              ) : origin.source ? (
                origin.source
              ) : origin.bundled ? (
                "Ships with oxplow"
              ) : (
                "Not on this machine"
              ),
          },
        ]}
      />
      {outOfDate ? (
        <p className="ox-rail-line" data-testid={`${testPrefix}-about-out-of-date`}>
          The source changed since it was deployed; deploy it to see the change.
        </p>
      ) : null}
    </div>
  );
}

/** Whether an extension's source changed after the deployment that runs:
 *  its live source hash against the deployed one. False with no
 *  deployment, or when it can't be read. */
export function useOutOfDate(extension: string | null, deployment: string | null): boolean {
  const [out, setOut] = useState(false);
  useEffect(() => {
    setOut(false);
    if (!extension || !deployment) return;
    let live = true;
    querySql("SELECT hash FROM v_extension_source WHERE name = ?1", [extension])
      .then((r) => {
        const hash = r.rows[0]?.[0];
        if (live) setOut(typeof hash === "string" && hash !== deployment);
      })
      .catch(() => {});
    return () => {
      live = false;
    };
  }, [extension, deployment]);
  return out;
}
