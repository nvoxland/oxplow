/// What search offers of the command bus (`.context/commands.md` "Offering
/// a command to a person"): every command a person may run that says how
/// a person meets it (its `ui`) and needs no ref, run the way its `ui`
/// says — open the page that gathers its input, run it in the background,
/// or run it and open what it made. Pure, given its deps.
import type { CommandEntry } from "./components/quickOpenResults.js";
import type { CommandOutcome, CommandSpec } from "./tauri-bridge/generated/bindings.js";
import { keyOf } from "./uri/uri.js";
import { withPageParams } from "./tabs/pageRefs.js";
import { offeredWhen, type WhenContext } from "./when.js";

/** Where a command is offered from: what its input's bindings take, and
 *  what its `when` is evaluated against. */
export interface OfferContext {
  streamId: string | null;
  threadId: string | null;
  /** The ref the page or row is about; search has none. */
  ref?: string | null;
  /** The window's context keys (`when.ts`); unsaid, none hold. */
  when?: WhenContext;
}

export interface OfferDeps {
  /** Open a page by tab id (`page:new-task`). */
  openPage(tabId: string): void;
  /** Open one of the window's own forms (`new-thread`, `commit`): a
   *  `ui.form` that isn't a tab id. */
  openForm(name: string): void;
  /** Run it as the person (asking first where it asks); its outcome, or
   *  `null` when it failed or waits for their confirmation. */
  run(label: string, id: string, input: unknown): Promise<CommandOutcome | null>;
  /** Run it as a background task, reporting a failure. */
  runInBackground(label: string, id: string, input: unknown): void;
  /** The page command `id` opens after a run that gave `result` — its
   *  `ui.open_after`, bound by the server (`command_open_after`); `null`
   *  when there's none to open (a failure is reported). */
  openAfter(id: string, result: unknown): Promise<string | null>;
}

const BINDINGS: Record<string, (ctx: OfferContext) => string | null | undefined> = {
  "{{stream}}": (c) => (c.streamId ? c.streamId : null),
  "{{thread}}": (c) => (c.threadId ? c.threadId : null),
  "{{uri}}": (c) => c.ref,
  "{{uri.id}}": (c) => (c.ref ? (keyOf(c.ref) ?? null) : null),
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

/** `spec`'s entry, run with `input` (or its form). */
function offer(spec: CommandSpec, input: unknown, ctx: OfferContext, deps: OfferDeps, ref?: string): CommandEntry {
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
    // Its `when` holds here now; one that doesn't is greyed in a menu and
    // not run by its shortcut.
    enabled: offeredWhen(ui.when, ctx.when ?? {}),
    run: () => {
      // A tab id (`page:new-task`) is a page — given the ref the offer is
      // about, as `?uri=`, to prefill from; anything else one of the
      // window's own forms.
      if (ui.form) {
        if (!ui.form.includes(":")) return deps.openForm(ui.form);
        return deps.openPage(ref ? withPageParams(ui.form, { uri: ref }) : ui.form);
      }
      if (ui.background) return deps.runInBackground(ui.label, spec.id, input);
      void deps.run(ui.label, spec.id, input).then(async (out) => {
        if (!out || !ui.open_after) return;
        const page = await deps.openAfter(spec.id, out.result);
        if (page) deps.openPage(page);
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
    if (bound) out.push(offer(spec, bound.input, ctx, deps));
  }
  return out;
}

/** The commands of `specs` the daemon says apply to `ref` (`applies`, its
 *  `ref_commands`: the commands about its kind whose `ui.about_when` holds
 *  on its row) — what a ref's page menu and its rows' right-click menus
 *  offer — with `ref` bound into their input (`{ uri: "{{uri}}" }` when
 *  they say none). */
export function refOffers(
  specs: CommandSpec[],
  ref: string,
  applies: readonly string[],
  ctx: OfferContext,
  deps: OfferDeps,
): CommandEntry[] {
  const out: CommandEntry[] = [];
  for (const spec of specs) {
    const ui = spec.ui;
    if (!ui || !applies.includes(spec.id)) continue;
    const bound = ui.form ? { input: null } : bindInput(ui.input ?? { uri: "{{uri}}" }, { ...ctx, ref });
    if (bound) out.push(offer(spec, bound.input, ctx, deps, ref));
  }
  return out;
}
