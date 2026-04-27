import type { CSSProperties } from "react";
import { useEffect, useState } from "react";
import { setThreadPrompt, type Thread } from "../api.js";
import { Page } from "../tabs/Page.js";
import { normalizePromptForSave } from "./StreamSettingsPage.js";

export interface ThreadSettingsPageProps {
  streamId: string;
  thread: Thread | null;
  onClose?(): void;
  onSaved?(updated: Thread[]): void;
}

/**
 * Per-thread settings page. Today this is just the custom prompt
 * textarea. Additional per-thread fields can land in this page rather
 * than re-introducing a modal.
 */
export function ThreadSettingsPage({ streamId, thread, onClose, onSaved }: ThreadSettingsPageProps) {
  const [prompt, setPrompt] = useState("");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [savedMessage, setSavedMessage] = useState<string | null>(null);

  useEffect(() => {
    setPrompt(thread?.custom_prompt ?? "");
    setError(null);
    setSavedMessage(null);
  }, [thread?.id, thread?.custom_prompt]);

  async function handleSave() {
    if (!thread) return;
    setSaving(true);
    setError(null);
    setSavedMessage(null);
    try {
      const next = await setThreadPrompt(streamId, thread.id, normalizePromptForSave(prompt));
      setSavedMessage("Saved. Applies to newly-started agent sessions.");
      onSaved?.(next);
    } catch (e) {
      setError(String(e instanceof Error ? e.message : e));
    } finally {
      setSaving(false);
    }
  }

  return (
    <Page
      testId="page-thread-settings"
      title={thread ? `Thread settings — ${thread.title}` : "Thread settings"}
      actions={
        onClose ? (
          <button type="button" onClick={onClose} style={buttonStyle}>
            Close
          </button>
        ) : null
      }
    >
      <div style={{ padding: "20px 24px", maxWidth: 720 }}>
        {!thread ? (
          <div style={{ color: "var(--text-secondary)", fontSize: 13 }}>
            This thread is no longer available.
          </div>
        ) : (
          <>
            <Section title="Custom prompt">
              <Hint>
                Appended to the agent's system prompt for this thread (on top of any
                stream-level prompt). Applies to agent sessions started after Save —
                existing sessions keep the prompt they launched with.
              </Hint>
              <textarea
                data-testid="thread-settings-prompt"
                value={prompt}
                onChange={(e) => setPrompt(e.target.value)}
                disabled={saving}
                rows={10}
                placeholder="Enter standing instructions for this thread…"
                style={textareaStyle}
              />
            </Section>

            <div style={actionsRowStyle}>
              {error ? (
                <span style={{ color: "var(--severity-critical)", fontSize: 12 }}>{error}</span>
              ) : null}
              {savedMessage ? (
                <span style={{ color: "var(--text-secondary)", fontSize: 12 }}>{savedMessage}</span>
              ) : null}
              <span style={{ flex: 1 }} />
              <button
                type="button"
                data-testid="thread-settings-save"
                onClick={() => void handleSave()}
                disabled={saving}
                style={primaryButtonStyle}
              >
                {saving ? "Saving…" : "Save"}
              </button>
            </div>
          </>
        )}
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

const textareaStyle: CSSProperties = {
  width: "100%",
  background: "var(--surface-card)",
  color: "var(--text-primary)",
  border: "1px solid var(--border-subtle)",
  borderRadius: 6,
  padding: 10,
  fontFamily: "inherit",
  fontSize: 13,
  resize: "vertical",
  minHeight: 160,
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
