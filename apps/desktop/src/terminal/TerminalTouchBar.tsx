/// The keys an on-screen keyboard lacks, under a terminal on a touch screen
/// (`TOUCH_KEYS`: Esc, Shift+Tab, Tab, a Ctrl that applies to the next key
/// typed, the arrows), and Compose: a real text field (the keyboard's
/// dictation and long-press paste work there) whose Insert puts the text in
/// the terminal as a paste. Nothing here presses Enter: what the person
/// composes waits in the agent's input until they send it.
import { useState, type CSSProperties, type SyntheticEvent } from "react";

import { TOUCH_KEYS, keyBytes } from "./touchKeys.js";
import { touchBarStyle, touchKeyStyle, touchRowStyle } from "../touchBarStyles.js";

export interface TerminalTouchBarProps {
  /** Write `bytes` to the terminal, as typed. */
  send(bytes: string): void;
  /** The program's cursor-key mode now (asked at each press). */
  applicationCursor(): boolean;
  /** Whether Ctrl is on: the next key typed becomes its control character. */
  ctrl: boolean;
  onCtrl(on: boolean): void;
  /** The person pressed Esc (an agent records the interrupt). */
  onEscape?(): void;
  /** Put `text` in the terminal as a paste. */
  onInsert(text: string): void;
}

/** A press that keeps the focus where it is: the terminal keeps it, so the
 *  on-screen keyboard stays open. */
const keepFocus = (event: SyntheticEvent) => event.preventDefault();

export function TerminalTouchBar({ send, applicationCursor, ctrl, onCtrl, onEscape, onInsert }: TerminalTouchBarProps) {
  const [composing, setComposing] = useState(false);
  const [text, setText] = useState("");
  const close = () => {
    setComposing(false);
    setText("");
  };
  const insert = () => {
    if (!text) return;
    onInsert(text);
    close();
  };
  return (
    <div data-testid="terminal-touch-bar" style={touchBarStyle}>
      {composing ? (
        <div style={composeStyle}>
          <textarea
            data-testid="terminal-compose-text"
            aria-label="Text to put in the terminal"
            autoFocus
            rows={3}
            value={text}
            onChange={(e) => setText(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Escape") {
                e.preventDefault();
                close();
              } else if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
                e.preventDefault();
                insert();
              }
            }}
            style={fieldStyle}
          />
          <div style={touchRowStyle}>
            <button type="button" data-testid="terminal-compose-insert" disabled={!text} onClick={insert} style={touchKeyStyle(false)}>
              Insert
            </button>
            <button type="button" data-testid="terminal-compose-cancel" onClick={close} style={touchKeyStyle(false)}>
              Cancel
            </button>
          </div>
        </div>
      ) : null}
      <div style={touchRowStyle}>
        {TOUCH_KEYS.map((key) => (
          <button
            key={key.id}
            type="button"
            data-testid={`terminal-key-${key.id}`}
            aria-label={key.name}
            aria-pressed={key.id === "ctrl" ? ctrl : undefined}
            onPointerDown={keepFocus}
            onMouseDown={keepFocus}
            onClick={() => {
              if (key.id === "ctrl") {
                onCtrl(!ctrl);
                return;
              }
              send(keyBytes(key.id, applicationCursor()));
              if (key.id === "esc") onEscape?.();
            }}
            style={touchKeyStyle(key.id === "ctrl" && ctrl)}
          >
            {key.label}
          </button>
        ))}
        <button
          type="button"
          data-testid="terminal-compose"
          aria-expanded={composing}
          onClick={() => (composing ? close() : setComposing(true))}
          style={{ ...touchKeyStyle(composing), marginLeft: "auto" }}
        >
          Compose
        </button>
      </div>
    </div>
  );
}

const composeStyle: CSSProperties = { display: "flex", flexDirection: "column", gap: "var(--space-2)" };
const fieldStyle: CSSProperties = {
  width: "100%",
  boxSizing: "border-box",
  font: "inherit",
  fontSize: "var(--text-md)",
  padding: "var(--space-2)",
  background: "var(--surface-card)",
  color: "var(--text-primary)",
  border: "1px solid var(--border-strong)",
  borderRadius: "var(--radius-md)",
  resize: "vertical",
};
