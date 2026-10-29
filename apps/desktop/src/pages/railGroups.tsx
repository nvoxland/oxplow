/// What every page shows the same way: the rail's Selection (what a row
/// click picked, never an input), and About this page (what it is for,
/// its extension, the source file to edit, whether that source changed
/// since it was deployed), which its ☰ opens.
import { useEffect, useState } from "react";

import { querySql, type SqlCell } from "../api.js";
import { Facts, type Fact } from "../kit/Facts.js";
import { paramOf } from "../uri/uri.js";
import { definitionUri } from "./viewActions.js";
import { humanize } from "../kit/humanize.js";
import { RailGroup } from "../kit/RailGroup.js";
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
  id,
  uri,
  description,
  origin,
  params,
  outOfDate,
  onOpenPage,
  testPrefix,
}: {
  /** The page's id (`<extension>/<slug>`). */
  id: string;
  /** The URI the tab shows, when it is in one. */
  uri?: string | null;
  /** What the page is for (its `description`). */
  description: string;
  origin: PageProvenance;
  /** The run's params: Source opens Definition with them. */
  params: Record<string, SqlCell>;
  /** Its source changed after the deployment that runs. */
  outOfDate: boolean;
  onOpenPage?(ref: TabRef): void;
  testPrefix: string;
}) {
  // The same facts Definition's About opens with: its source is the copy
  // that runs, drawn as Definition with the run's params — never the file to edit.
  const running = origin.runningUri;
  const sourcePath = running ? paramOf(running, "path") : null;
  const link = (testId: string, text: string, ref: TabRef) =>
    onOpenPage ? (
      <button type="button" className="ox-link ox-fact-link" data-testid={testId} onClick={() => onOpenPage(ref)}>
        {text}
      </button>
    ) : (
      <span data-testid={testId}>{text}</span>
    );
  const facts: Fact[] = [
    { key: "id", label: "Id", value: id },
  ];
  if (running && sourcePath) {
    facts.push({ key: "definition", label: "Definition", value: link(`${testPrefix}-about-definition`, sourcePath, { id: definitionUri(running, params) }) });
  }
  facts.push(
    { key: "origin", label: "Origin", value: origin.bundled ? "ships with oxplow" : "yours" },
    { key: "runs", label: "Runs", value: origin.deployment ? `deployment ${origin.deployment.slice(0, 12)}` : "loaded from its source" },
  );
  if (uri) facts.push({ key: "uri", label: "URI", value: <span className="ox-mono" data-testid={`${testPrefix}-about-uri`}>{uri}</span> });
  return (
    <div data-testid={`${testPrefix}-about`}>
      {description ? <p className="ox-about-description">{description}</p> : null}
      <Facts facts={facts} />
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
