/// "Language servers" section body for SettingsPage: every known
/// server (.oxplow/project.yaml + Mason-installed) with source/binary/running
/// state, restart + remove actions, and an install strip (free-text
/// Mason package name + curated suggestion chips).
///
/// Usability contract (.context/usability.md): no modals — Remove is an
/// InlineConfirm on the row; install failures land in opErrorsStore
/// (RailHud Errors section), not alerts. Install progress rides the
/// existing BackgroundTaskKind::Lsp indicator.

import type { CSSProperties } from "react";
import { useEffect, useState } from "react";

import {
  installLspPackage,
  removeLspPackage,
  restartLspServer,
  type LspServerListing,
} from "../api.js";
import {
  lspServers,
  refreshLspServers,
  subscribeLspServers,
} from "../lsp-servers-store.js";
import { SUGGESTIONS } from "../lspSuggestions.js";
import { recordOpError } from "./opErrorsStore.js";
import { InlineConfirm } from "./InlineConfirm.js";

/// Row presentation, pure for tests: badges + which actions apply.
export interface ServerRowModel {
  badges: string[];
  binaryMissing: boolean;
  canRemove: boolean;
  canRestart: boolean;
}

export function describeServerRow(info: LspServerListing): ServerRowModel {
  const badges: string[] = [];
  if (info.source === "yaml") {
    badges.push("project.yaml");
  } else {
    badges.push(info.version ? `installed ${info.version}` : "installed");
  }
  if (info.runningStreams.length > 0) {
    badges.push(
      info.runningStreams.length === 1 ? "running" : `running ×${info.runningStreams.length}`,
    );
  }
  return {
    badges,
    binaryMissing: !info.binaryExists,
    canRemove: info.source === "installed" && info.packageName != null,
    canRestart: info.runningStreams.length > 0,
  };
}

/// Curated install chips, minus languages that already have a server.
export function availableSuggestions(servers: LspServerListing[]): { language: string; pkg: string }[] {
  const covered = new Set(servers.map((s) => s.languageId));
  const seen = new Set<string>();
  return Object.entries(SUGGESTIONS).flatMap(([language, pkg]) => {
    if (covered.has(language) || seen.has(pkg)) return [];
    seen.add(pkg);
    return [{ language, pkg }];
  });
}

export function LspServersSection() {
  const [servers, setServers] = useState<LspServerListing[]>(() => lspServers());
  const [packageName, setPackageName] = useState("");
  const [installing, setInstalling] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  useEffect(() => {
    const unsubscribe = subscribeLspServers(() => setServers(lspServers()));
    void refreshLspServers();
    return unsubscribe;
  }, []);

  async function install(pkg: string) {
    const name = pkg.trim();
    if (!name || installing) return;
    setInstalling(name);
    setNotice(null);
    try {
      await installLspPackage(name);
      setPackageName("");
      setNotice(`Installed ${name}.`);
    } catch (e) {
      recordOpError({
        label: `Install language server: ${name}`,
        message: e instanceof Error ? e.message : String(e),
      });
      setNotice(`Install of ${name} failed — see Errors in the rail.`);
    } finally {
      setInstalling(null);
      void refreshLspServers();
    }
  }

  async function restart(info: LspServerListing) {
    setNotice(null);
    try {
      for (const streamId of info.runningStreams) {
        await restartLspServer(streamId, info.languageId);
      }
      setNotice(`Restarted ${info.languageId} server.`);
    } catch (e) {
      recordOpError({
        label: `Restart language server: ${info.languageId}`,
        message: e instanceof Error ? e.message : String(e),
      });
    } finally {
      void refreshLspServers();
    }
  }

  async function remove(info: LspServerListing) {
    if (!info.packageName) return;
    setNotice(null);
    try {
      await removeLspPackage(info.packageName);
      setNotice(`Removed ${info.packageName}.`);
    } catch (e) {
      recordOpError({
        label: `Remove language server: ${info.packageName}`,
        message: e instanceof Error ? e.message : String(e),
      });
    } finally {
      void refreshLspServers();
    }
  }

  const suggestions = availableSuggestions(servers);

  return (
    <div data-testid="settings-lsp-section">
      {servers.length === 0 ? (
        <div style={emptyStyle}>
          No language servers configured. Install one below, or add an <code>lsp.servers</code>{" "}
          entry to <code>.oxplow/project.yaml</code>.
        </div>
      ) : (
        <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
          {servers.map((info) => {
            const row = describeServerRow(info);
            return (
              <div key={`${info.languageId}:${info.command}`} style={rowStyle}>
                <span style={{ minWidth: 110, fontWeight: 600 }}>{info.languageId}</span>
                <span style={commandStyle} title={[info.command, ...info.args].join(" ")}>
                  {info.command.split("/").pop()}
                </span>
                {row.badges.map((badge) => (
                  <span key={badge} style={badgeStyle}>
                    {badge}
                  </span>
                ))}
                {row.binaryMissing ? <span style={missingStyle}>binary missing</span> : null}
                <span style={{ flex: 1 }} />
                {row.canRestart ? (
                  <button type="button" style={smallButtonStyle} onClick={() => void restart(info)}>
                    Restart
                  </button>
                ) : null}
                {row.canRemove ? (
                  <InlineConfirm
                    triggerLabel="Remove"
                    confirmLabel="Remove"
                    triggerStyle={smallButtonStyle}
                    testIdPrefix={`lsp-remove-${info.languageId}`}
                    onConfirm={() => void remove(info)}
                  />
                ) : (
                  <span style={yamlHintStyle}>edit .oxplow/project.yaml</span>
                )}
              </div>
            );
          })}
        </div>
      )}

      <div style={installRowStyle}>
        <input
          type="text"
          data-testid="settings-lsp-install-input"
          value={packageName}
          placeholder="Mason package name, e.g. rust-analyzer"
          disabled={installing != null}
          onChange={(event) => setPackageName(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter") void install(packageName);
            if (event.key === "Escape") setPackageName("");
          }}
          style={installInputStyle}
        />
        <button
          type="button"
          style={smallButtonStyle}
          disabled={installing != null || !packageName.trim()}
          onClick={() => void install(packageName)}
        >
          {installing ? `Installing ${installing}…` : "Install"}
        </button>
      </div>
      {suggestions.length > 0 ? (
        <div style={chipsRowStyle}>
          {suggestions.map(({ language, pkg }) => (
            <button
              key={pkg}
              type="button"
              style={chipStyle}
              disabled={installing != null}
              title={`Install ${pkg} (${language})`}
              onClick={() => void install(pkg)}
            >
              + {pkg}
            </button>
          ))}
        </div>
      ) : null}
      {notice ? <div style={noticeStyle}>{notice}</div> : null}
    </div>
  );
}

const rowStyle: CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 8,
  padding: "5px 8px",
  border: "1px solid var(--border-subtle)",
  borderRadius: 6,
  fontSize: "var(--text-sm)",
  color: "var(--text-primary)",
};

const commandStyle: CSSProperties = {
  fontFamily: "ui-monospace, monospace",
  fontSize: "var(--text-xs)",
  color: "var(--text-secondary)",
  overflow: "hidden",
  textOverflow: "ellipsis",
  whiteSpace: "nowrap",
  maxWidth: 220,
};

const badgeStyle: CSSProperties = {
  fontSize: 10,
  padding: "1px 6px",
  borderRadius: 8,
  border: "1px solid var(--border-subtle)",
  color: "var(--text-secondary)",
  whiteSpace: "nowrap",
};

const missingStyle: CSSProperties = {
  ...badgeStyle,
  color: "var(--severity-critical)",
  borderColor: "var(--severity-critical)",
};

const yamlHintStyle: CSSProperties = {
  fontSize: 10,
  color: "var(--text-muted)",
  whiteSpace: "nowrap",
};

const smallButtonStyle: CSSProperties = {
  background: "var(--surface-tab-inactive)",
  color: "var(--text-primary)",
  border: "1px solid var(--border-subtle)",
  padding: "3px 8px",
  borderRadius: 6,
  cursor: "pointer",
  fontFamily: "inherit",
  fontSize: "var(--text-xs)",
};

const installRowStyle: CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 8,
  marginTop: 10,
};

const installInputStyle: CSSProperties = {
  flex: 1,
  background: "var(--surface-card)",
  color: "var(--text-primary)",
  border: "1px solid var(--border-subtle)",
  borderRadius: 6,
  padding: "5px 10px",
  fontFamily: "ui-monospace, monospace",
  fontSize: "var(--text-xs)",
};

const chipsRowStyle: CSSProperties = {
  display: "flex",
  flexWrap: "wrap",
  gap: 6,
  marginTop: 8,
};

const chipStyle: CSSProperties = {
  ...smallButtonStyle,
  borderRadius: 10,
  fontSize: 10,
  padding: "2px 8px",
};

const emptyStyle: CSSProperties = {
  fontSize: "var(--text-xs)",
  color: "var(--text-secondary)",
  padding: "8px 0",
};

const noticeStyle: CSSProperties = {
  marginTop: 8,
  fontSize: "var(--text-xs)",
  color: "var(--text-secondary)",
};
