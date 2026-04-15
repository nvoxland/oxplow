export function shouldHandleTerminalPageKey(event: {
  key: string;
  altKey: boolean;
  ctrlKey: boolean;
  metaKey: boolean;
  shiftKey: boolean;
}): boolean {
  if (event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) {
    return false;
  }
  return event.key === "PageUp" || event.key === "PageDown";
}

export function shouldReturnTerminalToPrompt(event: {
  key: string;
  altKey: boolean;
  ctrlKey: boolean;
  metaKey: boolean;
}): boolean {
  if (event.altKey || event.ctrlKey || event.metaKey) {
    return false;
  }

  if (event.key === "Escape" || event.key === "Enter" || event.key === "Backspace") {
    return true;
  }

  return event.key.length === 1;
}

export function wheelDeltaToScrollLines(event: {
  deltaY: number;
  deltaMode: number;
}): number {
  if (event.deltaY === 0) return 0;

  if (event.deltaMode === 1) {
    return clampSigned(event.deltaY);
  }

  if (event.deltaMode === 2) {
    return clampSigned(event.deltaY * 12);
  }

  return clampSigned(event.deltaY / 8);
}

export function shouldRouteWheelToTmuxHistory(state: {
  mode: "live" | "history";
  bufferType: "normal" | "alternate";
  mouseTrackingMode: "none" | "x10" | "vt200" | "drag" | "any";
}): boolean {
  if (state.mode === "history") {
    return true;
  }

  return state.bufferType === "alternate" && state.mouseTrackingMode === "none";
}

function clampSigned(value: number): number {
  const rounded = Math.trunc(value);
  if (rounded !== 0) return rounded;
  return value < 0 ? -1 : 1;
}
