export interface DaemonProbeState {
  consecutiveFailures: number;
  unavailable: boolean;
}

export interface DaemonProbeDecision {
  next: DaemonProbeState;
  refresh: boolean;
}

export const INITIAL_DAEMON_PROBE_STATE: DaemonProbeState = {
  consecutiveFailures: 0,
  unavailable: false,
};

const FAILURE_THRESHOLD = 3;

export function advanceDaemonProbeState(
  state: DaemonProbeState,
  daemonAlive: boolean,
): DaemonProbeDecision {
  if (daemonAlive) {
    return {
      next: {
        consecutiveFailures: 0,
        unavailable: false,
      },
      refresh: state.unavailable,
    };
  }

  const consecutiveFailures = state.consecutiveFailures + 1;
  return {
    next: {
      consecutiveFailures,
      unavailable: consecutiveFailures >= FAILURE_THRESHOLD,
    },
    refresh: false,
  };
}
