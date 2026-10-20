import { useEffect, useId, useRef, useState } from "react";

import { querySql, type SqlCell, type SqlQueryResult } from "../api.js";
import { recordOpError } from "../components/opErrorsStore.js";
import { parseParamInput } from "../lens/lensModel.js";
import { paramKind, paramOptions, paramOptionsSql, type ParamKind, type ParamOption } from "../lens/lensParams.js";
import type { LensParam, ParamType } from "../tauri-bridge/generated/bindings.js";
import { humanize } from "./humanize.js";
import { Popover } from "./Popover.js";
import { DEFAULT_RANGE_PRESETS, rangePresetLabel, splitRange } from "./rangePresets.js";
import { useDebounced } from "./useDebounced.js";

/** How long a search waits for the next keystroke before it applies. */
const SEARCH_DEBOUNCE_MS = 250;
/** The rows a list shows before the rest fold behind More. */
const SHOWN_ROWS = 6;

type Query = (sql: string) => Promise<SqlQueryResult>;

export interface ParamRowProps {
  param: LensParam;
  value: SqlCell;
  onApply(name: string, value: SqlCell): void;
  /** Reads the options of a branch or id picker; `querySql` unless a test injects one. */
  query?: Query;
}

/**
 * One lens param in a page's rail: its label beside its value. Clicking
 * the value edits it in place, or, when the control doesn't fit the row
 * (a custom date range), in a popover over the page. Enter applies,
 * Escape keeps the value from before the edit, and focus comes back to
 * the value. A bool has nothing to open: its value is the checkbox.
 */
export function ParamRow({ param, value, onApply, query = querySql }: ParamRowProps) {
  const name = param.name;
  const label = param.label ?? humanize(name);
  const type = paramType(param);
  const kind = type === "text" ? paramKind(name) : null;
  const options = usePickerOptions(kind, query);
  // What it applied shows at once; the run's echo replaces it when it comes.
  const [shown, setShown] = useState<SqlCell>(value);
  useEffect(() => setShown(value), [value]);
  const [editing, setEditing] = useState(false);
  const valueButton = useRef<HTMLButtonElement>(null);
  const refocus = useRef(false);
  useEffect(() => {
    if (!editing && refocus.current) {
      refocus.current = false;
      valueButton.current?.focus();
    }
  }, [editing]);
  const labelId = useId();

  const apply = (next: SqlCell) => {
    setShown(next);
    onApply(name, next);
  };
  const done = () => {
    refocus.current = true;
    setEditing(false);
  };
  const said = summary(param, type, shown, options);
  const editor = { name, label, value: shown, onApply: apply, onDone: done };

  return (
    <div
      className={sameValue(type, shown, param.default ?? null) ? "ox-param" : "ox-param ox-param-changed"}
      data-testid={`lens-param-${name}`}
      onBlur={(e) => {
        // Focus left the row (not into its popover): the edit is over.
        if (editing && !e.currentTarget.contains(e.relatedTarget as Node | null)) setEditing(false);
      }}
    >
      <span className="ox-param-label" id={labelId}>
        {label}
      </span>
      {type === "bool" ? (
        <input
          type="checkbox"
          className="ox-param-check"
          data-testid={`lens-param-${name}-edit`}
          aria-label={label}
          checked={truthy(shown)}
          onChange={(e) => apply(e.target.checked)}
        />
      ) : editing ? (
        <span className="ox-param-edit">
          {type === "choice" ? (
            <SelectEditor {...editor} options={(param.options ?? []).map((o) => ({ value: o.value, label: o.label }))} />
          ) : type === "branch" ? (
            <BranchEditor {...editor} query={query} />
          ) : type === "search" ? (
            <SearchEditor {...editor} />
          ) : type === "date_range" ? (
            <RangeEditor {...editor} presets={param.presets && param.presets.length > 0 ? param.presets : DEFAULT_RANGE_PRESETS} />
          ) : kind ? (
            <PickerEditor {...editor} options={options} />
          ) : (
            <TextEditor {...editor} />
          )}
        </span>
      ) : (
        <button
          ref={valueButton}
          type="button"
          className="ox-param-value"
          data-testid={`lens-param-${name}-value`}
          aria-describedby={labelId}
          title={said ?? undefined}
          onClick={() => setEditing(true)}
        >
          {said ?? <span className="ox-faint">Any</span>}
        </button>
      )}
    </div>
  );
}

export interface ParamListProps {
  params: readonly LensParam[];
  values: Record<string, SqlCell>;
  onApply(name: string, value: SqlCell): void;
  query?: Query;
}

/** A page's params as rows, in their declared order: six, the rest behind
 *  More (N) — except one that is set, which always shows. */
export function ParamList({ params, values, onApply, query }: ParamListProps) {
  const [all, setAll] = useState(false);
  const shown = params.filter((p, i) => all || i < SHOWN_ROWS || !sameValue(paramType(p), values[p.name] ?? null, p.default ?? null));
  const folded = params.length - shown.length;
  return (
    <div className="ox-params">
      {shown.map((p) => (
        <ParamRow key={p.name} param={p} value={values[p.name] ?? null} onApply={onApply} query={query} />
      ))}
      {folded > 0 ? (
        <button type="button" className="ox-link ox-params-more" data-testid="lens-params-more" onClick={() => setAll(true)}>
          More ({folded})
        </button>
      ) : null}
    </div>
  );
}

/** A param's type, as its lens says or as it follows from `options`. */
function paramType(param: LensParam): ParamType {
  return param.type ?? ((param.options?.length ?? 0) > 0 ? "choice" : "text");
}

/** Whether a value is the param's default: a bool by truth (its echo is
 *  0/1), the rest by text. */
export function isDefault(param: LensParam, value: SqlCell): boolean {
  return sameValue(paramType(param), value, param.default ?? null);
}

function sameValue(type: ParamType, a: SqlCell, b: SqlCell): boolean {
  if (type === "bool") return truthy(a) === truthy(b);
  return text(a) === text(b);
}

function truthy(value: SqlCell): boolean {
  return value === true || (typeof value === "number" && value !== 0) || (typeof value === "string" && value !== "" && value !== "0" && value !== "false");
}

function text(value: SqlCell): string {
  return value === null || value === undefined ? "" : String(value);
}

/** What a row says its value is; null when it has none. */
function summary(param: LensParam, type: ParamType, value: SqlCell, options: ParamOption[]): string | null {
  const t = text(value);
  if (t === "") return type === "branch" ? "All branches" : null;
  switch (type) {
    case "choice":
      return param.options?.find((o) => o.value === t)?.label ?? t;
    case "date_range": {
      const ends = splitRange(t);
      return ends ? `${ends[0] || "…"} – ${ends[1] || "…"}` : rangePresetLabel(t);
    }
    case "text":
      return options.find((o) => o.value === value)?.label ?? t;
    default:
      return t;
  }
}

/** An id param's options (`lensParams.ts`), read once; none for any other. */
function usePickerOptions(kind: ParamKind | null, query: Query): ParamOption[] {
  const [options, setOptions] = useState<ParamOption[]>([]);
  useEffect(() => {
    if (!kind) return;
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
  return options;
}

interface EditorProps {
  name: string;
  label: string;
  value: SqlCell;
  onApply(value: SqlCell): void;
  /** The edit is over: the row shows its value again. */
  onDone(): void;
}

function TextEditor({ name, label, value, onApply, onDone: done }: EditorProps) {
  const onDone = useOnce(done);
  const [draft, setDraft] = useState(text(value));
  const commit = () => {
    if (onDone.over()) return;
    if (draft !== text(value)) onApply(parseParamInput(draft));
    onDone();
  };
  return (
    <input
      autoFocus
      className="ox-input"
      data-testid={`lens-param-${name}-edit`}
      aria-label={label}
      value={draft}
      onChange={(e) => setDraft(e.target.value)}
      onKeyDown={(e) => {
        if (e.key === "Enter") commit();
        if (e.key === "Escape") onDone();
      }}
      onBlur={commit}
    />
  );
}

/** Applies as you type (after a pause), at once on Enter; Escape puts back
 *  the value it opened with. */
function SearchEditor({ name, label, value, onApply, onDone: done }: EditorProps) {
  const onDone = useOnce(done);
  const opened = useRef(text(value));
  const applied = useRef(text(value));
  const [draft, setDraft] = useState(text(value));
  const apply = (next: string) => {
    if (next === applied.current) return;
    applied.current = next;
    onApply(next === "" ? null : next);
  };
  const settled = useDebounced(draft, SEARCH_DEBOUNCE_MS);
  useEffect(() => apply(settled), [settled]);
  return (
    <input
      autoFocus
      type="search"
      className="ox-input"
      data-testid={`lens-param-${name}-edit`}
      aria-label={label}
      value={draft}
      onChange={(e) => setDraft(e.target.value)}
      onKeyDown={(e) => {
        if (e.key === "Enter") {
          apply(draft);
          onDone();
        }
        if (e.key === "Escape") {
          apply(opened.current);
          onDone();
        }
      }}
      onBlur={() => {
        if (onDone.over()) return;
        apply(draft);
        onDone();
      }}
    />
  );
}

/** `done` that runs once: a blur after Enter or Escape (the input
 *  leaving the page) is no second end, and applies nothing. */
function useOnce(done: () => void): (() => void) & { over(): boolean } {
  const ran = useRef(false);
  const once = () => {
    if (ran.current) return;
    ran.current = true;
    done();
  };
  return Object.assign(once, { over: () => ran.current });
}

interface Option {
  value: string;
  label: string;
}

/** A select that applies the option picked and closes; Escape closes. */
function SelectEditor({ name, label, value, onApply, onDone, options, empty }: EditorProps & { options: Option[]; empty?: Option }) {
  const current = text(value);
  return (
    <select
      autoFocus
      className="ox-select"
      data-testid={`lens-param-${name}-edit`}
      aria-label={label}
      value={current}
      onChange={(e) => {
        onApply(e.target.value === "" && empty ? null : e.target.value);
        onDone();
      }}
      onKeyDown={(e) => {
        if (e.key === "Escape") onDone();
      }}
    >
      {empty ? <option value="">{empty.label}</option> : null}
      {current !== "" && !options.some((o) => o.value === current) ? <option value={current}>{current}</option> : null}
      {options.map((o) => (
        <option key={o.value} value={o.value}>
          {o.label}
        </option>
      ))}
    </select>
  );
}

const BRANCHES_SQL = "SELECT name FROM v_branch WHERE kind = 'local' ORDER BY name";

function BranchEditor(props: EditorProps & { query: Query }) {
  const { query } = props;
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
  return <SelectEditor {...props} empty={{ value: "", label: "All branches" }} options={branches.map((b) => ({ value: b, label: b }))} />;
}

/** One of oxplow's id params: pick what it names, by title. */
function PickerEditor({ name, label, value, onApply, onDone, options }: EditorProps & { options: ParamOption[] }) {
  const at = options.findIndex((o) => o.value === value);
  return (
    <select
      autoFocus
      className="ox-select"
      data-testid={`lens-param-${name}-edit`}
      aria-label={label}
      value={at === -1 ? "" : String(at)}
      onChange={(e) => {
        const picked = options[Number(e.target.value)];
        if (picked) onApply(picked.value);
        onDone();
      }}
      onKeyDown={(e) => {
        if (e.key === "Escape") onDone();
      }}
    >
      {at === -1 ? <option value="">{value === null ? "—" : String(value)}</option> : null}
      {options.map((o, i) => (
        <option key={i} value={String(i)}>
          {o.label}
        </option>
      ))}
    </select>
  );
}

const CUSTOM = "__custom__";

/** Its presets in place; Custom… opens two dates in a popover, which
 *  apply as they change. */
function RangeEditor({ name, label, value, onApply, onDone, presets }: EditorProps & { presets: string[] }) {
  const current = text(value);
  const ends = splitRange(current);
  const [custom, setCustom] = useState(ends !== null);
  const anchor = useRef<HTMLSelectElement>(null);
  const [from, to] = ends ?? ["", ""];
  return (
    <>
      <select
        ref={anchor}
        autoFocus
        className="ox-select"
        data-testid={`lens-param-${name}-edit`}
        aria-label={label}
        value={custom ? CUSTOM : current}
        onChange={(e) => {
          if (e.target.value === CUSTOM) setCustom(true);
          else {
            onApply(e.target.value);
            onDone();
          }
        }}
        onKeyDown={(e) => {
          if (e.key === "Escape") onDone();
        }}
      >
        {!custom && current !== "" && !presets.includes(current) ? <option value={current}>{rangePresetLabel(current)}</option> : null}
        {presets.map((p) => (
          <option key={p} value={p}>
            {rangePresetLabel(p)}
          </option>
        ))}
        <option value={CUSTOM}>Custom…</option>
      </select>
      {custom ? (
        <Popover anchor={anchor} label={`${label}: custom range`} onClose={onDone} testId={`lens-param-${name}-custom`}>
          <div className="ox-form">
            <label className="ox-field">
              <span className="ox-label">From</span>
              <input
                autoFocus
                type="date"
                className="ox-input"
                data-testid={`lens-param-${name}-from`}
                value={from}
                onChange={(e) => onApply(`${e.target.value}..${to}`)}
              />
            </label>
            <label className="ox-field">
              <span className="ox-label">To</span>
              <input type="date" className="ox-input" data-testid={`lens-param-${name}-to`} value={to} onChange={(e) => onApply(`${from}..${e.target.value}`)} />
            </label>
          </div>
        </Popover>
      ) : null}
    </>
  );
}
