/**
 * The slice of `ws.WebSocket` that the runtime's IPC-driven sockets and the
 * pty/LSP bridges actually use. Defining this lets `RuntimeSocket` (a thin
 * adapter over an Electron IPC channel) and the real `ws.WebSocket` both be
 * accepted by the `attachPane` / `attachClient` / `agentPty.attach`
 * helpers without `as any` casts.
 *
 * Keep this interface minimal; only add a method when a bridge actually
 * needs it.
 */
export interface BridgeSocket {
  readonly OPEN: number;
  readyState: number;
  send(message: string): void;
  close(): void;
  on(event: string, listener: (...args: any[]) => void): unknown;
  off?(event: string, listener: (...args: any[]) => void): unknown;
  removeListener?(event: string, listener: (...args: any[]) => void): unknown;
}
