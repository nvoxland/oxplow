export interface DaemonProbeState {
  unavailable: boolean;
}

export interface DaemonProbeDecision {
  next: DaemonProbeState;
  refresh: boolean;
}

export const INITIAL_DAEMON_PROBE_STATE: DaemonProbeState = {
  unavailable: false,
};

export function advanceDaemonProbeState(
  state: DaemonProbeState,
  daemonAlive: boolean,
): DaemonProbeDecision {
  if (daemonAlive) {
    return {
      next: INITIAL_DAEMON_PROBE_STATE,
      refresh: state.unavailable,
    };
  }

  return {
    next: {
      unavailable: true,
    },
    refresh: false,
  };
}
