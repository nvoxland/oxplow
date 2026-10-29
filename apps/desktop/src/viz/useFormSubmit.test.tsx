import { afterAll, afterEach, expect, mock, test } from "bun:test";
import { act, cleanup, renderHook } from "@testing-library/react";

import type { CommandOutcome } from "../tauri-bridge/generated/bindings.js";

// A form's submit toasts when it ran; one whose command came back undoable
// offers Undo, as a pressed action does.

const realApi = { ...(await import("../api.js")) };
const realToasts = { ...(await import("../components/toastStore.js")) };
afterAll(() => {
  mock.module("../api.js", () => realApi);
  mock.module("../components/toastStore.js", () => realToasts);
});
const toasts: { message: string; onUndo?: () => void }[] = [];
const undone: number[] = [];
mock.module("../components/toastStore.js", () => ({ ...realToasts, showToast: (t: { message: string; onUndo?: () => void }) => toasts.push(t) }));
mock.module("../api.js", () => ({ ...realApi, undoCommand: async (id: number) => void undone.push(id) }));
const { useFormSubmit } = await import("./useFormSubmit.js");

afterEach(() => {
  cleanup();
  toasts.length = 0;
  undone.length = 0;
});

const outcome = (inverse: boolean): CommandOutcome => ({ result: null, audit_id: 7, event_id: null, inverse: inverse ? {} : null }) as unknown as CommandOutcome;

test("an undoable run offers Undo, which undoes it", async () => {
  const { result } = renderHook(() => useFormSubmit("Rename", async () => outcome(true), () => {}));
  await act(() => result.current.submit({}, false));
  expect(toasts[0]?.message).toBe("Rename: done.");
  await act(async () => toasts[0]!.onUndo!());
  expect(undone).toEqual([7]);
});

test("a run that can't be undone just says it's done", async () => {
  const { result } = renderHook(() => useFormSubmit("Fork Conversation", async () => outcome(false), () => {}));
  await act(() => result.current.submit({}, false));
  expect(toasts[0]?.onUndo).toBeUndefined();
});
