// Latest-request-wins for async fetches whose inputs change: a slow
// response for old params (another thread, another lens) must not land
// over the new ones. `begin()` a request and apply its result only while
// the returned check still says it's current.
import { useEffect, useRef } from "react";

export interface RequestGuard {
  /** Start a request; the returned check is true until a newer one begins. */
  begin(): () => boolean;
  /** Retire every request in flight (inputs changed, or unmounting). */
  cancel(): void;
}

export function createRequestGuard(): RequestGuard {
  let latest = 0;
  return {
    begin() {
      const id = ++latest;
      return () => id === latest;
    },
    cancel() {
      latest++;
    },
  };
}

/** A component's guard, cancelled on unmount. */
export function useRequestGuard(): RequestGuard {
  const ref = useRef<RequestGuard | null>(null);
  ref.current ??= createRequestGuard();
  const guard = ref.current;
  useEffect(() => () => guard.cancel(), [guard]);
  return guard;
}
