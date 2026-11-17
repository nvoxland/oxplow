import { useId, useState } from "react";
import type { VizRun } from "../api.js";
import { Markdown } from "../components/Markdown/Markdown.js";
import { DataTable } from "../kit/DataTable.js";
import { Facts } from "../kit/Facts.js";
import { Section } from "../kit/Section.js";
import { SettingEditor } from "./SettingEditor.js";
import { displayValue, editsInline, inventory, levelName, LEVELS, type Level, type Setting } from "./settingsModel.js";

/** Registry-driven settings composed from the same sections and kit as other
 *  pages. The page's query chooses the rows (category, search, changed). */
export function SettingsViz({ run }: { run: VizRun }) {
  const rows = inventory(run);
  // A page that states the level (`settings: { level }`) saves every change there.
  const fixedLevel = (run.config.settings?.level ?? null) as Level | null;
  return (
    <div data-testid="settings-viz">
      {!rows.length && <p className="ox-muted">No settings match these filters.</p>}
      {rows.map((setting) => (
        <SettingSection
          key={setting.key}
          id={`${run.config.id}/${setting.key}`}
          setting={setting}
          fixedLevel={fixedLevel}
        />
      ))}
    </div>
  );
}

function SettingSection({ setting, id, fixedLevel }: { setting: Setting; id: string; fixedLevel: Level | null }) {
  const levelId = useId();
  const [chosenLevel, setLevel] = useState<Level>(
    LEVELS.includes(setting.origin as Level) ? (setting.origin as Level) : "project",
  );
  const level = fixedLevel ?? chosenLevel;
  // A scalar is edited in its section; anything else opens its editor.
  const inline = editsInline(setting);
  const [editing, setEditing] = useState(inline);
  const effective = setting.unavailable ? "Unavailable" : setting.secret ? setting.isSet ? "•••••••• (set)" : "Not set" : displayValue(setting.value);
  const levelValue = (at: Level | "default") => {
    if (at === "default") return setting.secret ? "Not displayed" : displayValue(setting.defaultValue);
    if (setting.statuses[at] === "unavailable") return "Unavailable";
    if (setting.statuses[at] !== "set") return "Inherited";
    return setting.secret ? "•••••••• (set)" : displayValue(setting.overrides[at]);
  };
  const source = setting.unavailable
    ? "unavailable"
    : setting.origin
      ? levelName(setting.origin as Level | "default")
      : "not set";
  const levelSelect = (label: string) =>
    fixedLevel ? (
      <span>{levelName(fixedLevel)}</span>
    ) : (
    <select
      id={levelId}
      className="ox-select"
      aria-label={`${label} for ${setting.title}`}
      value={level}
      onChange={(e) => {
        setLevel(e.target.value as Level);
        setEditing(inline);
      }}
    >
      {LEVELS.map((l) => (
        <option key={l} value={l}>
          {levelName(l)}
        </option>
      ))}
    </select>
    );
  const changedFor = LEVELS.filter((l) => setting.statuses[l] === "set").map(levelName);
  const originIndex = setting.origin ? LEVELS.indexOf(setting.origin as Level) : -1;
  return (
    <Section id={id} title={setting.title}>
      <div className="ox-form">
        {setting.doc && <Markdown value={setting.doc} />}
        {changedFor.length > 0 && <p className="ox-muted">Changed for: {changedFor.join(", ")}</p>}
        <Facts
          facts={[
            {
              key: "effective",
              label: "Effective value",
              value: (
                <>
                  {effective}
                  <span className="ox-muted"> · Source: {source}</span>
                </>
              ),
            },
            ...(inline || fixedLevel
              ? []
              : [
                  {
                    key: "level",
                    label: <label htmlFor={levelId}>View/edit level</label>,
                    value: levelSelect("View/edit level"),
                  },
                ]),
          ]}
        />
        {originIndex >= 0 && originIndex < LEVELS.indexOf(level) && (
          <p className="ox-muted">
            The value for {levelName(setting.origin as Level)} stays in use here. A change for {levelName(level)} applies
            wherever that value is inherited.
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
                  at === "default" || fixedLevel ? (
                    levelName(at)
                  ) : (
                    <button
                      type="button"
                      className="ox-button"
                      onClick={() => {
                        setLevel(at);
                        setEditing(true);
                      }}
                    >
                      {levelName(at)}
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
              {
                key: "schema",
                label: "Schema",
                value: (
                  <pre>
                    <code>{JSON.stringify(setting.schema, null, 2)}</code>
                  </pre>
                ),
              },
            ]}
          />
        </details>
        {!inline && (
          <div className="ox-actions">
            <button className="ox-button" type="button" onClick={() => setEditing((v) => !v)}>
              {editing ? "Collapse editor" : `Edit value: ${levelName(level)}`}
            </button>
          </div>
        )}
        {/* Ordinary drafts stay mounted; secret drafts are dropped on collapse. */}
        <div hidden={!editing}>
          {(!setting.secret || editing) && (
            <SettingEditor
              key={`${setting.key}:${level}:${setting.secret ? "secret" : "ordinary"}`}
              setting={setting}
              level={level}
            />
          )}
          {fixedLevel ? (
            <p className="ox-muted">Saves to {levelName(fixedLevel)}</p>
          ) : (
            inline && (
              <label htmlFor={levelId} className="ox-muted">
                Saves to {levelSelect("Saves to")}
              </label>
            )
          )}
        </div>
      </div>
    </Section>
  );
}
