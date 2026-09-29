/**
 * Runs an async fetch on demand and delivers only the newest run's result:
 * a slower earlier response never overwrites a newer one, and nothing is
 * delivered after `close()` (a component that moved on).
 */
export function latestWins<T>(
  fetch: () => Promise<T>,
  onValue: (value: T) => void,
  onError: (err: unknown) => void,
): { run: () => void; close: () => void } {
  let newest = 0;
  let closed = false;
  return {
    run() {
      const mine = ++newest;
      fetch().then(
        (value) => {
          if (!closed && mine === newest) onValue(value);
        },
        (err) => {
          if (!closed && mine === newest) onError(err);
        },
      );
    },
    close() {
      closed = true;
    },
  };
}
