import { useEffect, useState, type CSSProperties } from "react";
import { Page } from "../tabs/Page.js";
import type { TabRef } from "../tabs/tabState.js";
import { lensRef } from "../tabs/pageRefs.js";
import type { Stream } from "../tauri-bridge/index.js";
import {
  describeSchema,
  querySql,
  saveLens,
  type LensRun,
  type LensViz,
  type SchemaEntity,
} from "../api.js";
import { LensResultView } from "../lens/LensResultView.js";
import { adHocLens, slugify } from "../lens/lensModel.js";
import { recordOpError } from "../components/opErrorsStore.js";

export interface ExploreDataPageProps {
  stream: Stream | null;
  onOpenPage(ref: TabRef): void;
}

const SAMPLE_LIMIT = 50;
const VIZ_OPTIONS: LensViz[] = ["table", "list", "number", "markdown"];

/**
 * Explore Data: the semantic layer's catalog (every `v_*` entity with its
 * column docs), an editable SQL box over it, and "Save as Lens" to keep a
 * query as a page. The core, deliberately simple starting point for
 * people who want to see what data exists before asking an agent for a
 * lens. See `.context/semantic-layer.md` and `.context/extensions.md`.
 */
export function ExploreDataPage({ stream, onOpenPage }: ExploreDataPageProps) {
  const [entities, setEntities] = useState<SchemaEntity[]>([]);
  const [selected, setSelected] = useState<string | null>(null);
  const [sql, setSql] = useState("");
  const [viz, setViz] = useState<LensViz>("table");
  const [run, setRun] = useState<LensRun | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    describeSchema()
      .then(setEntities)
      .catch((e) => setError(String(e)));
  }, []);

  async function execute(query: string, as: LensViz = viz) {
    setError(null);
    try {
      const result = await querySql(query, [], null);
      setRun({ lens: adHocLens(query, as), params: {}, result, alert: null });
    } catch (e) {
      setRun(null);
      setError(e instanceof Error ? e.message : String(e));
    }
  }

  function pick(name: string) {
    const q = `SELECT * FROM ${name} LIMIT ${SAMPLE_LIMIT}`;
    setSelected(name);
    setSql(q);
    setViz("table");
    void execute(q, "table");
  }

  const entity = entities.find((e) => e.name === selected) ?? null;

  const catalog = (
    <ul data-testid="explore-entities" style={{ listStyle: "none", margin: 0, padding: 0 }}>
      {entities.map((e) => (
        <li key={e.name}>
          <button
            type="button"
            data-testid={`explore-entity-${e.name}`}
            title={e.description}
            onClick={() => pick(e.name)}
            style={{ ...entityButtonStyle, fontWeight: e.name === selected ? 600 : 400 }}
          >
            {e.name}
          </button>
        </li>
      ))}
    </ul>
  );

  return (
    <Page testId="page-explore-data" title="Explore Data" layout="details" rightRail={catalog} rightRailTitle="Data">
      <p style={{ color: "var(--text-secondary)", marginTop: 0 }}>
        Everything oxplow knows, as read-only SQL views. Pick one on the right, tweak the query, and save it as a
        lens to keep it as a page — or ask your agent to build one for you.
      </p>
      {entity ? (
        <details data-testid="explore-columns" style={{ marginBottom: 12 }}>
          <summary style={{ cursor: "pointer" }}>
            <strong>{entity.name}</strong> — {entity.description}
          </summary>
          <table style={docsTableStyle}>
            <tbody>
              {entity.columns.map((c) => (
                <tr key={c.name}>
                  <td style={docsCellStyle}>
                    <code>{c.name}</code>
                  </td>
                  <td style={{ ...docsCellStyle, color: "var(--text-muted)" }}>{c.sqlType}</td>
                  <td style={docsCellStyle}>{c.doc}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </details>
      ) : null}
      <textarea
        data-testid="explore-sql"
        value={sql}
        placeholder="SELECT … FROM v_task …   (Cmd/Ctrl+Enter runs)"
        onChange={(e) => setSql(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
            e.preventDefault();
            void execute(sql);
          }
        }}
        rows={5}
        style={sqlStyle}
      />
      <div style={{ display: "flex", gap: 8, alignItems: "center", margin: "8px 0 16px" }}>
        <button type="button" data-testid="explore-run" disabled={!sql.trim()} onClick={() => void execute(sql)}>
          Run
        </button>
        <label style={{ fontSize: "var(--text-sm)" }}>
          Show as{" "}
          <select
            data-testid="explore-viz"
            value={viz}
            onChange={(e) => {
              const next = e.target.value as LensViz;
              setViz(next);
              if (run) setRun({ ...run, lens: adHocLens(run.lens.query, next) });
            }}
          >
            {VIZ_OPTIONS.map((v) => (
              <option key={v} value={v}>
                {v}
              </option>
            ))}
          </select>
        </label>
        <span style={{ flex: 1 }} />
        {run ? <SaveAsLens query={run.lens.query} viz={viz} stream={stream} onOpenPage={onOpenPage} /> : null}
      </div>
      {error ? (
        <div data-testid="explore-error" style={errorStyle}>
          {error}
        </div>
      ) : null}
      {run ? <LensResultView run={run} onOpenPage={onOpenPage} /> : null}
    </Page>
  );
}

/** Inline "Save as Lens" strip: extension + title → a lens file in this
 *  stream's worktree, then opens it. Enter saves, Escape cancels. */
function SaveAsLens({
  query,
  viz,
  stream,
  onOpenPage,
}: {
  query: string;
  viz: LensViz;
  stream: Stream | null;
  onOpenPage(ref: TabRef): void;
}) {
  const [open, setOpen] = useState(false);
  const [extension, setExtension] = useState("mine");
  const [title, setTitle] = useState("");

  async function save() {
    if (!title.trim() || !extension.trim()) return;
    try {
      const lens = await saveLens(
        extension.trim(),
        slugify(title),
        { title: title.trim(), description: "", query, viz },
        stream?.id ?? null,
      );
      setOpen(false);
      setTitle("");
      onOpenPage(lensRef(lens.id));
    } catch (e) {
      recordOpError({ label: "Save lens", message: e instanceof Error ? e.message : String(e) });
    }
  }

  if (!open) {
    return (
      <button type="button" data-testid="explore-save-open" onClick={() => setOpen(true)}>
        Save as Lens
      </button>
    );
  }
  const onKey = (e: React.KeyboardEvent) => {
    if (e.key === "Enter") void save();
    if (e.key === "Escape") setOpen(false);
  };
  return (
    <span style={{ display: "flex", gap: 6, alignItems: "center" }}>
      <input
        data-testid="explore-save-extension"
        value={extension}
        onChange={(e) => setExtension(e.target.value)}
        onKeyDown={onKey}
        title="Extension (folder under oxplow/extensions/)"
        style={{ width: 90 }}
      />
      <input
        data-testid="explore-save-title"
        autoFocus
        value={title}
        placeholder="Lens title"
        onChange={(e) => setTitle(e.target.value)}
        onKeyDown={onKey}
      />
      <button type="button" data-testid="explore-save" disabled={!title.trim()} onClick={() => void save()}>
        Save
      </button>
    </span>
  );
}

const entityButtonStyle: CSSProperties = {
  background: "none",
  border: "none",
  padding: "4px 0",
  font: "inherit",
  fontFamily: "var(--font-mono, monospace)",
  fontSize: "var(--text-sm)",
  color: "var(--text-primary)",
  cursor: "pointer",
  textAlign: "left",
};
const sqlStyle: CSSProperties = {
  width: "100%",
  boxSizing: "border-box",
  fontFamily: "var(--font-mono, monospace)",
  fontSize: "var(--text-sm)",
};
const docsTableStyle: CSSProperties = { borderCollapse: "collapse", fontSize: "var(--text-xs)", marginTop: 6 };
const docsCellStyle: CSSProperties = { padding: "2px 8px 2px 0", verticalAlign: "top" };
const errorStyle: CSSProperties = {
  fontFamily: "var(--font-mono, monospace)",
  fontSize: "var(--text-xs)",
  color: "var(--severity-critical)",
  whiteSpace: "pre-wrap",
  marginBottom: 12,
};
