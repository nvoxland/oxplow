/// The catalog (P6.D2, target §13.2): what oxplow knows and can do, so the
/// person knows what to ask — the questions each capability and extension
/// answers (each with Ask), the data behind them (`v_model` by owner), and
/// what can be configured (`config.list_keys`, changed in Settings).
import type { CSSProperties } from "react";
import { useEffect, useState } from "react";

import { insertIntoAgent } from "../agent-input-bus.js";
import { querySql, runCommand } from "../api.js";
import { chipStyle } from "../components/Prompts/SuggestedPrompts.js";
import { promptsBySource } from "../components/Prompts/promptModel.js";
import { usePromptCatalog } from "../components/Prompts/usePromptCatalog.js";
import { NO_READS, useRerunOnChange } from "../lens/lensRerun.js";
import { Page } from "../tabs/Page.js";
import { indexRef } from "../tabs/pageRefs.js";
import type { TabRef } from "../tabs/tabState.js";
import type { Reads } from "../tauri-bridge/generated/bindings.js";
import { MODELS_SQL, models, type ModelRow } from "./exploreData.js";

interface ConfigKeyRow {
  key: string;
  doc: string;
  human_only: boolean;
  set: boolean;
}

export function CatalogPage({ streamId, onOpenPage }: { streamId: string | null; onOpenPage(ref: TabRef): void }) {
  const catalog = usePromptCatalog(streamId);
  const [data, setData] = useState<ModelRow[]>([]);
  const [reads, setReads] = useState<Reads>(NO_READS);
  const [keys, setKeys] = useState<ConfigKeyRow[]>([]);
  const loadData = () =>
    querySql(MODELS_SQL, [], null)
      .then((r) => {
        setData(models(r));
        setReads(r.reads);
      })
      .catch(() => setData([]));
  useEffect(() => {
    void loadData();
    runCommand("config.list_keys", {})
      .then((o) => setKeys(Array.isArray(o.result) ? (o.result as ConfigKeyRow[]) : []))
      .catch(() => setKeys([]));
  }, []);
  useRerunOnChange(reads, () => void loadData());

  const owners = new Map<string, ModelRow[]>();
  for (const m of data) owners.set(m.owner, [...(owners.get(m.owner) ?? []), m]);

  return (
    <Page testId="page-catalog" title="Catalog" kind="catalog">
      <div style={{ display: "flex", flexDirection: "column", gap: 16, padding: 12, overflow: "auto" }}>
        <section>
          <h2 style={h2Style}>What you can ask</h2>
          <p style={hintStyle}>Ask puts the question in the agent's input for you to edit and send.</p>
          {promptsBySource(catalog).map((g) => (
            <div key={`${g.kind}:${g.label}`} data-testid={`catalog-prompts-${g.kind}-${g.label}`} style={groupStyle}>
              <div style={labelStyle}>
                {g.label}
                <span style={hintStyle}> {g.kind === "capability" ? "core" : "extension"}</span>
              </div>
              <div style={{ display: "flex", flexWrap: "wrap", gap: 6 }}>
                {g.prompts.map((p) => (
                  <button
                    key={p.prompt}
                    type="button"
                    style={chipStyle}
                    title={p.about ? `Also offered on ${p.about} pages` : "Put this in the agent's input (it isn't sent)"}
                    onClick={() => insertIntoAgent(p.prompt)}
                  >
                    {p.prompt}
                  </button>
                ))}
              </div>
            </div>
          ))}
        </section>
        <section>
          <h2 style={h2Style}>What data there is</h2>
          {[...owners.entries()].map(([owner, rows]) => (
            <div key={owner} data-testid={`catalog-data-${owner}`} style={groupStyle}>
              <div style={labelStyle}>{owner}</div>
              {rows.map((m) => (
                <div key={m.view} style={{ fontSize: "var(--text-sm)" }}>
                  <code>{m.view}</code> <span style={hintStyle}>{m.description}</span>
                </div>
              ))}
            </div>
          ))}
        </section>
        <section>
          <h2 style={h2Style}>
            What you can configure{" "}
            <button type="button" data-testid="catalog-open-settings" onClick={() => onOpenPage(indexRef("settings"))}>
              Open Settings
            </button>
          </h2>
          {keys.map((k) => (
            <div key={k.key} data-testid={`catalog-config-${k.key}`} style={{ fontSize: "var(--text-sm)" }}>
              <code>{k.key}</code>
              {k.human_only ? <span style={hintStyle}> (yours to set)</span> : null} <span style={hintStyle}>{k.doc}</span>
            </div>
          ))}
        </section>
      </div>
    </Page>
  );
}

const h2Style: CSSProperties = { fontSize: "var(--text-md)", margin: "0 0 6px" };
const hintStyle: CSSProperties = { color: "var(--text-secondary)", fontSize: "var(--text-xs)" };
const labelStyle: CSSProperties = { fontWeight: 600, fontSize: "var(--text-sm)", marginBottom: 4 };
const groupStyle: CSSProperties = { marginBottom: 10 };
