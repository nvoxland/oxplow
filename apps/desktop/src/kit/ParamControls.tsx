import { useEffect, useRef, useState, type CSSProperties, type ReactNode } from "react";

import { querySql, type SqlCell, type SqlQueryResult } from "../api.js";
import { recordOpError } from "../components/opErrorsStore.js";
import { parseParamInput } from "../lens/lensModel.js";
import { paramKind, paramOptions, paramOptionsSql, type ParamKind, type ParamOption } from "../lens/lensParams.js";
import type { LensParam } from "../tauri-bridge/generated/bindings.js";
import { DEFAULT_RANGE_PRESETS, rangePresetLabel, splitRange } from "./rangePresets.js";
import { useDebounced } from "./useDebounced.js";

/** How long a search waits for the next keystroke before it applies. */
const SEARCH_DEBOUNCE_MS = 250;

type Query = (sql: string) => Promise<SqlQueryResult>;

export interface ParamControlProps {
  param: LensParam;
  value: SqlCell;
  onApply(name: string, value: SqlCell): void;
  /** The toolbar look: the label is the control's name, not a row above it. */
  compact?: boolean;
  /** Reads the options of a branch or id picker; `querySql` unless a test injects one. */
  query?: Query;
}

/** One lens param, drawn as the control its type calls for. */
export function ParamControl({ param, value, onApply, compact = false, query = querySql }: ParamControlProps) {
  const label = param.label ?? param.name;
  const common = { name: param.name, label, compact, value, onApply };
  const type = param.type ?? ((param.options?.length ?? 0) > 0 ? "choice" : "text");
  switch (type) {
    case "choice":
      return <ChoiceControl {...common} options={param.options ?? []} />;
    case "bool":
      return <BoolControl {...common} />;
    case "search":
      return <SearchControl {...common} />;
    case "branch":
      return <BranchControl {...common} query={query} />;
    case "date_range":
      return <RangeControl {...common} presets={param.presets && param.presets.length > 0 ? param.presets : DEFAULT_RANGE_PRESETS} />;
    case "text": {
      const kind = paramKind(param.name);
      return kind ? <PickerControl {...common} kind={kind} query={query} /> : <TextControl {...common} />;
    }
  }
}

interface ControlProps {
  name: string;
  label: string;
  compact: boolean;
  value: SqlCell;
  onApply(name: string, value: SqlCell): void;
}

const labelStyle: CSSProperties = { display: "flex", flexDirection: "column", gap: 4, fontSize: "var(--text-sm)" };

/** The label row above a control, or nothing when the control carries its own name. */
function Labelled({ label, compact, children }: { label: string; compact: boolean; children: ReactNode }) {
  if (compact) return <>{children}</>;
  return (
    <label style={labelStyle}>
      {label}
      {children}
    </label>
  );
}

function shownText(value: SqlCell): string {
  return value === null || value === undefined ? "" : String(value);
}

function TextControl({ name, label, compact, value, onApply }: ControlProps) {
  const shown = shownText(value);
  const [text, setText] = useState(shown);
  useEffect(() => setText(shown), [shown]);
  return (
    <Labelled label={label} compact={compact}>
      <input
        className="ox-input"
        data-testid={`lens-param-${name}`}
        aria-label={compact ? label : undefined}
        placeholder={compact ? label : undefined}
        value={text}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") onApply(name, parseParamInput(text));
          if (e.key === "Escape") setText(shown);
        }}
        onBlur={() => {
          if (text !== shown) onApply(name, parseParamInput(text));
        }}
      />
    </Labelled>
  );
}

function ChoiceControl({ name, label, compact, options, value, onApply }: ControlProps & { options: { value: string; label: string }[] }) {
  return (
    <Labelled label={label} compact={compact}>
      <select
        className="ox-select"
        data-testid={`lens-param-${name}`}
        aria-label={compact ? label : undefined}
        value={shownText(value)}
        onChange={(e) => onApply(name, e.target.value)}
      >
        {options.map((o) => (
          <option key={o.value} value={o.value}>
            {o.label}
          </option>
        ))}
      </select>
    </Labelled>
  );
}

/** One of oxplow's id params: pick what it names, by title. */
function PickerControl({ name, label, compact, kind, value, onApply, query }: ControlProps & { kind: ParamKind; query: Query }) {
  const [options, setOptions] = useState<ParamOption[]>([]);
  useEffect(() => {
    let live = true;
    query(paramOptionsSql(kind))
      .then((r) => {
        if (live) setOptions(paramOptions(r));
      })
      .catch((e: unknown) => recordOpError({ label: `List ${kind}s`, message: e instanceof Error ? e.message : String(e) }));
    return () => {
      live = false;
    };
  }, [kind, query]);
  const at = options.findIndex((o) => o.value === value);
  return (
    <Labelled label={label} compact={compact}>
      <select
        className="ox-select"
        data-testid={`lens-param-${name}`}
        aria-label={compact ? label : undefined}
        value={at === -1 ? "" : String(at)}
        onChange={(e) => {
          const picked = options[Number(e.target.value)];
          if (picked) onApply(name, picked.value);
        }}
      >
        {at === -1 ? <option value="">{value === null ? "—" : String(value)}</option> : null}
        {options.map((o, i) => (
          <option key={i} value={String(i)}>
            {o.label}
          </option>
        ))}
      </select>
    </Labelled>
  );
}

/** A bool echoes back as 1/0 (or true/false); checked when truthy. */
function BoolControl({ name, label, value, onApply }: ControlProps) {
  const checked = value === true || (typeof value === "number" && value !== 0) || (typeof value === "string" && value !== "" && value !== "0" && value !== "false");
  return (
    <label style={{ display: "flex", alignItems: "center", gap: 6, fontSize: "var(--text-sm)" }}>
      <input type="checkbox" data-testid={`lens-param-${name}`} checked={checked} onChange={(e) => onApply(name, e.target.checked)} />
      {label}
    </label>
  );
}

/** Applies as you type (after a pause), at once on Enter; Escape clears. */
function SearchControl({ name, label, compact, value, onApply }: ControlProps) {
  const shown = shownText(value);
  const [text, setText] = useState(shown);
  const applied = useRef(shown);
  useEffect(() => {
    applied.current = shown;
    setText(shown);
  }, [shown]);
  const apply = (next: string) => {
    if (next === applied.current) return;
    applied.current = next;
    onApply(name, next === "" ? null : next);
  };
  const settled = useDebounced(text, SEARCH_DEBOUNCE_MS);
  useEffect(() => apply(settled), [settled]);
  return (
    <Labelled label={label} compact={compact}>
      <input
        type="search"
        className="ox-input"
        data-testid={`lens-param-${name}`}
        aria-label={compact ? label : undefined}
        placeholder={compact ? label : undefined}
        value={text}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") apply(text);
          if (e.key === "Escape") {
            setText("");
            apply("");
          }
        }}
      />
    </Labelled>
  );
}

const BRANCHES_SQL = "SELECT name FROM v_branch WHERE kind = 'local' ORDER BY name";

function BranchControl({ name, label, compact, value, onApply, query }: ControlProps & { query: Query }) {
  const [branches, setBranches] = useState<string[]>([]);
  useEffect(() => {
    let live = true;
    query(BRANCHES_SQL)
      .then((r) => {
        if (live) setBranches(r.rows.map((row) => String(row[0])));
      })
      .catch((e: unknown) => recordOpError({ label: "List branches", message: e instanceof Error ? e.message : String(e) }));
    return () => {
      live = false;
    };
  }, [query]);
  const current = shownText(value);
  return (
    <Labelled label={label} compact={compact}>
      <select
        className="ox-select"
        data-testid={`lens-param-${name}`}
        aria-label={compact ? label : undefined}
        value={current}
        onChange={(e) => onApply(name, e.target.value === "" ? null : e.target.value)}
      >
        <option value="">All branches</option>
        {current !== "" && !branches.includes(current) ? <option value={current}>{current}</option> : null}
        {branches.map((b) => (
          <option key={b} value={b}>
            {b}
          </option>
        ))}
      </select>
    </Labelled>
  );
}

const CUSTOM = "__custom__";

function RangeControl({ name, label, compact, presets, value, onApply }: ControlProps & { presets: string[] }) {
  const text = shownText(value);
  const ends = splitRange(text);
  const [custom, setCustom] = useState(ends !== null);
  useEffect(() => {
    if (ends !== null) setCustom(true);
    else if (text !== "") setCustom(false);
  }, [text]);
  const [from, to] = ends ?? ["", ""];
  const apply = (nextFrom: string, nextTo: string) => onApply(name, `${nextFrom}..${nextTo}`);
  return (
    <Labelled label={label} compact={compact}>
      <span style={{ display: "flex", gap: 6, alignItems: "center" }}>
        <select
          className="ox-select"
          data-testid={`lens-param-${name}`}
          aria-label={compact ? label : undefined}
          value={custom ? CUSTOM : text}
          onChange={(e) => {
            if (e.target.value === CUSTOM) setCustom(true);
            else {
              setCustom(false);
              onApply(name, e.target.value);
            }
          }}
        >
          {!custom && text !== "" && !presets.includes(text) ? <option value={text}>{rangePresetLabel(text)}</option> : null}
          {presets.map((p) => (
            <option key={p} value={p}>
              {rangePresetLabel(p)}
            </option>
          ))}
          <option value={CUSTOM}>Custom…</option>
        </select>
        {custom ? (
          <>
            <input type="date" className="ox-input" data-testid={`lens-param-${name}-from`} aria-label={`${label} from`} value={from} onChange={(e) => apply(e.target.value, to)} />
            <input type="date" className="ox-input" data-testid={`lens-param-${name}-to`} aria-label={`${label} to`} value={to} onChange={(e) => apply(from, e.target.value)} />
          </>
        ) : null}
      </span>
    </Labelled>
  );
}
