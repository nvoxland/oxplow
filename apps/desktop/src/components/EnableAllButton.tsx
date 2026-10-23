import { useState } from "react";

import { enableAllAgain } from "../contributionHealth.js";

/// Turn every failure-disabled contribution back on in one go. Shown only
/// when `count` says there is any.
export function EnableAllButton({ count }: { count: number }) {
  const [enabling, setEnabling] = useState(false);
  if (count < 1) return null;
  return (
    <button
      type="button"
      data-testid="extension-enable-all-all"
      title="Turn every disabled contribution back on, on this machine"
      disabled={enabling}
      onClick={() => {
        setEnabling(true);
        void enableAllAgain().finally(() => setEnabling(false));
      }}
    >
      {enabling ? "Enabling…" : `Enable All Disabled (${count})`}
    </button>
  );
}
