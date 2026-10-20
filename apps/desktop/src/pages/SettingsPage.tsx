import { useEffect, useState, type ReactNode } from "react";
import {
  getConfig,
  setAgentSetting,
  setAgents,
  setAgentPromptAppend,
  subscribeOxplowEvents,
} from "../api.js";
import { insertIntoAgent } from "../agent-input-bus.js";
import type { HarnessListing } from "../tauri-bridge/generated/bindings.js";
import { readSettings, type EffectiveSetting } from "../configValues.js";
import { askToChange, groupSettings, matchesSearch, valueText } from "./settingsModel.js";
import { DocumentLayout, type DocSection } from "../kit/DocumentLayout.js";
import { useDraft } from "../tabs/useViewParam.js";
import { useSearchFilter } from "../kit/SearchFilter.js";
import { Toolbar } from "../kit/Toolbar.js";
import type { PageAction } from "../kit/actions.js";
import { LspServersSection } from "../components/LspServersSection.js";
import { ExtensionsSection } from "../components/ExtensionsSection.js";
import { DataSection } from "../components/DataSection.js";
import { CapabilitiesSection } from "../components/CapabilitiesSection.js";
import { IntegrationsSection } from "../components/IntegrationsSection.js";
import { AiSection } from "../components/AiSection.js";
import { NewThreadSessionSetting } from "../components/NewThreadSessionSetting.js";
import { ProposalList } from "../kit/Approvals.js";
import { decide, proposalForSetting, useProposals, type Proposal } from "../proposals.js";
import { useAgentHarnesses } from "../useAgentHarnesses.js";
import { settingsRef } from "../tabs/pageRefs.js";
import type { TabRef } from "../tabs/tabState.js";
import { SETTINGS_AREAS, settingsArea, settingsSlot, type SettingsArea, type SettingsAreaId } from "./settingsAreas.js";

export interface SettingsPageProps {
  /** Which area (`page:settings?area=…`); the overview when absent. */
  area?: string | null;
  /** Where to land: one of the area's section ids, or an anchor inside one. */
  section?: string | null;
  /** Opens another area (or a link), in this tab. */
  onOpenPage(ref: TabRef): void;
}

/**
 * Settings, one Document page per area (`settingsAreas.ts`), all sharing a
 * rail that lists the areas. The overview is Settings as a view (P6.H1):
 * every setting, its value and where it comes from, with Ask the Agent to
 * Change This. The other areas hold the direct controls for what only a
 * person may set. Each area's sections are a slot an extension adds to
 * (`settings.<area>.sections`).
 */
export function SettingsPage({ area: areaId, section, onOpenPage }: SettingsPageProps) {
  const area = settingsArea(areaId);
  // Land on the section (or anchor) the link asked for.
  useEffect(() => {
    if (!section) return;
    const frame = requestAnimationFrame(() => {
      const target = document.querySelector(`[data-section="${CSS.escape(section)}"]`) ?? document.getElementById(section);
      target?.scrollIntoView({ behavior: "smooth", block: "start" });
    });
    return () => cancelAnimationFrame(frame);
  }, [area.id, section]);
  const open = (id: SettingsAreaId, anchor: string | null = null) => onOpenPage(settingsRef(id, anchor));
  const { sections, actions } = useAreaContent(area, open);
  return (
    <DocumentLayout
      testId="page-settings"
      title={area.id === "overview" ? "Settings" : area.title}
      intro={area.intro}
      sections={sections}
      actions={actions}
      slot={settingsSlot(area.id)}
      onOpenPage={onOpenPage}
      rail={<SettingsAreasNav current={area.id} onOpen={(id) => open(id)} />}
      railTitle="Settings"
    />
  );
}

/** The rail every Settings page shares: the areas, the one shown marked. */
function SettingsAreasNav({ current, onOpen }: { current: SettingsAreaId; onOpen(id: SettingsAreaId): void }) {
  return (
    <nav aria-label="Settings areas" data-testid="settings-areas" style={{ display: "flex", flexDirection: "column", gap: "var(--space-1)" }}>
      {SETTINGS_AREAS.map((a) => (
        <button
          key={a.id}
          type="button"
          data-testid={`settings-area-${a.id}`}
          aria-current={a.id === current ? "page" : undefined}
          className="ox-link"
          onClick={() => onOpen(a.id)}
          style={{
            background: "none",
            border: "none",
            padding: 0,
            font: "inherit",
            textAlign: "left",
            fontWeight: a.id === current ? "var(--weight-bold)" : undefined,
            color: a.id === current ? "var(--text-primary)" : undefined,
          }}
        >
          {a.title}
        </button>
      ))}
    </nav>
  );
}

/** An area's own sections (titled as the area model titles them) and its
 *  page actions. */
function useAreaContent(area: SettingsArea, open: (id: SettingsAreaId, anchor?: string | null) => void): { sections: DocSection[]; actions?: PageAction[] } {
  const agents = useAgentsForm(area.id === "agents");
  const title = (id: string) => area.sections.find((s) => s.id === id)?.title ?? id;
  const section = (id: string, body: ReactNode, description?: ReactNode): DocSection => ({
    id,
    title: title(id),
    description,
    testId: `settings-section-${id}`,
    body,
  });
  switch (area.id) {
    case "overview":
      return { sections: [section("every", <EverySetting onOpenControl={(id) => open(id)} />)] };
    case "agents":
      return {
        sections: [
          section(
            "agents",
            agents.agentsBody,
            "Enabled agents for this project. The first enabled agent is the default for new sessions.",
          ),
          section("new-threads", <NewThreadSessionSetting harnesses={agents.harnesses} />),
          section(
            "prompt",
            agents.promptBody,
            <>
              Text appended to every agent&apos;s system prompt, in <code>.oxplow/project.yaml</code>. It applies to sessions started after
              Save; running ones keep the prompt they launched with.
            </>,
          ),
        ],
        actions: [{ id: "save", label: "Save", primary: true, busy: agents.saving, disabled: !agents.loaded, run: agents.save }],
      };
    case "lsp":
      return {
        sections: [
          section(
            "servers",
            <LspServersSection />,
            <>
              Servers come from <code>.oxplow/project.yaml</code> (<code>lsp.servers</code>) or one-click installs from the Mason registry
              (in <code>.oxplow/lsp/</code>). Agents can configure these for you too.
            </>,
          ),
        ],
      };
    case "extensions":
      return {
        sections: [
          section(
            "health",
            <ExtensionsSection />,
            "Contributions turned off after failures, and the credentials extensions declare.",
          ),
        ],
      };
    case "data":
      return {
        sections: [
          section(
            "data",
            <DataSection />,
            "What oxplow can query: its own data and what extension sources bring in. Sources run here; set credentials under Extensions.",
          ),
        ],
      };
    case "integrations":
      return {
        sections: [
          section(
            "capabilities",
            <CapabilitiesSection />,
            <>
              Which work list tasks are filed on, how efforts open and close, how file versions are kept. The project&apos;s choice is
              saved in <code>.oxplow/project.yaml</code>; &ldquo;Just for me&rdquo; goes in <code>.oxplow/personal.yaml</code>.
            </>,
          ),
          section(
            "integrations",
            <IntegrationsSection />,
            "Outside systems that extensions connect through a program oxplow runs. Each instance's config is the team's; approving and running the program is per machine, and its credentials go to your OS keychain. Three failures in a row turn an instance off until you enable it again.",
          ),
        ],
      };
    case "ai":
      return {
        sections: [
          section(
            "ai",
            <AiSection />,
            "AI providers and roles are saved for all your projects; keys go to your OS keychain, never to a file. Changes apply at once.",
          ),
        ],
      };
  }
}

/** The overview: every setting, searchable, each with where its value
 *  comes from, Ask, and — for one only a person sets — its area. */
function EverySetting({ onOpenControl }: { onOpenControl(area: SettingsAreaId): void }) {
  const [settings, setSettings] = useState<EffectiveSetting[]>([]);
  const [error, setError] = useState<string | null>(null);
  const proposals = useProposals();
  // The search is the page's `q` view param.
  const filter = useSearchFilter("q", {
    label: "Search settings",
    placeholder: "Search settings",
    testId: "settings-search",
    inputProps: {
      onKeyDown: (e) => {
        if (e.key === "Escape") filter.onChange("");
      },
    },
  });
  // The view re-reads when the config changes, whoever changed it.
  useEffect(() => {
    const load = () =>
      void readSettings()
        .then((r) => setSettings(r.settings))
        .catch((e) => setError(String(e)));
    load();
    return subscribeOxplowEvents((event) => {
      if (event.kind === "configChanged") load();
    });
  }, []);
  return (
    <>
      <p className="ox-section-desc">
        When the agent changes a setting only a person may change, its change waits on the setting&apos;s row (and in Approvals) for you to
        approve — or change it yourself on its area&apos;s page.
      </p>
      <Toolbar filter={filter} />
      {error ? <p className="ox-error">{error}</p> : null}
      {groupSettings(settings.filter((s) => matchesSearch(s, filter.value))).map((g) => (
        <div key={g.title} data-testid={`settings-group-${g.title}`} style={{ marginBottom: "var(--space-3)" }}>
          <div className="ox-heading">{g.title}</div>
          {g.settings.map((s) => (
            <SettingRow key={s.key} setting={s} proposal={proposalForSetting(proposals, s.key)} onDecide={decide} onOpenControl={onOpenControl} />
          ))}
        </div>
      ))}
    </>
  );
}

/** The Agents area's form: which agents are on and in what order, each
 *  one's declared settings, and the prompt additions — saved together. */
function useAgentsForm(active: boolean) {
  const harnesses = useAgentHarnesses();
  // What is edited and not yet saved is this step's history state (a
  // draft; `null` until edited, which shows what is saved).
  const [loadedPrompt, setLoadedPrompt] = useState("");
  const [promptDraft, setPromptAppend, clearPrompt] = useDraft<string | null>("agentPrompt", null);
  const promptAppend = promptDraft ?? loadedPrompt;
  // The enabled harnesses in priority order; `null` until edited, which
  // shows what the project has (every harness when `agents:` names none).
  const [editedAgents, setEditedAgents, clearAgents] = useDraft<string[] | null>("agents", null);
  const agents = editedAgents ?? harnesses.filter((h) => h.enabled).map((h) => h.id);
  // Each harness's settings as loaded (`agentConfig.<harness>.<key>`) and
  // as edited, keyed `<harness>.<key>`.
  const [loadedSettings, setLoadedSettings] = useState<Record<string, string>>({});
  const [settingsDraft, setHarnessSettings, clearSettings] = useDraft<Record<string, string> | null>("agentSettings", null);
  const harnessSettings = settingsDraft ?? loadedSettings;
  const [loaded, setLoaded] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState<string | null>(null);

  useEffect(() => {
    if (!active) return;
    setLoaded(false);
    void getConfig()
      .then((config) => {
        setLoadedPrompt(config.agentPromptAppend ?? "");
        setLoadedSettings(harnessSettingValues(config.agentConfig ?? {}));
        setLoaded(true);
      })
      .catch((e) => {
        setError(String(e));
        setLoaded(true);
      });
  }, [active]);

  async function save() {
    setSaving(true);
    setError(null);
    setSaved(null);
    try {
      if (agents.length === 0) throw new Error("Enable at least one agent.");
      if (editedAgents) await setAgents(editedAgents);
      for (const h of harnesses) {
        for (const s of h.settings) {
          const at = `${h.id}.${s.key}`;
          const value = (harnessSettings[at] ?? "").trim();
          if (value !== (loadedSettings[at] ?? "")) await setAgentSetting(h.id, s.key, value || null);
        }
      }
      await setAgentPromptAppend(promptAppend);
      setLoadedPrompt(promptAppend);
      setLoadedSettings(harnessSettings);
      clearPrompt();
      clearSettings();
      clearAgents();
      setSaved("Saved. The prompt applies to sessions started from now on.");
    } catch (e) {
      setError(String(e instanceof Error ? e.message : e));
    } finally {
      setSaving(false);
    }
  }

  const status = error ? (
    <p className="ox-error" data-testid="settings-save-error">
      {error}
    </p>
  ) : saved ? (
    <p className="ox-muted ox-small" data-testid="settings-saved">
      {saved}
    </p>
  ) : null;
  const disabled = !loaded || saving;
  const agentsBody = (
    <>
      {status}
      <AgentPicker harnesses={harnesses} agents={agents} onChange={setEditedAgents} disabled={disabled} />
      {harnesses
        .filter((h) => agents.includes(h.id))
        .flatMap((h) =>
          h.settings.map((s) => {
            const at = `${h.id}.${s.key}`;
            return (
              <label key={at} className="ox-field" style={{ marginTop: "var(--space-3)" }}>
                <span className="ox-label">
                  {h.title} — {s.title}: {s.hint}
                </span>
                <input
                  data-testid={`settings-agent-${h.id}-${s.key}`}
                  className="ox-input"
                  type="text"
                  value={harnessSettings[at] ?? ""}
                  onChange={(event) => setHarnessSettings({ ...harnessSettings, [at]: event.target.value })}
                  disabled={disabled}
                  placeholder={s.placeholder}
                  style={{ width: 320 }}
                />
              </label>
            );
          }),
        )}
    </>
  );
  const promptBody = (
    <textarea
      data-testid="settings-page-prompt-append"
      className="ox-textarea"
      value={promptAppend}
      onChange={(event) => setPromptAppend(event.target.value)}
      disabled={disabled}
      rows={10}
      placeholder="e.g. Prefer red/green TDD. Never run destructive git commands without asking."
      style={{ width: "100%", fontFamily: "var(--font-mono)" }}
    />
  );
  return { harnesses, loaded, saving, save, agentsBody, promptBody };
}

/// The registered harnesses, the enabled ones first in their order.
function AgentPicker({
  harnesses,
  agents,
  onChange,
  disabled,
}: {
  harnesses: HarnessListing[];
  agents: string[];
  onChange(next: string[]): void;
  disabled: boolean;
}) {
  const ordered = [
    ...agents.flatMap((id) => harnesses.filter((h) => h.id === id)),
    ...harnesses.filter((h) => !agents.includes(h.id)),
  ];
  function setEnabled(agent: string, enabled: boolean) {
    if (enabled) {
      onChange(agents.includes(agent) ? agents : [...agents, agent]);
      return;
    }
    onChange(agents.filter((a) => a !== agent));
  }
  function move(agent: string, direction: -1 | 1) {
    const index = agents.indexOf(agent);
    const nextIndex = index + direction;
    if (index < 0 || nextIndex < 0 || nextIndex >= agents.length) return;
    const next = agents.slice();
    [next[index], next[nextIndex]] = [next[nextIndex]!, next[index]!];
    onChange(next);
  }
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "var(--space-2)" }}>
      {ordered.map(({ id: agent, title }) => {
        const enabled = agents.includes(agent);
        return (
          <label key={agent} style={{ display: "flex", alignItems: "center", gap: "var(--space-2)" }}>
            <input
              type="checkbox"
              checked={enabled}
              disabled={disabled || (enabled && agents.length === 1)}
              onChange={(event) => setEnabled(agent, event.target.checked)}
            />
            <span style={{ minWidth: 64 }}>{title}</span>
            {enabled ? (
              <>
                <button type="button" className="ox-button" disabled={disabled || agents.indexOf(agent) === 0} onClick={() => move(agent, -1)}>
                  Up
                </button>
                <button
                  type="button"
                  className="ox-button"
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

/** The area where a person-only setting's direct control is, if any. */
export function controlFor(key: string): SettingsAreaId | null {
  if (["agents", "agentConfig", "agentPromptAppend", "personal.newThreadSession"].includes(key)) return "agents";
  if (key === "ai" || key.startsWith("ai.")) return "ai";
  if (key === "lsp") return "lsp";
  if (key === "extensions") return "extensions";
  if (key === "extensionInstances" || key === "activeProviders") return "integrations";
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
  onOpenControl,
}: {
  setting: EffectiveSetting;
  proposal: Proposal | undefined;
  onDecide(p: Proposal, approve: boolean): Promise<void>;
  /** Opens the area that holds a person-only setting's control. */
  onOpenControl?(area: SettingsAreaId): void;
}) {
  const control = s.humanOnly ? controlFor(s.key) : null;
  return (
    <div data-testid={`setting-${s.key}`} style={{ padding: "var(--space-2)", borderBottom: "1px solid var(--border-subtle)" }}>
      <div style={{ display: "flex", alignItems: "baseline", gap: "var(--space-2)", flexWrap: "wrap" }}>
        <code className="ox-mono">{s.key}</code>
        <span data-testid={`setting-origin-${s.key}`} className="ox-badge" title="Where the value comes from">
          {ORIGIN_LABEL[s.origin]}
          {s.extension ? `: ${s.extension}` : ""}
        </span>
        {s.humanOnly ? <span className="ox-muted ox-small">yours to set</span> : null}
        <span style={{ flex: 1 }} />
        {control && onOpenControl ? (
          <button type="button" className="ox-link ox-small" style={linkButtonStyle} onClick={() => onOpenControl(control)}>
            Change It Here
          </button>
        ) : null}
        <button
          type="button"
          data-testid={`setting-ask-${s.key}`}
          className="ox-link ox-small"
          style={linkButtonStyle}
          title="Put a request to change this in the agent's input (it isn't sent)"
          onClick={() => insertIntoAgent(askToChange(s))}
        >
          Ask the Agent to Change This
        </button>
      </div>
      <div className="ox-mono ox-small" style={{ marginTop: 2 }}>
        {valueText(s.value)}
      </div>
      {s.doc ? <div className="ox-muted ox-small">{s.doc}</div> : null}
      {proposal ? (
        <div style={{ marginTop: "var(--space-1)" }}>
          <ProposalList proposals={[proposal]} compact onDecide={onDecide} />
        </div>
      ) : null}
    </div>
  );
}

const linkButtonStyle = { background: "none", border: "none", padding: 0, font: "inherit" } as const;

/// `agentConfig`'s string values, keyed `<harness>.<key>`.
function harnessSettingValues(agentConfig: Record<string, unknown>): Record<string, string> {
  const out: Record<string, string> = {};
  for (const [harness, fields] of Object.entries(agentConfig)) {
    if (!fields || typeof fields !== "object") continue;
    for (const [key, value] of Object.entries(fields as Record<string, unknown>)) {
      if (typeof value === "string") out[`${harness}.${key}`] = value;
    }
  }
  return out;
}
