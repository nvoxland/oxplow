import type { CSSProperties } from "react";
import { useState } from "react";

import { InlineConfirm } from "./InlineConfirm.js";
import { recordOpError } from "../opErrors.js";
import { showToast } from "./toastStore.js";

/// One declared credential — an extension's (its collectors share them)
/// or a provider instance's own: set or replace its value (it goes to the
/// OS keychain and never comes back), or clear it. `owner` is whose it is
/// (the extension, or `<extension>/<instance id>`) and `store` writes it
/// there.
export function CredentialRow({
  owner,
  name,
  set,
  store,
  onChanged,
}: {
  owner: string;
  name: string;
  set: boolean;
  /** Save `value` (null: forget it) where this credential lives. */
  store(value: string | null): Promise<unknown>;
  onChanged(): void;
}) {
  const [value, setValue] = useState("");
  const [saving, setSaving] = useState(false);
  const id = `${owner}-${name}`;

  async function save(v: string | null) {
    setSaving(true);
    try {
      await store(v);
      setValue("");
      showToast({ message: v ? `Saved ${name} to your keychain.` : `Cleared ${name}.` });
      onChanged();
    } catch (e) {
      recordOpError({ label: `Set credential ${name}`, message: String(e) });
    } finally {
      setSaving(false);
    }
  }

  return (
    <form
      data-testid={`credential-${id}`}
      style={{ display: "flex", alignItems: "center", gap: 8, marginTop: 4 }}
      onSubmit={(e) => {
        e.preventDefault();
        if (value.trim()) void save(value.trim());
      }}
    >
      <code>{name}</code>
      <span style={mutedStyle}>{set ? "Saved in keychain" : "Not set"}</span>
      <input
        data-testid={`credential-input-${id}`}
        type="password"
        autoComplete="off"
        value={value}
        placeholder={set ? "New value to replace it" : "Value"}
        onChange={(e) => setValue(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Escape") setValue("");
        }}
        style={{ flex: 1 }}
      />
      <button type="submit" data-testid={`credential-save-${id}`} disabled={!value.trim() || saving}>
        {saving ? "Saving…" : "Save"}
      </button>
      {set ? (
        <InlineConfirm
          triggerLabel="Clear"
          confirmLabel="Clear"
          testIdPrefix={`credential-clear-${id}`}
          disabled={saving}
          onConfirm={() => void save(null)}
        />
      ) : null}
    </form>
  );
}

const mutedStyle: CSSProperties = { fontSize: "var(--text-xs)", color: "var(--text-secondary)" };
