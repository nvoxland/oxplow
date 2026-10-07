import type { CSSProperties } from "react";
import { useEffect, useState } from "react";
import {
  effectiveConfig,
  getConfig,
  setAgentModel,
  setAgents,
  setAgentPromptAppend,
  subscribeOxplowEvents,
  type AgentKind,
} from "../api.js";
import { insertIntoAgent } from "../agent-input-bus.js";
import type { EffectiveSetting } from "../tauri-bridge/generated/bindings.js";
import { askToChange, groupSettings, matchesSearch, valueText } from "./settingsModel.js";
import { Page } from "../tabs/Page.js";
import { LspServersSection } from "../components/LspServersSection.js";
import { ExtensionsSection } from "../components/ExtensionsSection.js";
import { DataSection } from "../components/DataSection.js";
import { IntegrationsSection } from "../components/IntegrationsSection.js";
import { SettingsSlotSections } from "../lens/SettingsSlotSections.js";
import { AiSection } from "../components/AiSection.js";
import { ProposalCard } from "../components/Proposals/ProposalCard.js";
import { decide, proposalForSetting, useProposals, type Proposal } from "../proposals.js";
import { agentLabel, ALL_AGENT_KINDS } from "../agentKinds.js";
import { onSettingsSection, scrollToSettingsSection, SETTINGS_SECTIONS, takeSettingsSection } from "./settingsSections.js";

export interface SettingsPageProps {
  /** Closes the page (caller closes the tab). Optional — settings can be a
   *  long-lived tab too. */
  onClose?(): void;
}

/**
 * Settings as a view (P6.H1): every setting that shapes this project —
 * its value and where it comes from (default, your global config, the
 * project, an extension) — searchable, each with Ask the Agent to Change
 * This. Direct controls remain below only for what only a person may set
 * (agents, AI, language servers, extensions, integrations, programs).
 */
export function SettingsPage({ onClose }: SettingsPageProps) {
  // Land on the section an alert or link asked for (tsk1040): the one
  // asked before this opened, and each one asked while it's open.
  useEffect(() => {
    const pending = takeSettingsSection();
    if (pending) requestAnimationFrame(() => scrollToSettingsSection(pending));
    return onSettingsSection(scrollToSettingsSection);
  }, []);
  const [promptAppend, setPromptAppend] = useState("");
  const [agents, setAgentsState] = useState<AgentKind[]>(["claude"]);
  const [opencodeModel, setOpencodeModel] = useState("");
  const [settings, setSettings] = useState<EffectiveSetting[]>([]);
  const [search, setSearch] = useState("");
  const [loaded, setLoaded] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [savedMessage, setSavedMessage] = useState<string | null>(null);
  const proposals = useProposals();

  useEffect(() => {
    setLoaded(false);
    setError(null);
    setSavedMessage(null);
    void getConfig()
      .then((config) => {
        setPromptAppend(config.agentPromptAppend ?? "");
        setAgentsState(config.agents?.length ? config.agents : ["claude"]);
        setOpencodeModel(config.agentModels?.opencode ?? "");
        setLoaded(true);
      })
      .catch((e) => {
        setError(String(e));
        setLoaded(true);
      });
  }, []);

  // The view re-reads when the config changes, whoever changed it.
  useEffect(() => {
    const load = () =>
      void effectiveConfig()
        .then(setSettings)
        .catch((e) => setError(String(e)));
    load();
    return subscribeOxplowEvents((event) => {
      if (event.kind === "configChanged") load();
    });
  }, []);

  async function handleSave() {
    setSaving(true);
    setError(null);
    setSavedMessage(null);
    try {
      if (agents.length === 0) {
        throw new Error("Enable at least one agent.");
      }
      await setAgents(agents);
      await setAgentModel("opencode", opencodeModel.trim() || null);
      await setAgentPromptAppend(promptAppend);
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
      <div style={{ padding: "20px 24px", maxWidth: 820 }}>
        <nav data-testid="settings-index" aria-label="Settings sections" style={{ display: "flex", flexWrap: "wrap", gap: 6, marginBottom: 20 }}>
          {SETTINGS_SECTIONS.map((s) => (
            <button key={s.id} type="button" data-testid={`settings-index-${s.id}`} onClick={() => scrollToSettingsSection(s.id)}>
              {s.title}
            </button>
          ))}
        </nav>
        <Section title="Every Setting" id="settings-every">
          <Hint>
            What shapes this project, where each value comes from, and Ask the Agent to Change This. When the agent
            changes a setting only a person may change, its change waits on the setting's row (and in Approvals) for
            you to approve — or change it yourself below.
          </Hint>
          <input
            data-testid="settings-search"
            placeholder="Search settings"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Escape") setSearch("");
            }}
            style={{ ...numberInputStyle, width: "100%", marginBottom: 10 }}
          />
          {groupSettings(settings.filter((s) => matchesSearch(s, search))).map((g) => (
            <div key={g.title} data-testid={`settings-group-${g.title}`} style={{ marginBottom: 12 }}>
              <div style={groupTitleStyle}>{g.title}</div>
              {g.settings.map((s) => (
                <SettingRow key={s.key} setting={s} proposal={proposalForSetting(proposals, s.key)} onDecide={decide} />
              ))}
            </div>
          ))}
        </Section>

        <Section title="Agents" id="settings-agents">
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

        <Section title="Agent Prompt Additions" id="settings-prompt">
          <Hint>
            Text appended to every agent's system prompt. Applies to agent sessions started after Save —
            existing sessions keep the prompt they launched with. Stored in <code>.oxplow/project.yaml</code>.
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

        <Section title="Language Servers" id="settings-lsp">
          <Hint>
            Servers come from <code>.oxplow/project.yaml</code> (<code>lsp.servers</code>) or one-click
            installs from the Mason registry (landed in <code>.oxplow/lsp/</code>). Changes apply
            immediately — no Save needed. Agents can also configure these for you.
          </Hint>
          <LspServersSection />
        </Section>

        <Section title="Extensions" id="settings-extensions">
          <Hint>
            Extensions add lenses (pages you or your agent build over oxplow&apos;s data). They live in{" "}
            <code>oxplow/extensions/</code> and are ordinary project files: commit them to share with your
            team. Installing and updating apply immediately; no Save needed.
          </Hint>
          <ExtensionsSection />
        </Section>

        <Section title="Data" id="settings-data">
          <Hint>
            What oxplow can query: its own data and what extension sources bring in, with row counts. Lenses,
            metrics and agents read these through SQL. Sources run here, and programs the project&apos;s config would run
            are approved here; set credentials under Extensions.
          </Hint>
          <DataSection />
        </Section>

        <Section title="Integrations" id="settings-integrations">
          <Hint>
            Outside systems (an issue tracker, say) that extensions connect through a provider program. Each
            instance&apos;s config is saved in <code>.oxplow/project.yaml</code> for your team; approving and running
            the program is per machine. Enabling checks the config first; three failures in a row turn an
            instance off here until you enable it again. Its credentials go to your OS keychain.
          </Hint>
          <IntegrationsSection />
        </Section>

        <SettingsSlotSections section={(title, body) => <Section title={title}>{body}</Section>} />

        <Section title="AI" id="settings-ai">
          <Hint>
            Models oxplow itself can call: for summaries, typed questions, and extensions. Your coding agents
            are separate. Providers and roles are saved for all your projects; keys go to your OS keychain,
            never to a file. Changes apply immediately; no Save needed.
          </Hint>
          <AiSection />
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

/** Where a person-only setting's direct control is, if the page has one. */
function controlFor(key: string): string | null {
  if (["agents", "agentModels", "acpAgents", "agentPromptAppend"].includes(key)) return "settings-agents";
  if (key === "ai" || key.startsWith("ai.")) return "settings-ai";
  if (key === "lsp") return "settings-lsp";
  if (key === "extensions") return "settings-extensions";
  if (key === "extensionInstances") return "settings-integrations";
  return null;
}

const ORIGIN_LABEL: Record<EffectiveSetting["origin"], string> = {
  default: "default",
  global: "your global config",
  project: "project",
  personal: "just for you",
  extension: "extension",
};

/** One setting: its value, where it comes from, and — when the agent has
 *  proposed a change to it — that change with Approve and Decline. */
export function SettingRow({
  setting: s,
  proposal,
  onDecide,
}: {
  setting: EffectiveSetting;
  proposal: Proposal | undefined;
  onDecide(p: Proposal, approve: boolean): Promise<void>;
}) {
  const control = s.humanOnly ? controlFor(s.key) : null;
  return (
    <div data-testid={`setting-${s.key}`} style={settingRowStyle}>
      <div style={{ display: "flex", alignItems: "baseline", gap: 8, flexWrap: "wrap" }}>
        <code style={{ fontSize: "var(--text-sm)" }}>{s.key}</code>
        <span data-testid={`setting-origin-${s.key}`} style={originChipStyle} title="Where the value comes from">
          {ORIGIN_LABEL[s.origin]}
          {s.extension ? `: ${s.extension}` : ""}
        </span>
        {s.humanOnly ? <span style={hintInlineStyle}>yours to set</span> : null}
        <span style={{ flex: 1 }} />
        {control ? (
          <button
            type="button"
            style={linkButtonStyle}
            onClick={() => document.getElementById(control)?.scrollIntoView({ behavior: "smooth", block: "start" })}
          >
            Change It Here
          </button>
        ) : null}
        <button
          type="button"
          data-testid={`setting-ask-${s.key}`}
          style={linkButtonStyle}
          title="Put a request to change this in the agent's input (it isn't sent)"
          onClick={() => insertIntoAgent(askToChange(s))}
        >
          Ask the Agent to Change This
        </button>
      </div>
      <div style={{ fontFamily: "var(--font-mono)", fontSize: "var(--text-xs)", color: "var(--text-primary)", marginTop: 2 }}>
        {valueText(s.value)}
      </div>
      {s.doc ? <div style={hintInlineStyle}>{s.doc}</div> : null}
      {proposal ? (
        <div style={{ marginTop: 4 }}>
          <ProposalCard proposal={proposal} compact onDecide={onDecide} />
        </div>
      ) : null}
    </div>
  );
}

const groupTitleStyle: CSSProperties = { fontWeight: 600, fontSize: "var(--text-sm)", margin: "6px 0 4px" };
const settingRowStyle: CSSProperties = {
  padding: "6px 8px",
  borderBottom: "1px solid var(--border-subtle)",
};
const originChipStyle: CSSProperties = {
  fontSize: 10,
  padding: "1px 6px",
  borderRadius: 8,
  border: "1px solid var(--border-subtle)",
  color: "var(--text-secondary)",
};
const hintInlineStyle: CSSProperties = { fontSize: "var(--text-xs)", color: "var(--text-secondary)" };
const linkButtonStyle: CSSProperties = {
  background: "none",
  border: "none",
  padding: 0,
  color: "var(--accent)",
  cursor: "pointer",
  fontSize: "var(--text-xs)",
};

function Section({ title, id, children }: { title: string; id?: string; children: React.ReactNode }) {
  return (
    <section id={id} style={{ marginBottom: 28 }}>
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
