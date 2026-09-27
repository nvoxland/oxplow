/// The `settings` slot (tsk330): one Settings section per extension that
/// mounts lenses there, titled with the extension's name. Nothing renders
/// when no extension does. See `.context/extensions.md` → "Slots".
import { useEffect, useState, type ReactNode } from "react";

import { listExtensions, subscribeOxplowEvents } from "../api.js";
import { LensSlots } from "./LensSlots.js";
import { shouldRerunLens, slotExtensions } from "./lensModel.js";

const NO_PARAMS = {};

export function SettingsSlotSections({
  section,
}: {
  /** The host page's section chrome (title + body). */
  section(title: string, body: ReactNode): ReactNode;
}) {
  const [names, setNames] = useState<string[]>([]);
  useEffect(() => {
    const load = () =>
      void listExtensions(null)
        .then((exts) => setNames(slotExtensions(exts, "settings")))
        .catch(() => setNames([]));
    load();
    return subscribeOxplowEvents((event) => {
      if (shouldRerunLens({ kind: event.kind, path: (event as { path?: unknown }).path })) load();
    });
  }, []);
  return (
    <div data-testid="settings-slot">
      {names.map((name) => (
        <div key={name} data-testid={`settings-slot-${name}`}>
          {section(name, <LensSlots slot="settings" extension={name} params={NO_PARAMS} streamId={null} />)}
        </div>
      ))}
    </div>
  );
}
