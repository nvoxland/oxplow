/// What a terminal pane sends its session (tsk979), one message at a time:
/// each is sent only once the one before it returned, so a keystroke can't
/// overtake an earlier one — as separate requests over the daemon's HTTP
/// transport they could. The daemon writes a message's bytes before it
/// replies. Keystrokes made while a send is in flight wait together and go
/// as one message, so typing fast costs no extra round trips.

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

/** A sender over `send` (`forwardTerminalInput`); a failed send goes to
 *  `onError` and the next one still goes. */
export function terminalSender(
  send: (sessionId: string, message: string) => Promise<void>,
  onError: (error: unknown) => void,
): (sessionId: string, message: TerminalMessage) => void {
  const queue: Queued[] = [];
  let sending = false;

  async function drain() {
    sending = true;
    while (queue.length > 0) {
      const next = queue.shift()!;
      try {
        await send(next.sessionId, encode(next.message));
      } catch (e) {
        onError(e);
      }
    }
    sending = false;
  }

  return (sessionId, message) => {
    const last = queue[queue.length - 1];
    const joined = last && last.sessionId === sessionId ? merged(last.message, message) : null;
    if (last && joined) last.message = joined;
    else queue.push({ sessionId, message });
    if (!sending) void drain();
  };
}

/** `a` then `b` as one message, when both are keystrokes of one kind. */
function merged(a: TerminalMessage, b: TerminalMessage): TerminalMessage | null {
  if (a.type === "input" && b.type === "input") return { type: "input", data: a.data + b.data };
  if (a.type === "input-binary" && b.type === "input-binary") return { type: "input-binary", data: a.data + b.data };
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
