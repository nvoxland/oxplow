import type { CSSProperties } from "react";
import { useEffect, useState } from "react";
import {
  getConfig,
  setAgentPromptAppend,
  setGeneratedDirs,
  setSnapshotMaxFileBytes,
  setSnapshotRetentionDays,
} from "../api.js";

interface Props {
  open: boolean;
  onClose(): void;
}

export function SettingsModal({ open, onClose }: Props) {
  const [promptAppend, setPromptAppend] = useState("");
  const [retentionDays, setRetentionDays] = useState("7");
  const [maxFileMiB, setMaxFileMiB] = useState("5");
  const [generatedDirsText, setGeneratedDirsText] = useState("");
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
        setRetentionDays(String(config.snapshotRetentionDays));
        setMaxFileMiB((config.snapshotMaxFileBytes / (1024 * 1024)).toString());
        setGeneratedDirsText((config.generatedDirs ?? []).join("\n"));
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
    <div style={backdropStyle}>
      <div style={modalStyle}>
        <div style={headerStyle}>
          <span>Settings</span>
          <button type="button" onClick={onClose} style={closeBtnStyle} aria-label="Close settings">×</button>
        </div>
        <div style={bodyStyle}>
          <section style={sectionStyle}>
            <div style={sectionTitleStyle}>Agent prompt additions</div>
            <div style={sectionHintStyle}>
              Text appended to every agent's system prompt. Applies to agent sessions started after Save — existing sessions keep the prompt they launched with. Stored in <code>oxplow.yaml</code>.
            </div>
            <textarea
              value={promptAppend}
              onChange={(event) => setPromptAppend(event.target.value)}
              disabled={!loaded || saving}
              rows={12}
              placeholder="e.g. Prefer red/green TDD. Never run destructive git commands without asking."
              style={textareaStyle}
            />
          </section>
          <section style={sectionStyle}>
            <div style={sectionTitleStyle}>File snapshots</div>
            <div style={sectionHintStyle}>
              Snapshots capture the project's files around every agent turn so history and diffs stay available after branches change.
            </div>
            <label style={fieldLabelStyle}>
              <span>Retention (days)</span>
              <input
                type="number"
                min={0}
                step={1}
                value={retentionDays}
                onChange={(event) => setRetentionDays(event.target.value)}
                disabled={!loaded || saving}
                style={numberInputStyle}
              />
              <span style={fieldHintStyle}>0 disables pruning. Latest per-stream snapshot is always kept.</span>
            </label>
            <label style={fieldLabelStyle}>
              <span>Max file size (MiB)</span>
              <input
                type="number"
                min={0.001}
                step={0.5}
                value={maxFileMiB}
                onChange={(event) => setMaxFileMiB(event.target.value)}
                disabled={!loaded || saving}
                style={numberInputStyle}
              />
              <span style={fieldHintStyle}>Files larger than this get a stat-only entry (diffs show "oversize").</span>
            </label>
          </section>
          <section style={sectionStyle}>
            <div style={sectionTitleStyle}>Generated directories</div>
            <div style={sectionHintStyle}>
              Directory names (one per line, matched at any path segment) excluded from fs-watch and snapshot tracking. Added on top of the built-in list (node_modules, dist, build, .git, etc.).
            </div>
            <textarea
              value={generatedDirsText}
              onChange={(event) => setGeneratedDirsText(event.target.value)}
              disabled={!loaded || saving}
              rows={5}
              placeholder="e.g.&#10;coverage&#10;.cache"
              style={{ ...textareaStyle, minHeight: 90 }}
            />
          </section>
          <section style={sectionStyle}>
            <div style={actionsRowStyle}>
              {error ? <span style={{ color: "#ff6b6b", fontSize: 12 }}>{error}</span> : null}
              {savedMessage ? <span style={{ color: "var(--muted)", fontSize: 12 }}>{savedMessage}</span> : null}
              <span style={{ flex: 1 }} />
              <button type="button" onClick={onClose} style={buttonStyle} disabled={saving}>Cancel</button>
              <button type="button" onClick={() => void handleSave()} style={primaryButtonStyle} disabled={!loaded || saving}>
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
  boxShadow: "0 0 0 1px rgba(255,255,255,0.12), 0 24px 60px rgba(0,0,0,0.5)",
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

const fieldLabelStyle: CSSProperties = {
  display: "grid",
  gridTemplateColumns: "180px 140px 1fr",
  alignItems: "center",
  gap: 10,
  fontSize: 12,
};

const fieldHintStyle: CSSProperties = {
  color: "var(--muted)",
  fontSize: 11,
};

const numberInputStyle: CSSProperties = {
  background: "var(--bg)",
  color: "var(--fg)",
  border: "1px solid var(--border)",
  borderRadius: 4,
  padding: "4px 8px",
  fontFamily: "inherit",
  fontSize: 12,
};

const primaryButtonStyle: CSSProperties = {
  ...buttonStyle,
  background: "var(--accent)",
  border: "1px solid var(--accent)",
  color: "#fff",
};
