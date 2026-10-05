/// What a terminal pane sends its session (tsk979, tsk992), one message
/// at a time: each is sent only once the one before it returned, so a
/// keystroke can't overtake an earlier one — as separate requests over the
/// daemon's HTTP transport they could. The daemon writes a message's bytes
/// before it replies.
///
/// - Keystrokes made while a send is in flight wait together and go as one
///   message, so typing fast costs no extra round trips — except a bare
///   Escape, which keeps its own message: merged with the next key it would
///   read as an Alt sequence (`\x1b\r` is Shift+Enter); and an Enter, which
///   keeps its own too: merged with the text before it, a TUI reads the
///   chunk as a paste and doesn't submit (tsk1027).
/// - A newer resize replaces a waiting one.
/// - A send that doesn't answer within the timeout is reported and that
///   session's waiting messages are dropped — never delivered later in a
///   burst the person didn't see land; the next session's still go.
/// - Keystrokes typed while the session is still opening (no session id
///   yet) are held, up to `HELD_MAX` characters, and sent first once it
///   opens (`attach`), so a fast typist's first keys aren't lost (tsk993).
/// - A closed sender (its pane is gone) drops what waits and sends nothing
///   more.

/// One message to a terminal session; `data` is what xterm gave (text for
/// `input`, a byte per character for `input-binary`), encoded at send.
export type TerminalMessage =
  | { type: "input"; data: string }
  | { type: "input-binary"; data: string }
  | { type: "resize"; cols: number; rows: number };

type Queued = { sessionId: string; message: TerminalMessage };

export interface TerminalSender {
  /** Queue `message` for `sessionId`; with none yet (the session is
   *  opening), a keystroke is held for [`attach`] and anything else
   *  dropped. */
  send(sessionId: string | null, message: TerminalMessage): void;
  /** The session opened: send the keystrokes held for it, first. */
  attach(sessionId: string): void;
  /** Drop what waits and send nothing more: the pane is gone. */
  close(): void;
}

/** The most characters held for a session that hasn't opened yet. */
export const HELD_MAX = 4096;

/** How long a send may go unanswered before its session's backlog is
 *  dropped. */
export const SEND_TIMEOUT_MS = 5000;

/** A sender over `send` (`forwardTerminalInput`); a failed or unanswered
 *  send goes to `onError`. */
export function terminalSender(
  send: (sessionId: string, message: string) => Promise<void>,
  onError: (error: unknown) => void,
  { timeoutMs = SEND_TIMEOUT_MS }: { timeoutMs?: number } = {},
): TerminalSender {
  let queue: Queued[] = [];
  let held: TerminalMessage[] = [];
  let sending = false;
  let closed = false;

  const enqueue = (sessionId: string, message: TerminalMessage) => {
    const last = queue[queue.length - 1];
    const joined = last && last.sessionId === sessionId ? merged(last.message, message) : null;
    if (last && joined) last.message = joined;
    else queue.push({ sessionId, message });
  };

  async function drain() {
    sending = true;
    while (queue.length > 0 && !closed) {
      const next = queue.shift()!;
      let timer: ReturnType<typeof setTimeout> | undefined;
      const late = new Promise<"late">((resolve) => {
        timer = setTimeout(() => resolve("late"), timeoutMs);
      });
      try {
        const outcome = await Promise.race([send(next.sessionId, encode(next.message)).then(() => "sent" as const), late]);
        if (outcome === "late") {
          const dropped = queue.filter((q) => q.sessionId === next.sessionId).length;
          queue = queue.filter((q) => q.sessionId !== next.sessionId);
          onError(
            new Error(
              `the terminal didn't answer within ${timeoutMs / 1000} s` +
                (dropped > 0 ? `; ${dropped} waiting message${dropped === 1 ? " was" : "s were"} dropped` : ""),
            ),
          );
        }
      } catch (e) {
        onError(e);
      } finally {
        clearTimeout(timer);
      }
    }
    sending = false;
  }

  return {
    send(sessionId, message) {
      if (closed) return;
      if (sessionId === null) {
        const keystroke = message.type === "input" || message.type === "input-binary";
        const size = held.reduce((n, m) => n + ("data" in m ? m.data.length : 0), 0);
        if (keystroke && size + message.data.length <= HELD_MAX) held.push(message);
        return;
      }
      enqueue(sessionId, message);
      if (!sending) void drain();
    },
    attach(sessionId) {
      if (closed) return;
      const keys = held;
      held = [];
      // Ahead of anything already queued for it: they were typed first.
      const after = queue;
      queue = [];
      for (const m of keys) enqueue(sessionId, m);
      for (const q of after) enqueue(q.sessionId, q.message);
      if (queue.length > 0 && !sending) void drain();
    },
    close() {
      closed = true;
      queue = [];
      held = [];
    },
  };
}

/** `a` then `b` as one message, when they can be: keystrokes of one kind
 *  (not after a bare Escape, and never an Enter), resizes (the newer). */
function merged(a: TerminalMessage, b: TerminalMessage): TerminalMessage | null {
  if (a.type === "input" && b.type === "input" && !a.data.endsWith("\x1b") && !hasEnter(a.data) && !hasEnter(b.data)) {
    return { type: "input", data: a.data + b.data };
  }
  if (a.type === "input-binary" && b.type === "input-binary") return { type: "input-binary", data: a.data + b.data };
  if (a.type === "resize" && b.type === "resize") return b;
  return null;
}

/** An Enter (CR) in typed input. Merged into the text before it, a TUI
 *  such as Claude Code reads the chunk as a paste and the Enter as a newline:
 *  the prompt isn't sent (tsk1027). */
function hasEnter(data: string): boolean {
  return data.includes("\r");
}

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

/**
 * Encode a JS string as UTF-8 bytes, then base64. `btoa()` directly
 * rejects strings containing any character > U+00FF — pasting log
 * output with smart quotes / em-dashes / emoji used to throw
 * InvalidCharacterError and silently drop the paste. Going through
 * TextEncoder gets us proper UTF-8 round-tripping for the PTY.
 */
function utf8ToBase64(data: string) {
  const bytes = new TextEncoder().encode(data);
  let binary = "";
  // String.fromCharCode is fine for one byte at a time; chunked to
  // avoid the apply-with-large-array argument-limit pitfall.
  const chunkSize = 0x8000;
  for (let i = 0; i < bytes.length; i += chunkSize) {
    binary += String.fromCharCode.apply(null, Array.from(bytes.subarray(i, i + chunkSize)));
  }
  return btoa(binary);
}
