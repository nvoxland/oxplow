import { useState } from "react";
import type { VizRun } from "../api.js";
import { runCommand } from "../api.js";
import { Markdown } from "../components/Markdown/Markdown.js";
import { CommandConfirm } from "../components/CommandConfirm.js";
import { InlineError } from "../kit/InlineError.js";
import { useDraft } from "../tabs/useViewParam.js";
import { useFormSubmit } from "./useFormSubmit.js";
import { SettingField } from "./SettingField.js";
import { displayValue, emptyValue, inventory, LEVELS, same, validate, type Level, type Setting } from "./settingsModel.js";

type Draft = { value: unknown; remove: boolean; baseline: string };
const fingerprint = (setting: Setting, level: Level) => JSON.stringify([setting.statuses[level], setting.overrides[level]]);
const savedValue = (setting: Setting, level: Level): unknown => {
  for (const candidate of LEVELS.slice(LEVELS.indexOf(level))) {
    if (Object.hasOwn(setting.overrides, candidate)) return setting.overrides[candidate];
  }
  return setting.defaultValue !== undefined ? setting.defaultValue : emptyValue(setting.schema);
};

export function SettingsViz({ run }: { run: VizRun }) {
  const category = String(run.params.category ?? "");
  const q = String(run.params.q ?? "").trim().toLowerCase();
  const selected = String(run.params.setting ?? "");
  const rows = inventory(run).filter((s) => (!category || category === "all" || s.category === category) && (!q || `${s.title} ${s.doc} ${s.key}`.toLowerCase().includes(q)) && (!selected || s.key === selected));
  return <div data-testid="settings-viz">{!rows.length && <p className="ox-muted">No settings match these filters.</p>}{rows.map((setting) => <SettingSection key={setting.key} setting={setting} />)}</div>;
}

function SettingSection({ setting }: { setting: Setting }) {
  const [level, setLevel] = useState<Level>(LEVELS.includes(setting.origin as Level) ? setting.origin as Level : "project");
  const [editing, setEditing] = useState(false);
  return <section aria-label={setting.title} style={{ marginBottom: 28 }}>
    <h3>{setting.title}</h3>
    {setting.doc && <Markdown value={setting.doc} />}
    {setting.restartRequired && <p className="ox-muted">Restart required</p>}
    <p><strong>Effective value: </strong>{setting.unavailable ? "Unavailable" : setting.secret ? setting.isSet ? "•••••••• (set)" : "Not set" : displayValue(setting.value)}<span className="ox-muted"> · Source: {setting.unavailable ? "unavailable" : setting.origin ?? "not set"}</span></p>
    <label>View/edit level <select className="ox-input" aria-label={`View/edit level for ${setting.title}`} value={level} onChange={(e) => { setLevel(e.target.value as Level); setEditing(false); }}>{LEVELS.map((l) => <option key={l} value={l}>{l === "local" ? "Local (this machine and project)" : l === "project" ? "Project (shared)" : "Global (all projects)"}</option>)}</select></label>
    {setting.origin && LEVELS.indexOf(setting.origin as Level) >= 0 && LEVELS.indexOf(setting.origin as Level) < LEVELS.indexOf(level) && <p className="ox-muted">The {setting.origin} override remains in use here. Changes at {level} apply wherever that value is inherited.</p>}
    <details style={{ marginBlock: 10 }}><summary>All levels</summary><table className="ox-table"><thead><tr><th>Level</th><th>Value</th></tr></thead><tbody>{LEVELS.map((l) => <tr key={l}><td><button type="button" className="ox-button" onClick={() => { setLevel(l); setEditing(true); }}>{l}</button></td><td>{setting.statuses[l] === "unavailable" ? "Unavailable" : setting.statuses[l] !== "set" ? "Inherited" : setting.secret ? "•••••••• (set)" : displayValue(setting.overrides[l])}</td></tr>)}<tr><td>Default</td><td>{setting.secret ? "Not displayed" : displayValue(setting.defaultValue)}</td></tr></tbody></table></details>
    <details><summary>Technical details</summary><p>Key: <code>{setting.key}</code></p><p>Owner: {setting.owner}</p><p>Category: {setting.category}</p><p>Schema: {displayValue(setting.schema)}</p></details>
    <button className="ox-button" type="button" onClick={() => setEditing((v) => !v)}>{editing ? "Collapse editor" : `Edit ${level} value`}</button>
    {/* Keep an ordinary editor mounted while collapsed: drafts survive data refreshes. */}
    <div hidden={!editing}>{(!setting.secret || editing) && <SettingEditor key={`${setting.key}:${level}:${setting.secret ? "secret" : "ordinary"}`} setting={setting} level={level} />}</div>
  </section>;
}

function SettingEditor({ setting, level }: { setting: Setting; level: Level }) {
  // Secrets never enter tab history, including their baseline or the input's value.
  const [ordinary, setOrdinary, clearOrdinary] = useDraft<Draft | null>(`setting:${setting.key}:${level}`, null);
  const [secretDraft, setSecretDraft] = useState<Draft | null>(null);
  const [justSaved, setJustSaved] = useState<{ value: unknown; remove: boolean; over: string } | null>(null);
  const current = fingerprint(setting, level);
  const draft = setting.secret ? secretDraft : ordinary;
  const setDraft = (next: Draft | null) => setting.secret ? setSecretDraft(next) : setOrdinary(next);
  const clear = () => { setSecretDraft(null); clearOrdinary(); };
  const pendingSave = justSaved?.over === current ? justSaved : null;
  const baseValue = pendingSave ? pendingSave.value : setting.secret ? "" : savedValue(setting, level);
  const hasOverride = pendingSave ? !pendingSave.remove : setting.statuses[level] !== "unset";
  const value = draft ? draft.value : baseValue;
  const remove = draft?.remove ?? false;
  const dirty = !!draft && (remove ? hasOverride : setting.secret ? value !== "" : !hasOverride || !same(value, baseValue));
  const conflict = dirty && draft.baseline !== current;
  const errors = remove || !dirty ? [] : validate(setting.schema, value);
  const command = remove ? "oxplow.config.unset" : "oxplow.config.set";
  const form = useFormSubmit(`Save ${setting.title}`, (input, confirmed) => runCommand(String(input.command), input.payload, confirmed), () => {
    setJustSaved({ value: setting.secret ? "" : value, remove, over: current });
    clear();
  }, `setting/${setting.key}/${level}`);
  const change = (next: unknown) => setDraft({ value: next, remove: false, baseline: draft?.baseline ?? current });
  const submit = () => { if (dirty && !errors.length && !form.busy) void form.submit({ command, payload: { key: setting.key, level, ...(remove ? {} : { value }) } }, false); };
  const inherited = (): unknown => {
    const after = LEVELS.slice(LEVELS.indexOf(level) + 1);
    for (const lower of after) {
      if (setting.statuses[lower] === "unavailable") return "Unavailable (could not read inherited value)";
      if (setting.statuses[lower] === "set") return setting.secret ? "•••••••• (set)" : setting.overrides[lower];
    }
    return setting.defaultValue;
  };
  return <form style={{ marginTop: 12, display: "grid", gap: 10 }} onSubmit={(e) => { e.preventDefault(); submit(); }} onKeyDown={(e) => {
    if (e.key === "Escape") { e.stopPropagation(); clear(); }
    if (e.key === "Enter" && e.target instanceof HTMLTextAreaElement) { if (e.metaKey || e.ctrlKey) { e.preventDefault(); submit(); } }
    else if (e.key === "Enter" && e.target instanceof HTMLInputElement) { e.preventDefault(); submit(); }
  }}>
    <p className="ox-muted">Editing {level}. {hasOverride ? "An override is saved here." : "This level inherits its value. Saving creates an override."}{setting.secret && " Leave blank to keep the saved secret."}</p>
    {remove ? <p role="status">On Save, remove this override. This level will inherit: {setting.secret ? setting.statuses[level] === "unavailable" ? "Unavailable" : String(inherited() ?? "Not set") : displayValue(inherited())}.</p> : <SettingField schema={setting.schema} value={value} onChange={change} label={setting.title} secret={setting.secret} multiline={setting.ui.multiline === true || setting.ui.editor === "multiline" || setting.ui.widget === "textarea" || setting.schema.contentMediaType === "text/markdown"} />}
    {conflict && <p role="alert" className="ox-warning">The saved value changed while you were editing. Your draft is kept. Save replaces the current {level} value; Discard loads the latest value.</p>}
    {errors.map((error) => <p key={error} className="ox-error" role="alert">{error}</p>)}
    {form.serverErrors && Object.entries(form.serverErrors).map(([field, error]) => <p key={field} className="ox-error" role="alert">{field}: {error}</p>)}
    <div style={{ display: "flex", gap: 8 }}><button className="ox-button ox-button-primary" type="submit" disabled={!dirty || !!errors.length || form.busy}>Save</button><button className="ox-button" type="button" disabled={!draft || form.busy} onClick={clear}>Discard</button><button className="ox-button" type="button" disabled={!hasOverride || remove || form.busy} onClick={() => setDraft({ value: baseValue, remove: true, baseline: draft?.baseline ?? current })}>Reset to inherited value</button></div>
    {form.failure && <InlineError failure={form.failure} />}
    {form.pending && <CommandConfirm label={`Save ${setting.title}`} command={String(form.pending.command)} details={form.pendingDetails} onConfirm={() => void form.submit(form.pending!, true)} onCancel={form.cancel} />}
  </form>;
}
