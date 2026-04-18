import type { CSSProperties } from "react";
import { useEffect, useState } from "react";
import { getConfig, setAgentPromptAppend } from "../api.js";

interface Props {
  open: boolean;
  onClose(): void;
}

export function SettingsModal({ open, onClose }: Props) {
  const [promptAppend, setPromptAppend] = useState("");
  const [loaded, setLoaded] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [savedMessage, setSavedMessage] = useState<string | null>(null);

  useEffect(() => {
    if (!open) return;
    setLoaded(false);
    setError(null);
    setSavedMessage(null);
    void getConfig()
      .then((config) => {
        setPromptAppend(config.agentPromptAppend ?? "");
        setLoaded(true);
      })
      .catch((e) => {
        setError(String(e));
        setLoaded(true);
      });
  }, [open]);

  useEffect(() => {
    if (!open) return;
    function onKey(event: KeyboardEvent) {
      if (event.key === "Escape") onClose();
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  if (!open) return null;

  async function handleSave() {
    setSaving(true);
    setError(null);
    setSavedMessage(null);
    try {
      await setAgentPromptAppend(promptAppend);
      setSavedMessage("Saved. Applies to newly-started agent sessions.");
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  }

  return (
    <div style={backdropStyle} onClick={onClose}>
      <div style={modalStyle} onClick={(event) => event.stopPropagation()}>
        <div style={headerStyle}>
          <span>Settings</span>
          <button onClick={onClose} style={closeBtnStyle} aria-label="Close settings">×</button>
        </div>
        <div style={bodyStyle}>
          <section style={sectionStyle}>
            <div style={sectionTitleStyle}>Agent prompt additions</div>
            <div style={sectionHintStyle}>
              Text appended to every agent's system prompt. Applies to agent sessions started after Save — existing sessions keep the prompt they launched with. Stored in <code>newde.yaml</code>.
            </div>
            <textarea
              value={promptAppend}
              onChange={(event) => setPromptAppend(event.target.value)}
              disabled={!loaded || saving}
              rows={12}
              placeholder="e.g. Prefer red/green TDD. Never run destructive git commands without asking."
              style={textareaStyle}
            />
            <div style={actionsRowStyle}>
              {error ? <span style={{ color: "#ff6b6b", fontSize: 12 }}>{error}</span> : null}
              {savedMessage ? <span style={{ color: "var(--muted)", fontSize: 12 }}>{savedMessage}</span> : null}
              <span style={{ flex: 1 }} />
              <button onClick={onClose} style={buttonStyle} disabled={saving}>Cancel</button>
              <button onClick={() => void handleSave()} style={primaryButtonStyle} disabled={!loaded || saving}>
                {saving ? "Saving…" : "Save"}
              </button>
            </div>
          </section>
        </div>
      </div>
    </div>
  );
}

const backdropStyle: CSSProperties = {
  position: "fixed",
  inset: 0,
  background: "rgba(0,0,0,0.5)",
  display: "flex",
  alignItems: "center",
  justifyContent: "center",
  zIndex: 1100,
};

const modalStyle: CSSProperties = {
  background: "var(--bg)",
  border: "1px solid var(--border-strong)",
  borderRadius: 8,
  boxShadow: "0 24px 60px rgba(0,0,0,0.5)",
  minWidth: 520,
  maxWidth: 720,
  maxHeight: "80vh",
  display: "flex",
  flexDirection: "column",
};

const headerStyle: CSSProperties = {
  display: "flex",
  alignItems: "center",
  justifyContent: "space-between",
  padding: "10px 14px",
  borderBottom: "1px solid var(--border)",
  fontSize: 13,
  fontWeight: 600,
  background: "var(--bg-1, var(--bg-2))",
};

const closeBtnStyle: CSSProperties = {
  border: "none",
  background: "transparent",
  color: "var(--muted)",
  fontSize: 20,
  lineHeight: 1,
  cursor: "pointer",
};

const bodyStyle: CSSProperties = {
  padding: 14,
  overflow: "auto",
};

const sectionStyle: CSSProperties = {
  display: "flex",
  flexDirection: "column",
  gap: 8,
};

const sectionTitleStyle: CSSProperties = {
  fontSize: 12,
  textTransform: "uppercase",
  letterSpacing: 0.6,
  color: "var(--muted)",
};

const sectionHintStyle: CSSProperties = {
  fontSize: 12,
  color: "var(--muted)",
  lineHeight: 1.5,
};

const textareaStyle: CSSProperties = {
  width: "100%",
  background: "var(--bg)",
  color: "var(--fg)",
  border: "1px solid var(--border)",
  borderRadius: 4,
  padding: 10,
  fontFamily: "ui-monospace, monospace",
  fontSize: 12,
  resize: "vertical",
  minHeight: 160,
};

const actionsRowStyle: CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 8,
};

const buttonStyle: CSSProperties = {
  background: "var(--bg-2)",
  color: "var(--fg)",
  border: "1px solid var(--border)",
  padding: "4px 12px",
  borderRadius: 4,
  cursor: "pointer",
  fontFamily: "inherit",
  fontSize: 12,
};

const primaryButtonStyle: CSSProperties = {
  ...buttonStyle,
  background: "var(--accent)",
  border: "1px solid var(--accent)",
  color: "#fff",
};
