import type { CSSProperties } from "react";
import { useEffect, useState } from "react";
import {
  getConfig,
  setAgentPromptAppend,
  setGeneratedDirs,
  setSnapshotMaxFileBytes,
  setSnapshotRetentionDays,
} from "../api.js";
import { Page } from "../tabs/Page.js";

export interface SettingsPageProps {
  /** Closes the page (caller closes the tab). Optional — settings can be a
   *  long-lived tab too. */
  onClose?(): void;
}

/**
 * Settings rendered as a full page rather than a modal. Saves apply
 * immediately to oxplow.yaml (server side), so there's no lost-edit-on-stray-
 * click risk; the slideover/modal-bag of tradeoffs doesn't apply here.
 */
export function SettingsPage({ onClose }: SettingsPageProps) {
  const [promptAppend, setPromptAppend] = useState("");
  const [retentionDays, setRetentionDays] = useState("7");
  const [maxFileMiB, setMaxFileMiB] = useState("5");
  const [generatedDirsText, setGeneratedDirsText] = useState("");
  const [loaded, setLoaded] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [savedMessage, setSavedMessage] = useState<string | null>(null);

  useEffect(() => {
    setLoaded(false);
    setError(null);
    setSavedMessage(null);
    void getConfig()
      .then((config) => {
        setPromptAppend(config.agentPromptAppend ?? "");
        setRetentionDays(String(config.snapshotRetentionDays));
        setMaxFileMiB((config.snapshotMaxFileBytes / (1024 * 1024)).toString());
        setGeneratedDirsText((config.generatedDirs ?? []).join("\n"));
        setLoaded(true);
      })
      .catch((e) => {
        setError(String(e));
        setLoaded(true);
      });
  }, []);

  async function handleSave() {
    setSaving(true);
    setError(null);
    setSavedMessage(null);
    try {
      const days = Number(retentionDays);
      if (!Number.isFinite(days) || days < 0) {
        throw new Error("Snapshot retention days must be a non-negative number.");
      }
      const miB = Number(maxFileMiB);
      if (!Number.isFinite(miB) || miB <= 0) {
        throw new Error("Snapshot max file size must be a positive number.");
      }
      const bytes = Math.floor(miB * 1024 * 1024);
      if (bytes < 1024) {
        throw new Error("Snapshot max file size must be at least 1 KiB.");
      }
      const dirs = generatedDirsText
        .split(/\r?\n/)
        .map((line) => line.trim())
        .filter((line) => line.length > 0);

      await setAgentPromptAppend(promptAppend);
      await setSnapshotRetentionDays(days);
      await setSnapshotMaxFileBytes(bytes);
      await setGeneratedDirs(dirs);
      setSavedMessage("Saved. Agent prompt applies to newly-started sessions.");
    } catch (e) {
      setError(String(e instanceof Error ? e.message : e));
    } finally {
      setSaving(false);
    }
  }

  return (
    <Page
      testId="page-settings"
      title="Settings"
      kind="settings"
      actions={
        onClose ? (
          <button type="button" onClick={onClose} style={buttonStyle}>
            Close
          </button>
        ) : null
      }
    >
      <div style={{ padding: "20px 24px", maxWidth: 720 }}>
        <Section title="Agent prompt additions">
          <Hint>
            Text appended to every agent's system prompt. Applies to agent sessions started after Save —
            existing sessions keep the prompt they launched with. Stored in <code>oxplow.yaml</code>.
          </Hint>
          <textarea
            data-testid="settings-page-prompt-append"
            value={promptAppend}
            onChange={(event) => setPromptAppend(event.target.value)}
            disabled={!loaded || saving}
            rows={10}
            placeholder="e.g. Prefer red/green TDD. Never run destructive git commands without asking."
            style={textareaStyle}
          />
        </Section>

        <Section title="File snapshots">
          <Hint>
            Snapshots capture the project's files around every agent turn so history and diffs stay
            available after branches change.
          </Hint>
          <Field
            label="Retention (days)"
            hint="0 disables pruning. Latest per-stream snapshot is always kept."
            input={
              <input
                type="number"
                min={0}
                step={1}
                value={retentionDays}
                onChange={(event) => setRetentionDays(event.target.value)}
                disabled={!loaded || saving}
                style={numberInputStyle}
              />
            }
          />
          <Field
            label="Max file size (MiB)"
            hint='Files larger than this get a stat-only entry (diffs show "oversize").'
            input={
              <input
                type="number"
                min={0.001}
                step={0.5}
                value={maxFileMiB}
                onChange={(event) => setMaxFileMiB(event.target.value)}
                disabled={!loaded || saving}
                style={numberInputStyle}
              />
            }
          />
        </Section>

        <Section title="Generated directories">
          <Hint>
            Directory names (one per line, matched at any path segment) excluded from fs-watch and
            snapshot tracking. Added on top of the built-in list (node_modules, dist, build, .git, etc.).
          </Hint>
          <textarea
            value={generatedDirsText}
            onChange={(event) => setGeneratedDirsText(event.target.value)}
            disabled={!loaded || saving}
            rows={5}
            placeholder={"e.g.\ncoverage\n.cache"}
            style={{ ...textareaStyle, minHeight: 100 }}
          />
        </Section>

        <div style={actionsRowStyle}>
          {error ? <span style={{ color: "var(--severity-critical)", fontSize: 12 }}>{error}</span> : null}
          {savedMessage ? (
            <span style={{ color: "var(--text-secondary)", fontSize: 12 }}>{savedMessage}</span>
          ) : null}
          <span style={{ flex: 1 }} />
          <button
            type="button"
            data-testid="settings-page-save"
            onClick={() => void handleSave()}
            style={primaryButtonStyle}
            disabled={!loaded || saving}
          >
            {saving ? "Saving…" : "Save"}
          </button>
        </div>
      </div>
    </Page>
  );
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section style={{ marginBottom: 28 }}>
      <h2
        style={{
          fontSize: 11,
          fontWeight: 600,
          color: "var(--text-secondary)",
          textTransform: "uppercase",
          letterSpacing: 0.4,
          margin: "0 0 6px",
        }}
      >
        {title}
      </h2>
      {children}
    </section>
  );
}

function Hint({ children }: { children: React.ReactNode }) {
  return (
    <div style={{ fontSize: 12, color: "var(--text-secondary)", lineHeight: 1.5, marginBottom: 10 }}>
      {children}
    </div>
  );
}

function Field({ label, hint, input }: { label: string; hint?: string; input: React.ReactNode }) {
  return (
    <label style={{ display: "flex", alignItems: "center", gap: 12, marginBottom: 8, fontSize: 13 }}>
      <span style={{ minWidth: 180, color: "var(--text-primary)" }}>{label}</span>
      {input}
      {hint ? <span style={{ fontSize: 11, color: "var(--text-muted)" }}>{hint}</span> : null}
    </label>
  );
}

const textareaStyle: CSSProperties = {
  width: "100%",
  background: "var(--surface-card)",
  color: "var(--text-primary)",
  border: "1px solid var(--border-subtle)",
  borderRadius: 6,
  padding: 10,
  fontFamily: "ui-monospace, monospace",
  fontSize: 12,
  resize: "vertical",
  minHeight: 140,
};

const numberInputStyle: CSSProperties = {
  background: "var(--surface-card)",
  color: "var(--text-primary)",
  border: "1px solid var(--border-subtle)",
  borderRadius: 6,
  padding: "6px 10px",
  fontFamily: "inherit",
  fontSize: 13,
  width: 120,
};

const buttonStyle: CSSProperties = {
  background: "var(--surface-tab-inactive)",
  color: "var(--text-primary)",
  border: "1px solid var(--border-subtle)",
  padding: "6px 14px",
  borderRadius: 6,
  cursor: "pointer",
  fontFamily: "inherit",
  fontSize: 13,
};

const primaryButtonStyle: CSSProperties = {
  ...buttonStyle,
  background: "var(--accent)",
  borderColor: "var(--accent)",
  color: "var(--accent-on-accent)",
};

const actionsRowStyle: CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 12,
  paddingTop: 12,
  borderTop: "1px solid var(--border-subtle)",
};
