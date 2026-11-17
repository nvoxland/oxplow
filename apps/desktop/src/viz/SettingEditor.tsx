import { useState, type KeyboardEvent } from "react";
import { runCommand } from "../api.js";
import { CommandConfirm } from "../components/CommandConfirm.js";
import { InlineError } from "../kit/InlineError.js";
import { useDraft } from "../tabs/useViewParam.js";
import { useFormSubmit } from "./useFormSubmit.js";
import { SettingField } from "./SettingField.js";
import { displayValue, emptyValue, LEVELS, same, validate, type Level, type Setting } from "./settingsModel.js";

type Draft = { value: unknown; remove: boolean; baseline: string };

const fingerprint = (setting: Setting, level: Level) =>
  JSON.stringify([setting.statuses[level], setting.overrides[level]]);

const savedValue = (setting: Setting, level: Level): unknown => {
  for (const candidate of LEVELS.slice(LEVELS.indexOf(level))) {
    if (Object.hasOwn(setting.overrides, candidate)) return setting.overrides[candidate];
  }
  return setting.defaultValue !== undefined ? setting.defaultValue : emptyValue(setting.schema);
};

export function SettingEditor({ setting, level }: { setting: Setting; level: Level }) {
  // Secrets never enter tab history, including their baseline or the input's value.
  const [ordinary, setOrdinary, clearOrdinary] = useDraft<Draft | null>(`setting:${setting.key}:${level}`, null);
  const [secretDraft, setSecretDraft] = useState<Draft | null>(null);
  const [justSaved, setJustSaved] = useState<{ value: unknown; remove: boolean; over: string } | null>(null);
  const current = fingerprint(setting, level);
  const draft = setting.secret ? secretDraft : ordinary;
  const setDraft = (next: Draft | null) => (setting.secret ? setSecretDraft(next) : setOrdinary(next));
  const clear = () => {
    setSecretDraft(null);
    clearOrdinary();
  };
  const pendingSave = justSaved?.over === current ? justSaved : null;
  const baseValue = pendingSave ? pendingSave.value : setting.secret ? "" : savedValue(setting, level);
  const hasOverride = pendingSave ? !pendingSave.remove : setting.statuses[level] !== "unset";
  const value = draft ? draft.value : baseValue;
  const remove = draft?.remove ?? false;
  const dirty = !!draft && (remove ? hasOverride : setting.secret ? value !== "" : !hasOverride || !same(value, baseValue));
  const conflict = dirty && draft.baseline !== current;
  const errors = remove || !dirty ? [] : validate(setting.schema, value);
  const command = remove ? "oxplow.config.unset" : "oxplow.config.set";
  const form = useFormSubmit(
    `Save ${setting.title}`,
    (input, confirmed) => runCommand(String(input.command), input.payload, confirmed),
    () => {
      setJustSaved({ value: setting.secret ? "" : value, remove, over: current });
      clear();
    },
    `setting/${setting.key}/${level}`,
  );
  const change = (next: unknown) => setDraft({ value: next, remove: false, baseline: draft?.baseline ?? current });
  const submit = () => {
    if (dirty && !errors.length && !form.busy) {
      void form.submit({ command, payload: { key: setting.key, level, ...(remove ? {} : { value }) } }, false);
    }
  };
  const inherited = (): unknown => {
    const after = LEVELS.slice(LEVELS.indexOf(level) + 1);
    for (const lower of after) {
      if (setting.statuses[lower] === "unavailable") return "Unavailable (could not read inherited value)";
      if (setting.statuses[lower] === "set") return setting.secret ? "•••••••• (set)" : setting.overrides[lower];
    }
    return setting.defaultValue;
  };
  const multiline =
    setting.ui.multiline === true ||
    setting.ui.editor === "multiline" ||
    setting.ui.widget === "textarea" ||
    setting.schema.contentMediaType === "text/markdown";
  const inheritedText = setting.secret
    ? setting.statuses[level] === "unavailable"
      ? "Unavailable"
      : String(inherited() ?? "Not set")
    : displayValue(inherited());
  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key === "Escape") {
      e.stopPropagation();
      clear();
    }
    if (e.key === "Enter" && e.target instanceof HTMLTextAreaElement) {
      if (e.metaKey || e.ctrlKey) {
        e.preventDefault();
        submit();
      }
    } else if (e.key === "Enter" && e.target instanceof HTMLInputElement) {
      e.preventDefault();
      submit();
    }
  };
  return (
    <form
      className="ox-form"
      onSubmit={(e) => {
        e.preventDefault();
        submit();
      }}
      onKeyDown={onKeyDown}
    >
      <p className="ox-muted">
        Editing {level}. {hasOverride ? "An override is saved here." : "This level inherits its value. Saving creates an override."}
        {setting.secret && " Leave blank to keep the saved secret."}
      </p>
      {remove ? (
        <p role="status">On Save, remove this override. This level will inherit: {inheritedText}.</p>
      ) : (
        <SettingField
          schema={setting.schema}
          value={value}
          onChange={change}
          label={setting.title}
          secret={setting.secret}
          multiline={multiline}
        />
      )}
      {conflict && (
        <p role="alert" className="ox-warning">
          The saved value changed while you were editing. Your draft is kept. Save replaces the current {level} value; Discard loads the latest value.
        </p>
      )}
      {errors.map((error) => (
        <p key={error} className="ox-error" role="alert">
          {error}
        </p>
      ))}
      {/* A refusal's message says where in the value and why. */}
      {form.serverErrors &&
        Object.entries(form.serverErrors).map(([field, error]) => (
          <p key={field} className="ox-error" role="alert">
            {error}
          </p>
        ))}
      <div className="ox-actions">
        <button className="ox-button ox-primary" type="submit" disabled={!dirty || !!errors.length || form.busy}>
          Save
        </button>
        <button className="ox-button" type="button" disabled={!draft || form.busy} onClick={clear}>
          Discard
        </button>
        <button
          className="ox-button"
          type="button"
          disabled={!hasOverride || remove || form.busy}
          onClick={() => setDraft({ value: baseValue, remove: true, baseline: draft?.baseline ?? current })}
        >
          Reset to inherited value
        </button>
      </div>
      {form.failure && <InlineError failure={form.failure} />}
      {form.pending && (
        <CommandConfirm
          label={`Save ${setting.title}`}
          command={String(form.pending.command)}
          details={form.pendingDetails}
          onConfirm={() => void form.submit(form.pending!, true)}
          onCancel={form.cancel}
        />
      )}
    </form>
  );
}
