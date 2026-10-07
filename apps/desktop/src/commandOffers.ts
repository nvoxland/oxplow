/// What search offers of the command bus (`.context/commands.md` "Offering
/// a command to a person"): every command a person may run that says how
/// a person meets it (its `ui`) and needs no ref, run the way its `ui`
/// says — open the page that gathers its input, run it in the background,
/// or run it and open what it made. Pure, given its deps.
import type { CommandEntry } from "./components/quickOpenResults.js";
import type { CommandOutcome, CommandSpec } from "./tauri-bridge/generated/bindings.js";

/** Where a command is offered from: what its input's bindings take. */
export interface OfferContext {
  streamId: string | null;
  threadId: string | null;
  /** The ref the page or row is about; search has none. */
  ref?: string | null;
}

export interface OfferDeps {
  /** Open a page by tab id (`page:new-task`). */
  openPage(tabId: string): void;
  /** Run it as the person (asking first where it asks); its outcome, or
   *  `null` when it failed or waits for their confirmation. */
  run(label: string, id: string, input: unknown): Promise<CommandOutcome | null>;
  /** Run it as a background task, reporting a failure. */
  runInBackground(label: string, id: string, input: unknown): void;
}

const BINDINGS: Record<string, (ctx: OfferContext) => string | null | undefined> = {
  "{{stream}}": (c) => c.streamId,
  "{{thread}}": (c) => c.threadId,
  "{{ref}}": (c) => c.ref,
  "{{ref.id}}": (c) => (c.ref ? c.ref.slice(c.ref.indexOf(":") + 1) : null),
};

/** `template` with each string that is exactly a binding replaced from
 *  `ctx`; `null` when a binding has nothing here (the command is
 *  unavailable here). No template is an empty input. */
export function bindInput(template: unknown, ctx: OfferContext): { input: unknown } | null {
  let missing = false;
  const bind = (v: unknown): unknown => {
    if (typeof v === "string" && v in BINDINGS) {
      const bound = BINDINGS[v](ctx);
      if (bound == null) missing = true;
      return bound;
    }
    if (Array.isArray(v)) return v.map(bind);
    if (v && typeof v === "object") {
      return Object.fromEntries(Object.entries(v).map(([k, x]) => [k, bind(x)]));
    }
    return v;
  };
  const input = template == null ? {} : bind(template);
  return missing ? null : { input };
}

/** `template` with each `{{result.<field>}}` taken from `result`. */
function withResult(template: string, result: unknown): string {
  return template.replace(/\{\{result\.([a-z_]+)\}\}/g, (_, field: string) => {
    const v = (result as Record<string, unknown> | null)?.[field];
    return v == null ? "" : String(v);
  });
}

/** The ref-less commands of `specs` available in `ctx`, as search entries. */
export function commandOffers(specs: CommandSpec[], ctx: OfferContext, deps: OfferDeps): CommandEntry[] {
  const out: CommandEntry[] = [];
  for (const spec of specs) {
    const ui = spec.ui;
    if (!ui || ui.about) continue;
    // A form gathers the input itself; otherwise it must bind here.
    const bound = ui.form ? { input: null } : bindInput(ui.input, ctx);
    if (!bound) continue;
    const group = ui.group ?? "Commands";
    out.push({
      id: spec.id,
      group,
      label: ui.label,
      searchKey: [group, ui.label, ...ui.keywords].join(" ").toLowerCase(),
      run: () => {
        if (ui.form) return deps.openPage(ui.form);
        if (ui.background) return deps.runInBackground(ui.label, spec.id, bound.input);
        void deps.run(ui.label, spec.id, bound.input).then((out) => {
          if (out && ui.open_after) deps.openPage(withResult(ui.open_after, out.result));
        });
      },
    });
  }
  return out;
}
