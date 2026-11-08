/// Where a person's failed command shows (`.context/usability.md` "A
/// person's command that fails"): beside the control that ran it while that
/// control is on screen, else the toast. The window keeps each control's
/// last failure by its key (an anchor: `action/<id>`, `proposal/<id>`), so a
/// control drawn again (a list re-read) still shows it; and which recorded
/// failures it showed beside their control, so the toast skips those.

/** A control's last failure. */
export interface InlineFailure {
  message: string;
  code: string | null;
  /** The recorded failure (`v_op_error.event_id`) once the report lands. */
  errorId: string | null;
  at: number;
}

export interface FailureStore {
  get(key: string): InlineFailure | null;
  set(key: string, failure: InlineFailure): void;
  clear(key: string): void;
  subscribe(listener: () => void): () => void;
  /** A control with this key is on screen until the returned call. Counted,
   *  so a control drawn again before the old one goes stays on screen. */
  mount(key: string): () => void;
  isMounted(key: string): boolean;
}

export function createFailureStore(): FailureStore {
  const failures = new Map<string, InlineFailure>();
  const mounted = new Map<string, number>();
  const listeners = new Set<() => void>();
  const changed = () => {
    for (const l of [...listeners]) l();
  };
  return {
    get: (key) => failures.get(key) ?? null,
    set: (key, failure) => {
      failures.set(key, failure);
      changed();
    },
    clear: (key) => {
      if (failures.delete(key)) changed();
    },
    subscribe: (listener) => {
      listeners.add(listener);
      return () => void listeners.delete(listener);
    },
    mount: (key) => {
      mounted.set(key, (mounted.get(key) ?? 0) + 1);
      let done = false;
      return () => {
        if (done) return;
        done = true;
        const n = (mounted.get(key) ?? 1) - 1;
        if (n > 0) mounted.set(key, n);
        else mounted.delete(key);
      };
    },
    isMounted: (key) => (mounted.get(key) ?? 0) > 0,
  };
}

/** The recorded failures the window showed beside their control. */
export interface InlineClaims {
  /** A failure `report` will record, shown beside a control: once the id is
   *  known and the grace has passed, it is the control's if `onScreen()`
   *  still holds, else the toast's. */
  claim(report: Promise<string | null>, onScreen: () => boolean): void;
  shown(id: string): boolean;
  /** Every claim decided: the toast waits on this before announcing a
   *  recorded failure. */
  settled(): Promise<void>;
}

/** How long a control may be away (a list re-read draws it again) and still
 *  be the place its failure shows. */
export const INLINE_GRACE_MS = 750;

export function createInlineClaims(
  later: (f: () => void, ms: number) => void = (f, ms) => void setTimeout(f, ms),
  grace = INLINE_GRACE_MS,
): InlineClaims {
  const shown = new Set<string>();
  let pending = 0;
  let waiters: (() => void)[] = [];
  const done = () => {
    pending -= 1;
    if (pending > 0) return;
    const ready = waiters;
    waiters = [];
    for (const w of ready) w();
  };
  return {
    claim: (report, onScreen) => {
      pending += 1;
      const waited = new Promise<void>((r) => later(r, grace));
      void Promise.all([report, waited]).then(([id]) => {
        if (id && onScreen()) shown.add(id);
        done();
      });
    },
    shown: (id) => shown.has(id),
    settled: () => (pending === 0 ? Promise.resolve() : new Promise((r) => waiters.push(r))),
  };
}

/** The window's. */
export const failureStore: FailureStore = createFailureStore();
export const inlineClaims: InlineClaims = createInlineClaims();
