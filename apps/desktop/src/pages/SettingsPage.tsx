import type { CSSProperties } from "react";
import { useEffect, useState } from "react";
import {
  getConfig,
  setAgentModel,
  setAgents,
  setAgentPromptAppend,
  setGenerated,
  setSnapshotMaxFileBytes,
  setSnapshotRetentionDays,
  type AgentKind,
} from "../api.js";
import { Page } from "../tabs/Page.js";
import { LspServersSection } from "../components/LspServersSection.js";
import { agentLabel, ALL_AGENT_KINDS } from "../agentKinds.js";

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
  const [agents, setAgentsState] = useState<AgentKind[]>(["claude"]);
  const [opencodeModel, setOpencodeModel] = useState("");
  const [retentionDays, setRetentionDays] = useState("7");
  const [maxFileMiB, setMaxFileMiB] = useState("5");
  const [generatedText, setGeneratedText] = useState("");
  // The textarea edits `generated.exclude`; preserve any `include`
  // overrides set in oxplow.yaml across a save.
  const [generatedInclude, setGeneratedInclude] = useState<string[]>([]);
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
        setAgentsState(config.agents?.length ? config.agents : ["claude"]);
        setOpencodeModel(config.agentModels?.opencode ?? "");
        setRetentionDays(String(config.snapshotRetentionDays));
        setMaxFileMiB((config.snapshotMaxFileBytes / (1024 * 1024)).toString());
        setGeneratedText((config.generated?.exclude ?? []).join("\n"));
        setGeneratedInclude(config.generated?.include ?? []);
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
      const entries = generatedText
        .split(/\r?\n/)
        .map((line) => line.trim())
        .filter((line) => line.length > 0);

      if (agents.length === 0) {
        throw new Error("Enable at least one agent.");
      }
      await setAgents(agents);
      await setAgentModel("opencode", opencodeModel.trim() || null);
      await setAgentPromptAppend(promptAppend);
      await setSnapshotRetentionDays(days);
      await setSnapshotMaxFileBytes(bytes);
      await setGenerated({ exclude: entries, include: generatedInclude });
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
      actions={
        onClose ? (
          <button type="button" onClick={onClose} style={buttonStyle}>
            Close
          </button>
        ) : null
      }
    >
      <div style={{ padding: "20px 24px", maxWidth: 720 }}>
        <Section title="Agents">
          <Hint>
            Enabled agents for this project. The first enabled agent is the default for new threads.
          </Hint>
          <AgentPicker agents={agents} onChange={setAgentsState} disabled={!loaded || saving} />
          {agents.includes("opencode") ? (
            <div style={{ marginTop: 10 }}>
              <Hint>
                Model OpenCode launches with (<code>provider/model</code>, e.g.{" "}
                <code>github-copilot/gpt-5-mini</code>). Blank uses the built-in default. Applies to
                sessions started after Save.
              </Hint>
              <input
                data-testid="settings-opencode-model"
                type="text"
                value={opencodeModel}
                onChange={(event) => setOpencodeModel(event.target.value)}
                disabled={!loaded || saving}
                placeholder="github-copilot/gpt-5-mini"
                style={{ ...numberInputStyle, width: 320 }}
              />
            </div>
          ) : null}
        </Section>

        <Section title="Agent Prompt Additions">
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

        <Section title="File Snapshots">
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

        <Section title="Generated Directories">
          <Hint>
            Directory names (one per line, matched at any path segment) excluded from fs-watch,
            snapshot tracking, and the quick-open file index. Added on top of the built-in
            exclusions (.git, .oxplow). The quick-open index also skips .gitignore&apos;d paths
            automatically; list build dirs here so fs-watch and snapshots skip them too.
          </Hint>
          <textarea
            value={generatedText}
            onChange={(event) => setGeneratedText(event.target.value)}
            disabled={!loaded || saving}
            rows={5}
            placeholder={"e.g.\ncoverage\n.cache"}
            style={{ ...textareaStyle, minHeight: 100 }}
          />
        </Section>

        <Section title="Language Servers">
          <Hint>
            Servers come from <code>oxplow.yaml</code> (<code>lsp.servers</code>) or one-click
            installs from the Mason registry (landed in <code>.oxplow/lsp/</code>). Changes apply
            immediately — no Save needed. Agents can also configure these for you.
          </Hint>
          <LspServersSection />
        </Section>

        <div style={actionsRowStyle}>
          {error ? <span style={{ color: "var(--severity-critical)", fontSize: "var(--text-xs)" }}>{error}</span> : null}
          {savedMessage ? (
            <span style={{ color: "var(--text-secondary)", fontSize: "var(--text-xs)" }}>{savedMessage}</span>
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

const ALL_AGENTS: AgentKind[] = ALL_AGENT_KINDS;

function AgentPicker({
  agents,
  onChange,
  disabled,
}: {
  agents: AgentKind[];
  onChange(next: AgentKind[]): void;
  disabled: boolean;
}) {
  function setEnabled(agent: AgentKind, enabled: boolean) {
    if (enabled) {
      onChange(agents.includes(agent) ? agents : [...agents, agent]);
      return;
    }
    onChange(agents.filter((a) => a !== agent));
  }

  function move(agent: AgentKind, direction: -1 | 1) {
    const index = agents.indexOf(agent);
    const nextIndex = index + direction;
    if (index < 0 || nextIndex < 0 || nextIndex >= agents.length) return;
    const next = agents.slice();
    [next[index], next[nextIndex]] = [next[nextIndex], next[index]];
    onChange(next);
  }

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
      {ALL_AGENTS.map((agent) => {
        const enabled = agents.includes(agent);
        return (
          <label key={agent} style={agentRowStyle}>
            <input
              type="checkbox"
              checked={enabled}
              disabled={disabled || (enabled && agents.length === 1)}
              onChange={(event) => setEnabled(agent, event.target.checked)}
            />
            <span style={{ minWidth: 64 }}>{agentLabel(agent)}</span>
            {enabled ? (
              <>
                <button
                  type="button"
                  style={smallButtonStyle}
                  disabled={disabled || agents.indexOf(agent) === 0}
                  onClick={() => move(agent, -1)}
                >
                  Up
                </button>
                <button
                  type="button"
                  style={smallButtonStyle}
                  disabled={disabled || agents.indexOf(agent) === agents.length - 1}
                  onClick={() => move(agent, 1)}
                >
                  Down
                </button>
              </>
            ) : null}
          </label>
        );
      })}
    </div>
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
    <div style={{ fontSize: "var(--text-xs)", color: "var(--text-secondary)", lineHeight: 1.5, marginBottom: 10 }}>
      {children}
    </div>
  );
}

function Field({ label, hint, input }: { label: string; hint?: string; input: React.ReactNode }) {
  return (
    <label style={{ display: "flex", alignItems: "center", gap: 12, marginBottom: 8, fontSize: "var(--text-sm)" }}>
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
  fontSize: "var(--text-xs)",
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
  fontSize: "var(--text-sm)",
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
  fontSize: "var(--text-sm)",
};

const smallButtonStyle: CSSProperties = {
  ...buttonStyle,
  padding: "3px 8px",
  fontSize: "var(--text-xs)",
};

const agentRowStyle: CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 8,
  color: "var(--text-primary)",
  fontSize: "var(--text-sm)",
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
