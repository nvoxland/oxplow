/// Test support: person specs that say how a person meets them, and an
/// offer host recording what runs (`refCommands.ts`).
import type { CommandSpec, CommandUi } from "../tauri-bridge/generated/bindings.js";
import { setRefOfferHost } from "./refCommands.js";

export function personSpec(id: string, ui: Partial<CommandUi>): CommandSpec {
  return {
    id,
    summary: "",
    input_schema: {},
    invokers: { human: true, agent: false, lens: false },
    confirm: "never",
    undoable: false,
    lifecycle: "stable",
    atomicity: "tx",
    effect: "write",
    needs: [],
    op: null,
    ui: {
      label: id,
      group: null,
      keywords: [],
      about: null,
      input: null,
      form: null,
      open_after: null,
      background: false,
      shortcut: null,
      while_typing: false,
      menu: null,
      ...ui,
    },
  } as unknown as CommandSpec;
}

/** Route ref offers' runs into `ran`. */
export function recordRefOffers(ran: Array<[string, unknown]>): void {
  setRefOfferHost({
    ctx: { streamId: null, threadId: null },
    deps: {
      openPage: () => {},
      openForm: () => {},
      runInBackground: () => {},
      run: async (_label, id, input) => {
        ran.push([id, input]);
        return null;
      },
    },
  });
}
