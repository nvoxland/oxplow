import { checkoutStreamBranch, type Stream } from "../api.js";
import { BranchPicker, type PickedRef } from "./BranchPicker.js";

interface Props {
  stream: Stream | null;
  gitEnabled: boolean;
}

/**
 * Branch chip rendered inside the bottom dock rail (right-aligned). Opens an
 * IntelliJ-style branch picker popover on click and switches the current
 * stream's branch when an entry is selected.
 */
export function StatusBar({ stream, gitEnabled }: Props) {
  const canInteract = !!stream && gitEnabled;
  const label = stream ? stream.branch : "—";
  const title = !gitEnabled
    ? "Git not enabled for this workspace"
    : stream
      ? `Branch: ${stream.branch} (click to switch)`
      : "No stream selected";

  async function handlePick(target: PickedRef) {
    if (!stream) return;
    // Tags and remote refs are checked out via their local-name form; git will
    // create a tracking branch / detached HEAD as appropriate.
    await checkoutStreamBranch(stream.id, target.name);
  }

  return (
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
  );
}
