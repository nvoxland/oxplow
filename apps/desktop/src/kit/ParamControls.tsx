import { createContext, useCallback, useContext, useEffect, useId, useRef, useState } from "react";

import { querySql, type SqlCell, type SqlQueryResult } from "../api.js";
import { recordOpError } from "../opErrors.js";
import { isDefaultValue, parseParamInput } from "../lens/lensModel.js";
import { NO_READS, useRerunOnChange } from "../lens/lensRerun.js";
import { paramKind, paramOptions, paramOptionsSql, type ParamKind, type ParamOption } from "../lens/lensParams.js";
import type { PageParam, ParamType, Reads } from "../tauri-bridge/generated/bindings.js";
import { humanize } from "./humanize.js";
import { Popover } from "./Popover.js";
import { DEFAULT_RANGE_PRESETS, rangePresetLabel, splitRange } from "./rangePresets.js";
import { useHistoryState } from "../tabs/useHistoryState.js";
import { useDebounced } from "./useDebounced.js";
import { daemonPageReads, pageOptionsSource } from "../pages/pageReads.js";

/** How long a search waits for the next keystroke before it applies. */
const SEARCH_DEBOUNCE_MS = 250;
/** The rows a list shows before the rest fold behind More. */
const SHOWN_ROWS = 6;

type Query = (sql: string) => Promise<SqlQueryResult>;

/** What a choice's `options_from` is read with: which of the page's
 *  params what it names takes, and its rows for them. It names a page
 *  (`<ext>/<page>`, its first loader, or `<ext>/<page>#<loader>`), run
 *  once (`pages/pageReads.ts` `pageOptionsSource`: the daemon's, or
 *  `PageRail`'s over its page reads); a test gives another. */
export interface OptionsSource {
  params(ref: string): Promise<string[]>;
  list(ref: string, params: Record<string, SqlCell>, streamId: string | null): Promise<SqlQueryResult>;
}

export const OptionsSourceContext = createContext<OptionsSource>(pageOptionsSource(daemonPageReads));

export interface ParamRowProps {
  param: PageParam;
  value: SqlCell;
  onApply(name: string, value: SqlCell): void;
  /** Reads the options of a branch or id picker; `querySql` unless a test injects one. */
  query?: Query;
  /** The page's current param values: what a choice's `options_from` page
   *  is run with (the ones it declares). */
  values?: Record<string, SqlCell>;
  /** The stream the page runs for: what that page runs for. */
  streamId?: string | null;
  /** Reads what `options_from` names; the context's (a page file's rail's, or a test's). */
  optionsSource?: OptionsSource;
}

/**
 * One lens param in a page's rail: its label beside its value. Clicking
 * the value edits it in place, or, when the control doesn't fit the row
 * (a custom date range), in a popover over the page. Enter applies,
 * Escape keeps the value from before the edit, and focus comes back to
 * the value. A bool has nothing to open: its value is the checkbox.
 */
export function ParamRow({ param, value, onApply, query = querySql, values = {}, streamId = null, optionsSource }: ParamRowProps) {
  const fromContext = useContext(OptionsSourceContext);
  const name = param.name;
  const label = param.label ?? humanize(name);
  const type = paramType(param);
  const kind = type === "text" ? paramKind(name) : null;
  const options = usePickerOptions(kind, query);
  const listed = useListedOptions(type === "choice" ? (param.options_from ?? null) : null, values, streamId, optionsSource ?? fromContext);
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
  const said = summary(param, type, shown, options, listed);
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
            <SelectEditor {...editor} options={param.options_from ? listed : (param.options ?? []).map((o) => ({ value: o.value, label: o.label }))} />
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
  params: readonly PageParam[];
  values: Record<string, SqlCell>;
  onApply(name: string, value: SqlCell): void;
  query?: Query;
  /** The stream the page runs for (a choice's `options_from` page runs for it). */
  streamId?: string | null;
}

/** A page's params as rows, in their declared order: six, the rest behind
 *  More (N) — except one that is set, which always shows. */
export function ParamList({ params, values, onApply, query, streamId = null }: ParamListProps) {
  const [all, setAll] = useHistoryState<boolean>("paramsMore", false);
  const shown = params.filter((p, i) => all || i < SHOWN_ROWS || !sameValue(paramType(p), values[p.name] ?? null, p.default ?? null));
  const folded = params.length - shown.length;
  return (
    <div className="ox-params">
      {shown.map((p) => (
        <ParamRow key={p.name} param={p} value={values[p.name] ?? null} values={values} streamId={streamId} onApply={onApply} query={query} />
      ))}
      {folded > 0 ? (
        <button type="button" className="ox-link ox-params-more" data-testid="lens-params-more" onClick={() => setAll(true)}>
          More ({folded})
        </button>
      ) : null}
    </div>
  );
}

/** A param's type, as its lens says or as it follows from `options` /
 *  `options_from`. */
function paramType(param: PageParam): ParamType {
  return param.type ?? ((param.options?.length ?? 0) > 0 || param.options_from ? "choice" : "text");
}

/** Whether a value is the param's default (`isDefaultValue`). */
export function isDefault(param: PageParam, value: SqlCell): boolean {
  return isDefaultValue(param, value);
}

function sameValue(type: ParamType, a: SqlCell, b: SqlCell): boolean {
  return isDefaultValue({ type, options: [], default: b }, a);
}

function truthy(value: SqlCell): boolean {
  return value === true || (typeof value === "number" && value !== 0) || (typeof value === "string" && value !== "" && value !== "0" && value !== "false");
}

function text(value: SqlCell): string {
  return value === null || value === undefined ? "" : String(value);
}

/** One end of a custom range as the row says it: a date as it is, a
 *  date-time (a chart's brush) to the minute. */
function rangeEnd(end: string): string {
  return end === "" ? "…" : end.length > 10 ? end.slice(0, 16).replace("T", " ") : end;
}

/** What a row says its value is; null when it has none. A dynamic choice
 *  says the raw value until its options arrive. */
function summary(param: PageParam, type: ParamType, value: SqlCell, options: ParamOption[], listed: Option[]): string | null {
  const t = text(value);
  if (t === "") return type === "branch" ? "All branches" : null;
  switch (type) {
    case "choice":
      return (param.options_from ? listed : param.options ?? []).find((o) => o.value === t)?.label ?? t;
    case "date_range": {
      const ends = splitRange(t);
      return ends ? `${rangeEnd(ends[0])} – ${rangeEnd(ends[1])}` : rangePresetLabel(t);
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
  /** An `<optgroup>` the option sits in (a dynamic choice's `group`). */
  group?: string | null;
}

/** The options of a choice whose `options_from` names a page:
 *  its rows for the page's stream with the page's values it declares,
 *  read again when what it read changes (the one place a param control
 *  follows data). Rows are `value`, `label`, optional `group`. */
function useListedOptions(ref: string | null, values: Record<string, SqlCell>, streamId: string | null, source: OptionsSource): Option[] {
  const [options, setOptions] = useState<Option[]>([]);
  const [reads, setReads] = useState<Reads>(NO_READS);
  const [declared, setDeclared] = useState<string[] | null>(null);
  useEffect(() => {
    setDeclared(null);
    if (!ref) return;
    let live = true;
    source
      .params(ref)
      .then((names) => {
        if (live) setDeclared(names);
      })
      .catch((e: unknown) => recordOpError({ label: `Read ${ref}`, message: e instanceof Error ? e.message : String(e) }));
    return () => {
      live = false;
    };
  }, [ref, source]);
  const params = declared ? Object.fromEntries(declared.filter((n) => n in values).map((n) => [n, values[n] ?? null])) : null;
  const paramsKey = JSON.stringify(params);
  const live = useRef(true);
  const load = useCallback(() => {
    if (!ref || !params) return;
    const started = live;
    source
      .list(ref, params, streamId)
      .then((result) => {
        if (!started.current) return;
        setOptions(listedOptions(result));
        setReads(result.reads ?? NO_READS);
      })
      .catch((e: unknown) => recordOpError({ label: `List ${ref}`, message: e instanceof Error ? e.message : String(e) }));
    // `params` is `paramsKey`'s content.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [ref, paramsKey, streamId, source]);
  useEffect(() => {
    live.current = true;
    load();
    return () => {
      live.current = false;
    };
  }, [load]);
  useRerunOnChange(reads, load);
  return options;
}

/** A choice lens's rows as options: `value`, `label` (the value when
 *  none), optional `group`. */
function listedOptions(result: SqlQueryResult): Option[] {
  const v = result.columns.indexOf("value");
  const l = result.columns.indexOf("label");
  const g = result.columns.indexOf("group");
  return result.rows.map((row) => {
    const value = text(row[v] ?? null);
    const label = row[l];
    const group = g === -1 ? null : row[g];
    return { value, label: label === null || label === undefined ? value : String(label), group: group === null || group === undefined ? null : String(group) };
  });
}

/** A select that applies the option picked and closes; Escape closes.
 *  Options with a `group` sit in an `<optgroup>`, after the ungrouped. */
function SelectEditor({ name, label, value, onApply, onDone, options, empty }: EditorProps & { options: Option[]; empty?: Option }) {
  const current = text(value);
  const groups: string[] = [];
  for (const o of options) if (o.group && !groups.includes(o.group)) groups.push(o.group);
  const option = (o: Option) => (
    <option key={o.value} value={o.value}>
      {o.label}
    </option>
  );
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
      {options.filter((o) => !o.group).map(option)}
      {groups.map((group) => (
        <optgroup key={group} label={group}>
          {options.filter((o) => o.group === group).map(option)}
        </optgroup>
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
  // A date input holds a date; a brushed end keeps its time until edited.
  const [from, to] = (ends ?? ["", ""]).map((e) => e.slice(0, 10));
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
