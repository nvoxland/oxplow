import { useId, useState } from "react";
import type { VizRun } from "../api.js";
import { Markdown } from "../components/Markdown/Markdown.js";
import { DataTable } from "../kit/DataTable.js";
import { Facts } from "../kit/Facts.js";
import { Section } from "../kit/Section.js";
import { SettingEditor } from "./SettingEditor.js";
import { displayValue, inventory, LEVELS, type Level, type Setting } from "./settingsModel.js";

/** Registry-driven settings composed from the same sections and kit as other pages. */
export function SettingsViz({ run }: { run: VizRun }) {
  const category = String(run.params.category ?? "");
  const q = String(run.params.q ?? "").trim().toLowerCase();
  const selected = String(run.params.setting ?? "");
  const rows = inventory(run).filter(
    (s) =>
      (!category || category === "all" || s.category === category) &&
      (!q || `${s.title} ${s.doc} ${s.key}`.toLowerCase().includes(q)) &&
      (!selected || s.key === selected),
  );
  return (
    <div data-testid="settings-viz">
      {!rows.length && <p className="ox-muted">No settings match these filters.</p>}
      {rows.map((setting) => (
        <SettingSection key={setting.key} id={`${run.config.id}/${setting.key}`} setting={setting} />
      ))}
    </div>
  );
}

function SettingSection({ setting, id }: { setting: Setting; id: string }) {
  const levelId = useId();
  const [level, setLevel] = useState<Level>(
    LEVELS.includes(setting.origin as Level) ? (setting.origin as Level) : "project",
  );
  const [editing, setEditing] = useState(false);
  const effective = setting.unavailable ? "Unavailable" : setting.secret ? setting.isSet ? "•••••••• (set)" : "Not set" : displayValue(setting.value);
  const levelValue = (at: Level | "default") => {
    if (at === "default") return setting.secret ? "Not displayed" : displayValue(setting.defaultValue);
    if (setting.statuses[at] === "unavailable") return "Unavailable";
    if (setting.statuses[at] !== "set") return "Inherited";
    return setting.secret ? "•••••••• (set)" : displayValue(setting.overrides[at]);
  };
  const levelLabel = (l: Level) =>
    l === "local" ? "Just me in this project" : l === "project" ? "This project" : "All my projects";
  const originIndex = setting.origin ? LEVELS.indexOf(setting.origin as Level) : -1;
  return (
    <Section id={id} title={setting.title}>
      <div className="ox-form">
        {setting.doc && <Markdown value={setting.doc} />}
        {setting.restartRequired && <p className="ox-muted">Restart required</p>}
        <Facts
          facts={[
            {
              key: "effective",
              label: "Effective value",
              value: (
                <>
                  {effective}
                  <span className="ox-muted"> · Source: {setting.unavailable ? "unavailable" : setting.origin ?? "not set"}</span>
                </>
              ),
            },
            {
              key: "level",
              label: <label htmlFor={levelId}>View/edit level</label>,
              value: (
                <select
                  id={levelId}
                  className="ox-select"
                  aria-label={`View/edit level for ${setting.title}`}
                  value={level}
                  onChange={(e) => {
                    setLevel(e.target.value as Level);
                    setEditing(false);
                  }}
                >
                  {LEVELS.map((l) => (
                    <option key={l} value={l}>
                      {levelLabel(l)}
                    </option>
                  ))}
                </select>
              ),
            },
          ]}
        />
        {originIndex >= 0 && originIndex < LEVELS.indexOf(level) && (
          <p className="ox-muted">
            The {setting.origin} override remains in use here. Changes at {level} apply wherever that value is inherited.
          </p>
        )}
        <details>
          <summary>All levels</summary>
          <DataTable<Level | "default">
            rows={[...LEVELS, "default"]}
            rowKey={(at) => at}
            columns={[
              {
                key: "level",
                label: "Level",
                render: (at) =>
                  at === "default" ? (
                    "Default"
                  ) : (
                    <button
                      type="button"
                      className="ox-button"
                      onClick={() => {
                        setLevel(at);
                        setEditing(true);
                      }}
                    >
                      {at}
                    </button>
                  ),
              },
              { key: "value", label: "Value", render: levelValue },
            ]}
          />
        </details>
        <details>
          <summary>Technical details</summary>
          <Facts
            facts={[
              { key: "key", label: "Key", value: <code>{setting.key}</code> },
              { key: "owner", label: "Owner", value: setting.owner },
              { key: "category", label: "Category", value: setting.category },
              { key: "schema", label: "Schema", value: displayValue(setting.schema) },
            ]}
          />
        </details>
        <div className="ox-actions">
          <button className="ox-button" type="button" onClick={() => setEditing((v) => !v)}>
            {editing ? "Collapse editor" : `Edit ${level} value`}
          </button>
        </div>
        {/* Ordinary drafts stay mounted; secret drafts are dropped on collapse. */}
        <div hidden={!editing}>
          {(!setting.secret || editing) && (
            <SettingEditor
              key={`${setting.key}:${level}:${setting.secret ? "secret" : "ordinary"}`}
              setting={setting}
              level={level}
            />
          )}
        </div>
      </div>
    </Section>
  );
}
