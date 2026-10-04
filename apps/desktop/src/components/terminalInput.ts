/// What a terminal pane sends its session (tsk979, tsk992), one message
/// at a time: each is sent only once the one before it returned, so a
/// keystroke can't overtake an earlier one — as separate requests over the
/// daemon's HTTP transport they could. The daemon writes a message's bytes
/// before it replies.
///
/// - Keystrokes made while a send is in flight wait together and go as one
///   message, so typing fast costs no extra round trips — except a bare
///   Escape, which keeps its own message: merged with the next key it would
///   read as an Alt sequence (`\x1b\r` is Shift+Enter).
/// - Waiting scrolls sum, and a newer resize replaces a waiting one.
/// - A send that doesn't answer within the timeout is reported and that
///   session's waiting messages are dropped — never delivered later in a
///   burst the person didn't see land; the next session's still go.
/// - A closed sender (its pane is gone) drops what waits and sends nothing
///   more.

/// One message to a terminal session; `data` is what xterm gave (text for
/// `input`, a byte per character for `input-binary`), encoded at send.
export type TerminalMessage =
  | { type: "input"; data: string }
  | { type: "input-binary"; data: string }
  | { type: "resize"; cols: number; rows: number }
  | { type: "history-page"; direction: "up" | "down" }
  | { type: "history-scroll"; lines: number }
  | { type: "history-exit" };

type Queued = { sessionId: string; message: TerminalMessage };

export interface TerminalSender {
  /** Queue `message` for `sessionId`. */
  send(sessionId: string, message: TerminalMessage): void;
  /** Drop what waits and send nothing more: the pane is gone. */
  close(): void;
}

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
  let sending = false;
  let closed = false;

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
      const last = queue[queue.length - 1];
      const joined = last && last.sessionId === sessionId ? merged(last.message, message) : null;
      if (last && joined) last.message = joined;
      else queue.push({ sessionId, message });
      if (!sending) void drain();
    },
    close() {
      closed = true;
      queue = [];
    },
  };
}

/** `a` then `b` as one message, when they can be: keystrokes of one kind
 *  (not after a bare Escape), scrolls (summed), resizes (the newer). */
function merged(a: TerminalMessage, b: TerminalMessage): TerminalMessage | null {
  if (a.type === "input" && b.type === "input" && !a.data.endsWith("\x1b")) return { type: "input", data: a.data + b.data };
  if (a.type === "input-binary" && b.type === "input-binary") return { type: "input-binary", data: a.data + b.data };
  if (a.type === "history-scroll" && b.type === "history-scroll") return { type: "history-scroll", lines: a.lines + b.lines };
  if (a.type === "resize" && b.type === "resize") return b;
  return null;
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
