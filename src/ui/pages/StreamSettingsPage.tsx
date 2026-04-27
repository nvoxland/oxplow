import type { CSSProperties } from "react";
import { useEffect, useState } from "react";
import { setStreamPrompt, type Stream } from "../api.js";
import { Page } from "../tabs/Page.js";

export interface StreamSettingsPageProps {
  /** The stream being configured. Resolved by the host from `streamId`. */
  stream: Stream | null;
  /** Closes the page (caller closes the tab). */
  onClose?(): void;
  /** Optional hook fired after a save succeeds (refresh the stream list). */
  onSaved?(updated: Stream[]): void;
}

/**
 * Per-stream settings rendered as a full page (replaces the modal that
 * lived inside `StreamRail`'s "Stream settings" overlay). Today this is
 * just the custom prompt textarea — additional per-stream fields can
 * land in this page without growing a new modal.
 */
export function StreamSettingsPage({ stream, onClose, onSaved }: StreamSettingsPageProps) {
  const [prompt, setPrompt] = useState("");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [savedMessage, setSavedMessage] = useState<string | null>(null);

  useEffect(() => {
    setPrompt(stream?.custom_prompt ?? "");
    setError(null);
    setSavedMessage(null);
  }, [stream?.id, stream?.custom_prompt]);

  async function handleSave() {
    if (!stream) return;
    setSaving(true);
    setError(null);
    setSavedMessage(null);
    try {
      const trimmed = prompt.trim();
      const next = await setStreamPrompt(stream.id, trimmed.length === 0 ? null : trimmed);
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
      testId="page-stream-settings"
      title={stream ? `Stream settings — ${stream.title}` : "Stream settings"}
      actions={
        onClose ? (
          <button type="button" onClick={onClose} style={buttonStyle}>
            Close
          </button>
        ) : null
      }
    >
      <div style={{ padding: "20px 24px", maxWidth: 720 }}>
        {!stream ? (
          <div style={{ color: "var(--text-secondary)", fontSize: 13 }}>
            This stream is no longer available.
          </div>
        ) : (
          <>
            <Section title="Custom prompt">
              <Hint>
                Appended to the agent's system prompt for every thread inside this stream.
                Applies to agent sessions started after Save — existing sessions keep the prompt
                they launched with.
              </Hint>
              <textarea
                data-testid="stream-settings-prompt"
                value={prompt}
                onChange={(e) => setPrompt(e.target.value)}
                disabled={saving}
                rows={10}
                placeholder="Enter standing instructions for this stream…"
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
                data-testid="stream-settings-save"
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

/**
 * Pure-function helper exported for unit tests: normalizes the textarea
 * value to the persisted shape (null when blank, trimmed string
 * otherwise). Keeps the "saving an empty prompt clears it" rule
 * testable without spinning up React.
 */
export function normalizePromptForSave(raw: string): string | null {
  const trimmed = raw.trim();
  return trimmed.length === 0 ? null : trimmed;
}
