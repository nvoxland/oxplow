import { useMemo, useState, type ReactNode } from "react";
import type { PageRun, SqlCell, VizRun } from "../tauri-bridge/generated/bindings.js";
import { useHistoryState } from "../tabs/useHistoryState.js";
import { SaveTempViewForm, SaveViewForm } from "./SaveViewForm.js";
import {
  displayTable, resolveTableDisplay, TABLE_DISPLAY, TABLE_PRESENTATIONS, TABLE_ROLES, tablePageYaml,
  type TableDisplay, type TablePresentation as Presentation,
} from "./tableDisplay.js";
import { TEMP_EXTENSION } from "./viewActions.js";

interface Props {
  id: string;
  run: PageRun;
  table: VizRun;
  loader: string;
  title: string;
  streamId: string | null;
  inlineYaml?: string;
  /** The params the person set on the page (not those it resolved). */
  given: Record<string, SqlCell>;
  onSaved?: (uri: string) => void;
  children: (view: VizRun) => ReactNode;
}

const labelOf = (viz: Presentation) => viz.charAt(0).toUpperCase() + viz.slice(1);

export function TablePresentation({ id, run, table, loader, title, streamId, inlineYaml, given, onSaved, children }: Props) {
  const [choice, choose] = useHistoryState(`show-as:${run.page.id}:${id}`, TABLE_DISPLAY);
  const display = useMemo(() => resolveTableDisplay(choice, table.result), [choice, table.result]);
  const [saving, setSaving] = useState<TableDisplay | null>(null);
  const columns = [...new Set(table.result.columns)];
  const canSave = Boolean(onSaved && columns.length);
  const temp = inlineYaml === undefined && run.page.origin.extension === TEMP_EXTENSION;
  const form = saving && onSaved ? { kind: "page" as const, initialTitle: `${title} · ${labelOf(saving.viz)}`, onClose: () => setSaving(null), onSaved } : null;
  const spec = (yaml?: string) => tablePageYaml({
    run, loader, display: saving ?? display, title, given, columns: table.config.columns,
    ...(yaml === undefined ? {} : { inlineYaml: yaml }),
  });
  return (
    <>
      <div className="ox-toolbar" data-testid={`show-as-${id}`} style={{ flexWrap: "wrap", gap: 8, marginBottom: 8 }}>
        <label>
          Show as{" "}
          <select className="ox-select" value={display.viz} onChange={(e) => {
            choose({ viz: e.target.value as Presentation, fields: {} });
            setSaving(null);
          }}>
            {TABLE_PRESENTATIONS.map((viz) => <option key={viz} value={viz}>{labelOf(viz)}</option>)}
          </select>
        </label>
        {TABLE_ROLES[display.viz].map(({ key, label, optional }) => (
          <label key={key}>
            {label}{" "}
            <select className="ox-select" value={display.fields[key] ?? ""} onChange={(e) => {
              choose({ ...choice, fields: { ...choice.fields, [key]: e.target.value } });
              setSaving(null);
            }}>
              {optional ? <option value="">None</option> : null}
              {columns.map((column) => (
                <option key={column} value={column}>{table.config.columns.find((c) => c.key === column)?.label ?? column}</option>
              ))}
            </select>
          </label>
        ))}
        {canSave ? <button type="button" title="Save this table as a page" onClick={() => setSaving(display)}>Save as Page</button> : null}
      </div>
      {form && (temp
        ? <SaveTempViewForm {...form} file={`${run.page.slug}.yaml`} streamId={streamId} transform={spec} />
        : <SaveViewForm {...form} specYaml={spec(inlineYaml)} />)}
      {children(displayTable(table, display))}
    </>
  );
}
