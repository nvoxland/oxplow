/// A button whose command decides whether it asks first (tsk898): the
/// command's spec (`get_command`'s `confirm`) is the one rule. When it
/// asks — `always` or `destructive`, and until the spec has loaded — the
/// first click arms an `InlineConfirm` and the second runs; when it is
/// `never`, a click runs. The caller renders its own button from `run`.

import type { ReactNode } from "react";
import { useEffect, useState } from "react";

import { getCommand } from "../api.js";
import { InlineConfirm } from "./InlineConfirm.js";

/** Whether `command` asks for confirmation; `true` until its spec loads. */
export function useCommandAsks(command: string): boolean {
  const [asks, setAsks] = useState(true);
  useEffect(() => {
    let live = true;
    getCommand(command)
      .then((spec) => {
        if (live) setAsks(spec.confirm !== "never");
      })
      .catch(() => {
        // Unknown: keep asking.
      });
    return () => {
      live = false;
    };
  }, [command]);
  return asks;
}

export function SpecConfirm({
  command,
  onConfirm,
  confirmLabel,
  testIdPrefix,
  children,
}: {
  /** The command the button runs (`oxplow.vcs.push`). */
  command: string;
  onConfirm(): void;
  confirmLabel: string;
  testIdPrefix?: string;
  /** The button, given what its click does. */
  children(run: () => void): ReactNode;
}) {
  const asks = useCommandAsks(command);
  if (!asks) return <>{children(onConfirm)}</>;
  return (
    <InlineConfirm onConfirm={onConfirm} confirmLabel={confirmLabel} testIdPrefix={testIdPrefix}>
      {children}
    </InlineConfirm>
  );
}
