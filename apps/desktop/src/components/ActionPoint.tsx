/// A named action point a page places (`{ action_point: <id>, context }`):
/// the commands that joined it, drawn as nothing, a button or a menu
/// (`actionPointDraw.ts`). Each runs the way the window runs any offer,
/// with the point's context bound into its input by the daemon. A menu is
/// the ContextMenu opened under its button: Escape and a click outside
/// close it, and the button toggles it.
import { useEffect, useRef, useState } from "react";
import { ChevronDown } from "lucide-react";

import { actionPointCommands } from "../api.js";
import { pointOffers } from "../commandOffers.js";
import { recordOpError } from "../opErrors.js";
import { usePersonCommands } from "../personCommandsStore.js";
import { ContextMenu } from "./ContextMenu.js";
import { drawsFor } from "./actionPointDraw.js";
import { refOfferHost } from "./refCommands.js";
import type { CommandEntry } from "./quickOpenResults.js";
import type { MenuPosition } from "../menu.js";

export function ActionPoint({ point, context }: { point: string; context: Record<string, unknown> }) {
  const specs = usePersonCommands();
  const [entries, setEntries] = useState<CommandEntry[]>([]);
  const [label, setLabel] = useState(point);
  const [at, setAt] = useState<MenuPosition | null>(null);
  const button = useRef<HTMLButtonElement | null>(null);
  const key = JSON.stringify(context);

  useEffect(() => {
    let live = true;
    const host = refOfferHost();
    if (!host) return;
    actionPointCommands(point, JSON.parse(key) as Record<string, unknown>)
      .then((offer) => {
        if (!live) return;
        setLabel(offer.label);
        setEntries(pointOffers(specs, offer.commands, host.ctx, host.deps));
      })
      .catch((e: unknown) => {
        if (live) setEntries([]);
        recordOpError({ label: `Load ${point}`, message: e instanceof Error ? e.message : String(e) });
      });
    return () => {
      live = false;
    };
  }, [point, key, specs]);

  const draws = drawsFor(entries.length);
  if (draws === "none") return null;
  if (draws === "button") {
    const only = entries[0]!;
    return (
      <button type="button" data-testid={`action-point-${point}`} disabled={only.enabled === false} onClick={only.run}>
        {only.label}
      </button>
    );
  }
  const toggle = () => {
    if (at) return setAt(null);
    const r = button.current?.getBoundingClientRect();
    if (r) setAt({ x: r.left, y: r.bottom + 4 });
  };
  return (
    <>
      <button
        ref={button}
        type="button"
        data-testid={`action-point-${point}`}
        aria-haspopup="menu"
        aria-expanded={at !== null}
        onMouseDown={(e) => {
          if (at) e.stopPropagation();
        }}
        onClick={toggle}
      >
        {label} <ChevronDown size={14} aria-hidden />
      </button>
      {at ? (
        <ContextMenu
          position={at}
          onClose={() => setAt(null)}
          items={entries.map((e) => ({ id: `action-point-command-${e.id}`, label: e.label, enabled: e.enabled ?? true, run: e.run }))}
        />
      ) : null}
    </>
  );
}
