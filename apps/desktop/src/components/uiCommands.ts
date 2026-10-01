/// Extensions' commands in core menus (P6b.C4, `ui.commands`): a page's
/// nav-bar menu (`menu`) and a row's right-click menu (`context`) offer
/// the commands about that ref's kind, grouped under the provider or
/// extension, each run as the person with the ref bound into its input.
import type { MenuItem } from "../menu.js";
import { parseRef } from "../refs/ref.js";
import type { UiCommand, UiPlacement } from "../tauri-bridge/generated/bindings.js";

/** The commands about `ref`'s kind that show in `placement`. */
export function uiCommandsAbout(commands: UiCommand[], ref: string, placement: UiPlacement): UiCommand[] {
  const kind = parseRef(ref)?.kind;
  if (!kind) return [];
  return commands.filter((c) => c.about === kind && c.placement.includes(placement));
}

/** `input` with every whole-value `{{ref}}` / `{{ref.id}}` string bound
 *  to `ref` and its id. */
export function bindRefInput(input: unknown, ref: string): unknown {
  const id = parseRef(ref)?.id ?? ref;
  const walk = (v: unknown): unknown => {
    if (typeof v === "string") {
      const m = /^\s*\{\{\s*ref(\.id)?\s*\}\}\s*$/.exec(v);
      if (!m) return v;
      return m[1] ? id : ref;
    }
    if (Array.isArray(v)) return v.map(walk);
    if (v && typeof v === "object") {
      return Object.fromEntries(Object.entries(v as Record<string, unknown>).map(([k, x]) => [k, walk(x)]));
    }
    return v;
  };
  return walk(input);
}

/** Commands by group (a provider's id, or an extension's name), in the
 *  order groups first appear. */
export function groupUiCommands(commands: UiCommand[]): { group: string; commands: UiCommand[] }[] {
  const out: { group: string; commands: UiCommand[] }[] = [];
  for (const c of commands) {
    const g = out.find((x) => x.group === c.group);
    if (g) g.commands.push(c);
    else out.push({ group: c.group, commands: [c] });
  }
  return out;
}

/** A row menu's tail for `ref`: a separator, then one submenu per group.
 *  Empty when there's nothing to offer. */
export function uiCommandMenuItems(
  commands: UiCommand[],
  ref: string,
  run: (command: UiCommand, input: unknown) => void,
): MenuItem[] {
  if (commands.length === 0) return [];
  return [
    { id: "ui-commands-separator", label: "", enabled: false, separator: true },
    ...groupUiCommands(commands).map((g) => ({
      id: `ui-commands-${g.group}`,
      label: g.group,
      enabled: true,
      submenu: g.commands.map((c) => ({
        id: `ui-command-${c.id}`,
        label: c.label,
        enabled: true,
        run: () => run(c, bindRefInput(c.input, ref)),
      })),
    })),
  ];
}
