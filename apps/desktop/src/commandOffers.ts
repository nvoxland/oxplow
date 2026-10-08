/// What search offers of the command bus (`.context/commands.md` "Offering
/// a command to a person"): every command a person may run that says how
/// a person meets it (its `ui`) and needs no ref, run the way its `ui`
/// says — open the page that gathers its input, run it in the background,
/// or run it and open what it made. Pure, given its deps.
import type { CommandEntry } from "./components/quickOpenResults.js";
import type { CommandOutcome, CommandSpec } from "./tauri-bridge/generated/bindings.js";
import { streamRef, threadRef } from "./recordRefs.js";

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
  /** Open one of the window's own forms (`new-thread`, `commit`): a
   *  `ui.form` that isn't a tab id. */
  openForm(name: string): void;
  /** Whether it can run here now (the window has something for it to act
   *  on); unsaid, it can. One that can't is listed greyed in a menu and
   *  not offered by search. */
  available?(spec: CommandSpec): boolean;
  /** Run it as the person (asking first where it asks); its outcome, or
   *  `null` when it failed or waits for their confirmation. */
  run(label: string, id: string, input: unknown): Promise<CommandOutcome | null>;
  /** Run it as a background task, reporting a failure. */
  runInBackground(label: string, id: string, input: unknown): void;
}

const BINDINGS: Record<string, (ctx: OfferContext) => string | null | undefined> = {
  "{{stream}}": (c) => (c.streamId ? streamRef(c.streamId) : null),
  "{{thread}}": (c) => (c.threadId ? threadRef(c.threadId) : null),
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

/** `spec`'s entry, run with `input` (or its form). */
function offer(spec: CommandSpec, input: unknown, deps: OfferDeps): CommandEntry {
  const ui = spec.ui!;
  const group = ui.group ?? "Commands";
  return {
    id: spec.id,
    group,
    label: ui.label,
    searchKey: [group, ui.label, ...ui.keywords].join(" ").toLowerCase(),
    shortcut: ui.shortcut ?? undefined,
    whileTyping: ui.while_typing,
    menu: ui.menu ?? null,
    enabled: deps.available ? deps.available(spec) : true,
    run: () => {
      // A tab id (`page:new-task`) is a page; anything else one of the
      // window's own forms.
      if (ui.form) return ui.form.includes(":") ? deps.openPage(ui.form) : deps.openForm(ui.form);
      if (ui.background) return deps.runInBackground(ui.label, spec.id, input);
      void deps.run(ui.label, spec.id, input).then((out) => {
        if (out && ui.open_after) deps.openPage(withResult(ui.open_after, out.result));
      });
    },
  };
}

/** The ref-less commands of `specs` available in `ctx`, as search entries. */
export function commandOffers(specs: CommandSpec[], ctx: OfferContext, deps: OfferDeps): CommandEntry[] {
  const out: CommandEntry[] = [];
  for (const spec of specs) {
    const ui = spec.ui;
    if (!ui || ui.about) continue;
    // A form gathers the input itself; otherwise it must bind here.
    const bound = ui.form ? { input: null } : bindInput(ui.input, ctx);
    if (bound) out.push(offer(spec, bound.input, deps));
  }
  return out;
}

/** The commands of `specs` about `ref`'s kind (`ui.about`) — what a
 *  ref's page menu and its rows' right-click menus offer — with `ref`
 *  bound into their input (`{ ref: "{{ref}}" }` when they say none). */
export function refOffers(specs: CommandSpec[], ref: string, ctx: OfferContext, deps: OfferDeps): CommandEntry[] {
  const kind = ref.slice(0, ref.indexOf(":"));
  if (!kind) return [];
  const out: CommandEntry[] = [];
  for (const spec of specs) {
    const ui = spec.ui;
    if (!ui || ui.about !== kind) continue;
    const bound = ui.form ? { input: null } : bindInput(ui.input ?? { ref: "{{ref}}" }, { ...ctx, ref });
    if (bound) out.push(offer(spec, bound.input, deps));
  }
  return out;
}
