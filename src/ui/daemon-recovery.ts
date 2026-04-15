export function shouldRefreshAfterDaemonRecovery(wasUnavailable: boolean, daemonAlive: boolean): boolean {
  return wasUnavailable && daemonAlive;
}
