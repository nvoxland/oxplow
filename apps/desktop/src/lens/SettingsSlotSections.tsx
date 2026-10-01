/// The `settings` slot (tsk330): one Settings section per extension that
/// mounts lenses there, titled with the extension's name. Nothing renders
/// when no extension does. See `.context/extensions.md` → "Slots".
import type { ReactNode } from "react";

import { useExtensions } from "../extensionsStore.js";
import { LensSlots } from "./LensSlots.js";
import { slotExtensions } from "./lensModel.js";

const NO_PARAMS = {};

export function SettingsSlotSections({
  section,
}: {
  /** The host page's section chrome (title + body). */
  section(title: string, body: ReactNode): ReactNode;
}) {
  const names = slotExtensions(useExtensions(null) ?? [], "settings.section");
  return (
    <div data-testid="settings-slot">
      {names.map((name) => (
        <div key={name} data-testid={`settings-slot-${name}`}>
          {section(name, <LensSlots slot="settings.section" extension={name} params={NO_PARAMS} streamId={null} />)}
        </div>
      ))}
    </div>
  );
}
