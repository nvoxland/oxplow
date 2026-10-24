/// A terminal's session (`.context/terminal.md` "The terminal viz and its
/// host"): the PTY its URI names — an `oxplow/terminal` shell or an
/// `oxplow/agent_session`'s harness — opened at the surface's size and
/// replayed into it, followed (output, exit), and opened again when it
/// must be: Start Again, the daemon coming back, the host moving it to
/// another conversation (`conversation` changing). What the person types
/// goes straight to the terminal's URI: the daemon keeps it in order and
/// holds it until the PTY is up.
/// An agent's (`startOnActivate`) spawns nothing until it is shown, but
/// attaches at once to one that runs. Unmounting detaches: the process
/// keeps running.
import { useCallback, useEffect, useRef, useState } from "react";
import type { Terminal } from "@xterm/xterm";

import { desktopBridge, onRemoteReconnect } from "../api.js";
import { logUi } from "../logger.js";
import {
  type ExitOutcome,
  failedLaunch,
  isStopped,
  mayStartNow,
  readSessionMessage,
  reopened,
  RESTARTED_NOTICE,
} from "../components/terminalSession.js";

/** A PTY session's output or exit, as `pty:event` carries it. */
interface TerminalEvent {
  sessionId: string;
  message: string;
}

export interface TerminalSessionOptions {
  /** An agent's: spawn only once shown (attach to a running one at once). */
  startOnActivate?: boolean;
  /** Its view is on screen. */
  shown: boolean;
  /** The person stopped it: open nothing until it is started. */
  stopped?: boolean;
  /** What it is bound to (an agent's conversation); a change opens it
   *  again once the running process has exited, or now if it had. */
  conversation?: string | null;
}

export interface TerminalSession {
  /** The surface opened: follow the session into `term`. Returns the stop. */
  attach(term: Terminal): () => void;
  /** What the person typed (the only input a session takes: oxplow never
   *  types at an agent), binary input, and a resize — in order. */
  input(data: string): void;
  binary(data: string): void;
  resize(cols: number, rows: number): void;
  /** Open it again (Start Again, Retry). */
  reopen(): void;
  /** The session's live working directory, for relative links. */
  cwd(): Promise<string | null>;
  /** Its process exited (or never started): why. */
  ended: { exitCode: number | null; outcome: ExitOutcome } | null;
  /** Stopped, by the row or as the open found out (another window). */
  stopped: boolean;
}

export function useTerminalSession(uri: string, opts: TerminalSessionOptions): TerminalSession {
  const sessionIdRef = useRef<string | null>(null);
  const reopenRef = useRef<(() => void) | null>(null);
  const activateRef = useRef<(() => void) | null>(null);
  const activatedRef = useRef(false);
  const endedRef = useRef(false);
  const launchFailedRef = useRef(false);
  const [ended, setEnded] = useState<{ exitCode: number | null; outcome: ExitOutcome } | null>(null);
  const [stoppedHere, setStoppedHere] = useState(false);
  const isStoppedNow = !!opts.stopped || stoppedHere;
  const live = useRef({ shown: opts.shown, stopped: isStoppedNow, startOnActivate: !!opts.startOnActivate });
  live.current = { shown: opts.shown, stopped: isStoppedNow, startOnActivate: !!opts.startOnActivate };

  useEffect(() => {
    if (!opts.stopped) setStoppedHere(false);
  }, [opts.stopped]);

  // The host moved it to another conversation: open that one once the
  // stopped process has exited, or now if it already had.
  const restartPendingRef = useRef(false);
  const conversationSeen = useRef(opts.conversation);
  useEffect(() => {
    if (opts.conversation === conversationSeen.current) return;
    conversationSeen.current = opts.conversation;
    if (endedRef.current) reopenRef.current?.();
    else restartPendingRef.current = true;
  }, [opts.conversation]);

  // Started again: open it now (or again, if its process had ended).
  useEffect(() => {
    if (isStoppedNow) return;
    if (!live.current.shown && live.current.startOnActivate) return;
    if (!activatedRef.current) activateRef.current?.();
    else if (endedRef.current) reopenRef.current?.();
  }, [isStoppedNow]);
  useEffect(() => {
    if (opts.shown && !live.current.stopped) activateRef.current?.();
  }, [opts.shown]);

  // Unmounted: detach, never kill (closing is the row's command).
  useEffect(
    () => () => {
      const sessionId = sessionIdRef.current;
      sessionIdRef.current = null;
      if (sessionId) void desktopBridge().closeTerminalSession(sessionId);
    },
    [],
  );

  const attach = useCallback(
    (term: Terminal) => {
      let disposed = false;
      // Output may arrive before the open answers (a replay sent from
      // inside it): held until the session id is known.
      const pendingEvents: TerminalEvent[] = [];
      const applyEvent = (event: TerminalEvent) => {
        const msg = readSessionMessage(event.message);
        if (msg?.kind === "data") term.write(msg.bytes);
        if (msg?.kind === "exit") {
          endedRef.current = true;
          if (restartPendingRef.current) {
            restartPendingRef.current = false;
            reopenRef.current?.();
            return;
          }
          setEnded({ exitCode: msg.exitCode, outcome: msg.outcome });
        }
      };
      const unsubscribe = desktopBridge().onTerminalEvent((event) => {
        if (sessionIdRef.current === null) {
          pendingEvents.push(event);
          return;
        }
        if (event.sessionId !== sessionIdRef.current) return;
        applyEvent(event);
      });

      let opening = false;
      const openSession = (again: boolean) => {
        if (opening || disposed) return;
        opening = true;
        logUi("info", "opening terminal session", { uri, cols: term.cols, rows: term.rows, again });
        void desktopBridge()
          .openTerminalSession(uri, term.cols, term.rows)
          .then(({ sessionId, replayB64 }) => {
            if (disposed) {
              void desktopBridge().closeTerminalSession(sessionId);
              return;
            }
            if (reopened(sessionIdRef.current, sessionId) === "same") return;
            if (again) {
              term.reset();
              term.write(RESTARTED_NOTICE);
            }
            sessionIdRef.current = sessionId;
            endedRef.current = false;
            launchFailedRef.current = false;
            setEnded(null);
            // The ring buffer: the screen as it was left.
            if (replayB64) {
              try {
                const bin = atob(replayB64);
                const bytes = new Uint8Array(bin.length);
                for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
                term.write(bytes);
              } catch {}
            }
            for (const event of pendingEvents) if (event.sessionId === sessionId) applyEvent(event);
            pendingEvents.length = 0;
            if (live.current.shown) term.focus();
            logUi("info", "terminal session opened", { uri, sessionId, again });
          })
          .catch((error) => {
            if (isStopped(error)) {
              setStoppedHere(true);
              return;
            }
            logUi("error", "terminal session open failed", { uri, error: String(error) });
            // No process: say why where "Session ended" shows, with Retry.
            endedRef.current = true;
            launchFailedRef.current = true;
            setEnded(failedLaunch(error));
          })
          .finally(() => {
            opening = false;
          });
      };
      activatedRef.current = false;
      activateRef.current = () => {
        if (activatedRef.current) return;
        activatedRef.current = true;
        openSession(false);
      };
      reopenRef.current = () => openSession(true);
      const { startOnActivate, shown, stopped } = live.current;
      if (mayStartNow(startOnActivate, shown, stopped)) {
        activateRef.current();
      } else {
        // Hidden and not started: attach to one that already runs (a
        // lookup spawns nothing); otherwise wait to be shown.
        void desktopBridge()
          .lookupTerminalSession(uri)
          .then((running) => {
            if (running && !disposed) activateRef.current?.();
          })
          .catch(() => {});
      }
      // The daemon came back: opening again keeps a live session and
      // replaces a dead one.
      const offReconnect = onRemoteReconnect(() => {
        if (!endedRef.current || launchFailedRef.current) openSession(true);
      });
      return () => {
        disposed = true;
        unsubscribe();
        offReconnect();
        reopenRef.current = null;
        activateRef.current = null;
      };
    },
    [uri],
  );

  const send = useCallback(
    (message: TerminalMessage) => {
      // An ended session takes no keys: they must not reach the next one.
      if (endedRef.current && message.type !== "resize") return;
      desktopBridge().terminalInput(uri, encode(message));
    },
    [uri],
  );

  const cwd = useCallback(async () => {
    const id = sessionIdRef.current;
    return id ? desktopBridge().terminalSessionCwd(id) : null;
  }, []);

  return {
    attach,
    input: (data) => send({ type: "input", data }),
    binary: (data) => send({ type: "input-binary", data }),
    resize: (cols, rows) => send({ type: "resize", cols, rows }),
    reopen: () => reopenRef.current?.(),
    cwd,
    ended,
    stopped: isStoppedNow,
  };
}

/** One message to a terminal; `data` is what xterm gave (text for
 *  `input`, a byte per character for `input-binary`). */
type TerminalMessage =
  | { type: "input"; data: string }
  | { type: "input-binary"; data: string }
  | { type: "resize"; cols: number; rows: number };

/** `message` as the PTY's protocol reads it (`oxplow_app::terminal_sessions`). */
function encode(message: TerminalMessage): string {
  switch (message.type) {
    case "input":
      return JSON.stringify({ type: "input", bytes: utf8ToBase64(message.data) });
    case "input-binary":
      return JSON.stringify({ type: "input-binary", bytes: binaryToBase64(message.data) });
    default:
      return JSON.stringify(message);
  }
}

function binaryToBase64(data: string) {
  let binary = "";
  for (let i = 0; i < data.length; i++) {
    binary += String.fromCharCode(data.charCodeAt(i) & 0xff);
  }
  return btoa(binary);
}

/** A JS string as UTF-8 bytes, then base64: `btoa()` alone refuses any
 *  character above U+00FF (a pasted smart quote or emoji). */
function utf8ToBase64(data: string) {
  const bytes = new TextEncoder().encode(data);
  let binary = "";
  // Chunked: `apply` with a large array exceeds the argument limit.
  const chunkSize = 0x8000;
  for (let i = 0; i < bytes.length; i += chunkSize) {
    binary += String.fromCharCode.apply(null, Array.from(bytes.subarray(i, i + chunkSize)));
  }
  return btoa(binary);
}
