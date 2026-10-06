import { vcsCheckoutBranch } from "../api.js";
import type { Stream } from "../tauri-bridge/index.js";
import { BackgroundTaskIndicator } from "./BackgroundTaskIndicator.js";
import { BranchPicker, type PickedRef } from "./BranchPicker.js";
import { AlertsIndicator } from "./Alerts/AlertsIndicator.js";
import type { TabRef } from "../tabs/tabState.js";

interface Props {
  stream: Stream | null;
  vcsEnabled: boolean;
  /** Opens the Alerts page from the bell. */
  onOpenPage(ref: TabRef): void;
}

/**
 * Right-aligned bottom-rail composite: background-task indicator (only
 * visible when something's running) plus the IntelliJ-style branch
 * picker chip. Clicking the branch chip opens the picker; clicking the
 * task indicator opens its own popover with the live task list.
 */
export function StatusBar({ stream, vcsEnabled, onOpenPage }: Props) {
  const canInteract = !!stream && vcsEnabled;
  const label = stream ? stream.branch : "—";
  const title = !vcsEnabled
    ? "Git not enabled for this workspace"
    : stream
      ? `Branch: ${stream.branch} (click to switch)`
      : "No stream selected";

  async function handlePick(target: PickedRef) {
    if (!stream) return;
    // Tags and remote refs are checked out via their local-name form; git will
    // create a tracking branch / detached HEAD as appropriate. The branch
    // reconciler records the new branch on the stream.
    await vcsCheckoutBranch(stream.id, target.name);
  }

  return (
    <div style={{ display: "inline-flex", alignItems: "center", gap: 8 }}>
      <AlertsIndicator onOpenPage={onOpenPage} />
      <BackgroundTaskIndicator />
      <BranchPicker
      label={
        <>
          <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" style={{ display: "block" }}>
            <line x1="6" y1="3" x2="6" y2="15" />
            <circle cx="18" cy="6" r="3" />
            <circle cx="6" cy="18" r="3" />
            <path d="M18 9a9 9 0 0 1-9 9" />
          </svg>
          <span style={{ marginLeft: 6 }}>{label}</span>
        </>
      }
      title={title}
      currentBranch={stream?.branch ?? null}
      disabled={!canInteract}
      anchor="top"
      align="right"
      mode="manage"
      streamId={stream?.id ?? null}
      onPick={handlePick}
      />
    </div>
  );
}
